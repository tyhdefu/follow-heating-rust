use crate::brain::BrainFailure;
use crate::GPIOManager;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};

pub const HEAT_CIRCULATION_PUMP: usize = 5;

pub trait HeatCirculationPumpControl {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure>;
}

impl<T> HeatCirculationPumpControl for T
    where T: GPIOManager {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(HEAT_CIRCULATION_PUMP, self, on, "Failed to set heating circulation pump pin")
    }

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(HEAT_CIRCULATION_PUMP, self, "Failed to get heating circulation pump pin")
    }
}