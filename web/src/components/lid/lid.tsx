import "./lid.css";

/** 맥북 뚜껑 — 열림/닫힘을 글자 대신 각도로 보여준다. */
export function Lid({ closed }: { closed: boolean }) {
  return (
    <div className="lid" data-closed={closed} aria-hidden>
      <span className="lid-screen" />
      <span className="lid-deck" />
    </div>
  );
}
