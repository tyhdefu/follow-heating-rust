use std::thread::sleep;
use std::time::Duration;

use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};
use crate::io::gpio::GPIOError;
use crate::python_like::control::heating_control::{HeatCirculationPumpControl, HeatPumpControl};
use crate::{brain_fail, GPIOManager, GPIOMode, HeatingControl};
use log::debug;

#[derive(Clone)]
pub struct GPIOPins {
    /// Turns on/off the heat pump and its valve
    pub heat_pump_pin: usize,
    /// Turns on/off the pump that pumps water through radiators
    pub heat_circulation_pump_pin: usize,
    /// Opens / closes the valve located just outside of TKRT.
    pub tank_valve_pin: usize,
    /// Opens / closes the valve between the TKFL and heat exchanger
    pub heating_valve_pin: usize,
    /// Turns on/off the pump between the TKFL and heat exchanger (next to the valve)
    pub heating_extra_pump: usize,
}

pub struct GPIOHeatingControl<G: GPIOManager> {
    gpio_manager: G,
    pins: GPIOPins,
}

impl<G: GPIOManager> GPIOHeatingControl<G> {
    pub fn create(pins: GPIOPins, mut gpio_manager: G) -> Result<Self, GPIOError> {
        gpio_manager.setup(pins.heat_pump_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heat_circulation_pump_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.tank_valve_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heating_valve_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heating_extra_pump, &GPIOMode::Output)?;
        Ok(Self { gpio_manager, pins })
    }

    fn set_valve(&mut self, pin: usize, open: bool, name: &str) -> Result<(), BrainFailure> {
        debug!("Changing {} Valve to {}", name, to_valve_state(open));
        translate_set_gpio(
            pin,
            &mut self.gpio_manager,
            open,
            &format!("Failed to set {} Valve pin", name),
        )
    }

    fn get_valve(&self, pin: usize, name: &str) -> Result<bool, BrainFailure> {
        translate_get_gpio(
            pin,
            &self.gpio_manager,
            &format!("Failed to get {} Valve pin", name),
        )
    }

    fn set_heat_pump_state(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Changing HP (and its valve) to {:?}", on);
        translate_set_gpio(
            self.pins.heat_pump_pin,
            &mut self.gpio_manager,
            on,
            "Failed to set Heat Pump pin",
        )
    }

    fn is_hp_on(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(
            self.pins.heat_pump_pin,
            &self.gpio_manager,
            "Failed to get Heat Pump pin",
        )
    }

    fn set_tank_valve(&mut self, open: bool) -> Result<(), BrainFailure> {
        self.set_valve(self.pins.tank_valve_pin, open, "Tank")
    }

    fn get_extra_heating_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(
            self.pins.heating_extra_pump,
            &self.gpio_manager,
            "Failed to get additional heating pump pin",
        )
    }

    fn set_heating_valve(&mut self, open: bool) -> Result<(), BrainFailure> {
        self.set_valve(self.pins.heating_valve_pin, open, "heating")
    }

    fn set_extra_heating_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        translate_set_gpio(
            self.pins.heating_extra_pump,
            &mut self.gpio_manager,
            on,
            "Failed to set additional heating pump",
        )
    }

    fn wait_for_valves_to_open() {
        debug!("Waiting for valves to open");
        sleep(Duration::from_secs(5))
    }

    fn wait_for_water_to_slow() {
        debug!("Waiting for water to slow down after turning off a pump");
        sleep(Duration::from_secs(5))
    }
}

impl<G: GPIOManager + 'static> HeatingControl for GPIOHeatingControl<G> {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl {
        self
    }

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl {
        self
    }
}

impl<G: GPIOManager> HeatPumpControl for GPIOHeatingControl<G> {
    fn try_set_heat_pump(&mut self, mode: HeatPumpMode) -> Result<(), BrainFailure> {
        match mode {
            HeatPumpMode::HotWaterOnly => {
                self.set_extra_heating_pump(false)?;
                Self::wait_for_water_to_slow();

                self.set_tank_valve(true)?;
                self.set_heating_valve(false)?;
                Self::wait_for_valves_to_open();

                self.set_heat_pump_state(true)?;
            }
            HeatPumpMode::HeatingOnly => {
                self.set_heating_valve(true)?;
                self.set_tank_valve(false)?;
                Self::wait_for_valves_to_open();

                self.set_extra_heating_pump(true)?;
                self.set_heat_pump_state(true)?;
            }
            HeatPumpMode::MostlyHotWater => {
                self.set_extra_heating_pump(false)?;
                Self::wait_for_water_to_slow();

                self.set_heating_valve(true)?;
                self.set_tank_valve(true)?;
                Self::wait_for_valves_to_open();

                self.set_heat_pump_state(true)?;
            }
            HeatPumpMode::DrainTank => {
                self.set_heat_pump_state(false)?;
                Self::wait_for_water_to_slow();

                self.set_tank_valve(true)?;
                self.set_heating_valve(true)?;
                Self::wait_for_valves_to_open();

                self.set_extra_heating_pump(true)?;
            }
            HeatPumpMode::Off => {
                self.set_heat_pump_state(false)?;
                self.set_extra_heating_pump(false)?;
                Self::wait_for_water_to_slow();

                self.set_tank_valve(false)?;
                self.set_heating_valve(false)?;
            }
        }
        Ok(())
    }

    fn try_get_heat_pump(&self) -> Result<HeatPumpMode, BrainFailure> {
        let tank_valve_open = self.get_valve(self.pins.tank_valve_pin, "Tank")?;
        let heating_valve_open = self.get_valve(self.pins.heating_valve_pin, "Heating")?;
        let extra_pump_on = self.get_extra_heating_pump()?;

        if !self.is_hp_on()? {
            if extra_pump_on && tank_valve_open && heating_valve_open {
                return Ok(HeatPumpMode::DrainTank);
            }
            return Ok(HeatPumpMode::Off);
        }

        if extra_pump_on && !heating_valve_open {
            return Err(brain_fail!(
                "Extra pump should not be on when its valve is not!"
            ));
        }

        Ok(match (tank_valve_open, heating_valve_open) {
            (true, true) => HeatPumpMode::MostlyHotWater,
            (true, false) => HeatPumpMode::HotWaterOnly,
            (false, true) => HeatPumpMode::HeatingOnly,
            (false, false) => {
                let msg = format!(
                    "Value configuration was invalid: HP is on. Tank Valve: {}, Heating Valve: {}",
                    to_valve_state(heating_valve_open),
                    to_valve_state(heating_valve_open)
                );
                return Err(brain_fail!(&msg));
            }
        })
    }
}

impl<G: GPIOManager> HeatCirculationPumpControl for GPIOHeatingControl<G> {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Setting CP to {}", if on { "On" } else { "Off" });
        translate_set_gpio(
            self.pins.heat_circulation_pump_pin,
            &mut self.gpio_manager,
            on,
            "Failed to set Heat Circulation Pump pin",
        )
    }

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure> {
        translate_get_gpio(
            self.pins.heat_circulation_pump_pin,
            &self.gpio_manager,
            "Failed to get Heat Circulation Pump pin",
        )
    }
}

fn to_valve_state(open: bool) -> String {
    match open {
        true => "Open",
        false => "Closed",
    }
    .to_owned()
}

#[cfg(test)]
mod test {
    use crate::{
        brain::{
            python_like::control::heating_control::{HeatPumpControl, HeatPumpMode},
            BrainFailure,
        },
        io::gpio::{dummy::Dummy, GPIOError, GPIOManager, GPIOState},
    };

    use super::{GPIOHeatingControl, GPIOPins};

    const GPIO_PINS: GPIOPins = GPIOPins {
        heat_pump_pin: 1000,
        heat_circulation_pump_pin: 1001,
        tank_valve_pin: 1002,
        heating_valve_pin: 1003,
        heating_extra_pump: 1004,
    };

    #[test]
    fn test_get_and_set() -> Result<(), BrainFailure> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        fn check_pin(
            controls: &mut GPIOHeatingControl<impl GPIOManager>,
            pin: usize,
            expected: GPIOState,
        ) {
            assert_eq!(controls.gpio_manager.get_pin(pin).unwrap(), expected);
        }

        controls.try_set_heat_pump(HeatPumpMode::HotWaterOnly)?;
        assert_eq!(controls.try_get_heat_pump()?, HeatPumpMode::HotWaterOnly);
        check_pin(&mut controls, GPIO_PINS.heat_pump_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.heating_extra_pump, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.heating_valve_pin, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.tank_valve_pin, GPIOState::Low);

        controls.try_set_heat_pump(HeatPumpMode::MostlyHotWater)?;
        assert_eq!(controls.try_get_heat_pump()?, HeatPumpMode::MostlyHotWater);
        check_pin(&mut controls, GPIO_PINS.heat_pump_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.heating_extra_pump, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.heating_valve_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.tank_valve_pin, GPIOState::Low);

        controls.try_set_heat_pump(HeatPumpMode::HeatingOnly)?;
        assert_eq!(controls.try_get_heat_pump()?, HeatPumpMode::HeatingOnly);
        check_pin(&mut controls, GPIO_PINS.heat_pump_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.heating_extra_pump, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.heating_valve_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.tank_valve_pin, GPIOState::High);

        controls.try_set_heat_pump(HeatPumpMode::DrainTank)?;
        assert_eq!(controls.try_get_heat_pump()?, HeatPumpMode::DrainTank);
        check_pin(&mut controls, GPIO_PINS.heat_pump_pin, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.heating_extra_pump, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.heating_valve_pin, GPIOState::Low);
        check_pin(&mut controls, GPIO_PINS.tank_valve_pin, GPIOState::Low);

        controls.try_set_heat_pump(HeatPumpMode::Off)?;
        assert_eq!(controls.try_get_heat_pump()?, HeatPumpMode::Off);
        check_pin(&mut controls, GPIO_PINS.heat_pump_pin, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.heating_extra_pump, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.heating_valve_pin, GPIOState::High);
        check_pin(&mut controls, GPIO_PINS.tank_valve_pin, GPIOState::High);

        Ok(())
    }

    #[test]
    fn test_error_on_get_bad_valves() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .gpio_manager
            .set_pin(GPIO_PINS.heating_extra_pump, &GPIOState::Low)?;

        controls
            .gpio_manager
            .set_pin(GPIO_PINS.heating_valve_pin, &GPIOState::High)?;

        controls
            .gpio_manager
            .set_pin(GPIO_PINS.heat_pump_pin, &GPIOState::Low)?;

        match controls.try_get_heat_pump() {
            Ok(state) => panic!("Expected error, got: {:?}", state),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_off_works() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .try_set_heat_pump(HeatPumpMode::HeatingOnly)
            .expect("Should be able to go into HeatingOnly HeatPumpMode");

        controls
            .try_set_heat_pump(HeatPumpMode::Off)
            .expect("Should be able to turn off Heat pump");

        let gpio = controls.gpio_manager;
        assert_eq!(gpio.get_pin(GPIO_PINS.heat_pump_pin)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_extra_pump)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_valve_pin)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.tank_valve_pin)?, GPIOState::High);

        Ok(())
    }

    #[test]
    fn test_heating_only_works() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .try_set_heat_pump(HeatPumpMode::Off)
            .expect("Should be able to turn off Heat pump");

        controls
            .try_set_heat_pump(HeatPumpMode::HeatingOnly)
            .expect("Should be able to go into HeatingOnly HeatPumpMode");

        let gpio = controls.gpio_manager;
        assert_eq!(gpio.get_pin(GPIO_PINS.heat_pump_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_extra_pump)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_valve_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.tank_valve_pin)?, GPIOState::High);

        Ok(())
    }

    #[test]
    fn test_hot_water_only_works() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .try_set_heat_pump(HeatPumpMode::Off)
            .expect("Should be able to turn off Heat pump");

        controls
            .try_set_heat_pump(HeatPumpMode::HotWaterOnly)
            .expect("Should be able to go into HotWaterOnly HeatPumpMode");

        let gpio = controls.gpio_manager;
        assert_eq!(gpio.get_pin(GPIO_PINS.heat_pump_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_extra_pump)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_valve_pin)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.tank_valve_pin)?, GPIOState::Low);

        Ok(())
    }

    #[test]
    fn test_drain_tank_works() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .try_set_heat_pump(HeatPumpMode::Off)
            .expect("Should be able to turn off Heat pump");

        controls
            .try_set_heat_pump(HeatPumpMode::DrainTank)
            .expect("Should be able to go into DrainTank HeatPumpMode");

        let gpio = controls.gpio_manager;
        assert_eq!(gpio.get_pin(GPIO_PINS.heat_pump_pin)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_extra_pump)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_valve_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.tank_valve_pin)?, GPIOState::Low);

        Ok(())
    }

    #[test]
    fn test_mostly_hot_water_works() -> Result<(), GPIOError> {
        let gpio_manager = Dummy::default();
        let mut controls = GPIOHeatingControl::create(GPIO_PINS.clone(), gpio_manager).unwrap();

        controls
            .try_set_heat_pump(HeatPumpMode::Off)
            .expect("Should be able to turn off Heat pump");

        controls
            .try_set_heat_pump(HeatPumpMode::MostlyHotWater)
            .expect("Should be able to go into MostlyHotWater HeatPumpMode");

        let gpio = controls.gpio_manager;
        assert_eq!(gpio.get_pin(GPIO_PINS.heat_pump_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_extra_pump)?, GPIOState::High);
        assert_eq!(gpio.get_pin(GPIO_PINS.heating_valve_pin)?, GPIOState::Low);
        assert_eq!(gpio.get_pin(GPIO_PINS.tank_valve_pin)?, GPIOState::Low);

        Ok(())
    }
}
