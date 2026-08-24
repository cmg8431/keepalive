import type { ReactNode } from "react";
import "./row.css";

/** 아이콘 · 제목/설명 · 액션으로 이루어진 한 줄. 리스트의 기본 단위. */
export function Row({
  icon,
  title,
  detail,
  actions,
  children,
}: {
  icon?: ReactNode;
  title: ReactNode;
  detail?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <li className="row">
      {icon}
      <div className="row-body">
        <strong>{title}</strong>
        {detail ? <span>{detail}</span> : null}
      </div>
      {actions ? <div className="row-actions">{actions}</div> : null}
      {children}
    </li>
  );
}

export function RowList({ children }: { children: ReactNode }) {
  return <ul className="row-list">{children}</ul>;
}
