use super::*;

fn headers_with_auth() -> UpstreamWsHeaders {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer client-token"),
    );
    headers.insert(
        axum::http::header::COOKIE,
        HeaderValue::from_static("session=abc"),
    );
    headers.insert(
        axum::http::header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("warp-proto"),
    );
    headers.insert(
        axum::http::header::USER_AGENT,
        HeaderValue::from_static("warp-client"),
    );
    UpstreamWsHeaders::from_request_headers(&headers)
}

#[test]
fn apply_upstream_headers_preserves_client_auth_when_no_override() {
    let forwarded = headers_with_auth();
    let mut headers = HeaderMap::new();

    apply_upstream_headers(&mut headers, "", &forwarded).unwrap();

    assert_eq!(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer client-token")
    );
    assert_eq!(
        headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok()),
        Some("session=abc")
    );
    assert_eq!(
        headers
            .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok()),
        Some("warp-proto")
    );
}

#[test]
fn apply_upstream_headers_prefers_override_token() {
    let forwarded = headers_with_auth();
    let mut headers = HeaderMap::new();

    apply_upstream_headers(&mut headers, "oz-secret", &forwarded).unwrap();

    assert_eq!(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer oz-secret")
    );
    assert_eq!(
        headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok()),
        Some("session=abc")
    );
}
