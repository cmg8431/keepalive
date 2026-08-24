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
  projects: string[];
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
  const [error, setError] = useState<string | null>(null);
  const t = dicts[lang];

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
      .then(setStatus)
      .catch(() => setError("unreachable"));
    source = new EventSource("/api/events");
    source.onmessage = (e) => {
      setError(null);
      setStatus(JSON.parse(e.data));
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
        <StatusView t={t} status={status} refresh={refresh} />
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
  refresh,
}: {
  t: (typeof dicts)["en"];
  status: Status | null;
  refresh: () => Promise<void>;
}) {
  const [spawnDir, setSpawnDir] = useState("");
  if (!status) return <p className="muted">{t.connecting}</p>;

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
          <p className="muted">{heroDetail}</p>
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
        />
        <Stat
          label={t.temp}
          value={
            status.temperature_celsius === null ? "—" : `${status.temperature_celsius.toFixed(0)}°C`
          }
        />
        <Stat label={t.lid} value={status.lid_closed ? t.lidClosed : t.lidOpen} />
      </section>

      <section className="card">
        <h2>{t.holdsTitle}</h2>
        {status.sessions.length === 0 && <p className="muted">{t.holdsEmpty}</p>}
        <ul>
          {status.sessions.map((s) => (
            <li key={s.id} className="session">
              <div>
                <strong>{holdName(s, t)}</strong>
                <span className="badge running">{s.source}</span>
                <div className="muted small">
                  {t.activeFor(Math.max(1, Math.round(s.active_secs / 60)))} ·{" "}
                  {t.ttlLeft(Math.round(s.expires_in_secs / 60))}
                </div>
              </div>
            </li>
          ))}
        </ul>
        <div className="row">
          <button onClick={() => act(() => post("/api/hold", { minutes: 60 }))}>{t.hold1h}</button>
          <button onClick={() => act(() => post("/api/hold", { minutes: 180 }))}>{t.hold3h}</button>
          <button className="danger" onClick={() => act(() => post("/api/sleep"))}>
            {t.letSleep}
          </button>
        </div>
      </section>

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
            <li key={m.name} className="session">
              <div>
                <strong>{m.dir.split("/").pop()}</strong>
                <span className={`badge ${m.status}`}>
                  {m.status === "running" ? t.running : t.abandoned}
                </span>
                {m.respawn_count > 0 && (
                  <span className="muted small"> {t.revived(m.respawn_count)}</span>
                )}
                <div className="muted small">
                  {m.name} · {m.cmd}
                </div>
              </div>
              <button
                className="danger small"
                onClick={() => act(() => post("/api/kill", { name: m.name }))}
              >
                {t.kill}
              </button>
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
    </>
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
                <strong>{p.label}</strong>
                {p.recommended && <span className="badge running">{t.recommended}</span>}
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

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <div className="stat-value">{value}</div>
      <div className="muted small">{label}</div>
    </div>
  );
}
