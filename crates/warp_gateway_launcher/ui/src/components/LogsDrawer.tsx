import { useEffect, useRef } from "react";
import { Trash2 } from "lucide-react";
import { useLogs } from "@/hooks/useLogs";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "./ui/dialog";
import { Button } from "./ui/button";

export function LogsDrawer({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { lines, clear } = useLogs(open);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines.length]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <div className="flex items-center justify-between">
            <DialogTitle>Logs</DialogTitle>
            <Button variant="ghost" size="sm" onClick={clear}><Trash2 /> Clear</Button>
          </div>
        </DialogHeader>
        <div className="h-[420px] overflow-y-auto rounded-lg border border-border bg-bg p-3">
          {lines.length === 0 ? (
            <p className="text-[13px] text-text-dim">(no logs yet)</p>
          ) : (
            lines.map((l, i) => (
              <div key={i} className="mono whitespace-pre-wrap text-[12px] leading-relaxed text-text-dim">
                {l}
              </div>
            ))
          )}
          <div ref={endRef} />
        </div>
      </DialogContent>
    </Dialog>
  );
}