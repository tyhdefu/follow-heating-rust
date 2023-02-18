use std::fmt::Display;
use std::str::FromStr;

use chrono::{Utc, DateTime};
use serde::Deserialize;

use crate::brain::BrainFailure;

pub trait ActiveDevices {
    fn get_active_devices(&mut self, time: &DateTime<Utc>) -> Result<Vec<Device>, BrainFailure>;
}

#[derive(Debug, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct Device {
    name: String,
}

impl Device {
    pub fn new(name: String) -> Self {
        Self {
            name,
        }
    }
}

impl From<String> for Device {
    fn from(value: String) -> Self {
        Device::new(value)
    }
}

impl Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}