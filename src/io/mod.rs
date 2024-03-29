pub mod controls;
pub mod devices;
pub mod dummy;
pub mod dummy_io_bundle;
pub mod gpio;
pub mod live_data;
pub mod robbable;
pub mod temperatures;
pub mod wiser;

use crate::brain::python_like::control::devices::ActiveDevices;
use crate::io::robbable::{Dispatchable, DispatchedRobbable};
use crate::python_like::control::heating_control::HeatingControl;
use crate::python_like::control::misc_control::MiscControls;
use crate::TemperatureManager;
use crate::WiserManager;
use log::error;

pub struct IOBundle {
    temperature_manager: Box<dyn TemperatureManager>,
    heating_control: Dispatchable<Box<dyn HeatingControl>>,
    misc_controls: Box<dyn MiscControls>,
    wiser: Box<dyn WiserManager>,
    active_devices: Box<dyn ActiveDevices>,
}

impl IOBundle {
    pub fn new(
        temperature_manager: impl TemperatureManager + 'static,
        heating_control: impl HeatingControl + 'static,
        misc_controls: impl MiscControls + 'static,
        wiser: impl WiserManager + 'static,
        active_devices: impl ActiveDevices + 'static,
    ) -> IOBundle {
        IOBundle {
            temperature_manager: Box::new(temperature_manager),
            heating_control: Dispatchable::of(Box::new(heating_control)),
            misc_controls: Box::new(misc_controls),
            wiser: Box::new(wiser),
            active_devices: Box::new(active_devices),
        }
    }

    pub fn temperature_manager(&self) -> &dyn TemperatureManager {
        &*self.temperature_manager
    }

    pub fn heating_control(&mut self) -> &mut Dispatchable<Box<dyn HeatingControl>> {
        &mut self.heating_control
    }

    pub fn dispatch_heating_control(
        &mut self,
    ) -> Result<DispatchedRobbable<Box<dyn HeatingControl>>, ()> {
        if !matches!(self.heating_control, Dispatchable::Available(_)) {
            return Err(());
        }
        let old = std::mem::replace(&mut self.heating_control, Dispatchable::Changing);
        if let Dispatchable::Available(available) = old {
            let (robbable, dispatched) = available.dispatch();
            self.heating_control = Dispatchable::InUse(robbable);
            Ok(dispatched)
        } else {
            self.heating_control = old;
            error!("GPIO should have been in an available state as we had checked just before.");
            Err(())
        }
    }

    pub fn misc_controls(&mut self) -> &mut dyn MiscControls {
        &mut *self.misc_controls
    }

    pub fn wiser(&self) -> &dyn WiserManager {
        &*self.wiser
    }

    pub fn active_devices(&mut self) -> &mut dyn ActiveDevices {
        &mut *self.active_devices
    }
}

