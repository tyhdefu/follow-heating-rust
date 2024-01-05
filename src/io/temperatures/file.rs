use std::{cell::RefCell, collections::HashMap, fs, path::PathBuf, sync::Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{error, trace, warn};
use serde::Deserialize;

use crate::io::live_data::{check_age, AgeType, CachedPrevious};

use super::{Sensor, TemperatureManager};

/// How old the temps.json file is allowed to be before being considered invalid.
const MAX_FILE_AGE: i64 = 60;
/// How old a sensor reading is allowed to be before the reading being considered stale.
const MAX_READING_AGE: i64 = 90;

pub struct LiveFileTemperatures {
    file: PathBuf,
    last_data: CachedPrevious<TempsFileData>,
}

impl LiveFileTemperatures {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            last_data: CachedPrevious::none(),
        }
    }

    pub fn read_temps_data(&self) -> Result<TempsFileData, String> {
        let s = fs::read_to_string(&self.file)
            .map_err(|e| format!("Failed to read {:?}: {}", self.file, e))?;

        serde_json::from_str(&s)
            .map_err(|e| format!("Failed to deserialize: {:?}: {}\n{}", self.file, e, s))
    }
}

#[async_trait]
impl TemperatureManager for LiveFileTemperatures {
    async fn retrieve_sensors(&mut self) -> Result<(), String> {
        Ok(())
    }

    async fn retrieve_temperatures(&self) -> Result<HashMap<Sensor, f32>, String> {
        let temps_data = match self.read_temps_data() {
            Ok(data) => {
                self.last_data.update(data.clone());
                data
            }
            Err(e) => {
                let previous_data = self.last_data.get().ok_or_else(|| {
                    format!(
                        "Failed to get temps ({:?}) and no last was available: {}",
                        self.file, e
                    )
                })?;
                warn!("Error reading current data: {}, using last valid", e);
                previous_data
            }
        };

        let file_age = check_age(temps_data.timestamp, MAX_FILE_AGE);
        match file_age.age_type() {
            AgeType::Good => {
                trace!("{:?}: {}", self.file, file_age);
            }
            AgeType::GettingOld => {
                warn!("{:?}: {}", self.file, file_age);
            }
            AgeType::TooOld => {
                return Err(format!(
                    "{:?}: {} - is it being updated?",
                    self.file, file_age
                ));
            }
        };

        let mut temps = HashMap::new();
        for (sensor, reading) in temps_data.temps {
            let reading_age = check_age(reading.timestamp, MAX_READING_AGE);
            match reading_age.age_type() {
                AgeType::Good => {
                    trace!("{} {}", sensor, reading_age);
                }
                AgeType::GettingOld => {
                    warn!("{} {} - will reject soon.", sensor, reading_age);
                }
                AgeType::TooOld => {
                    error!(
                        "Rejecting {} {} - Treating it as having no value.",
                        sensor, reading_age
                    );
                    continue;
                }
            };
            temps.insert(sensor, reading.value);
        }

        Ok(temps)
    }
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
pub struct TempsFileData {
    timestamp: DateTime<Utc>,
    temps: HashMap<Sensor, TimestampedTemperature>,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
pub struct TimestampedTemperature {
    value: f32,
    timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod test {
    use chrono::TimeZone;

    use crate::time_util::test_utils::{date, time};

    use super::*;

    const EXAMPLE_DATA: &str = r#"
    {
        "temps": {
            "TKBT": {
                "timestamp": "2024-01-03T19:51:42Z",
                "value": 14.79
            },
            "TKEN": {
                "timestamp": "2024-01-03T19:51:29Z",
                "value": 12.58
            },
            "TKEX": {
                "timestamp": "2024-01-03T19:51:29Z",
                "value": 46.54
            },
            "TKTP": {
                "timestamp": "2024-01-03T19:51:42Z",
                "value": 51.93
            }
        },
        "timestamp": "2024-01-03T19:51:42Z"
    }
    "#;

    #[test]
    fn test_deserialize() {
        let file_data: TempsFileData = serde_json::from_str(EXAMPLE_DATA).unwrap();

        let data_timestamp = Utc.from_utc_datetime(&date(2024, 1, 3).and_time(time(19, 51, 42)));
        let tkbt_tktp_timestamp =
            Utc.from_utc_datetime(&date(2024, 1, 3).and_time(time(19, 51, 42)));
        let tken_tkex_timestamp =
            Utc.from_utc_datetime(&date(2024, 1, 3).and_time(time(19, 51, 29)));

        let mut temps = HashMap::new();
        temps.insert(
            Sensor::TKBT,
            TimestampedTemperature {
                value: 14.79,
                timestamp: tkbt_tktp_timestamp,
            },
        );
        temps.insert(
            Sensor::TKEN,
            TimestampedTemperature {
                value: 12.58,
                timestamp: tken_tkex_timestamp,
            },
        );
        temps.insert(
            Sensor::TKEX,
            TimestampedTemperature {
                value: 46.54,
                timestamp: tken_tkex_timestamp,
            },
        );
        temps.insert(
            Sensor::TKTP,
            TimestampedTemperature {
                value: 51.93,
                timestamp: tkbt_tktp_timestamp,
            },
        );

        let expected = TempsFileData {
            timestamp: data_timestamp,
            temps,
        };
        assert_eq!(file_data, expected);
    }
}
