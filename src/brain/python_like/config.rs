use std::time::Duration;
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;
use crate::python_like::immersion_heater::{ImmersionHeaterModel};
use crate::python_like::overrun_config::OverrunConfig;
use crate::brain::python_like::working_temp::WorkingTemperatureRange;

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PythonBrainConfig {
    /// Configuration that controls on/off cycles of the heat pump when
    /// the tank reaches too hot of a temperature.
    hp_circulation: HeatPumpCirculationConfig,
    /// How long (in seconds) it takes for the heat pump to fully turn on
    #[serde_as(as = "DurationSeconds")]
    hp_enable_time: Duration,

    //hp_fully_reneable_min_time: Duration,

    //max_heating_hot_water: f32,
    //max_heating_hot_water_delta: f32,
    temp_before_circulate: f32,

    //try_not_to_turn_on_heat_pump_after: NaiveTime,
    //try_not_to_turnon_heat_pump_end_threshold: Duration,
    //try_not_to_turn_on_heat_pump_extra_delta: f32,

    /// If we cannot calculate the working range using wiser, we fallback to this,
    /// though this is usually rapidly replaced with the last used (calculated) working temperature range
    default_working_range: WorkingTemperatureRange,
    working_temp_model: WorkingTempModelConfig,

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

    pub fn get_working_temp_model(&self) -> &WorkingTempModelConfig {
        &self.working_temp_model
    }

    pub fn get_hp_enable_time(&self) -> &Duration {
        &self.hp_enable_time
    }

    pub fn get_temp_before_circulate(&self) -> f32 {
        self.temp_before_circulate
    }
}

impl Default for PythonBrainConfig {
    fn default() -> Self {
        PythonBrainConfig {
            // In use
            hp_circulation: HeatPumpCirculationConfig::default(),
            default_working_range: WorkingTemperatureRange::from_min_max(42.0, 45.0),
            working_temp_model: WorkingTempModelConfig::default(),
            overrun_during: OverrunConfig::default(),
            immersion_heater_model: ImmersionHeaterModel::default(),
            hp_enable_time: Duration::from_secs(70),
            temp_before_circulate: 33.0,

            // Not used - Vet/delete
            //hp_fully_reneable_min_time: Duration::from_secs(15 * 60),
            //max_heating_hot_water: 42.0,
            //max_heating_hot_water_delta: 5.0,
            //try_not_to_turn_on_heat_pump_after: NaiveTime::from_hms(19, 30, 0),
            //try_not_to_turnon_heat_pump_end_threshold: Duration::from_secs(20 * 60),
            //try_not_to_turn_on_heat_pump_extra_delta: 5.0,
        }
    }
}

impl AsRef<OverrunConfig> for PythonBrainConfig {
    fn as_ref(&self) -> &OverrunConfig {
        &self.overrun_during
    }
}

impl AsRef<HeatPumpCirculationConfig> for PythonBrainConfig {
    fn as_ref(&self) -> &HeatPumpCirculationConfig {
        &self.hp_circulation
    }
}

impl AsRef<WorkingTempModelConfig> for PythonBrainConfig {
    fn as_ref(&self) -> &WorkingTempModelConfig {
        &self.working_temp_model
    }
}

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

/// A graph of 1/-x where x is difference
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkingTempModelConfig {
    /// The maximum value that lower bound of the working range can be.
    /// (The horizontal asymptote)
    graph_max_lower_temp: f32,
    /// The y-axis stretch factor
    /// (affects the steepness of the graph)
    multiplicand: f32,
    /// The left shift of the graph,
    /// (chops of the very steep bit of 1/-x
    left_shift: f32,
    /// The maximum difference
    /// (chops of the very flat bit of the graph)
    /// Must be less than base range size
    difference_cap: f32,
    /// The base range of max - min, gets capped difference subtracted from it,
    /// causing the range to be tightened at higher temperatures
    /// Must be greater than difference cap
    base_range_size: f32,
}

impl WorkingTempModelConfig {
    pub fn get_max_lower_temp(&self) -> f32 {
        self.graph_max_lower_temp
    }

    pub fn get_multiplicand(&self) -> f32 {
        self.multiplicand
    }

    pub fn get_left_shift(&self) -> f32 {
        self.left_shift
    }

    pub fn get_difference_cap(&self) -> f32 {
        self.difference_cap
    }

    pub fn get_base_range_size(&self) -> f32 {
        self.base_range_size
    }
}

impl Default for WorkingTempModelConfig {
    fn default() -> Self {
        Self {
            graph_max_lower_temp: 53.2,
            multiplicand: 10.0,
            left_shift: 0.6,
            difference_cap: 2.5,
            base_range_size: 4.5
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
    use crate::python_like::overrun_config::OverrunBap;
    use crate::Sensor;
    use crate::time::test_utils::time;
    use crate::time::timeslot::ZonedSlot;
    use super::*;

    #[test]
    fn test_deserialize_config() {
        let config_str = std::fs::read_to_string("test/python_brain/test_brain_config_with_overrun.toml").expect("Failed to read config file.");
        let config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");

        let mut expected = PythonBrainConfig::default();
        let baps = vec![
            OverrunBap::new(ZonedSlot::Local((time(01, 00, 00)..time(04, 30, 00)).into()), 50.1, Sensor::from("1".to_owned())),
            OverrunBap::new_with_min(ZonedSlot::Local((time(03, 20, 00)..time(04, 30, 00)).into()), 46.0, Sensor::from("2".to_owned()), 30.0),
            OverrunBap::new_with_min(ZonedSlot::Local((time(04, 00, 00)..time(04, 30, 00)).into()), 48.0, Sensor::from("3".to_owned()), 47.0),
            OverrunBap::new(ZonedSlot::Utc((time(12, 00, 00)..time(14, 50, 00)).into()), 46.1, Sensor::from("4".to_owned())),
            OverrunBap::new_with_min(ZonedSlot::Utc((time(11, 00, 00)..time(15, 50, 00)).into()), 21.5, Sensor::from("5".to_owned()), 10.1),
        ];
        expected.overrun_during = OverrunConfig::new(baps);
        assert_eq!(expected.get_overrun_during(), config.get_overrun_during(), "Overrun during not equal");
        assert_eq!(expected, config)
    }

    #[test]
    fn test_can_deserialize_full() {
        let config_str = std::fs::read_to_string("test/python_brain/test_brain_config.toml").expect("Failed to read config file.");
        let config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");
    }
}