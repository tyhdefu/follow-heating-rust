use crate::brain::{BrainFailure, CorrectiveActions};
use crate::{brain_fail, GPIOManager, GPIOState};

pub mod heating_impl;
pub mod misc_impl;

fn translate_set_gpio(
    pin: usize,
    gpio: &mut impl GPIOManager,
    on: bool,
    msg: &str,
) -> Result<(), BrainFailure> {
    let gpio_state = &if on { GPIOState::Low } else { GPIOState::High };
    gpio.set_pin(pin, gpio_state).map_err(|gpio_err| {
        brain_fail!(
            format!("{}: {:?}", msg, gpio_err),
            CorrectiveActions::unknown_heating()
        )
    })
}

fn translate_get_gpio(
    pin: usize,
    gpio: &impl GPIOManager,
    msg: &str,
) -> Result<bool, BrainFailure> {
    gpio.get_pin(pin)
        .map(|state| matches!(state, GPIOState::Low))
        .map_err(|err| {
            brain_fail!(
                format!("{}: {:?}", msg, err),
                CorrectiveActions::unknown_heating()
            )
        })
}
