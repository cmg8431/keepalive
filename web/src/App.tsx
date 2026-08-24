import { useEffect, useState } from "react";

type Session = { id: string; source: string; expires_in_secs: number };
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

async function post(path: string, body?: unknown) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  return res.json();
}

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [spawnDir, setSpawnDir] = useState("");

  useEffect(() => {
    let source: EventSource | null = null;
    const connect = () => {
      source = new EventSource("/api/events");
      source.onmessage = (e) => {
        setError(null);
        setStatus(JSON.parse(e.data));
      };
      source.onerror = () => setError("daemon unreachable — retrying");
    };
    fetch("/api/status")
      .then((r) => r.json())
      .then(setStatus)
      .catch(() => setError("daemon unreachable"));
    connect();
    return () => source?.close();
  }, []);

  const act = async (fn: () => Promise<unknown>) => {
    const result = (await fn()) as { ok?: boolean; error?: string };
    if (result && result.ok === false) setError(result.error ?? "request failed");
    const fresh = await fetch("/api/status").then((r) => r.json());
    setStatus(fresh);
  };

  if (!status)
    return (
      <main className="wrap">
        <h1>keepalive</h1>
        <p className="muted">{error ?? "connecting…"}</p>
      </main>
    );

  return (
    <main className="wrap">
      <header className={`hero ${status.awake ? "awake" : "idle"}`}>
        <div className="pulse" />
        <div>
          <h1>{status.awake ? "Awake" : "Sleeping normally"}</h1>
          <p className="muted">
            {status.awake
              ? status.clamshell_active
                ? "held awake — lid-close safe"
                : "held awake — idle sleep blocked"
              : status.cutout_latched
                ? "safety cutout latched — waiting for recovery"
                : "no active holds"}
          </p>
        </div>
      </header>

      {error && <p className="error">{error}</p>}

      <section className="stats">
        <Stat
          label="battery"
          value={
            status.battery_percent === null
              ? "—"
              : `${status.battery_percent}%${status.on_ac_power ? " ⚡" : ""}`
          }
        />
        <Stat
          label="temp"
          value={
            status.temperature_celsius === null
              ? "—"
              : `${status.temperature_celsius.toFixed(0)}°C`
          }
        />
        <Stat label="lid" value={status.lid_closed ? "closed" : "open"} />
      </section>

      <section className="card">
        <h2>Wake holds</h2>
        {status.sessions.length === 0 && <p className="muted">none</p>}
        <ul>
          {status.sessions.map((s) => (
            <li key={s.id}>
              <span className="mono">{s.id}</span>
              <span className="muted"> · {s.source} · {Math.round(s.expires_in_secs / 60)}m left</span>
            </li>
          ))}
        </ul>
        <div className="row">
          <button onClick={() => act(() => post("/api/hold", { minutes: 60 }))}>
            Hold 1h
          </button>
          <button onClick={() => act(() => post("/api/hold", { minutes: 180 }))}>
            Hold 3h
          </button>
          <button className="danger" onClick={() => act(() => post("/api/sleep"))}>
            Let it sleep
          </button>
        </div>
      </section>

      <section className="card">
        <h2>Agent sessions</h2>
        {status.managed.length === 0 && <p className="muted">no managed sessions</p>}
        <ul>
          {status.managed.map((m) => (
            <li key={m.name} className="session">
              <div>
                <span className="mono">{m.name}</span>
                <span className={`badge ${m.status}`}>{m.status}</span>
                {m.respawn_count > 0 && (
                  <span className="muted"> revived ×{m.respawn_count}</span>
                )}
                <div className="muted small">{m.dir}</div>
              </div>
              <button
                className="danger small"
                onClick={() => act(() => post("/api/kill", { name: m.name }))}
              >
                kill
              </button>
            </li>
          ))}
        </ul>
        <div className="spawn">
          {status.projects.length > 0 ? (
            <>
              <select value={spawnDir} onChange={(e) => setSpawnDir(e.target.value)}>
                <option value="">choose a project…</option>
                {status.projects.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
              <button
                disabled={!spawnDir}
                onClick={() => act(() => post("/api/spawn", { dir: spawnDir }))}
              >
                New session
              </button>
            </>
          ) : (
            <p className="muted small">
              add <span className="mono">projects = ["/path"]</span> to the config to
              spawn sessions from here
            </p>
          )}
        </div>
      </section>
    </main>
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
