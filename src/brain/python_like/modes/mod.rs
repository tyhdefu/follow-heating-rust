use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use log::info;
use tokio::runtime::Runtime;
use crate::python_like::modes::heating_mode::SharedData;
use crate::{BrainFailure, IOBundle, PythonBrainConfig, Sensor, TemperatureManager};
use crate::brain::python_like::modes::intention::Intention;
use crate::python_like::working_temp::WorkingRange;
use crate::time_util::mytime::TimeProvider;

pub mod circulate;
pub mod heat_up_to;
pub mod heating_mode;
pub mod intention;

pub trait Mode {
    fn update(&mut self, shared_data: &mut SharedData, rt: &Runtime, config: &PythonBrainConfig, info_cache: &mut InfoCache, io_bundle: &mut IOBundle, time: &impl TimeProvider) -> Result<Intention, BrainFailure>;
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
            info!("{}", self.working_temp_range);
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
