import type { ReactNode } from "react";
import { IconTile } from "../icon-tile";
import "./action-tile.css";

/** 큰 탭 타깃 하나 = 액션 하나. 화면 상단에 오는 주요 행동. */
export function ActionTile({
  icon,
  label,
  onClick,
  disabled,
}: {
  icon: ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button className="action" onClick={onClick} disabled={disabled}>
      <IconTile>{icon}</IconTile>
      <span>{label}</span>
    </button>
  );
}

export function ActionGrid({ children }: { children: ReactNode }) {
  return <div className="actions">{children}</div>;
}
