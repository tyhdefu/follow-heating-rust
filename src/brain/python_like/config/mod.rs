use std::path::{Path, PathBuf};
use std::time::Duration;
use log::{debug, error, info};
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;
use heat_pump_circulation::HeatPumpCirculationConfig;
use working_temp_model::WorkingTempModelConfig;
use crate::brain::boost_active_rooms::config::BoostActiveRoomsConfig;
use crate::brain::immersion_heater::config::ImmersionHeaterModelConfig;
use crate::brain::python_like::config::min_hp_runtime::MinHeatPumpRuntime;
use crate::python_like::config::overrun_config::OverrunConfig;
use crate::brain::python_like::working_temp::WorkingTemperatureRange;
use crate::time_util::timeslot::ZonedSlot;

pub mod heat_pump_circulation;
pub mod working_temp_model;
pub mod overrun_config;
pub mod min_hp_runtime;

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

    temp_before_circulate: f32,

    min_hp_runtime: MinHeatPumpRuntime,

    /// If we cannot calculate the working range using wiser, we fallback to this,
    /// though this is usually rapidly replaced with the last used (calculated) working temperature range
    default_working_range: WorkingTemperatureRange,
    working_temp_model: WorkingTempModelConfig,

    #[serde(flatten)]
    additive_config: PythonBrainAdditiveConfig,
}

#[derive(Clone, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PythonBrainAdditiveConfig {
    /// Which directories (relative to working directory of the binary)
    /// should be searched for additive configuration files
    include_config_directories: Vec<PathBuf>,

    overrun_during: OverrunConfig,
    immersion_heater_model: ImmersionHeaterModelConfig,
    boost_active_rooms: BoostActiveRoomsConfig,
    /// Times at which to ignore the wiser heating.
    no_heating: Vec<ZonedSlot>,
}

impl PythonBrainAdditiveConfig {
    pub fn combine(&mut self, other: Self) {
        self.include_config_directories.append(&mut other.include_config_directories.clone());
        self.overrun_during.combine(other.overrun_during);
        self.immersion_heater_model.combine(other.immersion_heater_model);
        self.boost_active_rooms.combine(other.boost_active_rooms);
        self.no_heating.extend(other.no_heating);
    }
}

impl PythonBrainConfig {
    pub fn get_hp_circulation_config(&self) -> &HeatPumpCirculationConfig {
        &self.hp_circulation
    }

    pub fn get_default_working_range(&self) -> &WorkingTemperatureRange {
        &self.default_working_range
    }

    pub fn get_overrun_during(&self) -> &OverrunConfig {
        &self.additive_config.overrun_during
    }

    pub fn get_immersion_heater_model(&self) -> &ImmersionHeaterModelConfig {
        &self.additive_config.immersion_heater_model
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

    pub fn get_boost_active_rooms(&self) -> &BoostActiveRoomsConfig {
        &self.additive_config.boost_active_rooms
    }

    pub fn get_no_heating(&self) -> &Vec<ZonedSlot> {
        &self.additive_config.no_heating
    }

    pub fn get_min_hp_runtime(&self) -> &MinHeatPumpRuntime {
        &self.min_hp_runtime
    }
}

impl Default for PythonBrainConfig {
    fn default() -> Self {
        PythonBrainConfig {
            // In use
            hp_circulation: HeatPumpCirculationConfig::default(),
            default_working_range: WorkingTemperatureRange::from_min_max(42.0, 45.0),
            working_temp_model: WorkingTempModelConfig::default(),
            hp_enable_time: Duration::from_secs(70),
            temp_before_circulate: 33.0,
            additive_config: PythonBrainAdditiveConfig::default(),
            min_hp_runtime: Default::default(),
        }
    }
}

impl AsRef<OverrunConfig> for PythonBrainConfig {
    fn as_ref(&self) -> &OverrunConfig {
        &self.additive_config.overrun_during
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

impl AsRef<MinHeatPumpRuntime> for PythonBrainConfig {
    fn as_ref(&self) -> &MinHeatPumpRuntime {
        &self.min_hp_runtime
    }
}

const PYTHON_BRAIN_CONFIG_FILE: &str = "python_brain.toml";

pub fn try_read_python_brain_config() -> Option<PythonBrainConfig> {
    try_read_python_brain_config_file(PYTHON_BRAIN_CONFIG_FILE)
}

const CONFIG_LOG_TARGET: &str = "config";

pub fn try_read_python_brain_config_file(path: impl AsRef<Path>) -> Option<PythonBrainConfig> {
    let python_brain_config = std::fs::read_to_string(path);
    let mut main_config: PythonBrainConfig = match python_brain_config {
        Ok(str) => {
            match toml::from_str(&str) {
                Ok(x) => Some(x),
                Err(e) => {
                    error!("Failed to deserialize python brain config {:?}", e);
                    None
                }
            }
        },
        Err(e) => {
            error!("Failed to read python brain config {:?}", e);
            None
        }
    }?;
    debug!(target: CONFIG_LOG_TARGET, "Base config: {:?}", main_config);
    let mut config_dirs_to_parse = main_config.additive_config.include_config_directories.clone();
    let mut parsed_config_directories = vec![];
    let mut additive_configs = vec![];


    while !config_dirs_to_parse.is_empty() {
        let mut found = read_additive_config_dirs(&config_dirs_to_parse);
        // Move all to_parse to parsed.
        parsed_config_directories.append(&mut config_dirs_to_parse);

        for additional in &found {
            for new_config_dir in &additional.include_config_directories {
                if parsed_config_directories.contains(&new_config_dir) {
                    debug!(target: CONFIG_LOG_TARGET, "Discovered new config directory to be parsed: {:?}", new_config_dir);
                    config_dirs_to_parse.push(new_config_dir.clone());
                }
            }
        }
        // Move all found into the additive configs.
        additive_configs.append(&mut found);
    }

    info!(target: CONFIG_LOG_TARGET, "Found {} extra config files", additive_configs.len());

    for additive in additive_configs {
        main_config.additive_config.combine(additive);
    }

    Some(main_config)
}

fn read_additive_config_dirs(directories: &Vec<PathBuf>) -> Vec<PythonBrainAdditiveConfig> {
    let mut additional_configs = vec![];
    for included_config_dir in directories {
        debug!(target: CONFIG_LOG_TARGET, "Locating additional config files in {:?}", included_config_dir);
        let dir = match included_config_dir.read_dir() {
            Ok(dir) => dir,
            Err(err) => {
                error!(target: CONFIG_LOG_TARGET, "Failed to get list of files in {:?}: {}", included_config_dir, err);
                continue;
            }
        };

        for file in dir {
            let dir_entry = match file {
                Ok(dir_entry) => dir_entry,
                Err(dir_entry_err) => {
                    error!(target: CONFIG_LOG_TARGET, "Failed to get directory listing for directory {:?}: {}", included_config_dir, dir_entry_err);
                    continue;
                }
            };

            if let Some(extension) = dir_entry.path().extension() {
                if extension != "toml" {
                    continue;
                }
            }
            else {
                continue;
            }

            match read_additive_config(dir_entry.path()) {
                Ok(additional_config) => {
                    debug!(target: CONFIG_LOG_TARGET, "Read additional config file {:?}", dir_entry.path());
                    additional_configs.push(additional_config);
                }
                Err(err) => {
                    error!(target: CONFIG_LOG_TARGET, "Failed to read additional config file: {:?}: {}", dir_entry.path(), err);
                }
            }
        }
    }
    additional_configs
}

pub fn read_additive_config(file: PathBuf) -> Result<PythonBrainAdditiveConfig, String> {
    let s = std::fs::read_to_string(&file)
        .map_err(|err| format!("Failed to read additional config file ({:?}): {}", file, err))?;

    toml::from_str(&s)
        .map_err(|err| format!("Error deserializing additional config file ({:?}): {}", file, err))
}

#[cfg(test)]
mod tests {
    use crate::brain::immersion_heater::config::ImmersionHeaterModelPart;
    use crate::python_like::config::overrun_config::OverrunBap;
    use crate::Sensor;
    use crate::time_util::test_utils::{local_time_slot, time, utc_time_slot};
    use super::*;

    #[test]
    fn test_deserialize_config() {
        let config_str = std::fs::read_to_string("test/python_brain/test_brain_config_with_overrun.toml").expect("Failed to read config file.");
        let config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");

        let mut expected = PythonBrainConfig::default();
        let baps = vec![
            OverrunBap::new(local_time_slot(01, 00, 00, 04, 30, 00), 50.1, "1".into()),
            OverrunBap::new_with_min(local_time_slot(03, 20, 00, 04, 30, 00), 46.0, "2".into(), 30.0),
            OverrunBap::new_with_min(local_time_slot(04, 00, 00, 04, 30, 00), 48.0, "3".into(), 47.0),
            OverrunBap::new(utc_time_slot(12, 00, 00, 14, 50, 00), 46.1, "4".into()),
            OverrunBap::new_with_min(utc_time_slot(11, 00, 00, 15, 50, 00), 21.5, "5".into(), 10.1),
        ];
        expected.additive_config.overrun_during = OverrunConfig::new(baps);
        assert_eq!(expected.get_overrun_during(), config.get_overrun_during(), "Overrun during not equal");
        assert_eq!(expected, config)
    }

    #[test]
    fn test_can_deserialize_full() {
        let config_str = std::fs::read_to_string("test/python_brain/test_brain_config.toml").expect("Failed to read config file.");
        let _config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");
    }

    #[test]
    fn test_deserialize_included_files() {
        let config = try_read_python_brain_config_file("test/python_brain/multiple_files/main.toml")
            .expect("Should get a config!");

        let expected = PythonBrainConfig {
            hp_circulation: HeatPumpCirculationConfig::new(70, 30, 300),
            hp_enable_time: Duration::from_secs(70),
            default_working_range: WorkingTemperatureRange::from_min_max(42.0, 45.0),
            working_temp_model: WorkingTempModelConfig::new(53.2, 10.0, 0.6, 2.5, 4.5),
            additive_config: PythonBrainAdditiveConfig {
                include_config_directories: vec!["test/python_brain/multiple_files/additional".into()],
                overrun_during: OverrunConfig::new(vec![
                    // The overruns in the main config.
                    OverrunBap::new_with_min(local_time_slot(00, 30, 00,
                                                             04, 30, 00),
                                             43.6, Sensor::TKTP, 36.0),

                    OverrunBap::new_with_min(local_time_slot(04, 00, 00,
                                                             04, 30, 00),
                                             43.0, Sensor::TKTP, 41.0),

                    OverrunBap::new_with_min(local_time_slot(04, 00, 00,
                                                             04, 30, 00),
                                             36.0, Sensor::TKBT, 30.0),

                    // The overrun in the additional config

                    OverrunBap::new_with_min(local_time_slot(00, 30, 00,
                                                             04, 30, 00),
                                             50.0, Sensor::TKFL, 45.0)
                ]),
                immersion_heater_model: ImmersionHeaterModelConfig::new(vec![
                    ImmersionHeaterModelPart::from_time_points((time(00,30,00), 35.0), (time(00, 36, 00), 35.0), Sensor::TKBT),
                ]),
                boost_active_rooms: Default::default(),
                no_heating: vec![
                    local_time_slot(04, 15, 00, 04, 30, 00),
                ],
            },
            ..Default::default()
        };

        assert_eq!(config, expected, "\nGot: {:#?}\n---------\nExpected: {:#?}", config, expected);
    }
}
