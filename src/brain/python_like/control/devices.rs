use std::fmt::Display;

use chrono::{Utc, DateTime};
use serde::Deserialize;

use crate::brain::BrainFailure;

/// Handles obtaining data about which devices are active.
pub trait ActiveDevices {
    /// Get all devices currently considered active.
    fn get_active_devices(&mut self, time: &DateTime<Utc>) -> Result<Vec<Device>, BrainFailure>;

    /// Get all devices that were active within the last x minutes.
    fn get_active_devices_within(&mut self, time: &DateTime<Utc>, minutes: usize) -> Result<Vec<Device>, BrainFailure>;
}

#[derive(Debug, Deserialize, Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct Device {
    name: String,
}

impl Device {
    pub fn new(name: String) -> Self {
        Self {
            name,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
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