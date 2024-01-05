use std::{fs, net::IpAddr, path::PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{error, trace, warn};
use serde::Deserialize;

use crate::io::live_data::{check_age, AgeType, CachedPrevious};

use super::{
    hub::{IpWiserHub, WiserHub, WiserRoomData},
    WiserManager,
};

pub struct FileAndHub {
    file: PathBuf,
    last_data: CachedPrevious<WiserFileData>,
    hub: IpWiserHub,
}

impl FileAndHub {
    pub fn new(file: PathBuf, ip: IpAddr, secret: String) -> Self {
        Self {
            file,
            hub: IpWiserHub::new(ip, secret),
            last_data: CachedPrevious::none(),
        }
    }

    fn retrieve_data(&self) -> Result<WiserFileData, String> {
        let data = fs::read_to_string(&self.file)
            .map_err(|e| format!("Error reading {:?}: {}", self.file, e))?;

        serde_json::from_str(&data)
            .map_err(|e| format!("Error deserializing {:?}: {}\n{}", self.file, e, data))
    }
}

/// How long before we reject the file for being too outdated.
/// If this is too old then our data collection is broken.
const MAX_FILE_AGE_SECONDS: i64 = 2 * 60;
/// How long before we reject the data within the file for being too outdated
/// i.e. how long wiser we allow wiser to not respond for before taking action
const MAX_WISER_AGE_SECONDS: i64 = 5 * 60;

#[async_trait]
impl WiserManager for FileAndHub {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>> {
        let data = self.hub.get_room_data().await;
        if let Err(e) = data {
            error!("Error retrieving hub data: {:?}", e);
            return None;
        }
        let data = data.unwrap();
        get_turn_off_time(&data)
    }

    async fn get_heating_on(&self) -> Result<bool, ()> {
        let wiser_file_data = match self.retrieve_data() {
            Ok(data) => {
                self.last_data.update(data.clone());
                data
            }
            Err(e) => match self.last_data.get() {
                Some(data) => {
                    warn!("Failed to get current wiser data: {}, using previous", e);
                    data
                }
                None => {
                    error!(
                        "Failed to get current wiser data: {}, and no previous available.",
                        e
                    );
                    return Err(());
                }
            },
        };

        let file_age = check_age(wiser_file_data.timestamp, MAX_FILE_AGE_SECONDS);
        match file_age.age_type() {
            AgeType::Good => {
                trace!("{:?} {}", self.file, file_age)
            }
            AgeType::GettingOld => warn!("{:?}: {}", self.file, file_age),
            AgeType::TooOld => {
                error!("{:?}: {} - file is not up to date", self.file, file_age);
                return Err(());
            }
        };

        let wiser_heating_age = check_age(
            wiser_file_data.wiser.heating.timestamp,
            MAX_WISER_AGE_SECONDS,
        );
        match wiser_heating_age.age_type() {
            AgeType::Good => {
                trace!("heating on in: {:?}: {}", self.file, wiser_heating_age);
            }
            AgeType::GettingOld => warn!("heating on in: {:?}: {}", self.file, wiser_heating_age),
            AgeType::TooOld => {
                error!("heating on in {:?} {}", self.file, wiser_heating_age);
                return Err(());
            }
        }
        return Ok(wiser_file_data.wiser.heating.on);
    }

    fn get_wiser_hub(&self) -> &dyn WiserHub {
        &self.hub
    }
}

fn get_turn_off_time(data: &[WiserRoomData]) -> Option<DateTime<Utc>> {
    data.iter()
        .filter_map(|room| room.get_override_timeout())
        .max()
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
struct WiserFileData {
    pub wiser: WiserData,
    pub timestamp: DateTime<Utc>,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
struct WiserData {
    pub heating: TimestampedOnValue,
    //pub away_mode: TimestampedOnValue,
}

#[derive(Deserialize, Debug, PartialEq, Clone)]
struct TimestampedOnValue {
    pub on: bool,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod test {
    use chrono::TimeZone;

    use crate::time_util::test_utils::{date, time};

    use super::*;

    const EXAMPLE_DATA: &str = r#"
    {
        "timestamp": "2024-01-03T15:35:32Z",
        "wiser": {
            "away_mode": {
                "on": false,
                "timestamp": "2024-01-03T15:35:29Z"
            },
            "heating": {
                "on": false,
                "timestamp": "2024-01-03T15:35:29Z"
            }
        }
    }
    "#;

    #[test]
    fn test_deserialize() {
        let actual: WiserFileData = serde_json::from_str(EXAMPLE_DATA).unwrap();

        let main_timestamp = Utc.from_utc_datetime(&date(2024, 1, 3).and_time(time(15, 35, 32)));
        let heating_timestamp = Utc.from_utc_datetime(&date(2024, 1, 3).and_time(time(15, 35, 29)));

        let expected = WiserFileData {
            timestamp: main_timestamp,
            wiser: WiserData {
                heating: TimestampedOnValue {
                    on: false,
                    timestamp: heating_timestamp,
                },
            },
        };

        assert_eq!(actual, expected);
    }
}
