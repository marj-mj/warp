// Typed wrappers around Tauri command + event surface.
// Keep names and shapes in sync with crates/warp_gateway_launcher/src/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Adapter,
  DeleteProviderResult,
  EndpointInfo,
  GatewayOptions,
  GatewayStatus,
  LaunchPhase,
  LaunchPhaseEvent,
  OverrideSupport,
  ProbeResult,
  ProviderInput,
  ProxyOptions,
  ProxyStatus,
  StoredProvider,
  ToolPaths,
  WireApi,
} from "./types";

export const api = {
  // Provider store
  listProviders: () => invoke<StoredProvider[]>("list_providers"),
  saveProvider: (provider: ProviderInput) =>
    invoke<StoredProvider[]>("save_provider", { provider }),
  deleteProvider: (name: string) => invoke<DeleteProviderResult>("delete_provider", { name }),

  // Gateway
  gatewayStatus: () => invoke<GatewayStatus>("gateway_status"),
  startGateway: (provider: ProviderInput, options: GatewayOptions) =>
    invoke<GatewayStatus>("start_gateway", { provider, options }),
  stopGateway: () => invoke<void>("stop_gateway"),

  // Proxy
  proxyStatus: () => invoke<ProxyStatus>("proxy_status"),
  startProxy: (options: ProxyOptions) => invoke<ProxyStatus>("start_proxy", { options }),
  stopProxy: () => invoke<void>("stop_proxy"),
  warpOverrideSupport: () => invoke<OverrideSupport>("warp_override_support"),
  launchWarpViaProxy: (httpRoot: string, wsRoot: string) =>
    invoke<void>("launch_warp_via_proxy", { httpRoot, wsRoot }),

  // Launch flow
  startLaunchFlow: (provider: ProviderInput, options: GatewayOptions) =>
    invoke<void>("start_launch_flow", { provider, options }),
  cancelLaunch: () => invoke<void>("cancel_launch"),
  resetLaunch: () => invoke<void>("reset_launch"),
  launchPhase: () => invoke<LaunchPhase>("launch_phase"),

  // Probe
  startProbe: (baseUrl: string, apiKey: string | null) =>
    invoke<ProbeResult>("start_probe", { baseUrl, apiKey }),

  // Tunnel
  startTunnel: (port: number) => invoke<void>("start_tunnel", { port }),
  stopTunnel: () => invoke<void>("stop_tunnel"),

  // Tools
  detectTools: () => invoke<ToolPaths>("detect_tools"),
  installWarp: () => invoke<void>("install_warp"),
  installCloudflared: () => invoke<void>("install_cloudflared"),
  restartWarp: () => invoke<void>("restart_warp"),

  // Endpoint
  endpointUrl: (host: string, port: number) =>
    invoke<EndpointInfo>("endpoint_url", { host, port }),
  endpointConfigJson: (name: string, model: string, host: string, port: number) =>
    invoke<string>("endpoint_config_json", { name, model, host, port }),

  // Logs
  logSnapshot: () => invoke<string[]>("log_snapshot"),
  clearLogs: () => invoke<void>("clear_logs"),
};

// Event subscriptions ------------------------------------------------------

export const events = {
  onGatewayStatus: (cb: (s: GatewayStatus) => void) =>
    listen<GatewayStatus>("gateway://status", (e) => cb(e.payload)),
  onProxyStatus: (cb: (s: ProxyStatus) => void) =>
    listen<ProxyStatus>("proxy://status", (e) => cb(e.payload)),
  onLaunchPhase: (cb: (e: LaunchPhaseEvent) => void) =>
    listen<LaunchPhaseEvent>("launch://phase", (e) => cb(e.payload)),
  onTunnelUrl: (cb: (url: string) => void) =>
    listen<string>("tunnel://public-url", (e) => cb(e.payload)),
  onTunnelError: (cb: (msg: string) => void) =>
    listen<string>("tunnel://error", (e) => cb(e.payload)),
  onProbeResult: (cb: (r: ProbeResult) => void) =>
    listen<ProbeResult>("probe://result", (e) => cb(e.payload)),
  onLogLine: (cb: (line: string) => void) =>
    listen<string>("logs://line", (e) => cb(e.payload)),
  onLauncherMessage: (cb: (msg: string) => void) =>
    listen<string>("launcher://message", (e) => cb(e.payload)),
};

export type { UnlistenFn };

// Convenience constants used by the UI
export const WIRE_APIS: WireApi[] = ["chat", "responses"];
export const ADAPTERS: Adapter[] = [
  "openai_chat",
  "openai_responses",
  "anthropic_messages",
  "gemini_generate_content",
  "bridge_openai",
];