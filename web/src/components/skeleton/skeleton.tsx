import "./skeleton.css";

/** 로딩은 스피너가 아니라 들어올 모양으로 보여준다 — 값이 도착해도 레이아웃이 안 튄다. */
export function SkeletonCard({ rows = 3, title = true }: { rows?: number; title?: boolean }) {
  return (
    <div className="sk-card">
      {title && <span className="sk sk-title" />}
      {Array.from({ length: rows }, (_, i) => (
        <span key={i} className="sk" style={{ width: `${88 - i * 13}%` }} />
      ))}
    </div>
  );
}
