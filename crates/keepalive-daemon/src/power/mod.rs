#[cfg(target_os = "macos")]
mod iokit;
#[cfg(target_os = "macos")]
pub mod smc;
#[cfg(target_os = "macos")]
pub use iokit::WakeAssertion;
#[cfg(target_os = "macos")]
pub use smc::SmcReader;

#[cfg(not(target_os = "macos"))]
mod stub;
#[cfg(not(target_os = "macos"))]
pub use stub::{SmcReader, WakeAssertion};

#[derive(Debug, Clone, Copy)]
pub struct PowerStatus {
    pub battery_percent: Option<u8>,
    pub on_ac_power: bool,
}

pub fn read_power_status() -> PowerStatus {
    match std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
    {
        Ok(out) => parse_pmset_batt(&String::from_utf8_lossy(&out.stdout)),
        // No pmset (non-mac, tests): fail open as if on AC so guards don't misfire.
        Err(_) => PowerStatus {
            battery_percent: None,
            on_ac_power: true,
        },
    }
}

/// Reads AppleClamshellState off IOPMrootDomain. Absent (desktops) or any
/// failure reads as open — the safe direction for the thermal guard.
pub fn lid_closed() -> bool {
    match std::process::Command::new("/usr/sbin/ioreg")
        .args(["-r", "-k", "AppleClamshellState", "-d", "1"])
        .output()
    {
        Ok(out) => parse_lid(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => false,
    }
}

fn parse_lid(text: &str) -> bool {
    text.lines()
        .any(|l| l.contains("AppleClamshellState") && l.contains("Yes"))
}

fn parse_pmset_batt(text: &str) -> PowerStatus {
    let on_ac_power = text.contains("AC Power");
    let battery_percent = text.find('%').and_then(|i| {
        let digits: String = text[..i]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.chars().rev().collect::<String>().parse().ok()
    });
    PowerStatus {
        battery_percent,
        on_ac_power,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_battery_discharging() {
        let s = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=123)\t85%; discharging; 4:32 remaining present: true";
        let p = parse_pmset_batt(s);
        assert_eq!(p.battery_percent, Some(85));
        assert!(!p.on_ac_power);
    }

    #[test]
    fn parses_ac_charging() {
        let s = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=123)\t100%; charged; 0:00 remaining present: true";
        let p = parse_pmset_batt(s);
        assert_eq!(p.battery_percent, Some(100));
        assert!(p.on_ac_power);
    }

    #[test]
    fn parses_lid_state() {
        let closed = r#"+-o IOPMrootDomain  <class IOPMrootDomain>
    {
      "AppleClamshellState" = Yes
    }"#;
        assert!(parse_lid(closed));
        assert!(!parse_lid(&closed.replace("Yes", "No")));
        assert!(!parse_lid(""));
    }

    #[test]
    fn desktop_without_battery() {
        let s = "Now drawing from 'AC Power'";
        let p = parse_pmset_batt(s);
        assert_eq!(p.battery_percent, None);
        assert!(p.on_ac_power);
    }
}
