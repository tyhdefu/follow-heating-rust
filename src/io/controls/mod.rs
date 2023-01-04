use crate::{brain_fail, GPIOManager, GPIOState};
use crate::brain::{BrainFailure, CorrectiveActions};

pub mod heating_impl;
pub mod misc_impl;

fn translate_set_gpio(pin: usize, gpio: &mut impl GPIOManager, on: bool, msg: &'static str) -> Result<(), BrainFailure> {
    let gpio_state = &if on {GPIOState::LOW} else {GPIOState::HIGH};
    gpio.set_pin(pin, gpio_state)
        .map_err(|gpio_err| brain_fail!(format!("{}: {:?}", msg, gpio_err), CorrectiveActions::unknown_heating()))
}

fn translate_get_gpio(pin: usize, gpio: &impl GPIOManager, msg: &'static str) -> Result<bool, BrainFailure> {
    gpio.get_pin(pin)
        .map(|state| matches!(state, GPIOState::LOW))
        .map_err(|err| brain_fail!(format!("{}: {:?}", msg, err), CorrectiveActions::unknown_heating()))
}