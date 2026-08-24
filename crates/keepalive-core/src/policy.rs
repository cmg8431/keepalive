use crate::config::Config;
use std::time::Duration;

pub struct PolicyInput {
    pub active_sessions: usize,
    pub battery_percent: Option<u8>,
    pub on_ac_power: bool,
    pub held_for: Duration,
    pub temperature_celsius: Option<f64>,
    pub lid_closed: bool,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    StayAwake,
    AllowSleep(SleepReason),
}

#[derive(Debug, PartialEq)]
pub enum SleepReason {
    NoActiveSessions,
    ThermalCutout(f64),
    BatteryBelowFloor(u8),
    MaxHoldExceeded,
}

/// Safety guards always win over wake holds: a Mac in a bag must never be
/// kept awake past its thermal limit, battery floor, or the hard time cap.
/// Thermal only applies lid-closed: with the lid open, macOS's own thermal
/// management (and the visible fans) are the right authority.
pub fn evaluate(config: &Config, input: &PolicyInput) -> Decision {
    if input.active_sessions == 0 {
        return Decision::AllowSleep(SleepReason::NoActiveSessions);
    }
    if input.lid_closed
        && let Some(temp) = input.temperature_celsius
        && temp >= config.thermal_threshold_celsius
    {
        return Decision::AllowSleep(SleepReason::ThermalCutout(temp));
    }
    if !input.on_ac_power
        && let Some(pct) = input.battery_percent
        && pct < config.battery_floor_percent
    {
        return Decision::AllowSleep(SleepReason::BatteryBelowFloor(pct));
    }
    // 0 = 상한 없음. 사용자가 명시적으로 끈 경우이며, 배터리·온도 가드는 그대로 산다.
    if config.max_hold_hours > 0 && input.held_for >= config.max_hold() {
        return Decision::AllowSleep(SleepReason::MaxHoldExceeded);
    }
    Decision::StayAwake
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoutKind {
    Thermal,
    LowBattery,
}

/// Once a safety cutout trips, holds stay refused until conditions recover
/// past a hysteresis margin. Without this, the still-running agent's next
/// hook re-acquires within seconds and the machine oscillates.
#[derive(Debug, Default)]
pub struct CutoutLatch {
    tripped: Option<CutoutKind>,
}

const THERMAL_HYSTERESIS_CELSIUS: f64 = 5.0;
const BATTERY_HYSTERESIS_PERCENT: u8 = 5;

impl CutoutLatch {
    pub fn trip(&mut self, kind: CutoutKind) {
        self.tripped = Some(kind);
    }

    pub fn is_latched(&self) -> bool {
        self.tripped.is_some()
    }

    pub fn kind(&self) -> Option<CutoutKind> {
        self.tripped
    }

    /// Returns true if the latch cleared. A missing reading keeps the latch
    /// held: a cutout must not clear on absence of data.
    pub fn try_clear(&mut self, config: &Config, input: &PolicyInput) -> bool {
        let clear = match self.tripped {
            None => return false,
            Some(CutoutKind::Thermal) => {
                !input.lid_closed
                    || input.temperature_celsius.is_some_and(|t| {
                        t <= config.thermal_threshold_celsius - THERMAL_HYSTERESIS_CELSIUS
                    })
            }
            Some(CutoutKind::LowBattery) => {
                input.on_ac_power
                    || input.battery_percent.is_some_and(|p| {
                        p >= config.battery_floor_percent + BATTERY_HYSTERESIS_PERCENT
                    })
            }
        };
        if clear {
            self.tripped = None;
        }
        clear
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> PolicyInput {
        PolicyInput {
            active_sessions: 1,
            battery_percent: Some(80),
            on_ac_power: false,
            held_for: Duration::ZERO,
            temperature_celsius: Some(50.0),
            lid_closed: false,
        }
    }

    #[test]
    fn no_sessions_allows_sleep() {
        let i = PolicyInput {
            active_sessions: 0,
            ..input()
        };
        assert_eq!(
            evaluate(&Config::default(), &i),
            Decision::AllowSleep(SleepReason::NoActiveSessions)
        );
    }

    #[test]
    fn active_session_stays_awake() {
        assert_eq!(evaluate(&Config::default(), &input()), Decision::StayAwake);
    }

    #[test]
    fn battery_floor_forces_sleep_on_battery() {
        let i = PolicyInput {
            battery_percent: Some(25),
            ..input()
        };
        assert_eq!(
            evaluate(&Config::default(), &i),
            Decision::AllowSleep(SleepReason::BatteryBelowFloor(25))
        );
    }

    #[test]
    fn battery_floor_ignored_on_ac() {
        let i = PolicyInput {
            battery_percent: Some(25),
            on_ac_power: true,
            ..input()
        };
        assert_eq!(evaluate(&Config::default(), &i), Decision::StayAwake);
    }

    #[test]
    fn thermal_cutout_fires_lid_closed() {
        let i = PolicyInput {
            temperature_celsius: Some(85.0),
            lid_closed: true,
            ..input()
        };
        assert_eq!(
            evaluate(&Config::default(), &i),
            Decision::AllowSleep(SleepReason::ThermalCutout(85.0))
        );
    }

    #[test]
    fn thermal_ignored_lid_open() {
        let i = PolicyInput {
            temperature_celsius: Some(95.0),
            lid_closed: false,
            ..input()
        };
        assert_eq!(evaluate(&Config::default(), &i), Decision::StayAwake);
    }

    #[test]
    fn thermal_latch_holds_until_hysteresis() {
        let config = Config::default();
        let mut latch = CutoutLatch::default();
        latch.trip(CutoutKind::Thermal);
        let hot = PolicyInput {
            temperature_celsius: Some(78.0),
            lid_closed: true,
            ..input()
        };
        assert!(!latch.try_clear(&config, &hot));
        let missing = PolicyInput {
            temperature_celsius: None,
            lid_closed: true,
            ..input()
        };
        assert!(!latch.try_clear(&config, &missing));
        let cool = PolicyInput {
            temperature_celsius: Some(74.0),
            lid_closed: true,
            ..input()
        };
        assert!(latch.try_clear(&config, &cool));
        assert!(!latch.is_latched());
    }

    #[test]
    fn thermal_latch_clears_on_lid_open() {
        let mut latch = CutoutLatch::default();
        latch.trip(CutoutKind::Thermal);
        let opened = PolicyInput {
            temperature_celsius: Some(90.0),
            lid_closed: false,
            ..input()
        };
        assert!(latch.try_clear(&Config::default(), &opened));
    }

    #[test]
    fn battery_latch_clears_on_ac_or_recharge() {
        let config = Config::default();
        let mut latch = CutoutLatch::default();
        latch.trip(CutoutKind::LowBattery);
        let still_low = PolicyInput {
            battery_percent: Some(32),
            ..input()
        };
        assert!(!latch.try_clear(&config, &still_low));
        let plugged = PolicyInput {
            battery_percent: Some(20),
            on_ac_power: true,
            ..input()
        };
        latch.trip(CutoutKind::LowBattery);
        assert!(latch.try_clear(&config, &plugged));
        let recharged = PolicyInput {
            battery_percent: Some(35),
            ..input()
        };
        latch.trip(CutoutKind::LowBattery);
        assert!(latch.try_clear(&config, &recharged));
    }

    #[test]
    fn max_hold_forces_sleep() {
        let i = PolicyInput {
            held_for: Duration::from_secs(9 * 3600),
            ..input()
        };
        assert_eq!(
            evaluate(&Config::default(), &i),
            Decision::AllowSleep(SleepReason::MaxHoldExceeded)
        );
    }
}

#[cfg(test)]
mod unlimited_hold_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn max_hold_zero_disables_cap() {
        let config = Config {
            max_hold_hours: 0,
            ..Config::default()
        };
        let input = PolicyInput {
            active_sessions: 1,
            battery_percent: Some(90),
            on_ac_power: true,
            held_for: Duration::from_secs(60 * 60 * 100),
            temperature_celsius: Some(50.0),
            lid_closed: false,
        };
        assert_eq!(evaluate(&config, &input), Decision::StayAwake);
    }
}
