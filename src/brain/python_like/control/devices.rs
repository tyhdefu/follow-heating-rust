use std::fmt::Display;

use serde::Deserialize;

use crate::brain::BrainFailure;

pub trait ActiveDevices {
    fn get_active_devices(&mut self) -> Result<Vec<Device>, BrainFailure>;
}

#[derive(Debug, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct Device {
    #[serde(flatten)]
    name: String,
}

impl Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}