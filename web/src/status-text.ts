import type { dicts } from "./i18n";
import type { Session, Status } from "./types";

export function statusLine(status: Status | null, t: (typeof dicts)["en"]): [string, string] {
  if (!status) return [t.connecting, ""];
  if (status.cutout_latched) return [t.cutoutLatched, t.cutoutDetail];
  if (!status.awake) return [t.sleepingNormally, t.idleDetail];
  const labels = [...new Set(status.sessions.map((s) => holdName(s, t)))];
  const detail =
    labels.length === 1 ? t.awakeReasonOne(labels[0]) : t.awakeReasonMany(status.sessions.length);
  const held =
    status.held_for_secs != null && status.held_for_secs >= 60
      ? ` · ${t.heldFor(t.dur(status.held_for_secs))}`
      : "";
  return [t.awake, `${detail}${held}`];
}

export function holdName(s: Session, t: (typeof dicts)["en"]): string {
  if (s.label) return s.label;
  if (s.source === "manual") return t.holdsTitle;
  return `${s.source} · ${s.id.slice(0, 8)}`;
}
