import type { ReactNode } from "react";
import "./icon-tile.css";

export type TileTone = "neutral" | "blue" | "green" | "orange" | "red" | "cobalt" | "onGradient";

/** 틴트 배경 + 같은 hue 솔리드 아이콘. hue 는 장식이 아니라 카테고리 인코딩이다. */
export function IconTile({
  children,
  size = "md",
  tone = "neutral",
}: {
  children: ReactNode;
  size?: "md" | "lg";
  tone?: TileTone;
}) {
  return (
    <span className="icon-tile" data-size={size} data-tone={tone}>
      {children}
    </span>
  );
}
