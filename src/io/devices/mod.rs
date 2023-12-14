use chrono::{DateTime, Utc, TimeZone, Duration};
use itertools::Itertools;
use rev_lines::RevLines;
use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
};
use log::warn;

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
}

impl DevicesFromFile {
    pub fn create(config: &DevicesFromFileConfig) -> Self {
        Self::new(config.get_file().to_owned(), config.get_active_within_minutes())
    }

    pub fn new(file: String, active_within_minutes: usize) -> Self {
        Self {
            file,
            active_within_minutes,
        }
    }
}

impl ActiveDevices for DevicesFromFile {
    fn get_active_devices(&mut self, time: &DateTime<Utc>) -> Result<Vec<Device>, BrainFailure> {
        self.get_active_devices_within(time, self.active_within_minutes)
    }

    fn get_active_devices_within(&mut self, time: &DateTime<Utc>, minutes: usize) -> Result<Vec<Device>, BrainFailure> {
        let file = File::open(&self.file).map_err(|err| {
            brain_fail!(format!("Failed to open {} for reading: {}", self.file, err))
        })?;

        let rev_lines = RevLines::new(BufReader::new(file))
            .map_err(|err| brain_fail!(format!("Failed to read backwards: {}", err)))?;

        let mut device_map: HashMap<Device, DateTime<Utc>> = HashMap::new();

        let cut_off = time.clone() - Duration::seconds(60 * minutes as i64);

        for line in rev_lines {
            match parse_line(&line) {
                Err(msg) => {
                    warn!("Error parsing active device line '{}' => {}", line, msg);
                    continue;
                }
                Ok((device, time)) => {
                    if time < cut_off {
                        //println!("reached cut off time: {}", cut_off);
                        break;
                    }
                    device_map.entry(device).or_insert(time);
                }
            }
        }

        Ok(device_map.into_keys().collect_vec())
    }
}

/// Parse a arp log line. Currently in the format:
/// 2023-12-14T11:58:24+00:00 58:94:6b:b3:ab:7c 192.168.0.27 PlayroomServer
fn parse_line(s: &str) -> Result<(Device, DateTime<Utc>), String> {
    let mut split = s.split(' ');

    let time_part = split.next()
        .ok_or_else(|| format!("No time part separated by ' ' (1st column)"))?;

    let time = DateTime::parse_from_str(&time_part, "%Y-%m-%dT%H:%M:%S%:z")
        .map(|dt| Utc.from_utc_datetime(&dt.naive_utc()))
        .map_err(|err| format!("Invalid date: '{}': {}", time_part, err))?;

    let _mac = split.next()
        .ok_or_else(|| format!("No mac addr part separated by ' ' (2nd column)"))?;

    let _ip = split.next()
        .ok_or_else(|| format!("No ip addr part separated by ' ' (3rd column)"))?;

    let device_name_part = split.next()
        .ok_or_else(|| format!("No device name part found separated by ' ' (4th column)"))?;

    if device_name_part.is_empty() {
        return Err(format!("Device name empty!"));
    }

    let device = Device::new(device_name_part.to_owned());

    Ok((device, time))
}

#[cfg(test)]
mod test {
    use chrono::{Utc, TimeZone, NaiveDate};
    use itertools::Itertools;
    use crate::brain::python_like::control::devices::{ActiveDevices, Device};
    use crate::io::devices::DevicesFromFile;

    use super::parse_line;

    #[test]
    fn test_parse() {
        let s = "2023-02-12T09:59:54+00:00 cc:32:e5:7c:a5:94 192.168.0.17 TP-LINK";
        let (device, time) = parse_line(s).unwrap();
        assert_eq!(device, Device::new("TP-LINK".to_owned()));
        let expected_time = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2023, 02, 12).unwrap().and_hms_opt(09, 59, 54).unwrap());
        assert_eq!(time, expected_time);
    }

    #[test]
    fn test_parse_daylight_savings() {
        let s = "2023-03-26T19:06:44+01:00 58:94:6b:b3:ab:7c 192.168.0.27 PlayroomServer";
        let (device, time) = parse_line(s).unwrap();
        assert_eq!(device, Device::new("PlayroomServer".to_owned()));
        let expected_time = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2023, 03, 26).unwrap().and_hms_opt(18, 06, 44).unwrap());
        assert_eq!(time, expected_time);
    }

    #[test]
    fn test_parse_file() {
        let time = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(2023, 12, 14).unwrap().and_hms_opt(12, 58, 29).unwrap());
        let mut devices_from_file = DevicesFromFile::new("test/python_brain/active_devices/arp-log.txt".to_owned(), 8);
        let mut active_devices = devices_from_file.get_active_devices(&time)
            .expect("Should work!")
            .into_iter()
            .map(|device| format!("{}", device))
            .sorted()
            .collect_vec();

        let mut expected: Vec<String> = vec![
            "PlayroomServer".into(),
            "VirginCableRouter".into(),
            "TP-LINK".into(),
            "OfficeComputer".into(),
            "LeoPhone".into(),
            "JamesComputer".into(),
            "Printer".into(),
            "InvensysControls".into(),
            "PI2".into(),
            "SittingRoomTV".into(),
            "JamesPhone".into(),
            // NOT TP-LINK2!
        ];
        expected.sort();

        assert_eq!(expected, active_devices);
    }
}