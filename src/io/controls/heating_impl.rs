use log::debug;
use tokio::sync::mpsc::Sender;
use crate::brain::BrainFailure;
use crate::io::gpio::GPIOError;
use crate::{GPIOManager, GPIOMode, HeatingControl, PinUpdate, SysFsGPIO};
use crate::io::controls::{translate_get_gpio, translate_set_gpio};
use crate::python_like::control::heating_control::{HeatCirculationPumpControl, HeatPumpControl};

pub struct GPIOHeatingControl {
    gpio_manager: SysFsGPIO,
    heat_pump_pin: usize,
    heat_circulation_pump_pin: usize,
}

impl GPIOHeatingControl {
    pub fn create(heat_pump_pin: usize, heat_circulation_pump_pin: usize, sender: Sender<PinUpdate>) -> Result<Self, GPIOError>{
        let mut gpio_manager = SysFsGPIO::new(sender);
        gpio_manager.setup(heat_pump_pin, &GPIOMode::Output)?;
        gpio_manager.setup(heat_circulation_pump_pin, &GPIOMode::Output)?;
        Ok(Self {
            gpio_manager,
            heat_pump_pin,
            heat_circulation_pump_pin,
        })
    }
}

impl HeatingControl for GPIOHeatingControl {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl {
        self
    }

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl {
        self
    }
}

impl HeatPumpControl for GPIOHeatingControl {
    fn try_set_heat_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Setting HP to {}", if on { "On" } else { "Off" });
        translate_set_gpio(self.heat_pump_pin, &mut self.gpio_manager, on, "Failed to set Heat Pump pin")
    }

    fn try_get_heat_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(self.heat_pump_pin, &self.gpio_manager, "Failed to get Heat Pump pin")
    }
}

impl HeatCirculationPumpControl for GPIOHeatingControl {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Setting CP to {}", if on { "On" } else { "Off" });
        translate_set_gpio(self.heat_circulation_pump_pin, &mut self.gpio_manager, on, "Failed to set Heat Circulation Pump pin")
    }

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(self.heat_circulation_pump_pin, &self.gpio_manager, "Failed to get Heat Circulation Pump pin")
    }
}