use crate::brain::BrainFailure;
use crate::GPIOManager;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};

pub const IMMERSION_HEATER: usize = 6;

pub trait ImmersionHeaterControl {
    fn try_set_immersion_heater(&mut self, on: bool) -> Result<(), BrainFailure>;

    fn try_get_immersion_heater(&self) -> Result<bool, BrainFailure>;
}

impl<T> ImmersionHeaterControl for T
    where T: GPIOManager {
    fn try_set_immersion_heater(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(IMMERSION_HEATER, self, on, "Failed to set immersion heater pin")
    }

    fn try_get_immersion_heater(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(IMMERSION_HEATER, self, "Failed to get immersion heater pin")
    }
}