use crate::{GPIOManager, GPIOState};
use crate::brain::{BrainFailure, CorrectiveActions};

pub mod immersion_heater;
pub mod heat_pump;
pub mod heat_circulation_pump;

fn translate_set_gpio(pin: usize, gpio: &mut impl GPIOManager, on: bool, msg: &'static str) -> Result<(), BrainFailure> {
    let gpio_state = &if on {GPIOState::LOW} else {GPIOState::HIGH};
    gpio.set_pin(pin, gpio_state)
        .map_err(|gpio_err| BrainFailure::new(format!("{}: {:?}", msg, gpio_err), CorrectiveActions::unknown_gpio()))
}

fn translate_get_gpio(pin: usize, gpio: &impl GPIOManager, msg: &'static str) -> Result<bool, BrainFailure> {
    gpio.get_pin(pin)
        .map(|state| matches!(state, GPIOState::LOW))
        .map_err(|err| BrainFailure::new(format!("{}: {:?}", msg, err), CorrectiveActions::unknown_gpio()))
}