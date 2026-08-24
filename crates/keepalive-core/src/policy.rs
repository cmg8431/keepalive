use crate::config::Config;
use std::time::Duration;

pub struct PolicyInput {
    pub active_sessions: usize,
    pub battery_percent: Option<u8>,
    pub on_ac_power: bool,
    pub held_for: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    StayAwake,
    AllowSleep(SleepReason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SleepReason {
    NoActiveSessions,
    BatteryBelowFloor(u8),
    MaxHoldExceeded,
}

/// Safety guards always win over wake holds: a Mac in a bag must never be
/// kept awake past its battery floor or the configured hard time cap.
pub fn evaluate(config: &Config, input: &PolicyInput) -> Decision {
    if input.active_sessions == 0 {
        return Decision::AllowSleep(SleepReason::NoActiveSessions);
    }
    if !input.on_ac_power
        && let Some(pct) = input.battery_percent
        && pct < config.battery_floor_percent
    {
        return Decision::AllowSleep(SleepReason::BatteryBelowFloor(pct));
    }
    if input.held_for >= config.max_hold() {
        return Decision::AllowSleep(SleepReason::MaxHoldExceeded);
    }
    Decision::StayAwake
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
