use crate::brain::modes::intention::Intention;
use crate::time_util::mytime::TimeProvider;
use crate::{BrainFailure, IOBundle, PythonBrainConfig, Sensor, TemperatureManager};
use log::*;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use tokio::runtime::Runtime;

use self::working_temp::WorkingRange;

use super::python_like::config::overrun_config::DhwBap;

pub mod circulate;
pub mod dhw_only;
pub mod mixed;
pub mod equalise;
mod off;
pub mod on;
pub mod pre_circulate;
pub mod try_circulate;
pub mod turning_on;
pub mod working_temp;

pub mod heating_mode;

pub mod intention;

pub trait Mode: PartialEq {
    fn enter(
        &mut self,
        config: &PythonBrainConfig,
        runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure>;

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure>;
}

pub struct InfoCache {
    heating_state: HeatingState,
    temps: Option<Result<HashMap<Sensor, f32>, String>>,
    working_temp_range: WorkingRange,
}

impl InfoCache {
    pub fn create(heating_state: HeatingState, working_range: WorkingRange) -> Self {
        Self {
            heating_state,
            temps: None,
            working_temp_range: working_range,
        }
    }

    pub fn heating_on(&self) -> bool {
        self.heating_state.is_on()
    }

    /// Whether the wiser is calling for space heating
    pub fn heating_state(&self) -> &HeatingState {
        &self.heating_state
    }

    pub fn get_working_temp_range(&self) -> WorkingRange {
        self.working_temp_range.clone()
    }

    pub async fn get_temps(
        &mut self,
        temperature_manager: &dyn TemperatureManager,
    ) -> Result<HashMap<Sensor, f32>, String> {
        if self.temps.is_none() {
            self.temps = Some(temperature_manager.retrieve_temperatures().await);
        }
        self.temps.as_ref().unwrap().clone()
    }

    #[cfg(test)]
    pub fn reset_cache(&mut self) {
        self.temps = None;
    }
}

/// Whether the wiser is calling for space heating
/// Makes code more understandable and implements display.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct HeatingState(bool);

impl HeatingState {
    /// A state representing the wiser NOT calling for space heating
    pub const OFF: HeatingState = HeatingState::new(false);
    /// A state representing the wiser calling for space heating
    pub const ON: HeatingState = HeatingState::new(true);

    /// Create a new heating state. If on is true, the heating state is ON.
    pub const fn new(on: bool) -> Self {
        Self(on)
    }

    /// Check whether this heating state is on.
    pub fn is_on(&self) -> bool {
        self.0
    }

    /// Check whether this heating state is off.
    pub fn is_off(&self) -> bool {
        !self.is_on()
    }
}

impl Display for HeatingState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", if self.is_on() { "on" } else { "off" })
    }
}

/// Whether it is allowed to be in DHW mixed mode
/// This does not consider whether there is demand for heat, just the DHW dynamics
fn allow_dhw_mixed(temps: &HashMap<Sensor, f32>, slot: &DhwBap, mixing: bool) -> AllowDhwMixed {
    let temp = match temps.get(&slot.temps.sensor) {
        Some(temp) => temp,
        None => {
            error!("Sensor {} targeted by overrun didn't have a temperature associated.", slot.temps.sensor);
            return AllowDhwMixed::Error;
        }
    };

    if let Some(mixed) = &slot.mixed {
        let diff = temps.get(&Sensor::HPFL).unwrap_or(&0.0) - temps.get(&Sensor::TKTP).unwrap_or(&0.0);
        let ref_diff = if mixing { mixed.stop_hpfl_tktp_diff } else { mixed.start_hpfl_tktp_diff };
        if diff >= ref_diff {
            warn!("Legacy path: HPFL-TKTP={diff} >= {ref_diff} so forcing mixed regardless of minimum of {:.2?}",
                slot.temps.min);
            return AllowDhwMixed::Force;
        }
        else {
            debug!("HPFL-TKTP={diff} < {ref_diff} so not forcing mixed");
        }
    }

    if *temp >= slot.temps.min {
        return AllowDhwMixed::Can;
    }

    //info!("Below minimum of {:.2} - Overriding call for heat", slot.temps.min);

    return AllowDhwMixed::Cannot;
}

enum AllowDhwMixed {
    Error,
    Can,
    Force,
    Cannot,
}
