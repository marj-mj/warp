use axum::{
    extract::{Path, Query, Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::{sse::Event, IntoResponse, Response, Sse},
    Extension, Json,
};
use futures_util::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::gateway::GatewayEngine;
use crate::http::auth::{AuthError, Authenticator, Identity};
use crate::http::sse::SSEEvent;
use crate::http::types::{
    CancelTaskRequest, CancelTaskResponse, SpawnAgentRequest, SpawnAgentResponse,
    TaskStatusResponse,
};
use crate::utils::TaskId;

/// Header carrying the agent identity uid as an auth fallback.
const IDENTITY_UID_HEADER: &str = "x-agent-identity-uid";
/// Standard EventSource reconnection header.
const LAST_EVENT_ID_HEADER: &str = "last-event-id";

#[derive(Clone)]
pub struct AppState {
    pub gateway: Arc<GatewayEngine>,
    pub authenticator: Arc<Authenticator>,
}

/// Axum middleware that authenticates `/agent/*` requests and stores the
/// resolved [`Identity`] in the request extensions for downstream handlers.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let authorization = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let identity_uid = request
        .headers()
        .get(IDENTITY_UID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let identity = state
        .authenticator
        .authenticate(authorization.as_deref(), identity_uid.as_deref())
        .map_err(|err| match err {
            AuthError::MissingCredentials => {
                AppError::Unauthorized("missing or empty Authorization credentials".to_string())
            }
            AuthError::InvalidCredentials => {
                AppError::Unauthorized("invalid credentials".to_string())
            }
        })?;

    request.extensions_mut().insert(Arc::new(identity));
    Ok(next.run(request).await)
}

pub async fn spawn_agent_handler(
    State(state): State<AppState>,
    Extension(identity): Extension<Arc<Identity>>,
    Json(request): Json<SpawnAgentRequest>,
) -> Result<Json<SpawnAgentResponse>, AppError> {
    let outcome = state
        .gateway
        .spawn_agent_with_identity(request, (*identity).clone())
        .await;

    match outcome {
        crate::gateway::engine::SpawnOutcome::Spawned { task_id, run_id } => {
            Ok(Json(SpawnAgentResponse {
                task_id: task_id.to_string(),
                run_id,
                at_capacity: false,
            }))
        }
        crate::gateway::engine::SpawnOutcome::AtCapacity => Ok(Json(SpawnAgentResponse {
            task_id: String::new(),
            run_id: String::new(),
            at_capacity: true,
        })),
    }
}

pub async fn list_tasks_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let records = state.gateway.list_executions().await;
    let running = state.gateway.running_count().await;
    Ok(Json(serde_json::json!({
        "running": running,
        "max_concurrent": state.gateway.config().max_concurrent_tasks,
        "at_capacity": state.gateway.at_capacity().await,
        "tasks": records,
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct StreamQuery {
    /// Optional last seen event id, used to replay missed events on reconnect
    /// (mirrors the EventSource Last-Event-ID header).
    last_event_id: Option<u64>,
}

pub async fn stream_handler(
    State(state): State<AppState>,
    Path(task_id_str): Path<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let task_id = TaskId::from_string(task_id_str).map_err(AppError::BadRequest)?;

    // Resolve the last seen event id from the header (preferred) or query param.
    let last_event_id = headers
        .get(LAST_EVENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or(query.last_event_id);

    let stream_manager = state.gateway.stream_manager();
    let receiver = stream_manager
        .subscribe(&task_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Task not found: {}", task_id)))?;

    // Replay any events the client missed since last_event_id.
    let replay: Vec<Result<Event, Infallible>> = match last_event_id {
        Some(last_id) => stream_manager
            .replay_since(&task_id, last_id)
            .await
            .into_iter()
            .map(|envelope| {
                Ok(Event::default()
                    .id(envelope.id.to_string())
                    .event(envelope.event.event_name())
                    .data(envelope.event.to_json()))
            })
            .collect(),
        None => Vec::new(),
    };

    let live = BroadcastStream::new(receiver).filter_map(|result| match result {
        Ok(envelope) => Some(Ok(Event::default()
            .id(envelope.id.to_string())
            .event(envelope.event.event_name())
            .data(envelope.event.to_json()))),
        Err(_) => None,
    });

    let stream = tokio_stream::StreamExt::chain(tokio_stream::iter(replay), live);

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

pub async fn cancel_task_handler(
    State(state): State<AppState>,
    Json(request): Json<CancelTaskRequest>,
) -> Result<Json<CancelTaskResponse>, AppError> {
    let task_id = TaskId::from_string(request.task_id).map_err(AppError::BadRequest)?;

    let task_manager = state.gateway.task_manager();
    let success = task_manager.cancel_task(&task_id).await;

    if success {
        let stream_manager = state.gateway.stream_manager();
        let _ = stream_manager
            .send_event(
                &task_id,
                SSEEvent::Cancelled {
                    task_id: task_id.to_string(),
                    reason: None,
                },
            )
            .await;
    }

    Ok(Json(CancelTaskResponse {
        success,
        message: Some(if success {
            "Task cancelled successfully".to_string()
        } else {
            "Task not found or already completed".to_string()
        }),
    }))
}

pub async fn get_task_status_handler(
    State(state): State<AppState>,
    Path(task_id_str): Path<String>,
) -> Result<Json<TaskStatusResponse>, AppError> {
    let task_id = TaskId::from_string(task_id_str).map_err(AppError::BadRequest)?;

    let record = state
        .gateway
        .get_execution_record(&task_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Task not found: {}", task_id)))?;

    Ok(Json(TaskStatusResponse {
        task_id: task_id.to_string(),
        status: record.state.as_task_status(),
        progress: record.progress,
        result: record.result,
        error: record.error,
    }))
}

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = Json(serde_json::json!({ "error": message }));
        (status, body).into_response()
    }
}
