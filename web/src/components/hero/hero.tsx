import type { ReactNode } from "react";
import { IconTile } from "../icon-tile";
import "./hero.css";

export type HeroTone = "idle" | "awake" | "cutout";

/** 상태 히어로. 값 하나 · 문장 하나 · 진행 도트만 담는다. */
export function Hero({
  tone,
  icon,
  title,
  detail,
  value,
  unit,
  filled,
  total = 4,
}: {
  tone: HeroTone;
  icon: ReactNode;
  title: string;
  detail?: string;
  value: string;
  unit?: string;
  filled: number;
  total?: number;
}) {
  return (
    <section className="hero" data-tone={tone}>
      <IconTile size="lg" tone="onGradient">
        {icon}
      </IconTile>
      <p className="hero-title">{title}</p>
      {detail ? <p className="hero-detail">{detail}</p> : null}
      <div className="hero-bottom">
        <div className="hero-value">
          <strong>{value}</strong>
          {unit ? <span>{unit}</span> : null}
        </div>
        <div className="hero-dots" aria-hidden>
          {Array.from({ length: total }, (_, i) => (
            <span key={i} data-on={i < filled} />
          ))}
          <span className="lead" />
        </div>
      </div>
    </section>
  );
}
