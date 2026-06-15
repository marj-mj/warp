import { useEffect, useRef, useState } from "react";
import { Loader2, Save, Trash2, Wand2 } from "lucide-react";
import { api, ADAPTERS, WIRE_APIS } from "@/lib/tauri";
import { Adapter, ProbeResult, ProviderInput, StoredProvider, WireApi } from "@/lib/types";
import { Card, CardHeader, CardTitle } from "./ui/card";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Badge } from "./ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./ui/select";

function emptyForm(): ProviderInput {
  return {
    name: "",
    base_url: "",
    model: "",
    api_key: "",
    env_key: "",
    wire_api: "chat",
    adapter: "openai_chat",
  };
}

function fromStored(p: StoredProvider): ProviderInput {
  return {
    name: p.name,
    base_url: p.base_url,
    model: p.model ?? "",
    api_key: "",
    env_key: p.env_key ?? "",
    wire_api: p.wire_api,
    adapter: p.adapter,
  };
}

interface Props {
  provider: StoredProvider | null;
  isNew: boolean;
  onSaved: (providers: StoredProvider[], savedName: string) => void;
  onDeleted: (providers: StoredProvider[]) => void;
  onFormChange: (form: ProviderInput) => void;
}

export function ProviderEditor({ provider, isNew, onSaved, onDeleted, onFormChange }: Props) {
  const [form, setForm] = useState<ProviderInput>(emptyForm());
  const [probing, setProbing] = useState(false);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [error, setError] = useState("");
  const apiKeys = useRef(new Map<string, string>());

  useEffect(() => {
    const next = provider ? fromStored(provider) : emptyForm();
    if (provider) {
      next.api_key = apiKeys.current.get(provider.name) ?? "";
    }
    setForm(next);
    setProbe(null);
    setError("");
    onFormChange(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider?.name, isNew]);

  const update = (patch: Partial<ProviderInput>) => {
    const next = { ...form, ...patch };
    setForm(next);
    onFormChange(next);
  };

  const valid = form.name.trim() !== "" && form.base_url.trim() !== "";

  const save = async () => {
    try {
      const savedName = form.name.trim();
      const apiKey = form.api_key.trim();
      if (apiKey) {
        apiKeys.current.set(savedName, apiKey);
      } else {
        apiKeys.current.delete(savedName);
      }
      const providers = await api.saveProvider(form);
      onSaved(providers, savedName);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async () => {
    if (!provider) return;
    const providers = await api.deleteProvider(provider.name);
    onDeleted(providers);
  };

  const runProbe = async () => {
    setProbing(true);
    try {
      const apiKey = form.api_key.trim() || null;
      const result = await api.startProbe(form.base_url.trim(), apiKey);
      setProbe(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setProbing(false);
    }
  };

  const applyProbe = () => {
    if (!probe) return;
    update({
      adapter: probe.adapter as Adapter,
      wire_api: probe.wire_api as WireApi,
    });
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>{isNew ? "New provider" : "Provider details"}</CardTitle>
        {!isNew && form.env_key && <Badge tone="success">env: {form.env_key}</Badge>}
      </CardHeader>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <Label htmlFor="name">Name</Label>
          <Input id="name" value={form.name} onChange={(e) => update({ name: e.target.value })} placeholder="NROUTER" />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="model">Model</Label>
          <Input id="model" value={form.model} onChange={(e) => update({ model: e.target.value })} placeholder="gpt-5" />
        </div>
        <div className="col-span-2 space-y-1.5">
          <Label htmlFor="base">Base URL</Label>
          <Input id="base" value={form.base_url} onChange={(e) => update({ base_url: e.target.value })} placeholder="https://api.example.com/v1" />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="apikey">API key (in-memory only)</Label>
          <Input id="apikey" type="password" value={form.api_key} onChange={(e) => update({ api_key: e.target.value })} placeholder="sk-..." />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="envkey">Env key (persisted)</Label>
          <Input id="envkey" value={form.env_key} onChange={(e) => update({ env_key: e.target.value })} placeholder="OPENAI_API_KEY" />
        </div>
        <div className="space-y-1.5">
          <Label>Wire API</Label>
          <Select value={form.wire_api} onValueChange={(v) => update({ wire_api: v as WireApi })}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              {WIRE_APIS.map((w) => <SelectItem key={w} value={w}>{w}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label>Adapter</Label>
          <Select value={form.adapter} onValueChange={(v) => update({ adapter: v as Adapter })}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              {ADAPTERS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
      </div>

      {error && <p className="mt-3 text-[13px] text-danger">{error}</p>}

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" disabled={!valid} onClick={save}>
          <Save /> Save
        </Button>
        <Button variant="secondary" size="sm" disabled={probing || !form.base_url.trim()} onClick={runProbe}>
          {probing ? <Loader2 className="animate-spin" /> : <Wand2 />} Probe
        </Button>
        {!isNew && (
          <Button variant="danger" size="sm" onClick={remove}>
            <Trash2 /> Delete
          </Button>
        )}
      </div>

      {probe && (
        <div className="mt-4 rounded-lg border border-border bg-surface-2 p-3">
          <div className="flex items-center justify-between">
            <span className="text-[13px] text-text">
              Recommended: <span className="font-medium text-accent">{probe.adapter}</span> / {probe.wire_api}{" "}
              <Badge tone={probe.confidence === "high" ? "success" : "warning"}>{probe.confidence}</Badge>
            </span>
            <Button variant="ghost" size="sm" onClick={applyProbe}>Apply</Button>
          </div>
        </div>
      )}
    </Card>
  );
}
