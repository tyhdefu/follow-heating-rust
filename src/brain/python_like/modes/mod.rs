use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Runtime;
use crate::python_like::heating_mode::{HeatingMode, SharedData};
use crate::{BrainFailure, IOBundle, PythonBrainConfig, Sensor, TemperatureManager};
use crate::python_like::working_temp::WorkingRange;
use crate::time::mytime::TimeProvider;

pub mod circulate;
pub mod heat_up_to;

pub trait Mode {
    fn update(&mut self, shared_data: &mut SharedData, rt: &Runtime, config: &PythonBrainConfig, info_cache: &mut InfoCache, io_bundle: &mut IOBundle, time: &impl TimeProvider) -> Result<Intention, BrainFailure>;
}

#[derive(Debug)]
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

#[derive(Debug)]
pub enum ChangeState {
    FinishMode,
    BeginCirculating,
}

pub struct InfoCache {
    heating_on: bool,
    temps: Option<Result<HashMap<Sensor, f32>, String>>,
    working_temp_range: WorkingRange,
    working_temp_range_printed: AtomicBool,
}

impl InfoCache {

    pub fn create(heating_on: bool, working_range: WorkingRange) -> Self {
        Self {
            heating_on,
            temps: None,
            working_temp_range: working_range,
            working_temp_range_printed: AtomicBool::new(false),
        }
    }

    pub fn heating_on(&self) -> bool {
        self.heating_on
    }

    pub fn get_working_temp_range(&self) -> WorkingRange {
        if !self.working_temp_range_printed.swap(true, Ordering::Relaxed) {
            println!("{}", self.working_temp_range);
        }
        self.working_temp_range.clone()
    }

    pub async fn get_temps(&mut self, temperature_manager: &dyn TemperatureManager) -> Result<HashMap<Sensor, f32>, String> {
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