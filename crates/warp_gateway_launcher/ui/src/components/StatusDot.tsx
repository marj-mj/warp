import { cn } from "@/lib/utils";

type Tone = "success" | "warning" | "danger" | "info" | "accent" | "neutral";

const toneClass: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-danger",
  info: "bg-info",
  accent: "bg-accent",
  neutral: "bg-text-dim/40",
};

export function StatusDot({
  on,
  tone = "success",
  pulse = false,
}: {
  on: boolean;
  tone?: Tone;
  pulse?: boolean;
}) {
  return (
    <span
      className={cn(
        "inline-block h-2 w-2 rounded-full transition-colors",
        on ? toneClass[tone] : toneClass.neutral,
        on && pulse && "animate-pulseDot"
      )}
    />
  );
}