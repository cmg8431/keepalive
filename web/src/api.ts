/** 대시보드에서 데몬으로 나가는 유일한 통로. */
export async function post(path: string, body?: unknown) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  return res.json();
}

/** 메뉴바 패널로 열렸는지 — 창 자체가 투명해서 배경을 칠하면 안 된다. */
export const isPanel = new URLSearchParams(location.search).has("panel");

/** `/s/<세션>` 으로 들어오면 그 세션 화면을 바로 연다. */
export function deepLinkedSession(): string | null {
  const match = location.pathname.match(/^\/s\/([^/]+)\/?$/);
  return match ? decodeURIComponent(match[1]) : null;
}
