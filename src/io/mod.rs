pub mod gpio;
pub mod wiser;
pub mod temperatures;
pub mod dummy;
pub mod robbable;

use crate::TemperatureManager;
use crate::GPIOManager;
use crate::io::robbable::{Dispatchable, DispatchedRobbable, Robbable};
use crate::WiserManager;

pub struct IOBundle<T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager,
        W: WiserManager {
    temperature_manager: T,
    gpio: Dispatchable<G>,
    wiser: W
}

impl<T, G, W> IOBundle<T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager,
        W: WiserManager {

    pub fn new(temperature_manager: T, gpio: G, wiser: W) -> IOBundle<T, G, W> {
        IOBundle {
            temperature_manager,
            gpio: Dispatchable::of(gpio),
            wiser,
        }
    }

    pub fn temperature_manager(&self) -> &T {
        &self.temperature_manager
    }

    pub fn gpio(&mut self) -> &mut Dispatchable<G> {
        &mut self.gpio
    }

    pub fn dispatch_gpio(&mut self) -> Result<DispatchedRobbable<G>, ()> {
        if !matches!(self.gpio, Dispatchable::Available(_)) {
            return Err(());
        }
        let old = std::mem::replace(&mut self.gpio, Dispatchable::Changing);
        return if let Dispatchable::Available(available) = old {
            let (robbable, dispatched) = available.dispatch();
            self.gpio = Dispatchable::InUse(robbable);
            Ok(dispatched)
        }
        else {
            self.gpio = old;
            println!("GPIO should have been in an available state as we had checked just before.");
            Err(())
        };
    }

    pub fn wiser(&self) -> &W {
        &self.wiser
    }
}

fn assign_dispatched<T>(tuple: (Robbable<T>, DispatchedRobbable<T>), target: &mut Option<DispatchedRobbable<T>>) -> Dispatchable<T> {
    let (robbable, dispatched) = tuple;
    *target = Some(dispatched);
    return Dispatchable::InUse(robbable)
}