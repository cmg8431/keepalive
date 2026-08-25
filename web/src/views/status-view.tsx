import { useState } from "react";
import {
  BatteryMedium,
  Clock4,
  Hand,
  Laptop,
  Moon,
  SquareTerminal,
  Sun,
  Thermometer,
  Timer,
} from "lucide-react";
import { post } from "../api";
import type { dicts } from "../i18n";
import type { Session, Status, Tab } from "../types";
import { holdName, statusLine } from "../status-text";
import {
  ActionGrid,
  ActionTile,
  DurationPicker,
  EmptyState,
  Hero,
  IconTile,
  Lid,
  Row,
  RowList,
  Section,
  SkeletonCard,
  Widget,
  WidgetGrid,
} from "../components";

export function StatusSkeleton() {
  return (
    <>
      <SkeletonCard rows={2} />
      <SkeletonCard rows={3} />
      <SkeletonCard rows={4} />
    </>
  );
}


function nextExpiryLabel(
  status: Status,
  t: (typeof dicts)["en"],
): { value: string; unit: string } {
  if (status.sessions.length === 0) return { value: "—", unit: "" };
  const secs = Math.min(...status.sessions.map((s) => s.expires_in_secs));
  const mins = Math.max(1, Math.round(secs / 60));
  if (mins < 60) return { value: `${mins}`, unit: t.unitMin };
  return { value: `${Math.floor(mins / 60)}`, unit: t.unitHour };
}

/** 홀드 한 줄이 사람 말로 읽히게 만든다. `managed` 같은 내부 용어는 나가지 않는다. */
function holdLabels(s: Session, t: (typeof dicts)["en"]): [string, string] {
  const countdown =
    s.expires_in_secs > 30 * 24 * 3600 ? t.holdUntilOff : t.holdReleaseIn(t.dur(s.expires_in_secs));
  if (s.source === "manual") return [t.holdSourceManual, countdown];
  if (s.source === "managed") {
    return [t.holdManaged(s.label ?? ""), `${t.holdManagedDetail} · ${countdown}`];
  }
  const agent = sourceName(s.source);
  return [agent, `${s.label ? `${t.holdAgentDetail(s.label)} · ` : ""}${countdown}`];
}

function sourceName(source: string): string {
  return (
    {
      "claude-code": "Claude Code",
      codex: "Codex",
      cursor: "Cursor",
      "gemini-cli": "Gemini",
      mcp: "MCP",
    }[source] ?? source
  );
}

/* Actions first, readings second: the screen opens with the two things people
   came to do (hold it awake / let it sleep) and compresses the telemetry that
   used to fill three tiles into one line. */
export function StatusView({
  t,
  status,
  hist,
  refresh,
  setTab,
  onOpenTerminal,
}: {
  t: (typeof dicts)["en"];
  status: Status | null;
  hist: { temp: number[]; batt: number[] };
  refresh: () => Promise<void>;
  setTab: (tab: Tab) => void;
  onOpenTerminal: (name: string) => void;
}) {
  const [showAll, setShowAll] = useState(false);
  if (!status) return <StatusSkeleton />;

  const act = async (fn: () => Promise<unknown>) => {
    await fn();
    await refresh();
  };

  /* 관리 세션은 이미 홀드를 하나씩 만든다. 둘을 따로 나열하면 같은 사실이
     두 번 보이므로, 세션 쪽으로 접어 한 줄에 "무엇이 붙잡고 있는지"와
     "그걸로 들어가는 길"을 함께 둔다. */
  const rows = [
    ...status.managed.map((m) => {
      const hold = status.sessions.find((s) => s.source === "managed" && s.label === m.name);
      const countdown = hold ? t.holdReleaseIn(t.dur(hold.expires_in_secs)) : t.holdNotHolding;
      return {
        key: m.name,
        kind: "session" as const,
        tone: (m.waiting ? "orange" : m.status === "running" ? "green" : "orange") as
          | "green"
          | "orange",
        title: m.dir.split("/").pop() ?? m.name,
        detail: m.waiting
          ? t.waitingHint
          : `${m.status === "running" ? t.running : t.abandoned} · ${countdown}`,
        session: m.name,
        holdId: hold?.id,
      };
    }),
    ...status.sessions
      .filter((s) => !(s.source === "managed" && status.managed.some((m) => m.name === s.label)))
      .map((s) => {
        const [title, detail] = holdLabels(s, t);
        return {
          key: s.id,
          kind: (s.source === "manual" ? "manual" : "agent") as "manual" | "agent",
          tone: "green" as const,
          title,
          detail,
          session: undefined as string | undefined,
          holdId: s.id,
        };
      }),
  ];
  const batteryWarn =
    status.battery_percent !== null &&
    !status.on_ac_power &&
    status.battery_percent <= status.battery_floor_percent + 5;
  const tempWarn =
    status.temperature_celsius !== null &&
    status.temperature_celsius >= status.thermal_threshold_celsius - 5;

  const heroValue = status.awake
    ? nextExpiryLabel(status, t)
    : { value: `${status.battery_percent ?? "—"}`, unit: "%" };
  const [heroTitle, heroDetail] = statusLine(status, t);

  return (
    <>
      <Hero
        tone={status.cutout_latched ? "cutout" : status.awake ? "awake" : "idle"}
        icon={status.awake ? <Sun size={17} /> : <Moon size={17} />}
        title={heroTitle}
        detail={heroDetail}
        value={heroValue.value}
        unit={heroValue.unit}
        filled={Math.min(status.sessions.length, 4)}
      />

      <Section title={t.quickActions}>
        <DurationPicker
          maxMinutes={(status.max_hold_hours || 8) * 60}
          unlimited={status.max_hold_hours === 0}
          label={t.holdHowLong}
          formatDuration={(m) => t.dur(m * 60)}
          untilLabel={(clock) => t.holdUntilClock(clock)}
          confirmLabel={(d) => t.holdFor(d)}
          doneLabel={t.holdDone}
          unlimitedLabel={t.holdUnlimitedOn}
          locale={t.locale}
          foreverLabel={t.holdForever}
          onHold={(m) => act(() => post("/api/hold", { minutes: m }))}
          onForever={() => act(() => post("/api/hold", { forever: true }))}
        />
        <ActionGrid>
          <ActionTile
            icon={<Moon size={15} />}
            label={t.actionSleep}
            disabled={status.sessions.length === 0}
            onClick={() => act(() => post("/api/sleep"))}
          />
          <ActionTile
            icon={<SquareTerminal size={15} />}
            label={t.actionNewSession}
            onClick={() => setTab("sessions")}
          />
        </ActionGrid>
      </Section>

      <Section title={t.nowLabel}>
        <WidgetGrid>
          <Widget
            icon={<BatteryMedium size={15} strokeWidth={1.6} />}
            label={t.battery}
            value={status.battery_percent === null ? "—" : `${status.battery_percent}`}
            unit="%"
            sub={status.on_ac_power ? t.charging : t.batteryFloor(status.battery_floor_percent)}
            warn={batteryWarn}
            visual={<Spark data={hist.batt} />}
          />
          <Widget
            icon={<Thermometer size={15} strokeWidth={1.6} />}
            label={t.temp}
            value={
              status.temperature_celsius === null
                ? "—"
                : `${status.temperature_celsius.toFixed(0)}`
            }
            unit="°C"
            sub={t.thermalCutout(Math.round(status.thermal_threshold_celsius))}
            warn={tempWarn}
            visual={<Spark data={hist.temp} />}
          />
          <Widget
            icon={<Laptop size={15} strokeWidth={1.6} />}
            label={t.lid}
            graphic={<Lid closed={status.lid_closed} />}
            value={status.lid_closed ? t.lidClosed : t.lidOpen}
            sub={status.clamshell_active ? t.lidHeld : ""}
          />
          <Widget
            icon={<SquareTerminal size={15} strokeWidth={1.6} />}
            label={t.tabSessions}
            value={`${status.managed.length}`}
            sub={status.managed.filter((m) => m.status === "running").length > 0 ? t.running : ""}
          />
        </WidgetGrid>
      </Section>

      <Section
        title={t.holdsNowTitle}
        aside={status.sessions.length > 0 ? t.holdsNowAside : undefined}
      >
        {rows.length === 0 ? (
          <EmptyState text={t.holdsNowEmpty} />
        ) : (
          <RowList>
            {(showAll ? rows : rows.slice(0, 3)).map((row) => (
              <Row
                key={row.key}
                icon={
                  <IconTile tone={row.tone}>
                    {row.kind === "manual" ? <Hand size={15} /> : <SquareTerminal size={15} />}
                  </IconTile>
                }
                title={row.title}
                detail={row.detail}
                actions={
                  <>
                    {row.session && (
                      <button className="primary small" onClick={() => onOpenTerminal(row.session!)}>
                        {t.openTerminal}
                      </button>
                    )}
                    {row.holdId && (
                      <button
                        className="small"
                        onClick={() => act(() => post("/api/release", { id: row.holdId }))}
                      >
                        {t.holdStop}
                      </button>
                    )}
                  </>
                }
              />
            ))}
            {!showAll && rows.length > 3 && (
              <li>
                <button className="more" onClick={() => setShowAll(true)}>
                  {t.moreHolds(rows.length - 3)}
                </button>
              </li>
            )}
          </RowList>
        )}
      </Section>

      <GuardsCard t={t} status={status} />

      <Section title={t.activityTitle}>
        {status.activity.length === 0 ? (
          <EmptyState text={t.activityEmpty} />
        ) : (
          <ul className="activity">
            {status.activity.slice(0, 6).map((e, i) => (
              <li key={`${e.t}-${i}`} className="activity-row">
                <span className="activity-time">{relTime(e.t, t)}</span>
                <span className="activity-text">{e.text}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </>
  );
}

export function relTime(epochSecs: number, t: (typeof dicts)["en"]): string {
  const delta = Math.floor(Date.now() / 1000) - epochSecs;
  if (delta < 60) return t.justNow;
  return t.ago(t.dur(delta));
}

function GuardsCard({ t, status }: { t: (typeof dicts)["en"]; status: Status }) {
  const batteryTripped =
    status.cutout_latched &&
    status.battery_percent !== null &&
    status.battery_percent <= status.battery_floor_percent + 5;
  const thermalTripped = status.cutout_latched && !batteryTripped;
  const guards: [string, string, "ok" | "tripped" | "active" | "off"][] = [
    [t.guardBattery, `${status.battery_floor_percent}%`, batteryTripped ? "tripped" : "ok"],
    [
      t.guardThermal,
      `${Math.round(status.thermal_threshold_celsius)}°C`,
      thermalTripped ? "tripped" : "ok",
    ],
    [t.guardMaxHold, t.maxHoldHours(status.max_hold_hours), "ok"],
    [t.guardClamshell, "", status.clamshell_active ? "active" : "off"],
  ];
  const badge = { ok: t.guardOk, tripped: t.guardTripped, active: t.guardActive, off: t.guardOff };
  return (
    <Section title={t.guardsTitle}>
      <ul className="guards">
        {guards.map(([name, value, state]) => (
          <li key={name} className={`guard ${state}`}>
            <span>{name}</span>
            <span className="guard-right">
              {value && <span className="muted">{value}</span>}
              <span
                className={`badge ${
                  state === "tripped" ? "abandoned" : state === "off" ? "neutral" : "running"
                }`}
              >
                {badge[state]}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

export function Spark({ data }: { data: number[] }) {
  const pts = data.filter((v) => Number.isFinite(v));
  if (pts.length < 2) return null;
  const min = Math.min(...pts);
  const range = Math.max(...pts) - min;
  const w = 60;
  const h = 14;
  // A flat series sits mid-height so it reads as "steady", not as a stray rule.
  const y = (v: number) => (range === 0 ? h / 2 : h - 1 - ((v - min) / range) * (h - 2));
  const path = pts
    .map((v, i) => `${((i / (pts.length - 1)) * w).toFixed(1)},${y(v).toFixed(1)}`)
    .join(" ");
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden>
      <polyline points={path} fill="none" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}
