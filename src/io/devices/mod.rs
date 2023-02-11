use std::{fmt::Display, str::FromStr, collections::HashMap};
use serde::de::Error;

use serde::Deserialize;

use crate::brain::{python_like::control::devices::{Device, ActiveDevices}, BrainFailure};

pub mod dummy;

pub struct DevicesFromFile {
    file: String,
    active_within_minutes: usize,
    devices: HashMap<MacAddr, Device>,
}

impl DevicesFromFile {
    pub fn create(config: &DevicesFromFileConfig) -> Self {
        let mut map = HashMap::new();
        for (device, mac) in config.device_mac_addresses.clone().into_iter() {
            if let Some(old) = map.get(&mac) {
                eprintln!("Duplicate mac address {mac} for both '{device}' and '{old}' - Using the last one in the config.")

            }
            map.insert(mac, device);
        }
        Self {
            file: config.file.clone(),
            devices: map,
            active_within_minutes: config.active_within_minutes,
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct DevicesFromFileConfig {
    file: String,
    active_within_minutes: usize,
    #[serde(flatten)]
    device_mac_addresses: HashMap<Device, MacAddr>,
}

impl ActiveDevices for DevicesFromFile {
    fn get_active_devices(&mut self) -> Result<Vec<Device>, BrainFailure> {

        Ok(vec![])
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Hash)]
struct MacAddr {
    b1: u8,
    b2: u8,
    b3: u8,
    b4: u8,
    b5: u8,
    b6: u8,
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:o}:{:o}:{:o}:{:o}:{:o}:{:o}", self.b1, self.b2, self.b3, self.b4, self.b5, self.b6)
    }
}

impl FromStr for MacAddr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const MAC_ADDR_BYTES: usize = 6;
        let mut data = vec![];
        for byte in s.splitn(MAC_ADDR_BYTES, ':') {
            let byte = u8::from_str_radix(&byte, 16) // HEX
            .map_err(|err| format!("Byte '{byte}' could not be parsed as a hexadecimal number: {err}"))?;
            data.push(byte);
        }

        if data.len() < MAC_ADDR_BYTES {
            return Err(format!("Not enough parts in mac address, expected {MAC_ADDR_BYTES}, got: {}", data.len()));
        }
        Ok(MacAddr { b1: data[0], b2: data[1], b3: data[2], b4: data[3], b5: data[4], b6: data[5] })
    }
}

impl<'de> Deserialize<'de> for MacAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de> {
        let string = String::deserialize(deserializer)?;
        string.parse().map_err(|err| D::Error::custom(err))
    }
}