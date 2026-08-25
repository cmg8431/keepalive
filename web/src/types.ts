/** 데몬이 내려주는 값의 모양. 모든 뷰가 이 한 곳을 본다. */

export type Session = {
  id: string;
  source: string;
  label: string | null;
  active_secs: number;
  expires_in_secs: number;
};
export type Managed = {
  name: string;
  dir: string;
  cmd: string;
  status: "running" | "abandoned";
  respawn_count: number;
  waiting: boolean;
};
export type Status = {
  ok: boolean;
  awake: boolean;
  sessions: Session[];
  managed: Managed[];
  battery_percent: number | null;
  on_ac_power: boolean;
  temperature_celsius: number | null;
  lid_closed: boolean;
  cutout_latched: boolean;
  clamshell_active: boolean;
  held_for_secs: number | null;
  activity: { t: number; text: string }[];
  projects: string[];
  battery_floor_percent: number;
  thermal_threshold_celsius: number;
  max_hold_hours: number;
};
export type ProviderState =
  | "not_installed"
  | "installing"
  | "waiting_for_login"
  | { connected: { ip: string } }
  | { failed: { error: string } };
export type Provider = {
  id: string;
  label: string;
  description: string;
  recommended: boolean;
  state: ProviderState;
};
export type Setup = {
  providers: Provider[];
  hooks: { name: string; installed: boolean }[];
  projects: string[];
  ntfy_topic: string;
  heartbeat_minutes: number;
  clamshell_ready: boolean;
  dashboard_url: string | null;
  dashboard_qr: string | null;
  magic_dns: string | null;
  https_enabled: boolean;
  lan_enabled: boolean;
  lan_url: string | null;
  lan_qr: string | null;
  ntfy_url: string | null;
  ntfy_qr: string | null;
};
export type RecentProject = { dir: string; name: string; source: string; last_seen: number };
export type Projects = { allowlist: string[]; recent: RecentProject[] };
export type BrowseEntry = { name: string; dir: string; is_repo: boolean };
export type BrowseResult = { dir: string; parent: string | null; entries: BrowseEntry[] };

export type Tab = "status" | "sessions" | "settings";
