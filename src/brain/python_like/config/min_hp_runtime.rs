use std::time::Duration;
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;
use crate::brain::python_like::modes::heating_mode::TargetTemperature;
use crate::io::temperatures::Sensor;

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde_as]
pub struct MinHeatPumpRuntime {
    /// Duration that the heat pump must stay on for, regardless
    /// of whether overruns / the wiser says it should no longer be on.
    #[serde_as(as = "DurationSeconds")]
    duration_secs: Duration,

    /// A target temperature which if reached will allow the heat pump to turn off
    /// despite any mini
    safety_cut_off: TargetTemperature,
}

impl MinHeatPumpRuntime {
    pub fn get_min_runtime(&self) -> &Duration {
        &self.duration_secs
    }

    pub fn get_safety_cut_off(&self) -> &TargetTemperature {
        &self.safety_cut_off
    }
}

impl Default for MinHeatPumpRuntime {
    fn default() -> Self {
        Self {
            duration_secs: Duration::from_secs(6 * 60),
            safety_cut_off: TargetTemperature::new(Sensor::TKTP, 50.0),
        }
    }
}