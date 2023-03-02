use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use log::debug;
use crate::brain::BrainFailure;
use crate::{HeatingControl, ImmersionHeaterControl, MiscControls};
use crate::python_like::control::heating_control::{HeatCirculationPumpControl, HeatPumpControl};
use crate::python_like::control::misc_control::WiserPowerControl;

pub trait DummyIO {
    type MessageType;
    type Config;

    fn create(config: &Self::Config) -> (Self, Sender<Self::MessageType>) where Self: Sized {
        let (sender, receiver) = mpsc::channel();
        let dummy_obj = Self::new(receiver, &config);
        return (dummy_obj, sender);
    }

    fn new(receiver: Receiver<Self::MessageType>, config: &Self::Config) -> Self;
}

pub fn read_all<T, F>(receiver: &Receiver<T>, on_value: F)
    where F: Fn(T) {
    loop {
        match receiver.try_recv() {
            Ok(x) => on_value(x),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => panic!("Disconnected!")
        }
    }
}

pub struct DummyAllOutputs {
    heat_pump_on: bool,
    heat_circulation_pump: bool,
    immersion_heater_on: bool,
    wiser_power_on: bool,
}

impl Default for DummyAllOutputs {
    fn default() -> Self {
        Self {
            heat_pump_on: false,
            heat_circulation_pump: false,
            immersion_heater_on: false,
            wiser_power_on: true,
        }
    }
}

fn to_on_off(on: bool) -> String {
    String::from(match on {
        true => "On",
        false => "Off",
    })
}

impl HeatPumpControl for DummyAllOutputs {
    fn try_set_heat_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Set HP to {}", to_on_off(on));
        self.heat_pump_on = on;
        Ok(())
    }

    fn try_get_heat_pump(&self) -> Result<bool, BrainFailure> {
        Ok(self.heat_pump_on)
    }
}

impl HeatCirculationPumpControl for DummyAllOutputs {
    fn try_set_heat_circulation_pump(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Set CP to {}", to_on_off(on));
        self.heat_circulation_pump = on;
        Ok(())
    }

    fn try_get_heat_circulation_pump(&self) -> Result<bool, BrainFailure> {
        Ok(self.heat_circulation_pump)
    }
}

impl HeatingControl for DummyAllOutputs {
    fn as_hp(&mut self) -> &mut dyn HeatPumpControl {
        self
    }

    fn as_cp(&mut self) -> &mut dyn HeatCirculationPumpControl {
        self
    }
}

impl ImmersionHeaterControl for DummyAllOutputs {
    fn try_set_immersion_heater(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Set immersion heater to {}", to_on_off(on));
        self.immersion_heater_on = on;
        Ok(())
    }

    fn try_get_immersion_heater(&self) -> Result<bool, BrainFailure> {
        Ok(self.immersion_heater_on)
    }
}

impl WiserPowerControl for DummyAllOutputs {
    fn try_set_wiser_power(&mut self, on: bool) -> Result<(), BrainFailure> {
        debug!("Turned wiser power {}", to_on_off(on));
        self.wiser_power_on = on;
        Ok(())
    }

    fn try_get_wiser_power(&mut self) -> Result<bool, BrainFailure> {
        Ok(self.wiser_power_on)
    }
}

impl MiscControls for DummyAllOutputs {
    fn as_ih(&mut self) -> &mut dyn ImmersionHeaterControl {
        self
    }

    fn as_wp(&mut self) -> &mut dyn WiserPowerControl {
        self
    }
}