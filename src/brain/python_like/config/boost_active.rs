use serde::Deserialize;
use serde_with::serde_as;
use serde_with::FromInto;
use crate::brain::python_like::control::devices::Device;

/// Configuration for how PythonBrain handles active devices
#[derive(Deserialize, PartialEq, Debug)]
pub struct BoostActiveRoomsConfig {
    parts: Vec<BoostActiveRoom>,
}

impl BoostActiveRoomsConfig {
    pub fn get_parts(&self) -> &Vec<BoostActiveRoom> {
        &self.parts
    }
}

#[serde_as]
#[derive(Deserialize, PartialEq, Debug)]
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
    use std::fs::read_to_string;
    use crate::brain::python_like::config::boost_active::{BoostActiveRoom, BoostActiveRoomsConfig};
    use crate::brain::python_like::control::devices::Device;

    #[test]
    fn test_deserialize() {
        let s = read_to_string("test/python_brain/boost_active/basic.toml").expect("Failed to read file");
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
                    increase: 0.5
                },
                BoostActiveRoom {
                    device: Device::new("MyPhone".into()),
                    room: "RoomTwo".to_string(),
                    increase: 0.5
                }
            ]
        };

        assert_eq!(config, expected);
    }
}