use serde_with::serde_as;
use serde::Deserialize;
use std::time::Duration;
use serde_with::DurationSeconds;

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HeatPumpCirculationConfig {
    /// How long (in seconds) the heat pump should stay on for before turning off
    /// (Should be less than the time it takes for it to turn on)
    #[serde_as(as = "DurationSeconds")]
    hp_pump_on_time: Duration,
    /// How long (in seconds) the heat pump should stay off before turning back on.
    #[serde_as(as = "DurationSeconds")]
    hp_pump_off_time: Duration,

    /// How long (in seconds) to sleep after going from On -> Circulation mode.
    #[serde_as(as = "DurationSeconds")]
    initial_hp_sleep: Duration,
}

impl HeatPumpCirculationConfig {
    #[cfg(test)]
    pub fn new(on_time: u64, off_time: u64, initial_sleep: u64) -> Self {
        Self {
            hp_pump_on_time: Duration::from_secs(on_time),
            hp_pump_off_time: Duration::from_secs(off_time),
            initial_hp_sleep: Duration::from_secs(initial_sleep),
        }
    }

    pub fn get_hp_on_time(&self) -> &Duration {
        &self.hp_pump_on_time
    }

    pub fn get_hp_off_time(&self) -> &Duration {
        &self.hp_pump_off_time
    }

    pub fn get_initial_hp_sleep(&self) -> &Duration {
        &self.initial_hp_sleep
    }
}

impl Default for HeatPumpCirculationConfig {
    fn default() -> Self {
        Self {
            hp_pump_on_time: Duration::from_secs(70),
            hp_pump_off_time: Duration::from_secs(30),
            initial_hp_sleep: Duration::from_secs(5 * 60),
        }
    }
}
