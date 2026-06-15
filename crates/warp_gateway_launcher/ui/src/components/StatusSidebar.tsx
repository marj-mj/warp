import { Plus, RefreshCw, Wrench } from "lucide-react";
import { GatewayStatus, ProxyStatus, StoredProvider, ToolPaths } from "@/lib/types";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { StatusDot } from "./StatusDot";

interface Props {
  providers: StoredProvider[];
  selected: string | null;
  onSelect: (name: string) => void;
  onNew: () => void;
  gateway: GatewayStatus;
  proxy: ProxyStatus;
  publicUrl: string | null;
  tools: ToolPaths;
  onRedetect: () => void;
  busyDetect: boolean;
}

function StatusRow({
  label,
  on,
  detail,
  tone,
  pulse,
}: {
  label: string;
  on: boolean;
  detail: string;
  tone: "success" | "warning" | "info" | "accent";
  pulse?: boolean;
}) {
  return (
    <div className="flex items-center justify-between py-1.5">
      <div className="flex items-center gap-2">
        <StatusDot on={on} tone={tone} pulse={pulse} />
        <span className="text-[13px] text-text">{label}</span>
      </div>
      <span className="mono text-[11px] text-text-dim">{detail}</span>
    </div>
  );
}

export function StatusSidebar({
  providers,
  selected,
  onSelect,
  onNew,
  gateway,
  proxy,
  publicUrl,
  tools,
  onRedetect,
  busyDetect,
}: Props) {
  const gwRunning = gateway.kind === "running";
  const pxRunning = proxy.kind === "running";

  return (
    <aside className="flex h-full w-[260px] shrink-0 flex-col gap-4 border-r border-border bg-surface/60 p-4">
      <div>
        <div className="mb-2 text-xs font-semibold uppercase tracking-wider text-text-dim">
          Status
        </div>
        <div className="card-strong px-3 py-2">
          <StatusRow
            label="Gateway"
            on={gwRunning}
            tone="success"
            pulse
            detail={gwRunning ? (gateway as any).addr : "off"}
          />
          <StatusRow
            label="Public URL"
            on={!!publicUrl}
            tone="warning"
            detail={publicUrl ? "live" : "-"}
          />
          <StatusRow label="Proxy" on={pxRunning} tone="info" detail={pxRunning ? "on" : "off"} />
          <StatusRow label="Warp" on={!!tools.warp} tone="accent" detail={tools.warp ? "found" : "-"} />
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs font-semibold uppercase tracking-wider text-text-dim">
            Providers
          </span>
          <Button size="icon" variant="ghost" onClick={onNew} title="New provider">
            <Plus />
          </Button>
        </div>
        <div className="min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
          {providers.length === 0 && (
            <p className="px-2 py-3 text-[13px] text-text-dim">No providers yet.</p>
          )}
          {providers.map((p) => (
            <button
              key={p.name}
              onClick={() => onSelect(p.name)}
              className={cn(
                "w-full rounded-md px-3 py-2 text-left transition-colors",
                selected === p.name
                  ? "bg-accent/15 text-text ring-1 ring-accent/40"
                  : "text-text-dim hover:bg-surface-2 hover:text-text"
              )}
            >
              <div className="truncate text-[13px] font-medium">{p.name || "(unnamed)"}</div>
              <div className="mono truncate text-[11px] text-text-dim">{p.base_url}</div>
            </button>
          ))}
        </div>
      </div>

      <div>
        <div className="mb-2 flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-text-dim">
            <Wrench className="h-3.5 w-3.5" /> Tools
          </span>
          <Button
            size="icon"
            variant="ghost"
            onClick={onRedetect}
            title="Re-detect"
            disabled={busyDetect}
          >
            <RefreshCw className={cn(busyDetect && "animate-spin")} />
          </Button>
        </div>
        <div className="card-strong space-y-1 px-3 py-2">
          <StatusRow
            label="Warp"
            on={!!tools.warp}
            tone="accent"
            detail={tools.warp ? "ok" : "missing"}
          />
          <StatusRow
            label="cloudflared"
            on={!!tools.cloudflared}
            tone="info"
            detail={tools.cloudflared ? "ok" : "missing"}
          />
        </div>
      </div>
    </aside>
  );
}