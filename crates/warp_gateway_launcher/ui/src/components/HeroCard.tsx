import { AlertTriangle, CheckCircle2, Loader2, Play, RotateCcw, Rocket, X } from "lucide-react";
import { GatewayStatus, LaunchPhase } from "@/lib/types";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { CopyButton } from "./CopyButton";

interface Props {
  hasProvider: boolean;
  providerValid: boolean;
  gateway: GatewayStatus;
  phase: LaunchPhase;
  progress: number;
  publicUrl: string | null;
  endpointUrl: string;
  configJson: string;
  busy: boolean;
  onLaunch: () => void;
  onCancel: () => void;
  onReset: () => void;
  onRestartWarp: () => void;
  onAddProvider: () => void;
}

function phaseLabel(phase: LaunchPhase): string {
  switch (phase.kind) {
    case "starting_gateway":
      return "Starting gateway...";
    case "starting_tunnel":
      return "Starting cloudflared tunnel...";
    case "waiting_public_url":
      return "Waiting for public HTTPS URL...";
    case "spawning_warp":
      return "Opening Warp...";
    case "done":
      return "Running";
    case "pending_warp_restart":
      return "Saved to Warp - restart Warp to apply";
    case "failed":
      return phase.message;
    default:
      return "Ready";
  }
}

export function HeroCard(props: Props) {
  const { phase, progress, busy } = props;

  // Empty state: no provider configured yet.
  if (!props.hasProvider) {
    return (
      <div className="card relative overflow-hidden p-8 text-center">
        <Rocket className="mx-auto mb-3 h-10 w-10 text-accent" />
        <h2 className="text-xl font-semibold text-text">Welcome to the Gateway Launcher</h2>
        <p className="mx-auto mt-1 max-w-md text-sm text-text-dim">
          Add an upstream provider to run the Managed Provider Gateway locally and point Warp at it.
        </p>
        <Button variant="primary" size="lg" className="mt-5" onClick={props.onAddProvider}>
          <Play /> Add your first provider
        </Button>
      </div>
    );
  }

  const failed = phase.kind === "failed";
  const done = phase.kind === "done";
  const pending = phase.kind === "pending_warp_restart";

  return (
    <div
      className={cn(
        "card relative overflow-hidden p-6 transition-shadow",
        done && "shadow-glow"
      )}
    >
      {/* accent stripe when running */}
      {done && <div className="absolute inset-x-0 top-0 h-0.5 bg-accent" />}

      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            {busy && <Loader2 className="h-4 w-4 animate-spin text-accent" />}
            {done && <CheckCircle2 className="h-5 w-5 text-success" />}
            {failed && <AlertTriangle className="h-5 w-5 text-danger" />}
            <h2
              className={cn(
                "text-lg font-semibold",
                done && "text-success",
                failed && "text-danger",
                !done && !failed && "text-text"
              )}
            >
              {done ? "Gateway running" : pending ? "Restart Warp to apply" : failed ? "Launch failed" : "Managed Provider Gateway"}
            </h2>
          </div>
          <p className="mt-1 text-sm text-text-dim">{phaseLabel(phase)}</p>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {!busy && !done && !pending && (
            <Button
              variant="primary"
              size="lg"
              disabled={!props.providerValid}
              onClick={props.onLaunch}
            >
              <Rocket /> Start gateway flow
            </Button>
          )}
          {busy && (
            <Button variant="danger" size="lg" onClick={props.onCancel}>
              <X /> Cancel
            </Button>
          )}
          {pending && (
            <Button variant="primary" size="lg" onClick={props.onRestartWarp}>
              <RotateCcw /> Restart Warp
            </Button>
          )}
          {(done || failed || pending) && (
            <Button variant="secondary" size="lg" onClick={props.onReset}>
              <RotateCcw /> Reset
            </Button>
          )}
        </div>
      </div>

      {(busy || done || pending) && (
        <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-surface-3">
          <div
            className="h-full rounded-full bg-accent transition-all duration-500"
            style={{ width: `${Math.round(progress * 100)}%` }}
          />
        </div>
      )}

      {(done || pending) && (
        <div className="mt-5 rounded-lg border border-border bg-surface-2 p-4">
          <div className="mb-1 text-xs font-medium uppercase tracking-wider text-text-dim">
            Custom endpoint for Warp
          </div>
          <div className="mono mb-3 break-all text-info">{props.endpointUrl}</div>
          <div className="flex flex-wrap gap-2">
            <CopyButton variant="primary" size="sm" value={props.configJson} label="Copy config JSON" />
            <CopyButton variant="secondary" size="sm" value={props.endpointUrl} label="Copy URL" />
          </div>
          <p className="mt-3 text-[13px] text-text-dim">
            Paste the JSON into Warp Settings -&gt; AI -&gt; Custom endpoints.
          </p>
        </div>
      )}
    </div>
  );
}