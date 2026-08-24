import type { ReactNode } from "react";
import { IconTile } from "../icon-tile";
import "./widget.css";

/** 단일 지표 위젯: 아이콘 · 라벨 · 큰 값(또는 그림) · 보조 시각화 */
export function Widget({
  icon,
  label,
  value,
  unit,
  sub,
  warn,
  graphic,
  visual,
}: {
  icon: ReactNode;
  label: string;
  value?: string;
  unit?: string;
  sub?: string;
  warn?: boolean;
  graphic?: ReactNode;
  visual?: ReactNode;
}) {
  return (
    <div className="widget" data-warn={warn ? "true" : "false"}>
      <IconTile>{icon}</IconTile>
      <span className="widget-label">{label}</span>
      <div className="widget-bottom">
        {graphic ?? (
          <div className="widget-value">
            <strong>{value}</strong>
            {unit ? <span>{unit}</span> : null}
          </div>
        )}
        {visual ?? (sub ? <span className="widget-sub">{sub}</span> : null)}
      </div>
    </div>
  );
}

export function WidgetGrid({ children }: { children: ReactNode }) {
  return <div className="widgets">{children}</div>;
}
