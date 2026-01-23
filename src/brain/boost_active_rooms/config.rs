use std::time::Duration;

use crate::brain::python_like::control::devices::Device;
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;

use serde_with::FromInto;

/// Configuration for how PythonBrain handles active devices
#[serde_as]
#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(default)]
pub struct BoostActiveRoomsConfig {
    /// How long to not apply / cancel any boosts for after a third party has turned off the room
    /// we were boosting.
    #[serde_as(as = "DurationSeconds")]
    interefere_off_leave_alone_time: Duration,
    /// How long to not apply / cancel any boosts for after a third party has changed the boost
    /// temperature of a room we were boosting.
    #[serde_as(as = "DurationSeconds")]
    interfere_change_leave_alone_time: Duration,
    /// Individual room boost entries
    parts: Vec<BoostActiveRoom>,
}

impl BoostActiveRoomsConfig {
    pub fn get_parts(&self) -> &Vec<BoostActiveRoom> {
        &self.parts
    }

    pub fn combine(&mut self, mut other: Self) {
        self.parts.append(&mut other.parts);
    }

    pub fn get_interfere_off_leave_alone_time(&self) -> &Duration {
        &self.interefere_off_leave_alone_time
    }

    pub fn get_interfere_change_leave_alone_time(&self) -> &Duration {
        &self.interfere_change_leave_alone_time
    }
}

impl Default for BoostActiveRoomsConfig {
    fn default() -> Self {
        Self {
            interefere_off_leave_alone_time: Duration::from_mins(60),
            interfere_change_leave_alone_time: Duration::from_mins(60),
            parts: Vec::default(),
        }
    }
}

#[serde_as]
#[derive(Deserialize, PartialEq, Debug, Clone)]
pub struct BoostActiveRoom {
    #[serde_as(as = "FromInto<String>")]
    device: Device,
    room: String,
    increase: f32,
}

impl BoostActiveRoom {
    pub fn get_device(&self) -> &Device {
        &self.device
    }

    pub fn get_room(&self) -> &str {
        &self.room
    }

    pub fn get_increase(&self) -> f32 {
        self.increase
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::read_to_string;

    #[test]
    fn test_deserialize() {
        let s = read_to_string("test/python_brain/boost_active/basic.toml")
            .expect("Failed to read file");
        println!("{}", s);
        let config: BoostActiveRoomsConfig = toml::from_str(&s).expect("Failed to deserialize");

        let expected = BoostActiveRoomsConfig {
            parts: vec![
                BoostActiveRoom {
                    device: Device::new("MyComputer".into()),
                    room: "RoomOne".to_string(),
                    increase: 1.0,
                },
                BoostActiveRoom {
                    device: Device::new("MyPhone".into()),
                    room: "RoomOne".to_string(),
                    increase: 0.5,
                },
                BoostActiveRoom {
                    device: Device::new("MyPhone".into()),
                    room: "RoomTwo".to_string(),
                    increase: 0.5,
                },
            ],
            interfere_change_leave_alone_time: Duration::from_mins(60),
            interefere_off_leave_alone_time: Duration::from_mins(60),
        };

        assert_eq!(config, expected);
    }
}

