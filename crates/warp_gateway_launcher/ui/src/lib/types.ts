// Mirrors the Rust serde types in crates/warp_gateway_launcher/src/*.

export type WireApi = "chat" | "responses";

export type Adapter =
  | "openai_chat"
  | "openai_responses"
  | "bridge_openai"
  | "anthropic_messages"
  | "gemini_generate_content";

export interface StoredProvider {
  name: string;
  base_url: string;
  model: string | null;
  wire_api: WireApi;
  adapter: Adapter;
  env_key: string | null;
}
export interface DeleteProviderResult {
  providers: StoredProvider[];
  gateway_stopped: boolean;
  tunnel_stopped: boolean;
  warp_endpoint_removed: boolean;
  warp_running: boolean;
  warnings: string[];
}

export interface ProviderInput {
  name: string;
  base_url: string;
  model: string;
  api_key: string;
  env_key: string;
  wire_api: WireApi;
  adapter: Adapter;
}

export interface GatewayOptions {
  host: string;
  port: number;
  disable_tools: boolean;
  disable_mcp: boolean;
  duplicate_window_secs: number;
  force_model_config_key: string;
}

export interface ProxyOptions {
  host: string;
  port: number;
  channel: WarpChannel;
  oz_token: string;
}

export type WarpChannel = "production" | "staging" | "dev";

export type GatewayStatus =
  | { kind: "stopped" }
  | { kind: "running"; addr: string }
  | { kind: "error"; message: string };

export type ProxyStatus =
  | { kind: "stopped" }
  | { kind: "running"; addr: string }
  | { kind: "error"; message: string };

export type LaunchPhaseKind =
  | "idle"
  | "starting_gateway"
  | "starting_tunnel"
  | "waiting_public_url"
  | "spawning_warp"
  | "done"
  | "pending_warp_restart"
  | "failed";

export type LaunchPhase =
  | { kind: Exclude<LaunchPhaseKind, "failed"> }
  | { kind: "failed"; message: string };

export interface LaunchPhaseEvent {
  phase: LaunchPhase;
  progress: number;
}

export interface ToolPaths {
  warp: string | null;
  cloudflared: string | null;
}

export interface ProbeResult {
  adapter: string;
  wire_api: string;
  compatibility_group: string;
  confidence: string;
  disable_tools: boolean;
  disable_mcp: boolean;
  chat_non_stream: string;
  chat_streaming: string;
  chat_tools: string;
  responses_non_stream: string;
  responses_streaming: string;
  responses_tools: string;
}

export interface EndpointInfo {
  url: string;
  public: boolean;
}

export type OverrideSupport = "supported" | "unsupported" | "unknown" | "missing";