use std::collections::HashMap;
use tokio::runtime::Runtime;
use crate::python_like::heating_mode::{HeatingMode, SharedData};
use crate::{BrainFailure, IOBundle, PythonBrainConfig, Sensor, TemperatureManager};
use crate::python_like::working_temp::WorkingTemperatureRange;

pub mod circulate;

pub trait Mode {
    fn update(&mut self, shared_data: &mut SharedData, rt: &Runtime, config: &PythonBrainConfig, info_cache: &mut InfoCache, io_bundle: &mut IOBundle) -> Result<Intention, BrainFailure>;
}

pub enum Intention {
    /// Shows that the heating should
    /// switch its state to this state
    Change(ChangeState),
    SwitchForce(HeatingMode),
    KeepState,
}

impl Intention {
    /// Turn off immediately
    pub fn off_now() -> Intention {
        Intention::SwitchForce(HeatingMode::Off)
    }

    /// Shows that this state has ended,
    /// and so another state must begin,
    /// if no state believes it should activate
    /// then this will turn everything off.
    pub fn finish() -> Intention {
        Intention::Change(ChangeState::FinishMode)
    }

    /// Tells it to switch into the circulating mode.
    pub fn begin_circulating() -> Intention {
        Intention::Change(ChangeState::BeginCirculating)
    }
}

pub enum ChangeState {
    FinishMode,
    BeginCirculating,
}

pub struct InfoCache {
    heating_on: bool,
    temps: Option<Result<HashMap<Sensor, f32>, String>>,
    working_temp_range: (WorkingTemperatureRange, Option<f32>),
}

impl InfoCache {

    pub fn create(heating_on: bool, working_range: (WorkingTemperatureRange, Option<f32>)) -> Self {
        Self {
            heating_on,
            temps: None,
            working_temp_range: working_range,
        }
    }

    pub fn heating_on(&self) -> bool {
        self.heating_on
    }

    pub fn get_working_temp_range(&self) -> (WorkingTemperatureRange, Option<f32>) {
        self.working_temp_range.clone()
    }

    pub async fn get_temps(&mut self, temperature_manager: &dyn TemperatureManager) -> Result<HashMap<Sensor, f32>, String> {
        if self.temps.is_none() {
            self.temps = Some(temperature_manager.retrieve_temperatures().await);
        }
        self.temps.as_ref().unwrap().clone()
    }

}