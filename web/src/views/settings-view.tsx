import { useEffect, useState } from "react";
import { Bell, Check, FolderGit2, Laptop, Plug, Smartphone, SquareTerminal } from "lucide-react";
import { post } from "../api";
import type { dicts, Lang } from "../i18n";
import type { ProviderState, Setup, Status } from "../types";
import { EmptyState, IconTile, Row, RowList, Section, Segmented, SkeletonCard } from "../components";

type Step = {
  id: string;
  icon: typeof Plug;
  title: string;
  desc: string;
  done: boolean;
  action?: { label: string; run: () => void };
};

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

/* Not a first-run wizard: any of these can come undone later (a sudo rule
   wiped by an OS update, Tailscale logged out). Finished steps collapse to a
   single line so the list is only ever as long as the work left. */
function Checklist({ t, steps }: { t: (typeof dicts)["en"]; steps: Step[] }) {
  const pending = steps.filter((s) => !s.done);
  const done = steps.length - pending.length;
  return (
    <Section title={t.sectionSetup} aside={pending.length > 0 && ` · ${t.sectionRemaining(pending.length)}`}>
      {pending.length === 0 ? (
        <EmptyState tone="ok" text={t.setupAllDone} />
      ) : (
        <RowList>
          {pending.map(({ id, icon: Icon, title, desc, action }) => (
            <li key={id} className="row-card">
              <span className="tile">
                <Icon size={16} />
              </span>
              <div className="row-body">
                <strong>{title}</strong>
                <span className="muted small">{desc}</span>
              </div>
              {action && (
                <button className="small" onClick={action.run}>
                  {action.label}
                </button>
              )}
            </li>
          ))}
        </RowList>
      )}
      {done > 0 && pending.length > 0 && (
        <p className="done-line">
          <Check size={12} strokeWidth={3} /> {done}
        </p>
      )}
    </Section>
  );
}

export function SettingsView({
  t,
  lang,
  setLang,
  status,
}: {
  t: (typeof dicts)["en"];
  lang: Lang;
  setLang: (l: Lang) => void;
  status: Status | null;
}) {
  const [setup, setSetup] = useState<Setup | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [testSent, setTestSent] = useState(false);
  const [qr, setQr] = useState<"dashboard" | "ntfy" | null>(null);
  const [httpsError, setHttpsError] = useState<string | null>(null);

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

  if (!setup)
    return (
      <>
        <SkeletonCard rows={3} />
        <SkeletonCard rows={2} />
      </>
    );

  const chooseProvider = async (id: string) => {
    const res = await post("/api/setup/provider", { id });
    setMessage(res.message ?? res.error ?? null);
    load();
  };

  const toggleNtfy = async (enable: boolean) => {
    await post("/api/setup/ntfy", { enable });
    load();
  };

  const toggleHttps = async (enable: boolean) => {
    setHttpsError(null);
    const res = await post("/api/setup/https", { enable });
    // Certificate issuance needs a one-time tailnet admin switch and the CLI
    // says so precisely — show it verbatim rather than a generic failure.
    if (!res.ok) setHttpsError(res.error ?? null);
    load();
  };

  const clamshellCmd = "sudo keepalive clamshell-setup";
  const copyText = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const tailscale = setup.providers.find((p) => p.id === "tailscale");
  const tailscaleIp =
    tailscale && typeof tailscale.state === "object" && "connected" in tailscale.state
      ? tailscale.state.connected.ip
      : null;
  const wired = setup.hooks.filter((h) => h.installed).map((h) => h.name);
  const steps: Step[] = [
    {
      id: "daemon",
      icon: Plug,
      title: t.stepDaemon,
      desc: t.stepDaemonDesc,
      done: status?.ok ?? false,
    },
    {
      id: "hooks",
      icon: SquareTerminal,
      title: t.stepHooks,
      desc: wired.length > 0 ? t.stepHooksSome(wired.join(", ")) : t.stepHooksNone,
      done: wired.length > 0,
      action: { label: t.copy, run: () => copyText("keepalive install") },
    },
    {
      id: "clamshell",
      icon: Laptop,
      title: t.stepClamshell,
      desc: setup.clamshell_ready ? t.stepClamshellDone : t.stepClamshellDesc,
      done: setup.clamshell_ready,
      action: { label: copied ? t.copied : t.copy, run: () => copyText(clamshellCmd) },
    },
    {
      id: "phone",
      icon: Smartphone,
      title: t.stepPhone,
      desc: tailscaleIp ? t.stepPhoneDone(tailscaleIp) : t.stepPhoneDesc,
      done: tailscaleIp !== null,
      action: { label: t.connect, run: () => chooseProvider("tailscale") },
    },
    {
      id: "notify",
      icon: Bell,
      title: t.stepNotify,
      desc: setup.ntfy_topic || t.stepNotifyDesc,
      done: setup.ntfy_topic !== "",
      action: { label: t.notifyEnable, run: () => toggleNtfy(true) },
    },
    {
      id: "projects",
      icon: FolderGit2,
      title: t.stepProjects,
      desc:
        setup.projects.length > 0 ? t.stepProjectsDone(setup.projects.length) : t.stepProjectsDesc,
      done: setup.projects.length > 0,
    },
  ];

  return (
    <>
      <Checklist t={t} steps={steps} />
      {message && <p className="muted small">{message}</p>}

      <Section title={t.settingsConnection}>
        <div className="row-card">
          <span className="tile">
            <Smartphone size={16} />
          </span>
          <div className="row-body">
            <strong>{tailscaleIp ? t.stateConnected : "Tailscale"}</strong>
            <span className="muted small mono">
              {setup.dashboard_url ?? providerStateLabel(tailscale?.state ?? "not_installed", t)}
            </span>
          </div>
          {setup.dashboard_qr ? (
            <button className="small" onClick={() => setQr(qr === "dashboard" ? null : "dashboard")}>
              {qr === "dashboard" ? t.hideQr : t.showQr}
            </button>
          ) : (
            <button className="small" onClick={() => chooseProvider("tailscale")}>
              {t.connect}
            </button>
          )}
        </div>
        {qr === "dashboard" && setup.dashboard_qr && (
          <div className="qr-block">
            <div className="qr" dangerouslySetInnerHTML={{ __html: setup.dashboard_qr }} />
            <p className="muted small">{t.scanDashboard}</p>
          </div>
        )}
        {setup.magic_dns && (
          <div className="row-card">
            <span className="tile">
              <Plug size={16} />
            </span>
            <div className="row-body">
              <strong>{t.settingsHttps}</strong>
              <span className="muted small">
                {setup.https_enabled ? `https://${setup.magic_dns}` : t.settingsHttpsDesc}
              </span>
            </div>
            <button
              className={setup.https_enabled ? "danger small" : "small"}
              onClick={() => toggleHttps(!setup.https_enabled)}
            >
              {setup.https_enabled ? t.httpsDisable : t.httpsEnable}
            </button>
          </div>
        )}
        {httpsError && <p className="error small">{httpsError}</p>}
      </Section>

      <Section title={t.settingsNotify}>
        <div className="row-card">
          <span className="tile">
            <Bell size={16} />
          </span>
          <div className="row-body">
            <strong>{setup.ntfy_topic ? t.notifyOn : t.stepNotify}</strong>
            <span className="muted small mono">{setup.ntfy_topic || t.stepNotifyDesc}</span>
          </div>
          {setup.ntfy_topic ? (
            <>
              <button className="small" onClick={() => setQr(qr === "ntfy" ? null : "ntfy")}>
                {qr === "ntfy" ? t.hideQr : t.showQr}
              </button>
              <button className="small" onClick={sendTest}>
                {testSent ? t.notifySent : t.notifyTest}
              </button>
              <button className="danger small" onClick={() => toggleNtfy(false)}>
                {t.notifyDisable}
              </button>
            </>
          ) : (
            <button className="small" onClick={() => toggleNtfy(true)}>
              {t.notifyEnable}
            </button>
          )}
        </div>
        {qr === "ntfy" && setup.ntfy_qr && (
          <div className="qr-block">
            <div className="qr" dangerouslySetInnerHTML={{ __html: setup.ntfy_qr }} />
            <p className="muted small">
              {t.scanNtfy}
              <br />
              {t.wakeHint}
            </p>
          </div>
        )}
      </Section>

      <Section title={t.langLabel}>
        <Segmented
          value={lang}
          onChange={setLang}
          options={[
            { value: "ko" as Lang, label: "한국어" },
            { value: "en" as Lang, label: "English" },
          ]}
        />
      </Section>
    </>
  );
}
