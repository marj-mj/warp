import { useCallback, useEffect, useRef, useState } from "react";
import { api, events } from "@/lib/tauri";
import {
  GatewayStatus,
  LaunchPhase,
  ProxyStatus,
  StoredProvider,
  ToolPaths,
} from "@/lib/types";

export interface LauncherState {
  providers: StoredProvider[];
  gateway: GatewayStatus;
  proxy: ProxyStatus;
  phase: LaunchPhase;
  progress: number;
  publicUrl: string | null;
  tools: ToolPaths;
  message: string;
}

const initial: LauncherState = {
  providers: [],
  gateway: { kind: "stopped" },
  proxy: { kind: "stopped" },
  phase: { kind: "idle" },
  progress: 0,
  publicUrl: null,
  tools: { warp: null, cloudflared: null },
  message: "",
};

export function useLauncher() {
  const [state, setState] = useState<LauncherState>(initial);
  const patch = useCallback((p: Partial<LauncherState>) => setState((s) => ({ ...s, ...p })), []);
  const mounted = useRef(true);

  const refreshProviders = useCallback(async () => {
    const providers = await api.listProviders();
    patch({ providers });
    return providers;
  }, [patch]);

  useEffect(() => {
    mounted.current = true;
    const unlistens: Array<Promise<() => void>> = [];

    (async () => {
      const [providers, gateway, proxy, phase, tools] = await Promise.all([
        api.listProviders(),
        api.gatewayStatus(),
        api.proxyStatus(),
        api.launchPhase(),
        api.detectTools(),
      ]);
      if (!mounted.current) return;
      patch({ providers, gateway, proxy, phase, tools });
    })();

    unlistens.push(events.onGatewayStatus((gateway) => patch({ gateway })));
    unlistens.push(events.onProxyStatus((proxy) => patch({ proxy })));
    unlistens.push(
      events.onLaunchPhase((e) => patch({ phase: e.phase, progress: e.progress }))
    );
    unlistens.push(
      events.onTunnelUrl((url) => patch({ publicUrl: url, message: `Public URL ready: ${url}` }))
    );
    unlistens.push(events.onTunnelError((msg) => patch({ message: msg })));

    return () => {
      mounted.current = false;
      unlistens.forEach((p) => p.then((fn) => fn()));
    };
  }, [patch]);

  return { state, patch, refreshProviders };
}