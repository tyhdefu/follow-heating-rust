use std::time::Duration;
use chrono::NaiveTime;
use serde::Deserialize;
use crate::python_like::immersion_heater::ImmersionHeaterModel;
use crate::python_like::overrun_config::{OverrunBap, OverrunConfig};
use crate::brain::python_like::working_temp::WorkingTemperatureRange;
use crate::Sensor;
use crate::time::timeslot::ZonedSlot;

#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PythonBrainConfig {
    /// Configuration that controls on/off cycles of the heat pump when
    /// the tank reaches too hot of a temperature.
    hp_circulation: HeatPumpCirculationConfig,
    /// How long it takes for the heat pump to fully turn on.
    hp_enable_time: Duration,

    hp_fully_reneable_min_time: Duration,

    max_heating_hot_water: f32,
    max_heating_hot_water_delta: f32,
    temp_before_circulate: f32,

    try_not_to_turn_on_heat_pump_after: NaiveTime,
    try_not_to_turnon_heat_pump_end_threshold: Duration,
    try_not_to_turn_on_heat_pump_extra_delta: f32,

    /// If we cannot calculate the working range using wiser, we fallback to this,
    /// though this is usually rapidly replaced with the last used (calculated) working temperature range
    default_working_range: WorkingTemperatureRange,

    heat_up_to_during_optimal_time: f32,
    overrun_during: OverrunConfig,
    immersion_heater_model: ImmersionHeaterModel,
}

impl PythonBrainConfig {
    pub fn get_hp_circulation_config(&self) -> &HeatPumpCirculationConfig {
        &self.hp_circulation
    }

    pub fn get_default_working_range(&self) -> &WorkingTemperatureRange {
        &self.default_working_range
    }

    pub fn get_overrun_during(&self) -> &OverrunConfig {
        &self.overrun_during
    }

    pub fn get_immersion_heater_model(&self) -> &ImmersionHeaterModel {
        &self.immersion_heater_model
    }

    pub fn get_hp_enable_time(&self) -> &Duration {
        &self.hp_enable_time
    }
}

impl Default for PythonBrainConfig {
    fn default() -> Self {
        PythonBrainConfig {
            // In use
            hp_circulation: HeatPumpCirculationConfig::default(),
            default_working_range: WorkingTemperatureRange::from_min_max(42.0, 45.0),
            overrun_during: OverrunConfig::new(vec![
                OverrunBap::new(ZonedSlot::Local((NaiveTime::from_hms(01, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 50.0, Sensor::TKTP),
                OverrunBap::new_with_min(ZonedSlot::Local((NaiveTime::from_hms(03, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 50.0, Sensor::TKTP, 43.0),
                OverrunBap::new_with_min(ZonedSlot::Local((NaiveTime::from_hms(03, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 49.0, Sensor::TKBT, 45.0),
                OverrunBap::new(ZonedSlot::Utc((NaiveTime::from_hms(12, 00, 00)..NaiveTime::from_hms(14, 50, 00)).into()), 46.0, Sensor::TKTP),
            ]),
            immersion_heater_model: ImmersionHeaterModel::from_time_points((NaiveTime::from_hms(01, 00, 00), 20.0), (NaiveTime::from_hms(04, 30, 00), 50.0)),
            hp_enable_time: Duration::from_secs(70),


            // Not used - Vet/delete
            hp_fully_reneable_min_time: Duration::from_secs(15 * 60),
            max_heating_hot_water: 42.0,
            max_heating_hot_water_delta: 5.0,
            temp_before_circulate: 33.0,
            try_not_to_turn_on_heat_pump_after: NaiveTime::from_hms(19, 30, 0),
            try_not_to_turnon_heat_pump_end_threshold: Duration::from_secs(20 * 60),
            try_not_to_turn_on_heat_pump_extra_delta: 5.0,
            heat_up_to_during_optimal_time: 45.0,
        }
    }
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HeatPumpCirculationConfig {
    /// How long the heat pump should stay on for before turning off
    /// (Should be less than the time it takes for it to turn on)
    hp_pump_on_time: Duration,
    /// How long the heat pump should stay off before turning back on.
    hp_pump_off_time: Duration,

    /// How long to sleep after going from On -> Circulation mode.
    initial_hp_sleep: Duration,
}

impl HeatPumpCirculationConfig {
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

pub fn try_read_python_brain_config() -> Option<PythonBrainConfig> {
    const PYTHON_BRAIN_CONFIG_FILE: &str = "python_brain.toml";
    let python_brain_config = std::fs::read_to_string(PYTHON_BRAIN_CONFIG_FILE);
    match python_brain_config {
        Ok(str) => {
            match toml::from_str(&str) {
                Ok(x) => Some(x),
                Err(e) => {
                    eprintln!("Failed to deserialize python brain config {:?}", e);
                    None
                }
            }
        },
        Err(e) => {
            eprintln!("Failed to read python brain config {:?}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_config() {
        let config_str = std::fs::read_to_string("test/test_brain_config_with_overrun.toml").expect("Failed to read config file.");
        let config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");

        let mut expected = PythonBrainConfig::default();
        let baps = vec![
            OverrunBap::new(ZonedSlot::Local((NaiveTime::from_hms(01, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 50.1, Sensor::from("1".to_owned())),
            OverrunBap::new_with_min(ZonedSlot::Local((NaiveTime::from_hms(03, 20, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 46.0, Sensor::from("2".to_owned()), 30.0),
            OverrunBap::new_with_min(ZonedSlot::Local((NaiveTime::from_hms(04, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into()), 48.0, Sensor::from("3".to_owned()), 47.0),
            OverrunBap::new(ZonedSlot::Utc((NaiveTime::from_hms(12, 00, 00)..NaiveTime::from_hms(14, 50, 00)).into()), 46.1, Sensor::from("4".to_owned())),
            OverrunBap::new_with_min(ZonedSlot::Utc((NaiveTime::from_hms(11, 00, 00)..NaiveTime::from_hms(15, 50, 00)).into()), 21.5, Sensor::from("5".to_owned()), 10.1),
        ];
        expected.overrun_during = OverrunConfig::new(baps);
        assert_eq!(expected.get_overrun_during(), config.get_overrun_during(), "Overrun during not equal");
        assert_eq!(expected, config)
    }
}