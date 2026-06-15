import { useEffect, useState } from "react";
import { ExternalLink, Play, Square } from "lucide-react";
import { api } from "@/lib/tauri";
import { OverrideSupport, ProxyOptions, ProxyStatus, WarpChannel } from "@/lib/types";
import { Card, CardHeader, CardTitle } from "./ui/card";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Badge } from "./ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./ui/select";

const CHANNELS: WarpChannel[] = ["production", "staging", "dev"];

interface Props {
  proxy: ProxyStatus;
  onStatus: (s: ProxyStatus) => void;
  onMessage: (m: string) => void;
}

export function ProxyPanel({ proxy, onStatus, onMessage }: Props) {
  const [options, setOptions] = useState<ProxyOptions>({
    host: "127.0.0.1",
    port: 8788,
    channel: "production",
    oz_token: "",
  });
  const [support, setSupport] = useState<OverrideSupport>("unknown");

  useEffect(() => {
    api.warpOverrideSupport().then(setSupport).catch(() => setSupport("unknown"));
  }, []);

  const running = proxy.kind === "running";
  const addr = running ? (proxy as any).addr : `${options.host}:${options.port}`;
  const httpRoot = `http://${addr}`;
  const wsRoot = httpRoot.replace(/^http/, "ws");

  const update = (patch: Partial<ProxyOptions>) => setOptions((o) => ({ ...o, ...patch }));

  const start = async () => {
    const status = await api.startProxy(options);
    onStatus(status);
    if (status.kind === "error") onMessage(status.message);
  };
  const stop = async () => {
    await api.stopProxy();
    onStatus({ kind: "stopped" });
  };
  const openWarp = async () => {
    try {
      await api.launchWarpViaProxy(httpRoot, wsRoot);
      onMessage("Warp opened with proxy overrides. Keep the proxy running.");
    } catch (e) {
      onMessage(String(e));
    }
  };

  const blocked = support === "unsupported" || support === "missing";
  const supportTone =
    support === "supported" ? "success" : support === "unsupported" ? "danger" : "warning";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Warp / OZ Proxy</CardTitle>
        <Badge tone={supportTone}>override: {support}</Badge>
      </CardHeader>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <Label>Channel</Label>
          <Select value={options.channel} onValueChange={(v) => update({ channel: v as WarpChannel })}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              {CHANNELS.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label>Port</Label>
          <Input
            type="number"
            value={options.port}
            disabled={running}
            onChange={(e) => update({ port: Number(e.target.value) })}
          />
        </div>
        <div className="col-span-2 space-y-1.5">
          <Label>OZ token (overrides Warp credentials)</Label>
          <Input
            type="password"
            value={options.oz_token}
            onChange={(e) => update({ oz_token: e.target.value })}
            placeholder="leave empty to pass through"
          />
        </div>
      </div>

      <div className="mt-4 rounded-lg border border-border bg-surface-2 p-3">
        <div className="mono text-[12px] text-info">{httpRoot}</div>
        <div className="mono text-[12px] text-text-dim">{wsRoot}</div>
      </div>

      {blocked && (
        <p className="mt-3 text-[13px] text-warning">
          This Warp build likely ignores WARP_*SERVER_URL overrides. Proxy mode needs a dev/local build.
        </p>
      )}
      {proxy.kind === "error" && <p className="mt-3 text-[13px] text-danger">{proxy.message}</p>}

      <div className="mt-4 flex flex-wrap gap-2">
        {running ? (
          <Button variant="danger" size="sm" onClick={stop}><Square /> Stop proxy</Button>
        ) : (
          <Button variant="secondary" size="sm" onClick={start}><Play /> Start proxy</Button>
        )}
        <Button variant="primary" size="sm" disabled={blocked} onClick={openWarp}>
          <ExternalLink /> Open Warp with proxy
        </Button>
      </div>
    </Card>
  );
}