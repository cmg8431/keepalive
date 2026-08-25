import { useEffect, useState } from "react";
import { FolderGit2, Globe, SquareTerminal } from "lucide-react";
import { post } from "../api";
import type { dicts } from "../i18n";
import type { BrowseResult, PortInfo, Projects, Status } from "../types";
import { EmptyState, IconTile, Row, RowList, Section, SkeletonCard } from "../components";
import { relTime } from "./status-view";

export function SessionsView({
  t,
  status,
  refresh,
  onOpenTerminal,
}: {
  t: (typeof dicts)["en"];
  status: Status | null;
  refresh: () => Promise<void>;
  onOpenTerminal: (name: string) => void;
}) {
  const [openLog, setOpenLog] = useState<string | null>(null);
  const [logText, setLogText] = useState("");
  const [reply, setReply] = useState("");
  if (!status) return <SkeletonCard rows={3} />;

  const act = async (fn: () => Promise<unknown>) => {
    await fn();
    await refresh();
  };

  const loadLog = async (name: string) => {
    const res = await post("/api/tail", { name });
    setLogText(res.ok ? res.output : (res.error ?? ""));
  };

  const toggleLog = async (name: string) => {
    if (openLog === name) {
      setOpenLog(null);
      return;
    }
    await loadLog(name);
    setOpenLog(name);
  };

  /** Answer the agent without opening the full terminal — usually one key. */
  const sendTo = async (name: string, body: { text?: string; key?: string }) => {
    await post("/api/send", { name, ...body });
    setReply("");
    // Give the agent a beat to redraw before re-reading the pane.
    setTimeout(() => loadLog(name), 300);
  };

  return (
    <>
      <Section title={t.sessionsTitle}>
        {status.managed.length === 0 ? (
          <EmptyState text={t.sessionsHint} />
        ) : (
          <RowList>
            {status.managed.map((m) => (
              <li key={m.name} className="row-card open">
                <span className="tile">
                  <SquareTerminal size={16} />
                </span>
                <div className="row-body">
                  <strong>
                    {m.dir.split("/").pop()}
                    {m.waiting && <span className="badge waiting">{t.waitingBadge}</span>}
                  </strong>
                  <span className="muted small">
                    {m.waiting
                      ? t.waitingHint
                      : `${m.status === "running" ? t.running : t.abandoned}${
                          m.respawn_count > 0 ? ` · ${t.revived(m.respawn_count)}` : ""
                        }${activityLabel(m.idle_secs, t)} · ${m.cmd}`}
                  </span>
                </div>
                <button className="primary small" onClick={() => onOpenTerminal(m.name)}>
                  {t.openTerminal}
                </button>
                <button className="small" onClick={() => toggleLog(m.name)}>
                  {openLog === m.name ? t.hideLog : t.viewLog}
                </button>
                <button
                  className="danger small"
                  onClick={() => act(() => post("/api/kill", { name: m.name }))}
                >
                  {t.kill}
                </button>
                {openLog === m.name && (
                  <>
                    <pre className="log">{logText || t.logEmpty}</pre>
                    <div className="reply">
                      {["y", "n", "1", "2", "3"].map((k) => (
                        <button key={k} className="key" onClick={() => sendTo(m.name, { text: k })}>
                          {k}
                        </button>
                      ))}
                      <button className="key" onClick={() => sendTo(m.name, { key: "Enter" })}>
                        ⏎
                      </button>
                      <button className="key" onClick={() => sendTo(m.name, { key: "Escape" })}>
                        esc
                      </button>
                    </div>
                    <form
                      className="reply-form"
                      onSubmit={(e) => {
                        e.preventDefault();
                        if (reply) sendTo(m.name, { text: reply, key: "Enter" });
                      }}
                    >
                      <input
                        value={reply}
                        onChange={(e) => setReply(e.target.value)}
                        placeholder={t.replyPlaceholder}
                      />
                      <button className="small" type="submit" disabled={!reply}>
                        {t.send}
                      </button>
                    </form>
                  </>
                )}
              </li>
            ))}
          </RowList>
        )}
      </Section>

      <PreviewCard t={t} />

      <ProjectsCard t={t} onSpawn={(dir) => act(() => post("/api/spawn", { dir }))} />
    </>
  );
}

/** "streaming" while output flows, "quiet for 3m" once it stops. */
function activityLabel(idleSecs: number | null, t: (typeof dicts)["en"]): string {
  if (idleSecs === null) return "";
  return ` · ${idleSecs < 30 ? t.activeNow : t.idleFor(t.dur(idleSecs))}`;
}

/**
 * Dev servers listening on the Mac, one tap away from a live preview on the
 * phone. Hidden entirely when nothing is listening — this card earns its
 * space only while something is running.
 */
function PreviewCard({ t }: { t: (typeof dicts)["en"] }) {
  const [ports, setPorts] = useState<PortInfo[]>([]);

  useEffect(() => {
    const load = () =>
      fetch("/api/ports")
        .then((r) => r.json())
        .then((res) => setPorts(res.ports ?? []))
        .catch(() => undefined);
    load();
    const timer = setInterval(load, 10000);
    return () => clearInterval(timer);
  }, []);

  if (ports.length === 0) return null;
  const onMac = ["localhost", "127.0.0.1"].includes(location.hostname);

  return (
    <Section title={t.previewTitle}>
      <RowList>
        {ports.map((p) => (
          <li key={p.port} className="row-card">
            <span className="tile">
              <Globe size={16} />
            </span>
            <div className="row-body">
              <strong className="mono">:{p.port}</strong>
              <span className="muted small">
                {p.process}
                {p.local_only && !onMac ? ` · ${t.previewLocalOnly}` : ""}
              </span>
            </div>
            {(onMac || !p.local_only) && (
              <a
                className="button small"
                href={`http://${location.hostname}:${p.port}`}
                target="_blank"
                rel="noreferrer"
              >
                {t.previewOpen}
              </a>
            )}
          </li>
        ))}
      </RowList>
    </Section>
  );
}

/**
 * Projects you can start a session in. The list fills itself from directories
 * agents have actually worked in, so the phone is useful before anyone edits a
 * config file; the browser below is for pinning anything else.
 */
function ProjectsCard({ t, onSpawn }: { t: (typeof dicts)["en"]; onSpawn: (dir: string) => void }) {
  const [projects, setProjects] = useState<Projects | null>(null);
  const [browsing, setBrowsing] = useState<BrowseResult | null>(null);

  const load = () =>
    fetch("/api/projects")
      .then((r) => r.json())
      .then(setProjects)
      .catch(() => undefined);

  useEffect(() => {
    load();
  }, []);

  const openBrowser = async (path?: string) => {
    const res = await fetch(`/api/browse${path ? `?path=${encodeURIComponent(path)}` : ""}`).then(
      (r) => r.json(),
    );
    if (res.ok) setBrowsing(res);
  };

  const pin = async (dir: string) => {
    await post("/api/projects/add", { dir });
    setBrowsing(null);
    await load();
  };

  const remove = async (dir: string) => {
    await post("/api/projects/remove", { dir });
    await load();
  };

  const pinned = projects?.allowlist ?? [];
  // A pinned directory that is also recent should appear once, as pinned.
  const recent = (projects?.recent ?? []).filter((p) => !pinned.includes(p.dir));

  const entry = (dir: string, name: string, meta: string, isPinned: boolean) => (
    <Row
      key={dir}
      icon={
        <IconTile tone="cobalt">
          <FolderGit2 size={16} />
        </IconTile>
      }
      title={name}
      detail={meta}
      actions={
        <>
          <button className="primary small" onClick={() => onSpawn(dir)}>
            {t.startHere}
          </button>
          <button className="small" onClick={() => (isPinned ? remove(dir) : pin(dir))}>
            {isPinned ? t.projectsRemove : t.projectsPin}
          </button>
        </>
      }
    />
  );

  return (
    <Section
      title={t.projectsTitle}
      aside={
        <button className="small" onClick={() => openBrowser()}>
          {t.projectsBrowse}
        </button>
      }
    >
      {pinned.length === 0 && recent.length === 0 && <EmptyState text={t.projectsEmpty} />}

      {pinned.length > 0 && (
        <RowList>{pinned.map((dir) => entry(dir, dir.split("/").pop() ?? dir, dir, true))}</RowList>
      )}

      {recent.length > 0 && (
        <>
          <h3 className="subsection">{t.projectsRecent}</h3>
          <RowList>
            {recent.map((p) =>
              entry(p.dir, p.name, `${p.source} · ${relTime(p.last_seen, t)}`, false),
            )}
          </RowList>
        </>
      )}

      {browsing && (
        <div className="browser">
          <div className="group-head">
            <span className="project-path">{browsing.dir}</span>
            <button className="small" onClick={() => setBrowsing(null)}>
              {t.termClose}
            </button>
          </div>
          <div className="reply">
            {browsing.parent && (
              <button className="small" onClick={() => openBrowser(browsing.parent!)}>
                ↑ {t.browseUp}
              </button>
            )}
            <button className="primary small" onClick={() => pin(browsing.dir)}>
              {t.browseHere}
            </button>
          </div>
          {browsing.entries.length === 0 && <p className="muted small">{t.browseEmpty}</p>}
          <ul className="browse-list">
            {browsing.entries.map((e) => (
              <li key={e.dir}>
                <button className="browse-row" onClick={() => openBrowser(e.dir)}>
                  {e.is_repo && <FolderGit2 size={13} />}
                  {e.name}
                </button>
                <button className="small" onClick={() => pin(e.dir)}>
                  {t.projectsAdd}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </Section>
  );
}
