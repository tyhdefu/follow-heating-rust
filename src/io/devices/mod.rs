use chrono::{DateTime, Utc, TimeZone, Duration};
use itertools::Itertools;
use rev_lines::RevLines;
use serde::de::Error;
use std::{
    collections::HashMap,
    fmt::Display,
    fs::File,
    io::BufReader,
    str::FromStr,
};
use log::warn;

use serde::Deserialize;

use crate::{
    brain::{
        python_like::control::devices::{ActiveDevices, Device},
        BrainFailure,
    },
    brain_fail,
    config::DevicesFromFileConfig,
};

pub mod dummy;

pub struct DevicesFromFile {
    file: String,
    active_within_minutes: usize,
    devices: HashMap<MacAddr, Device>,
}

impl DevicesFromFile {
    pub fn create(config: &DevicesFromFileConfig) -> Self {
        let mut map = HashMap::new();
        for (device, mac) in config.get_device_mac_addresses().clone().into_iter() {
            if let Some(old) = map.get(&mac) {
                warn!("Duplicate mac address {mac} for both '{device}' and '{old}' - Using the last one in the config.")
            }
            let device = Device::new(device);
            map.insert(mac, device);
        }
        Self {
            file: config.get_file().to_owned(),
            devices: map,
            active_within_minutes: config.get_active_within_minutes(),
        }
    }
}

impl ActiveDevices for DevicesFromFile {
    fn get_active_devices(&mut self, time: &DateTime<Utc>) -> Result<Vec<Device>, BrainFailure> {
        let file = File::open(&self.file).map_err(|err| {
            brain_fail!(format!("Failed to open {} for reading: {}", self.file, err))
        })?;
        
        let rev_lines = RevLines::new(BufReader::new(file))
            .map_err(|err| brain_fail!(format!("Failed to read backwards: {}", err)))?;
            
        let mut device_map: HashMap<Device, DateTime<Utc>> = HashMap::new();

        let cut_off = time.clone() - Duration::seconds(60 * self.active_within_minutes as i64);

        for line in rev_lines {
            
            match parse_line(&line) {
                Err(msg)=> {
                    warn!("Error parsing active device line '{}' => {}", line, msg);
                    continue;
                }
                Ok((mac, time)) => {
                    if time < cut_off {
                        //println!("reached cut off time: {}", cut_off);
                        break;
                    }
                    
                    if let Some(device) = self.devices.get(&mac) {
                        device_map.entry(device.clone()).or_insert(time);
                    }        
                }
            }
        }
        

        Ok(device_map.into_keys().collect_vec())
    }
}

fn parse_line(s: &str) -> Result<(MacAddr, DateTime<Utc>), String> {
    let mut split = s.split(' ');

    let time_part = split.next()
        .ok_or_else(|| format!("No time (first) part seperated by ' '"))?;

    let time = DateTime::parse_from_str(&time_part, "%Y-%m-%dT%H:%M:%S%:z")
        .map(|dt| Utc.from_utc_datetime(&dt.naive_utc()))
        .map_err(|err| format!("Invalid date: '{}': {}", time_part, err))?;

    let mac_part = split.next()
        .ok_or_else(|| format!("No mac addr (second) part seperated by ' '"))?;
   
    let mac: MacAddr = mac_part.parse()
        .map_err(|err| format!("Invalid mac address '{}': {}", mac_part, err))?;

    Ok((mac, time))
}

#[derive(PartialEq, Eq, Clone, Copy, Hash)]
pub struct MacAddr {
    b1: u8,
    b2: u8,
    b3: u8,
    b4: u8,
    b5: u8,
    b6: u8,
}

impl MacAddr {
    #[cfg(test)]
    pub fn new(b1: u8, b2: u8, b3: u8, b4: u8, b5: u8, b6:u8) -> Self {
        Self {
            b1,
            b2,
            b3,
            b4,
            b5,
            b6,
        }
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:o}:{:o}:{:o}:{:o}:{:o}:{:o}",
            self.b1, self.b2, self.b3, self.b4, self.b5, self.b6
        )
    }
}

impl core::fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl FromStr for MacAddr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const MAC_ADDR_BYTES: usize = 6;
        let mut data = vec![];
        for byte in s.splitn(MAC_ADDR_BYTES, ':') {
            let byte = u8::from_str_radix(&byte, 16) // HEX
                .map_err(|err| {
                    format!("Byte '{byte}' could not be parsed as a hexadecimal number: {err}")
                })?;
            data.push(byte);
        }

        if data.len() < MAC_ADDR_BYTES {
            return Err(format!(
                "Not enough parts in mac address, expected {MAC_ADDR_BYTES}, got: {}",
                data.len()
            ));
        }
        Ok(MacAddr {
            b1: data[0],
            b2: data[1],
            b3: data[2],
            b4: data[3],
            b5: data[4],
            b6: data[5],
        })
    }
}

impl<'de> Deserialize<'de> for MacAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        string.parse().map_err(|err| D::Error::custom(err))
    }
}

#[cfg(test)]
mod test {
    use chrono::{Utc, TimeZone, NaiveDate};

    use super::{MacAddr, parse_line};

    #[test]
    fn test_parse() {
        let s = "2023-02-12T09:59:54+00:00 00:00:00:00:00:00";
        let (mac, time) = parse_line(s).unwrap();
        assert_eq!(mac, MacAddr::new(0, 0, 0, 0, 0, 0));
        let expected_time = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2023, 02, 12).unwrap().and_hms_opt(09, 59, 54).unwrap());
        assert_eq!(time, expected_time);
    }

    #[test]
    fn test_parse_daylight_savings() {
        let s = "2023-03-26T19:06:44+01:00 01:00:00:00:00:00";
        let (mac, time) = parse_line(s).unwrap();
        assert_eq!(mac, MacAddr::new(1, 0, 0, 0, 0, 0));
        let expected_time = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2023, 03, 26).unwrap().and_hms_opt(18, 06, 44).unwrap());
        assert_eq!(time, expected_time);
    }
}