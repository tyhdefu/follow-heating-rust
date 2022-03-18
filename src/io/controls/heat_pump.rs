use crate::brain::BrainFailure;
use crate::GPIOManager;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};

pub const HEAT_PUMP_RELAY: usize = 26;

pub trait HeatPumpControl {
    fn try_set_heat_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_pump(&self) -> Result<bool, BrainFailure>;
}

impl<T> HeatPumpControl for T
    where T: GPIOManager {
    fn try_set_heat_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(HEAT_PUMP_RELAY, self, on, "Failed to set heat pump pin")
    }

    fn try_get_heat_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(HEAT_PUMP_RELAY, self, "Failed to get heat pump pin")
    }
}