import type { ReactNode } from "react";
import "./section-header.css";

/** 섹션 라벨 + 우측 보조 텍스트. 섹션은 항상 이걸로 연다. */
export function Section({
  title,
  aside,
  children,
}: {
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="section">
      <div className="section-header">
        <h2>{title}</h2>
        {aside ? <span className="aside">{aside}</span> : null}
      </div>
      {children}
    </section>
  );
}
