use crate::brain::python_like::modes::heating_mode::TargetTemperature;
use crate::io::temperatures::Sensor;
use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;
use std::time::Duration;

#[serde_as]
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MinHeatPumpRuntime {
    /// Duration that the heat pump must stay on for, regardless
    /// of whether overruns / the wiser says it should no longer be on.
    #[serde_as(as = "DurationSeconds")]
    duration_secs: Duration,

    /// A target temperature which if reached will allow the heat pump to turn off
    /// despite any mini
    safety_cut_off: TargetTemperature,
}

impl Default for MinHeatPumpRuntime {
    fn default() -> Self {
        Self {
            duration_secs: Duration::from_mins(6),
            safety_cut_off: TargetTemperature::new(Sensor::HPRT, 50.0),
        }
    }
}

