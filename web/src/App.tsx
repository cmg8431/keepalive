import { useEffect, useState, type CSSProperties } from "react";
import { Activity, ExternalLink, RotateCw, Settings as SettingsIcon, SquareTerminal } from "lucide-react";
import { dicts, initialLang, type Lang } from "./i18n";
import { deepLinkedSession, isPanel, post } from "./api";
import { statusLine } from "./status-text";
import type { Status, Tab } from "./types";
import { StatusView } from "./views/status-view";
import { SessionsView } from "./views/sessions-view";
import { SettingsView } from "./views/settings-view";
import { SessionTerminal } from "./Terminal";
import "./tokens.css";
import "./app.css";

export default function App() {
  const [lang, setLang] = useState<Lang>(initialLang());
  const [terminal, setTerminal] = useState<string | null>(deepLinkedSession);
  const [tab, setTab] = useState<Tab>(
    location.hash === "#settings" ? "settings" : deepLinkedSession() ? "sessions" : "status",
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

  const toggleWake = async () => {
    await (status?.awake ? post("/api/sleep") : post("/api/hold", { minutes: 60 }));
    await refresh();
  };

  const closeTerminal = () => {
    setTerminal(null);
    // Drop the deep-link path so a refresh doesn't reopen what was just closed.
    if (deepLinkedSession()) history.replaceState(null, "", "/");
  };

  return (
    <main className="wrap">
      <AppBar t={t} status={status} onToggle={toggleWake} onRefresh={refresh} />
      <Nav t={t} tab={tab} setTab={setTab} />
      <div className="content" key={terminal ? `term-${terminal}` : tab}>
        {error && <p className="error">{t.daemonUnreachable}</p>}
        {terminal && (
          <SessionTerminal
            name={terminal}
            dir={status?.managed.find((m) => m.name === terminal)?.dir}
            t={t}
            onClose={closeTerminal}
          />
        )}
        {!terminal && tab === "status" && (
          <StatusView
            t={t}
            status={status}
            hist={hist}
            refresh={refresh}
            setTab={setTab}
            onOpenTerminal={setTerminal}
          />
        )}
        {tab === "sessions" && (
          <SessionsView t={t} status={status} refresh={refresh} onOpenTerminal={setTerminal} />
        )}
        {!terminal && tab === "settings" && (
          <SettingsView t={t} lang={lang} setLang={setLang} status={status} />
        )}
      </div>
    </main>
  );
}

/* The bar answers "is it awake, and can I change that" without scrolling:
   state on the left, the one switch that matters next to it. */
function AppBar({
  t,
  status,
  onToggle,
  onRefresh,
}: {
  t: (typeof dicts)["en"];
  status: Status | null;
  onToggle: () => void;
  onRefresh: () => void;
}) {
  const awake = status?.awake ?? false;
  const [title, detail] = statusLine(status, t);
  const tone = status?.cutout_latched ? "cutout" : awake ? "awake" : "idle";
  return (
    <header className={`appbar ${tone}`}>
      <button
        type="button"
        role="switch"
        aria-checked={awake}
        aria-label={t.wakeSwitch}
        className={`switch ${awake ? "on" : ""}`}
        disabled={!status}
        onClick={onToggle}
      >
        <span className="knob" />
      </button>
      <div className="appbar-text">
        <strong>{title}</strong>
        {detail && <span className="muted small">{detail}</span>}
      </div>
      <div className="appbar-actions">
        <button className="icon-btn" title={t.refresh} aria-label={t.refresh} onClick={onRefresh}>
          <RotateCw size={15} />
        </button>
        {isPanel && (
          <button
            className="icon-btn"
            title={t.openBrowser}
            aria-label={t.openBrowser}
            onClick={() => post("/api/open-browser")}
          >
            <ExternalLink size={15} />
          </button>
        )}
      </div>
    </header>
  );
}

function Nav({
  t,
  tab,
  setTab,
}: {
  t: (typeof dicts)["en"];
  tab: Tab;
  setTab: (tab: Tab) => void;
}) {
  const items = [
    { id: "status" as const, icon: Activity, label: t.tabStatus },
    { id: "sessions" as const, icon: SquareTerminal, label: t.tabSessions },
    { id: "settings" as const, icon: SettingsIcon, label: t.tabSettings },
  ];
  const active = items.findIndex((i) => i.id === tab);
  return (
    <nav className="nav" style={{ "--active": active } as CSSProperties}>
      <span className="nav-indicator" aria-hidden />
      {items.map(({ id, icon: Icon, label }) => (
        <button
          key={id}
          className={tab === id ? "nav-item active" : "nav-item"}
          aria-current={tab === id}
          onClick={() => setTab(id)}
        >
          <Icon size={17} strokeWidth={1.9} />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );
}

