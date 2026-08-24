import { useEffect, useRef, useState } from "react";
import "./duration.css";

/* 프리셋 네 개를 늘어놓는 대신 눈금 하나로 고른다. 값은 드래그하는 동안 계속
   말해 주고(숫자 + 몇 시까지), 누른 뒤에는 눌렸다는 사실을 짧게 남긴다. */
const STEPS = [15, 30, 45, 60, 90, 120, 180, 240, 300, 360, 480, 720];

export function DurationPicker({
  maxMinutes,
  unlimited = false,
  label,
  formatDuration,
  untilLabel,
  confirmLabel,
  doneLabel,
  foreverLabel,
  unlimitedLabel,
  locale,
  onHold,
  onForever,
}: {
  maxMinutes: number;
  unlimited?: boolean;
  label: string;
  formatDuration: (minutes: number) => string;
  untilLabel: (clock: string) => string;
  confirmLabel: (duration: string) => string;
  doneLabel: string;
  foreverLabel: string;
  unlimitedLabel: string;
  locale: string;
  onHold: (minutes: number) => void;
  onForever: () => void;
}) {
  const steps = STEPS.filter((m) => m <= maxMinutes);
  const [index, setIndex] = useState(Math.max(0, steps.indexOf(60)));
  const [done, setDone] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const minutes = steps[Math.min(index, steps.length - 1)] ?? 60;
  const fill = `${(index / Math.max(1, steps.length - 1)) * 100}%`;
  const clock = new Date(Date.now() + minutes * 60_000).toLocaleTimeString(locale, {
    hour: "numeric",
    minute: "2-digit",
  });

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const flash = () => {
    setDone(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setDone(false), 1600);
  };

  return (
    <div className="duration" data-done={done}>
      <div className="duration-head">
        <span className="duration-label">{label}</span>
        <span className="duration-until">{unlimited ? unlimitedLabel : untilLabel(clock)}</span>
      </div>

      {/* key 를 값에 걸어 값이 바뀔 때마다 숫자가 새로 튀어오른다 */}
      <span className="duration-value" key={minutes}>
        {unlimited ? "∞" : formatDuration(minutes)}
      </span>

      <input
        type="range"
        min={0}
        max={steps.length - 1}
        step={1}
        value={index}
        aria-label={label}
        style={{ ["--fill" as string]: fill }}
        onChange={(e) => setIndex(Number(e.target.value))}
      />

      <div className="duration-actions">
        <button
          className="primary grow"
          onClick={() => {
            onHold(minutes);
            flash();
          }}
        >
          {done ? doneLabel : confirmLabel(formatDuration(minutes))}
        </button>
        <button
          onClick={() => {
            onForever();
            flash();
          }}
          title={foreverLabel}
          aria-pressed={unlimited}
          className={unlimited ? "selected" : ""}
        >
          ∞
        </button>
      </div>
    </div>
  );
}
