import { useEffect, useMemo, useState } from "react";
import { ScrollText, Settings2 } from "lucide-react";
import { api } from "@/lib/tauri";
import { GatewayOptions, ProviderInput, StoredProvider } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useLauncher } from "@/hooks/useLauncher";
import { StatusSidebar } from "@/components/StatusSidebar";
import { HeroCard } from "@/components/HeroCard";
import { ProviderEditor } from "@/components/ProviderEditor";
import { ProxyPanel } from "@/components/ProxyPanel";
import { LogsDrawer } from "@/components/LogsDrawer";
import { AdvancedDrawer } from "@/components/AdvancedDrawer";
import { Button } from "@/components/ui/button";

type Mode = "managed" | "proxy";

const defaultGatewayOptions: GatewayOptions = {
  host: "127.0.0.1",
  port: 8787,
  disable_tools: true,
  disable_mcp: true,
  duplicate_window_secs: 60,
  force_model_config_key: "",
};

export default function App() {
  const { state, patch } = useLauncher();
  const [mode, setMode] = useState<Mode>("managed");
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [form, setForm] = useState<ProviderInput | null>(null);
  const [gatewayOptions, setGatewayOptions] = useState<GatewayOptions>(defaultGatewayOptions);
  const [endpointUrl, setEndpointUrl] = useState("");
  const [configJson, setConfigJson] = useState("");
  const [logsOpen, setLogsOpen] = useState(false);
  const [advOpen, setAdvOpen] = useState(false);
  const [busyDetect, setBusyDetect] = useState(false);

  // Auto-select first provider once loaded.
  useEffect(() => {
    if (selectedName === null && !isNew && state.providers.length > 0) {
      setSelectedName(state.providers[0].name);
    }
  }, [state.providers, selectedName, isNew]);

  const selected: StoredProvider | null = useMemo(
    () => state.providers.find((p) => p.name === selectedName) ?? null,
    [state.providers, selectedName]
  );

  const providerValid = !!form && form.name.trim() !== "" && form.base_url.trim() !== "";
  const busy =
    state.phase.kind === "starting_gateway" ||
    state.phase.kind === "starting_tunnel" ||
    state.phase.kind === "waiting_public_url" ||
    state.phase.kind === "spawning_warp";

  // Refresh endpoint info whenever public URL / gateway changes.
  useEffect(() => {
    if (!form) return;
    api.endpointUrl(gatewayOptions.host, gatewayOptions.port).then((info) => setEndpointUrl(info.url));
    api
      .endpointConfigJson(form.name, form.model, gatewayOptions.host, gatewayOptions.port)
      .then(setConfigJson);
  }, [state.publicUrl, state.gateway, form, gatewayOptions.host, gatewayOptions.port]);

  const onLaunch = async () => {
    if (!form) return;
    try {
      await api.startLaunchFlow(form, gatewayOptions);
    } catch {
      /* phase event carries the failure */
    }
  };

  const redetect = async () => {
    setBusyDetect(true);
    const tools = await api.detectTools();
    patch({ tools });
    setBusyDetect(false);
  };

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-bg text-text">
      {/* Top bar */}
      <header className="flex h-12 shrink-0 items-center justify-between border-b border-border px-4">
        <div className="flex items-center gap-3">
          <span className="text-[15px] font-semibold">Warp Gateway Launcher</span>
          <div className="flex rounded-md bg-surface-2 p-0.5">
            {(["managed", "proxy"] as Mode[]).map((m) => (
              <button
                key={m}
                onClick={() => setMode(m)}
                className={cn(
                  "rounded px-3 py-1 text-[13px] transition-colors",
                  mode === m ? "bg-accent text-accent-fg" : "text-text-dim hover:text-text"
                )}
              >
                {m === "managed" ? "Managed Provider" : "Warp / OZ Proxy"}
              </button>
            ))}
          </div>
        </div>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="sm" onClick={() => setAdvOpen(true)}>
            <Settings2 /> Advanced
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setLogsOpen(true)}>
            <ScrollText /> Logs
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <StatusSidebar
          providers={state.providers}
          selected={selectedName}
          onSelect={(name) => {
            setSelectedName(name);
            setIsNew(false);
            if (state.phase.kind === "done" || state.phase.kind === "pending_warp_restart") {
              patch({ phase: { kind: "idle" }, progress: 0 });
            }
          }}
          onNew={() => {
            setIsNew(true);
            setSelectedName(null);
            if (state.phase.kind === "done" || state.phase.kind === "pending_warp_restart") {
              patch({ phase: { kind: "idle" }, progress: 0 });
            }
          }}
          gateway={state.gateway}
          proxy={state.proxy}
          publicUrl={state.publicUrl}
          tools={state.tools}
          onRedetect={redetect}
          busyDetect={busyDetect}
        />

        <main className="min-h-0 flex-1 overflow-y-auto p-6">
          <div className="mx-auto flex max-w-3xl flex-col gap-5">
            {mode === "managed" ? (
              <>
                <HeroCard
                  hasProvider={state.providers.length > 0 || isNew}
                  providerValid={providerValid}
                  gateway={state.gateway}
                  phase={state.phase}
                  progress={state.progress}
                  publicUrl={state.publicUrl}
                  endpointUrl={endpointUrl}
                  configJson={configJson}
                  busy={busy}
                  onLaunch={onLaunch}
                  onCancel={() => api.cancelLaunch()}
                  onReset={() => api.resetLaunch()}
                  onRestartWarp={async () => {
                    try {
                      await api.restartWarp();
                      await api.resetLaunch();
                      patch({ message: "Warp restarted with the latest gateway config." });
                    } catch (e) {
                      patch({ message: String(e) });
                    }
                  }}
                  onAddProvider={() => {
                    setIsNew(true);
                    setSelectedName(null);
                  }}
                />
                {(selected || isNew) && (
                  <ProviderEditor
                    provider={isNew ? null : selected}
                    isNew={isNew}
                    onFormChange={setForm}
                    onSaved={async (providers, savedName) => {
                      patch({ providers });
                      setIsNew(false);
                      setSelectedName(savedName);
                    }}
                    onDeleted={async (providers, message) => {
                      patch({ providers, message: message ?? "" });
                      setIsNew(false);
                      setSelectedName(providers[0]?.name ?? null);
                    }}
                  />
                )}
                {state.message && (
                  <p className="text-center text-[13px] text-text-dim">{state.message}</p>
                )}
              </>
            ) : (
              <ProxyPanel
                proxy={state.proxy}
                onStatus={(proxy) => patch({ proxy })}
                onMessage={(message) => patch({ message })}
              />
            )}
          </div>
        </main>
      </div>

      <LogsDrawer open={logsOpen} onOpenChange={setLogsOpen} />
      <AdvancedDrawer
        open={advOpen}
        onOpenChange={setAdvOpen}
        options={gatewayOptions}
        onChange={setGatewayOptions}
        gatewayRunning={state.gateway.kind === "running"}
      />
    </div>
  );
}