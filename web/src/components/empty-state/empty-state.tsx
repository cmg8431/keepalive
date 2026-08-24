import "./empty-state.css";

/** 빈 상태는 사과하지 않고, 다음에 할 일을 알려준다. */
export function EmptyState({ text, tone = "muted" }: { text: string; tone?: "muted" | "ok" }) {
  return (
    <p className="empty" data-tone={tone}>
      {text}
    </p>
  );
}
