import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { dicts } from "./i18n";

type Dict = (typeof dicts)["en"];

type Frame = {
  screen: string;
  cursor_x: number;
  cursor_y: number;
  cols: number;
  rows: number;
  ok?: boolean;
  error?: string;
};

/** Keys a phone keyboard cannot produce, plus the answers agents ask for most. */
const QUICK_KEYS: { label: string; send: string }[] = [
  { label: "esc", send: "\x1b" },
  { label: "tab", send: "\t" },
  { label: "↑", send: "\x1b[A" },
  { label: "↓", send: "\x1b[B" },
  { label: "1", send: "1" },
  { label: "2", send: "2" },
  { label: "3", send: "3" },
  { label: "y", send: "y" },
  { label: "n", send: "n" },
  { label: "⏎", send: "\r" },
  { label: "^C", send: "\x03" },
];

const encoder = new TextEncoder();

function toHex(data: string): string {
  return [...encoder.encode(data)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * A live view of a managed session that you can type into.
 *
 * The daemon streams the rendered screen rather than a byte stream, so each
 * frame is a full repaint: home the cursor, clear, write, then place the cursor
 * where tmux says it is. That costs a redraw per frame but means the browser is
 * never a tmux client — nothing it does can disturb the running agent.
 */
export function SessionTerminal({
  name,
  dir,
  t,
  onClose,
}: {
  name: string;
  dir?: string;
  t: Dict;
  onClose: () => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [state, setState] = useState<"connecting" | "live" | "closed">("connecting");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 12,
      lineHeight: 1.15,
      scrollback: 0,
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
      theme: {
        background: "#1a1a1c",
        foreground: "#ededed",
        cursor: "#8082ff",
        selectionBackground: "#3b3b3f",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;

    /* tmux 창을 뷰어 크기에 맞춘다. 안 맞으면 브라우저가 다르게 접힌 사본을
       그리게 되고, 그게 "깨져 보이는" 상태다. */
    /* tmux 는 붙어 있는 클라이언트 중 가장 작은 것에 pane 을 맞춘다. 그래서
       우리 크기를 강요할 수 없고, 강요했다고 믿으면 줄바꿈이 어긋나 화면이
       깨진다. pane 크기는 tmux 가 정하게 두고, 우리는 그 화면을 시트 폭에
       맞춰 확대/축소만 한다. */
    const scaleToFit = () => {
      const screen = host.querySelector<HTMLElement>(".xterm-screen");
      if (!screen) return;
      const natural = screen.getBoundingClientRect().width / (term.element?.style.transform ? 1 : 1);
      const available = host.clientWidth - 4;
      if (!natural || !available) return;
      const scale = Math.min(2, Math.max(0.5, available / (screen.offsetWidth || natural)));
      const wrapper = term.element;
      if (wrapper) {
        wrapper.style.transformOrigin = "top left";
        wrapper.style.transform = `scale(${scale.toFixed(3)})`;
      }
    };

    const requestSize = () => {
      void fetch(`/api/terminal/${encodeURIComponent(name)}/resize`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cols: term.cols, rows: term.rows }),
      });
    };
    // 붙어 있는 클라이언트가 없을 때만 실제로 먹는다 — 있으면 tmux 가 무시한다.
    // 레이아웃이 잡히기 전 값(0px)으로 요청하면 pane 이 쪼그라든다. 한 프레임 뒤에.
    const sizeTimer = window.setTimeout(() => {
      if (host.clientWidth < 200) return;
      try {
        fit.fit();
        requestSize();
      } catch {
        /* 아직이면 그냥 tmux 크기를 따른다 */
      }
    }, 120);
    scaleToFit();
    const observer = new ResizeObserver(() => scaleToFit());
    observer.observe(host);

    const send = (data: string) => {
      if (!data) return;
      void fetch(`/api/terminal/${encodeURIComponent(name)}/input`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ hex: toHex(data) }),
      });
    };
    const typed = term.onData(send);

    /* xterm 은 스크롤백을 갖고 있지 않다(매 프레임 전체 repaint 이므로).
       그래서 휠은 화살표가 아니라 PgUp/PgDn 으로 보내 tmux 쪽 히스토리를
       움직인다 — 팬 자체가 안내하는 그 방법이다. */
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      send(e.deltaY < 0 ? "\x1b[5~" : "\x1b[6~");
    };
    host.addEventListener("wheel", onWheel, { passive: false });

    const source = new EventSource(`/api/terminal/${encodeURIComponent(name)}/stream`);
    source.onmessage = (e) => {
      const frame: Frame = JSON.parse(e.data);
      if (frame.error) {
        setError(frame.error);
        return;
      }
      setError(null);
      setState("live");
      // \x1b[H home, \x1b[2J clear, then the pane, then the real cursor spot.
      // Rows are 1-based in CUP, tmux reports them 0-based.
      if (frame.cols && frame.rows && (term.cols !== frame.cols || term.rows !== frame.rows)) {
        term.resize(frame.cols, frame.rows);
        scaleToFit();
      }
      term.write(
        `\x1b[H\x1b[2J${frame.screen}\x1b[${frame.cursor_y + 1};${frame.cursor_x + 1}H`,
      );
    };
    source.onerror = () => setState("closed");

    return () => {
      window.clearTimeout(sizeTimer);
      host.removeEventListener("wheel", onWheel);
      observer.disconnect();
      source.close();
      typed.dispose();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [name]);

  const [message, setMessage] = useState("");
  const [copied, setCopied] = useState(false);
  const [linkShown, setLinkShown] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const sessionUrl = `${location.origin}/s/${encodeURIComponent(name)}`;

  /* tailnet 은 http 라 navigator.clipboard 가 없다(보안 컨텍스트가 아니다).
     그래서 실패하면 옛 방식으로 복사하고, 그것도 막히면 주소를 그대로 보여준다. */
  const copyLink = async () => {
    let ok = false;
    try {
      await navigator.clipboard.writeText(sessionUrl);
      ok = true;
    } catch {
      const area = document.createElement("textarea");
      area.value = sessionUrl;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      ok = document.execCommand("copy");
      area.remove();
    }
    if (ok) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } else {
      setLinkShown(true);
    }
  };

  const resume = async () => {
    if (!dir) return;
    const res = await fetch("/api/spawn", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ dir }),
    }).then((r) => r.json());
    setNote(res.ok ? t.resumeStarted : (res.error ?? t.resumeFailed));
    setTimeout(() => setNote(null), 3000);
  };

  const sendRaw = (data: string) => {
    void fetch(`/api/terminal/${encodeURIComponent(name)}/input`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ hex: toHex(data) }),
    });
    termRef.current?.focus();
  };

  return (
    <div className="terminal-sheet">
      <header className="terminal-bar">
        <div className="row">
          <strong>{name}</strong>
          <span className={`badge ${state === "live" ? "running" : "abandoned"}`}>
            {state === "live" ? t.termLive : state === "connecting" ? t.termConnecting : t.termClosed}
          </span>
        </div>
        <div className="row">
          <button className="small" onClick={copyLink} title={sessionUrl}>
            {copied ? t.copied : t.copyLink}
          </button>
          {dir && (
            <button className="small" onClick={resume}>
              {t.resumeHere}
            </button>
          )}
          <button className="small" onClick={onClose}>
            {t.termClose}
          </button>
        </div>
      </header>
      {error && <p className="error small terminal-error">{error}</p>}
      {note && <p className="small terminal-error">{note}</p>}
      {linkShown && <p className="small mono terminal-error">{sessionUrl}</p>}
      <div className="terminal-host" ref={hostRef} />
      <form
        className="terminal-say"
        onSubmit={(e) => {
          e.preventDefault();
          if (!message) return;
          sendRaw(`${message}\r`);
          setMessage("");
        }}
      >
        <input
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          placeholder={t.termSayPlaceholder}
        />
        <button className="primary small" type="submit" disabled={!message}>
          {t.send}
        </button>
      </form>
      <div className="terminal-keys">
        {QUICK_KEYS.map((k) => (
          <button key={k.label} className="key" onClick={() => sendRaw(k.send)}>
            {k.label}
          </button>
        ))}
      </div>
    </div>
  );
}
