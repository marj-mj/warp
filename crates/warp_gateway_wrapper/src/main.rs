use clap::{Parser, Subcommand};
use std::sync::Arc;
use warp_gateway_wrapper::http::auth::{AuthConfig, TokenEntry};
use warp_gateway_wrapper::mpg::{Adapter, GatewayConfig as MpgGatewayConfig, MpgServer, ProviderConfig, WireApi};
use warp_gateway_wrapper::mpg::config::ProvidersFile;
use warp_gateway_wrapper::proxy::{ProxyConfig, ProxyServer, WarpChannel};
use warp_gateway_wrapper::tools::builtin::{
    EchoTool, FilesystemTool, LongRunningTool, NetworkTool, ShellTool, SystemInfoTool,
};
use warp_gateway_wrapper::{GatewayEngine, HttpAdapter, Server, ServerConfig, StdioAdapter, ToolRegistry};

#[derive(Parser)]
#[command(name = "warp-gateway-wrapper")]
#[command(about = "Gateway wrapper for Warp MCP integration", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run in stdio mode (default for MCP)
    Stdio,
    /// Run legacy HTTP adapter (REST /api/execute + WebSocket /ws)
    Http {
        /// Address to bind the HTTP server
        #[arg(short, long, default_value = "127.0.0.1:3000")]
        addr: String,
    },
    /// Run the OZ-compatible agent server (/agent/run, SSE stream) with auth
    Serve {
        /// Host to bind
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to bind
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        /// Disable authentication (local development only)
        #[arg(long)]
        no_auth: bool,
    },
    /// Run the Managed Provider Gateway: a local OpenAI-compatible server
    /// that Warp Agent can be pointed at via a custom endpoint.
    Mpg {
        /// Host to bind
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to bind
        #[arg(short, long, default_value_t = 8787)]
        port: u16,
        /// Path to a providers JSON file (same shape as script/provider_gateway.providers.json).
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Provider name to use from the JSON file.
        #[arg(long)]
        provider: Option<String>,
        /// Override the upstream base URL (e.g. https://api.example.com/v1).
        #[arg(long)]
        upstream_base_url: Option<String>,
        /// Override the upstream API key (env name).
        #[arg(long)]
        upstream_env_key: Option<String>,
        /// Override the upstream wire API.
        #[arg(long, value_parser = ["chat", "responses"])]
        upstream_wire_api: Option<String>,
        /// Override the adapter.
        #[arg(long, value_parser = ["openai_chat", "openai_responses", "bridge_openai", "anthropic_messages", "gemini_generate_content"])]
        upstream_adapter: Option<String>,
        /// Default model id to advertise on /v1/models.
        #[arg(long)]
        upstream_model: Option<String>,
        /// Strip tools/tool_choice/parallel_tool_calls before forwarding.
        #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
        disable_tools: bool,
        /// Disable forwarding MCP context (informational; gateway has no MCP itself).
        #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
        disable_mcp: bool,
        /// Sliding window for the duplicate-request guard (seconds; 0 disables).
        #[arg(long, default_value_t = 60u64)]
        duplicate_window_secs: u64,
        /// Optional model_config_key the gateway should report as forced.
        #[arg(long)]
        force_model_config_key: Option<String>,
    },
    /// Probe an upstream provider's capabilities (chat/responses, streaming,
    /// tools) and print a recommended adapter + safe-mode defaults.
    MpgProbe {
        /// Upstream base URL (e.g. https://api.example.com/v1).
        #[arg(long)]
        base_url: String,
        /// Environment variable name holding the API key.
        #[arg(long)]
        env_key: Option<String>,
    },
    /// Run the transparent Warp proxy: forward client requests to a configured
    /// upstream Warp/OZ backend, attaching the configured Bearer token.
    Proxy {
        /// Host to bind
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to bind
        #[arg(short, long, default_value_t = 8788)]
        port: u16,
        /// Upstream Warp channel preset (production / staging / dev).
        #[arg(long, default_value = "production")]
        channel: String,
        /// Override the upstream HTTP root URL (overrides --channel).
        #[arg(long)]
        upstream_http: Option<String>,
        /// Override the upstream WebSocket RTC URL.
        #[arg(long)]
        upstream_ws_rtc: Option<String>,
        /// Override the upstream WebSocket sessions URL.
        #[arg(long)]
        upstream_ws_sessions: Option<String>,
    },
}

fn build_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    registry.register(Arc::new(LongRunningTool));
    registry.register(Arc::new(ShellTool));
    registry.register(Arc::new(FilesystemTool::default()));
    registry.register(Arc::new(NetworkTool::default()));
    registry.register(Arc::new(SystemInfoTool));
    registry
}

/// Parse auth tokens from the `WARP_GATEWAY_TOKENS` environment variable.
///
/// Format: comma-separated `uid:token[:privileged]` entries, e.g.
/// `alice:tok_a:privileged,bob:tok_b`. A non-privileged identity only gets
/// basic tools unless permissions are configured via file-based config later.
fn auth_config_from_env() -> AuthConfig {
    let raw = match std::env::var("WARP_GATEWAY_TOKENS") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return AuthConfig::default(),
    };

    let mut tokens = Vec::new();
    for entry in raw.split(',') {
        let parts: Vec<&str> = entry.split(':').collect();
        if parts.len() < 2 {
            tracing::warn!(entry = %entry, "ignoring malformed token entry");
            continue;
        }
        let privileged = parts.get(2).map(|p| *p == "privileged").unwrap_or(false);
        tokens.push(TokenEntry {
            token: parts[1].trim().to_string(),
            uid: parts[0].trim().to_string(),
            privileged,
            permissions: Vec::new(),
        });
    }

    AuthConfig {
        enabled: true,
        tokens,
    }
}

/// Initialize the tracing subscriber. Verbosity is controlled by the
/// RUST_LOG env var (defaults to info).
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    // Ignore the error if a global subscriber is already set (e.g. in tests).
    let _ = fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Commands::Stdio => {
            let engine = GatewayEngine::new(build_registry());
            let adapter = StdioAdapter::new(engine);
            tracing::info!("gateway started in stdio mode; listening on stdin");
            adapter.run().await?;
        }
        Commands::Http { addr } => {
            let engine = GatewayEngine::new(build_registry());
            let socket_addr: std::net::SocketAddr = addr.parse()?;
            let adapter = HttpAdapter::new(engine);
            adapter.serve(socket_addr).await?;
        }
        Commands::Serve { host, port, no_auth } => {
            let engine = Arc::new(GatewayEngine::new(build_registry()));
            let auth_config = if no_auth {
                tracing::warn!("starting agent server with authentication DISABLED");
                AuthConfig::disabled()
            } else {
                let config = auth_config_from_env();
                if config.enabled && config.tokens.is_empty() {
                    tracing::warn!(
                        "auth is enabled but no tokens configured; all requests will be rejected. \
                         Set WARP_GATEWAY_TOKENS or pass --no-auth for local dev."
                    );
                }
                config
            };

            // Start the background reaper that cleans up expired execution records.
            engine.start_cleanup_reaper();

            let server = Server::with_auth(ServerConfig { host, port }, engine, auth_config);
            server.run().await?;
        }
        Commands::Mpg {
            host,
            port,
            config,
            provider,
            upstream_base_url,
            upstream_env_key,
            upstream_wire_api,
            upstream_adapter,
            upstream_model,
            disable_tools,
            disable_mcp,
            duplicate_window_secs,
            force_model_config_key,
        } => {
            // Pick the base provider config: from JSON file if given, else build a
            // skeleton from CLI overrides.
            let mut provider_cfg = if let Some(path) = config {
                let name = provider.clone().ok_or_else(|| {
                    String::from("--provider <name> required when --config is set")
                })?;
                ProvidersFile::pick(&path, &name).map_err(|err| Box::<dyn std::error::Error>::from(err))?
            } else {
                ProviderConfig {
                    name: provider.clone().unwrap_or_else(|| "upstream".to_string()),
                    base_url: upstream_base_url.clone().unwrap_or_default(),
                    model: upstream_model.clone(),
                    wire_api: WireApi::Chat,
                    adapter: Adapter::OpenaiChat,
                    api_key: None,
                    env_key: upstream_env_key.clone(),
                }
            };

            // Apply CLI overrides on top of JSON defaults.
            if let Some(value) = upstream_base_url { provider_cfg.base_url = value; }
            if upstream_env_key.is_some() { provider_cfg.env_key = upstream_env_key; }
            if let Some(value) = upstream_model { provider_cfg.model = Some(value); }
            if let Some(value) = upstream_wire_api {
                provider_cfg.wire_api = if value == "responses" { WireApi::Responses } else { WireApi::Chat };
            }
            if let Some(value) = upstream_adapter {
                provider_cfg.adapter = match value.as_str() {
                    "openai_responses" => Adapter::OpenaiResponses,
                    "bridge_openai" => Adapter::BridgeOpenai,
                    "anthropic_messages" => Adapter::AnthropicMessages,
                    "gemini_generate_content" => Adapter::GeminiGenerateContent,
                    _ => Adapter::OpenaiChat,
                };
            }

            if provider_cfg.base_url.trim().is_empty() {
                return Err("upstream base URL is required (set via --upstream-base-url, --config, or WARP_MANAGED_PROVIDER_UPSTREAM_BASE_URL)".into());
            }

            let mut gateway_config = MpgGatewayConfig::new(provider_cfg);
            gateway_config.disable_tools = disable_tools;
            gateway_config.disable_mcp = disable_mcp;
            gateway_config.duplicate_window_secs = duplicate_window_secs;
            gateway_config.force_model_config_key = force_model_config_key;

            // Env overrides win last so production deployments can pin values.
            gateway_config.apply_env_overrides();

            let server = MpgServer::new(&host, port, gateway_config)?;
            server.run().await?;
        }
        Commands::MpgProbe { base_url, env_key } => {
            let api_key = env_key.and_then(|name| std::env::var(name).ok());
            let matrix =
                warp_gateway_wrapper::mpg::probe::probe_provider(&base_url, api_key.as_deref()).await;
            let output = serde_json::to_string_pretty(&matrix.to_json())
                .unwrap_or_else(|_| "{}".to_string());
            println!("{output}");
        }
        Commands::Proxy {
            host,
            port,
            channel,
            upstream_http,
            upstream_ws_rtc,
            upstream_ws_sessions,
        } => {
            let channel = match channel.to_ascii_lowercase().as_str() {
                "production" | "prod" => WarpChannel::Production,
                "staging" => WarpChannel::Staging,
                "dev" => WarpChannel::Dev,
                other => {
                    return Err(format!("unknown channel '{other}'").into());
                }
            };
            let mut config = ProxyConfig::for_channel(channel);
            if let Some(value) = upstream_http {
                config.upstream_http = value;
            }
            if let Some(value) = upstream_ws_rtc {
                config.upstream_ws_rtc = Some(value);
            }
            if let Some(value) = upstream_ws_sessions {
                config.upstream_ws_sessions = Some(value);
            }
            config.oz_token = ProxyConfig::token_from_env();
            if config.oz_token.is_empty() {
                tracing::warn!(
                    "no OZ token configured; the client's Authorization header will be passed \
                     through unchanged. Set WARP_GATEWAY_OZ_TOKEN to override."
                );
            }

            let server = ProxyServer::new(&host, port, config)?;
            server.run().await?;
        }
    }

    Ok(())
}
