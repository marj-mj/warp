import { GatewayOptions } from "@/lib/types";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "./ui/dialog";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Switch } from "./ui/switch";

interface Props {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  options: GatewayOptions;
  onChange: (o: GatewayOptions) => void;
  gatewayRunning: boolean;
}

export function AdvancedDrawer({ open, onOpenChange, options, onChange, gatewayRunning }: Props) {
  const update = (patch: Partial<GatewayOptions>) => onChange({ ...options, ...patch });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Advanced gateway settings</DialogTitle>
          <DialogDescription>
            Safe-mode and binding options. Some take effect on next start.
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-1.5">
            <Label>Host</Label>
            <Input value={options.host} disabled={gatewayRunning} onChange={(e) => update({ host: e.target.value })} />
          </div>
          <div className="space-y-1.5">
            <Label>Port</Label>
            <Input
              type="number"
              value={options.port}
              disabled={gatewayRunning}
              onChange={(e) => update({ port: Number(e.target.value) })}
            />
          </div>
          <div className="space-y-1.5">
            <Label>Duplicate window (s)</Label>
            <Input
              type="number"
              value={options.duplicate_window_secs}
              onChange={(e) => update({ duplicate_window_secs: Number(e.target.value) })}
            />
          </div>
          <div className="space-y-1.5">
            <Label>Force model config key</Label>
            <Input
              value={options.force_model_config_key}
              onChange={(e) => update({ force_model_config_key: e.target.value })}
            />
          </div>
        </div>

        <div className="mt-4 space-y-3">
          <label className="flex items-center justify-between">
            <span className="text-[13px] text-text">Disable tools (safe mode)</span>
            <Switch checked={options.disable_tools} onCheckedChange={(v) => update({ disable_tools: v })} />
          </label>
          <label className="flex items-center justify-between">
            <span className="text-[13px] text-text">Disable MCP</span>
            <Switch checked={options.disable_mcp} onCheckedChange={(v) => update({ disable_mcp: v })} />
          </label>
        </div>
      </DialogContent>
    </Dialog>
  );
}