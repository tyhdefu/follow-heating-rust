use std::thread::sleep;
use std::time::Duration;

use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::config::ControlConfig;
use crate::io::controls::{translate_get_gpio, translate_set_gpio};
use crate::io::gpio::GPIOError;
use crate::python_like::control::heating_control::{HeatCirculationPumpControl, HeatPumpControl};
use crate::{brain_fail, GPIOManager, GPIOMode, HeatingControl};
use log::{debug, trace, warn};

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

#[derive(Debug)]
enum Valve {
    /// Closing this valve will stop water going through the tank.
    Tank,
    /// Closing this valve will stop water going through the heating.
    Heating,
}

#[derive(Debug)]
enum Pump {
    /// The heat pump that actually heats the hot water.
    #[allow(clippy::enum_variant_names)]
    HeatPump,
    /// The pump in series (sometimes) with the heat pump that helps to increase the flow and is
    /// also used for circulating
    ExtraHeating,
    /// The pump that pushes water through the radiators
    HeatingCirculation,
}

pub struct GPIOHeatingControl<G: GPIOManager> {
    gpio_manager: G,
    pins: GPIOPins,
    should_sleep: bool,
    valve_start_open_time: Duration,
    valve_change_time: Duration,
    pump_water_slow_time: Duration,
    extra_heat_pump_water_slow_time: Duration,
}

impl<G: GPIOManager> GPIOHeatingControl<G> {
    pub fn create(
        pins: GPIOPins,
        mut gpio_manager: G,
        control_config: &ControlConfig,
    ) -> Result<Self, GPIOError> {
        gpio_manager.setup(pins.heat_pump_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heat_circulation_pump_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.tank_valve_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heating_valve_pin, &GPIOMode::Output)?;
        gpio_manager.setup(pins.heating_extra_pump, &GPIOMode::Output)?;
        Ok(Self {
            gpio_manager,
            pins,
            should_sleep: true,
            valve_start_open_time: *control_config.get_valve_start_open_time(),
            valve_change_time: *control_config.get_valve_change_time(),
            pump_water_slow_time: *control_config.get_pump_water_slow_time(),
            extra_heat_pump_water_slow_time: *control_config.get_heat_pump_water_slow_time(),
        })
    }

    #[cfg(test)]
    pub fn create_no_sleep(pins: GPIOPins, gpio_manager: G) -> Result<Self, GPIOError> {
        let mut control = Self::create(pins, gpio_manager, &ControlConfig::default())?;
        control.should_sleep = false;
        Ok(control)
    }

    fn get_valve_pin(&self, valve: &Valve) -> usize {
        match valve {
            Valve::Tank => self.pins.tank_valve_pin,
            Valve::Heating => self.pins.heating_valve_pin,
        }
    }

    fn get_pump_pin(&self, pump: &Pump) -> usize {
        match pump {
            Pump::HeatPump => self.pins.heat_pump_pin,
            Pump::ExtraHeating => self.pins.heating_extra_pump,
            Pump::HeatingCirculation => self.pins.heat_circulation_pump_pin,
        }
    }

    fn set_valve(&mut self, valve: &Valve, open: bool) -> Result<(), BrainFailure> {
        let pin = self.get_valve_pin(valve);
        debug!(
            "Changing {:?} Valve (GPIO: {}) to {}",
            valve,
            pin,
            to_valve_state(open)
        );
        translate_set_gpio(
            pin,
            &mut self.gpio_manager,
            open,
            &format!("Failed to set {:?} Valve pin", valve),
        )
    }

    fn get_valve(&self, valve: &Valve) -> Result<bool, BrainFailure> {
        let pin = self.get_valve_pin(valve);
        translate_get_gpio(
            pin,
            &self.gpio_manager,
            &format!("Failed to get {:?} Valve pin", valve),
        )
    }

    fn set_pump(&mut self, pump: &Pump, on: bool) -> Result<(), BrainFailure> {
        let pin = self.get_pump_pin(pump);
        debug!(
            "Changing {:?} Pump (GPIO: {}) to {}",
            pump,
            pin,
            to_valve_state(on)
        );
        translate_set_gpio(
            pin,
            &mut self.gpio_manager,
            on,
            &format!("Failed to set {:?} Pump pin", pump),
        )
    }

    fn get_pump(&self, pump: &Pump) -> Result<bool, BrainFailure> {
        let pin = self.get_pump_pin(pump);
        translate_get_gpio(
            pin,
            &self.gpio_manager,
            &format!("Failed to get {:?} Pump pin", pump),
        )
    }

    fn wait_for(&self, amount: Duration, why: &str) {
        let reason = format!("Waiting {}s for {}", amount.as_secs(), why);
        #[cfg(test)]
        if !self.should_sleep {
            warn!("TESTING - SKIPPING {}", reason);
            return;
        }
        debug!("{}", reason);
        sleep(amount);
    }

    fn switch_to_configuration(
        &mut self,
        config: &ValveAndPumpConfiguration,
    ) -> Result<(), BrainFailure> {
        // We need to be careful how any in what order / when we change the state of valves / pumps
        // to avoid causing unecessary pressure / stress on the pipework.
        //
        // 0. Stop the heat pump if it needs to be stoppd as it takes longer to stop.
        // 1. Stop any pumps that need to be stopped
        // 2. Wait for water to slow / pressure to reduce.
        // 3. Open any valves that need opening
        // 4. Wait for them to start opening as they take longer to open than close.
        // 5. Close any valves that need closing
        // 6. Wait for all valves to change.
        // 7. Start any pumps
        let mut hp_stopped = false;
        if !config.heat_pump_on {
            hp_stopped = self.change_pump_if_needed(&Pump::HeatPump, false)?;
            if hp_stopped {
                self.wait_for(self.extra_heat_pump_water_slow_time, "HP to start slowing");
            } else {
                debug!("Heat pump not stopped - not waiting.")
            }
        }
        let normal_pumps_stopped = self.update_pumps_if_needed(config, false)?;
        if normal_pumps_stopped || hp_stopped {
            self.wait_for(self.pump_water_slow_time, "Pumps / Water to slow");
        } else {
            debug!("No pumps stopped - not waiting.");
        }

        let any_valves_opened = self.update_valves_if_needed(config, true)?;
        if any_valves_opened {
            self.wait_for(self.valve_start_open_time, "Valves to start opening");
        } else {
            debug!("No valves to open - not waiting.");
        }

        let any_valves_closed = self.update_valves_if_needed(config, false)?;

        if any_valves_opened || any_valves_closed {
            self.wait_for(self.valve_change_time, "Valves to change");
        } else {
            debug!("No valves to open or close - not waiting.");
        }

        self.update_pumps_if_needed(config, true)?;

        Ok(())
    }

    /// Change pumps' state to the given state if they are not already in that state.
    /// To turn on pumps that need turning on, call with to: true
    /// To turn off pumps that need turning off, call with to: false
    fn update_pumps_if_needed(
        &mut self,
        config: &ValveAndPumpConfiguration,
        to: bool,
    ) -> Result<bool, BrainFailure> {
        let mut any_pumps_changed = false;
        if config.heat_pump_on == to && self.change_pump_if_needed(&Pump::HeatPump, to)? {
            any_pumps_changed = true;
        }

        if config.extra_heating_pump_on == to
            && self.change_pump_if_needed(&Pump::ExtraHeating, to)?
        {
            any_pumps_changed = true;
        }
        Ok(any_pumps_changed)
    }

    /// Change valves' state to the given state if they are not already in that state.
    /// To open valves that need opening, call with to: true
    /// To turn off pumps that need closing, call with to: false
    fn update_valves_if_needed(
        &mut self,
        config: &ValveAndPumpConfiguration,
        to: bool,
    ) -> Result<bool, BrainFailure> {
        let mut any_valves_changed = false;
        if config.heating_valve_open == to && self.change_valve_if_needed(&Valve::Heating, to)? {
            any_valves_changed = true;
        }

        if config.tank_valve_open == to && self.change_valve_if_needed(&Valve::Tank, to)? {
            any_valves_changed = true;
        }
        Ok(any_valves_changed)
    }

    /// Change the valve to the given state if needed.
    /// Returns whether the valve was changed.
    fn change_valve_if_needed(&mut self, valve: &Valve, open: bool) -> Result<bool, BrainFailure> {
        if self.get_valve(valve)? == open {
            trace!("{:?} was already {}", valve, to_valve_state(open));
            return Ok(false);
        }
        self.set_valve(valve, open)?;
        Ok(true)
    }

    /// Change the pump to the given state if needed.
    /// Returns whether the pump was changed.
    fn change_pump_if_needed(&mut self, pump: &Pump, open: bool) -> Result<bool, BrainFailure> {
        if self.get_pump(pump)? == open {
            trace!("{:?} was already {}", pump, to_pump_state(open));
            return Ok(false);
        }
        self.set_pump(pump, open)?;
        Ok(true)
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
        debug!("Changing to HeatPumpMode {:?}", mode);
        let configuration = match mode {
            HeatPumpMode::HotWaterOnly => ValveAndPumpConfiguration {
                heat_pump_on: true,
                extra_heating_pump_on: false,
                tank_valve_open: true,
                heating_valve_open: false,
            },
            HeatPumpMode::HeatingOnly => ValveAndPumpConfiguration {
                heat_pump_on: true,
                extra_heating_pump_on: true,
                tank_valve_open: false,
                heating_valve_open: true,
            },
            HeatPumpMode::MostlyHotWater => ValveAndPumpConfiguration {
                heat_pump_on: true,
                extra_heating_pump_on: false,
                tank_valve_open: true,
                heating_valve_open: true,
            },
            HeatPumpMode::DrainTank => ValveAndPumpConfiguration {
                heat_pump_on: false,
                extra_heating_pump_on: true,
                tank_valve_open: true,
                heating_valve_open: true,
            },
            HeatPumpMode::Off => ValveAndPumpConfiguration {
                heat_pump_on: false,
                extra_heating_pump_on: false,
                tank_valve_open: false,
                heating_valve_open: false,
            },
        };
        self.switch_to_configuration(&configuration)?;

        Ok(())
    }

    fn try_get_heat_pump(&self) -> Result<HeatPumpMode, BrainFailure> {
        let tank_valve_open = self.get_valve(&Valve::Tank)?;
        let heating_valve_open = self.get_valve(&Valve::Heating)?;
        let extra_pump_on = self.get_pump(&Pump::ExtraHeating)?;

        if !self.get_pump(&Pump::HeatPump)? {
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
        self.set_pump(&Pump::HeatingCirculation, on)
    }

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure> {
        self.get_pump(&Pump::HeatingCirculation)
    }
}

fn to_valve_state(open: bool) -> String {
    match open {
        true => "Open",
        false => "Closed",
    }
    .to_owned()
}

fn to_pump_state(on: bool) -> String {
    match on {
        true => "On",
        false => "Off",
    }
    .to_owned()
}

struct ValveAndPumpConfiguration {
    heat_pump_on: bool,
    extra_heating_pump_on: bool,
    tank_valve_open: bool,
    heating_valve_open: bool,
}

#[cfg(test)]
mod test {
    use crate::brain::python_like::control::heating_control::{HeatPumpControl, HeatPumpMode};
    use crate::brain::BrainFailure;
    use crate::io::gpio::dummy::Dummy;
    use crate::io::gpio::{GPIOError, GPIOManager, GPIOState};

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
        let mut controls =
            GPIOHeatingControl::create_no_sleep(GPIO_PINS.clone(), gpio_manager).unwrap();

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
