use tokio::sync::mpsc::Sender;
use crate::brain::BrainFailure;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};
use crate::python_like::control::misc_control::{ImmersionHeaterControl, WiserPowerControl};
use crate::{GPIOMode, MiscControls, PinUpdate};
use crate::{GPIOManager, SysFsGPIO};
use crate::io::gpio::GPIOError;

pub struct MiscGPIOControls {
    gpio: SysFsGPIO,
    immersion_heater_pin: usize,
    wiser_power_pin: usize,
}

impl MiscGPIOControls {
    pub fn create(immersion_heater_pin: usize, wiser_power_pin: usize, sender: Sender<PinUpdate>) -> Result<Self, GPIOError> {
        let mut gpio = SysFsGPIO::new(sender);
        gpio.setup(immersion_heater_pin, &GPIOMode::Output)?;
        gpio.setup(wiser_power_pin, &GPIOMode::Output)?;
        Ok(Self {
            gpio,
            immersion_heater_pin,
            wiser_power_pin,
        })
    }
}

impl MiscControls for MiscGPIOControls {
    fn as_ih(&mut self) -> &mut dyn ImmersionHeaterControl {
        self
    }

    fn as_wp(&mut self) -> &mut dyn WiserPowerControl {
        self
    }
}

impl ImmersionHeaterControl for MiscGPIOControls {
    fn try_set_immersion_heater(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(self.immersion_heater_pin, &mut self.gpio, on, "Failed to set immersion heater pin")
    }

    fn try_get_immersion_heater(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(self.immersion_heater_pin, &self.gpio, "Failed to get immersion heater pin")
    }
}

impl WiserPowerControl for MiscGPIOControls {
    // DEFAULT ON not OFF - So wrong way reported / set.

    fn try_set_wiser_power(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(self.wiser_power_pin, &mut self.gpio, !on, "Failed to set wiser power pin")
    }

    fn try_get_wiser_power(&mut self) -> Result<bool, BrainFailure> {
        translate_get_gpio(self.wiser_power_pin, &self.gpio, "Failed to get wiser power pin")
            .map(|b| !b)
    }
}