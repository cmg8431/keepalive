import { useEffect, useState } from "react";
import { dicts, initialLang, type Lang } from "./i18n";

type Session = {
  id: string;
  source: string;
  label: string | null;
  active_secs: number;
  expires_in_secs: number;
};
type Managed = {
  name: string;
  dir: string;
  cmd: string;
  status: "running" | "abandoned";
  respawn_count: number;
};
type Status = {
  ok: boolean;
  awake: boolean;
  sessions: Session[];
  managed: Managed[];
  battery_percent: number | null;
  on_ac_power: boolean;
  temperature_celsius: number | null;
  lid_closed: boolean;
  cutout_latched: boolean;
  clamshell_active: boolean;
  held_for_secs: number | null;
  activity: { t: number; text: string }[];
  projects: string[];
  battery_floor_percent: number;
  thermal_threshold_celsius: number;
  max_hold_hours: number;
};
type ProviderState =
  | "not_installed"
  | "installing"
  | "waiting_for_login"
  | { connected: { ip: string } }
  | { failed: { error: string } };
type Provider = {
  id: string;
  label: string;
  description: string;
  recommended: boolean;
  state: ProviderState;
};
type Setup = {
  providers: Provider[];
  ntfy_topic: string;
  heartbeat_minutes: number;
  clamshell_ready: boolean;
  dashboard_url: string | null;
  dashboard_qr: string | null;
  ntfy_url: string | null;
  ntfy_qr: string | null;
};

async function post(path: string, body?: unknown) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  return res.json();
}

const isPanel = new URLSearchParams(location.search).has("panel");

export default function App() {
  const [lang, setLang] = useState<Lang>(initialLang());
  const [tab, setTab] = useState<"status" | "settings">(
    location.hash === "#settings" ? "settings" : "status",
  );
  const [status, setStatus] = useState<Status | null>(null);
  const [hist, setHist] = useState<{ temp: number[]; batt: number[] }>({ temp: [], batt: [] });
  const [error, setError] = useState<string | null>(null);
  const t = dicts[lang];

  const track = (s: Status) => {
    setStatus(s);
    setHist((h) => ({
      temp: [...h.temp, s.temperature_celsius ?? NaN].slice(-60),
      batt: [...h.batt, s.battery_percent ?? NaN].slice(-60),
    }));
  };

  useEffect(() => {
    document.body.classList.toggle("panel", isPanel);
  }, []);

  useEffect(() => {
    localStorage.setItem("keepalive-lang", lang);
  }, [lang]);

  useEffect(() => {
    let source: EventSource | null = null;
    fetch("/api/status")
      .then((r) => r.json())
      .then(track)
      .catch(() => setError("unreachable"));
    source = new EventSource("/api/events");
    source.onmessage = (e) => {
      setError(null);
      track(JSON.parse(e.data));
    };
    source.onerror = () => setError("unreachable");
    return () => source?.close();
  }, []);

  const refresh = async () => {
    try {
      setStatus(await fetch("/api/status").then((r) => r.json()));
    } catch {
      /* SSE will recover */
    }
  };

  return (
    <main className="wrap">
      <nav className="tabs">
        <button className={tab === "status" ? "tab active" : "tab"} onClick={() => setTab("status")}>
          {t.tabStatus}
        </button>
        <button
          className={tab === "settings" ? "tab active" : "tab"}
          onClick={() => setTab("settings")}
        >
          {t.tabSettings}
        </button>
        <div className="tabs-spacer" />
        <span className="brand">keepalive</span>
      </nav>
      {error && <p className="error">{t.daemonUnreachable}</p>}
      {tab === "status" ? (
        <StatusView t={t} status={status} hist={hist} refresh={refresh} />
      ) : (
        <SettingsView t={t} lang={lang} setLang={setLang} />
      )}
    </main>
  );
}

function holdName(s: Session, t: (typeof dicts)["en"]): string {
  if (s.label) return s.label;
  if (s.source === "manual") return t.holdsTitle;
  return `${s.source} · ${s.id.slice(0, 8)}`;
}

function StatusView({
  t,
  status,
  hist,
  refresh,
}: {
  t: (typeof dicts)["en"];
  status: Status | null;
  hist: { temp: number[]; batt: number[] };
  refresh: () => Promise<void>;
}) {
  const [spawnDir, setSpawnDir] = useState("");
  const [copiedAttach, setCopiedAttach] = useState<string | null>(null);
  const [openLog, setOpenLog] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  if (!status) return <p className="muted">{t.connecting}</p>;

  const copyAttach = (name: string) => {
    navigator.clipboard.writeText(`tmux attach -t ${name}`);
    setCopiedAttach(name);
    setTimeout(() => setCopiedAttach(null), 1500);
  };

  const toggleLog = async (name: string) => {
    if (openLog === name) {
      setOpenLog(null);
      return;
    }
    const res = await post("/api/tail", { name });
    setLogText(res.ok ? res.output : (res.error ?? ""));
    setOpenLog(name);
  };

  const act = async (fn: () => Promise<unknown>) => {
    await fn();
    await refresh();
  };

  const labels = [...new Set(status.sessions.map((s) => holdName(s, t)))];
  const heroDetail = status.cutout_latched
    ? t.cutoutDetail
    : status.awake
      ? labels.length === 1
        ? t.awakeReasonOne(labels[0])
        : t.awakeReasonMany(status.sessions.length)
      : t.idleDetail;
  const nextExpiry =
    status.sessions.length > 0 ? Math.min(...status.sessions.map((s) => s.expires_in_secs)) : null;

  return (
    <>
      <header className={`hero ${status.awake ? "awake" : "idle"}`}>
        <div className="pulse" />
        <div>
          <h1>
            {status.cutout_latched ? t.cutoutLatched : status.awake ? t.awake : t.sleepingNormally}
            {status.awake && status.clamshell_active && (
              <span className="badge running">{t.lidSafe}</span>
            )}
          </h1>
          <p className="muted small">
            {heroDetail}
            {status.awake && status.held_for_secs != null && status.held_for_secs >= 60 && (
              <> · {t.heldFor(t.dur(status.held_for_secs))}</>
            )}
            {nextExpiry !== null && <> · {t.nextRelease(t.dur(nextExpiry))}</>}
          </p>
        </div>
      </header>

      <section className="stats">
        <Stat
          label={t.battery}
          value={
            status.battery_percent === null
              ? "—"
              : `${status.battery_percent}%${status.on_ac_power ? " ⚡" : ""}`
          }
          sub={status.on_ac_power ? t.charging : t.batteryFloor(status.battery_floor_percent)}
          warn={
            status.battery_percent !== null &&
            !status.on_ac_power &&
            status.battery_percent <= status.battery_floor_percent + 5
          }
          spark={hist.batt}
        />
        <Stat
          label={t.temp}
          value={
            status.temperature_celsius === null ? "—" : `${status.temperature_celsius.toFixed(0)}°C`
          }
          sub={t.thermalCutout(Math.round(status.thermal_threshold_celsius))}
          warn={
            status.temperature_celsius !== null &&
            status.temperature_celsius >= status.thermal_threshold_celsius - 5
          }
          spark={hist.temp}
        />
        <Stat
          label={t.lid}
          value={status.lid_closed ? t.lidClosed : t.lidOpen}
          sub={status.clamshell_active ? t.lidHeld : ""}
        />
      </section>

      <section className="card">
        <h2>{t.holdsTitle}</h2>
        {status.sessions.length === 0 && <p className="muted small">{t.holdsEmpty}</p>}
        <ul>
          {status.sessions.map((s) => (
            <li key={s.id} className="session">
              <div>
                <div className="row">
                  <strong>{holdName(s, t)}</strong>
                  <span className="badge neutral">{s.source}</span>
                </div>
                <div className="session-meta">{t.activeFor(t.dur(s.active_secs))}</div>
              </div>
              <div className="row">
                <div className="session-right">{t.ttlLeft(t.dur(s.expires_in_secs))}</div>
                <button
                  className="danger small"
                  onClick={() => act(() => post("/api/release", { id: s.id }))}
                >
                  {t.release}
                </button>
              </div>
            </li>
          ))}
        </ul>
        <div className="row">
          {(
            [
              [30, t.hold30m],
              [60, t.hold1h],
              [180, t.hold3h],
              [480, t.hold8h],
            ] as [number, string][]
          ).map(([minutes, label]) => (
            <button key={minutes} onClick={() => act(() => post("/api/hold", { minutes }))}>
              {label}
            </button>
          ))}
          <div className="tabs-spacer" />
          <button
            className="danger"
            disabled={status.sessions.length === 0}
            onClick={() => act(() => post("/api/sleep"))}
          >
            {t.letSleep}
          </button>
        </div>
      </section>

      <GuardsCard t={t} status={status} />

      <section className="card">
        <h2>{t.sessionsTitle}</h2>
        {status.managed.length === 0 && (
          <>
            <p className="muted">{t.sessionsEmpty}</p>
            <p className="muted small">{t.sessionsHint}</p>
          </>
        )}
        <ul>
          {status.managed.map((m) => (
            <li key={m.name} className={openLog === m.name ? "session open" : "session"}>
              <div>
                <div className="row">
                  <strong>{m.dir.split("/").pop()}</strong>
                  <span className={`badge ${m.status}`}>
                    {m.status === "running" ? t.running : t.abandoned}
                  </span>
                  {m.respawn_count > 0 && (
                    <span className="muted small">{t.revived(m.respawn_count)}</span>
                  )}
                </div>
                <div className="session-meta">
                  {m.name} · {m.cmd}
                </div>
              </div>
              <div className="row">
                <button className="small" onClick={() => toggleLog(m.name)}>
                  {openLog === m.name ? t.hideLog : t.viewLog}
                </button>
                <button
                  className="small"
                  title={`tmux attach -t ${m.name}`}
                  onClick={() => copyAttach(m.name)}
                >
                  {copiedAttach === m.name ? t.copied : "tmux"}
                </button>
                <button
                  className="danger small"
                  onClick={() => act(() => post("/api/kill", { name: m.name }))}
                >
                  {t.kill}
                </button>
              </div>
              {openLog === m.name && <pre className="log">{logText || t.logEmpty}</pre>}
            </li>
          ))}
        </ul>
        <div className="spawn">
          {status.projects.length > 0 ? (
            <>
              <select value={spawnDir} onChange={(e) => setSpawnDir(e.target.value)}>
                <option value="">{t.chooseProject}</option>
                {status.projects.map((p) => (
                  <option key={p} value={p}>
                    {p.split("/").pop()}
                  </option>
                ))}
              </select>
              <button
                disabled={!spawnDir}
                onClick={() => act(() => post("/api/spawn", { dir: spawnDir }))}
              >
                {t.newSession}
              </button>
            </>
          ) : (
            <p className="muted small">{t.noProjects}</p>
          )}
        </div>
      </section>

      <section className="card">
        <h2>{t.activityTitle}</h2>
        {status.activity.length === 0 && <p className="muted small">{t.activityEmpty}</p>}
        <ul className="activity">
          {status.activity.slice(0, 8).map((e, i) => (
            <li key={`${e.t}-${i}`} className="activity-row">
              <span className="activity-time">{relTime(e.t, t)}</span>
              <span className="activity-text">{e.text}</span>
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}

function relTime(epochSecs: number, t: (typeof dicts)["en"]): string {
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
    <section className="card">
      <h2>{t.guardsTitle}</h2>
      <ul>
        {guards.map(([name, value, state]) => (
          <li key={name} className="session">
            <span>{name}</span>
            <span className="row">
              {value && <span className="session-right">{value}</span>}
              <span className={`badge ${state === "tripped" ? "abandoned" : state === "off" ? "neutral" : "running"}`}>
                {badge[state]}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function providerStateLabel(state: ProviderState, t: (typeof dicts)["en"]): string {
  if (typeof state === "object") {
    if ("connected" in state) return `${t.stateConnected} · ${state.connected.ip}`;
    return `${t.stateFailed}: ${state.failed.error}`;
  }
  return {
    not_installed: t.stateNotInstalled,
    installing: t.stateInstalling,
    waiting_for_login: t.stateWaitingLogin,
  }[state];
}

function SettingsView({
  t,
  lang,
  setLang,
}: {
  t: (typeof dicts)["en"];
  lang: Lang;
  setLang: (l: Lang) => void;
}) {
  const [setup, setSetup] = useState<Setup | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [testSent, setTestSent] = useState(false);

  const sendTest = async () => {
    await post("/api/notify-test");
    setTestSent(true);
    setTimeout(() => setTestSent(false), 2000);
  };

  const load = () =>
    fetch("/api/setup")
      .then((r) => r.json())
      .then(setSetup)
      .catch(() => {});

  useEffect(() => {
    load();
    const timer = setInterval(load, 3000);
    return () => clearInterval(timer);
  }, []);

  if (!setup) return <p className="muted">{t.connecting}</p>;

  const chooseProvider = async (id: string) => {
    const res = await post("/api/setup/provider", { id });
    setMessage(res.message ?? res.error ?? null);
    load();
  };

  const toggleNtfy = async (enable: boolean) => {
    await post("/api/setup/ntfy", { enable });
    load();
  };

  const clamshellCmd = "sudo keepalive clamshell-setup";

  return (
    <>
      <section className="card">
        <h2>{t.settingsLanguage}</h2>
        <div className="row">
          <button className={lang === "ko" ? "selected" : ""} onClick={() => setLang("ko")}>
            한국어
          </button>
          <button className={lang === "en" ? "selected" : ""} onClick={() => setLang("en")}>
            English
          </button>
        </div>
      </section>

      <section className="card">
        <h2>{t.settingsConnection}</h2>
        <p className="muted small">{t.settingsConnectionDesc}</p>
        {setup.providers.map((p) => {
          const connected = typeof p.state === "object" && "connected" in p.state;
          return (
            <div key={p.id} className="provider">
              <div>
                <div className="row">
                  <strong>{p.label}</strong>
                  {p.recommended && <span className="badge running">{t.recommended}</span>}
                </div>
                <div className="muted small">{p.description}</div>
                <div className={connected ? "small ok" : "small muted"}>
                  {providerStateLabel(p.state, t)}
                </div>
              </div>
              {!connected && (
                <button onClick={() => chooseProvider(p.id)}>
                  {p.state === "not_installed" ? t.install : t.connect}
                </button>
              )}
            </div>
          );
        })}
        {message && <p className="muted small">{message}</p>}
        {setup.dashboard_qr && (
          <div className="qr-block">
            <div className="qr" dangerouslySetInnerHTML={{ __html: setup.dashboard_qr }} />
            <p className="muted small">
              {t.scanDashboard}
              <br />
              <span className="mono">{setup.dashboard_url}</span>
            </p>
          </div>
        )}
      </section>

      <section className="card">
        <h2>{t.settingsNotify}</h2>
        <p className="muted small">{t.settingsNotifyDesc}</p>
        <div className="row">
          {setup.ntfy_topic ? (
            <>
              <span className="badge running">{t.notifyOn}</span>
              <span className="mono small">{setup.ntfy_topic}</span>
              <button className="small" onClick={sendTest}>
                {testSent ? t.notifySent : t.notifyTest}
              </button>
              <button className="danger small" onClick={() => toggleNtfy(false)}>
                {t.notifyDisable}
              </button>
            </>
          ) : (
            <button onClick={() => toggleNtfy(true)}>{t.notifyEnable}</button>
          )}
        </div>
        {setup.ntfy_qr && (
          <div className="qr-block">
            <div className="qr" dangerouslySetInnerHTML={{ __html: setup.ntfy_qr }} />
            <p className="muted small">
              {t.scanNtfy}
              <br />
              {t.wakeHint}
            </p>
          </div>
        )}
      </section>

      <section className="card">
        <h2>{t.settingsClamshell}</h2>
        {setup.clamshell_ready ? (
          <p className="ok small">{t.clamshellReady}</p>
        ) : (
          <>
            <p className="muted small">{t.clamshellNotReady}</p>
            <div className="row">
              <span className="mono small">{clamshellCmd}</span>
              <button
                className="small"
                onClick={() => {
                  navigator.clipboard.writeText(clamshellCmd);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                }}
              >
                {copied ? t.copied : t.copy}
              </button>
            </div>
          </>
        )}
      </section>
    </>
  );
}

function Stat({
  label,
  value,
  sub,
  warn,
  spark,
}: {
  label: string;
  value: string;
  sub?: string;
  warn?: boolean;
  spark?: number[];
}) {
  return (
    <div className={warn ? "stat warn" : "stat"}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
      {sub ? <div className="stat-sub">{sub}</div> : null}
      {spark ? <Spark data={spark} /> : null}
    </div>
  );
}

function Spark({ data }: { data: number[] }) {
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
