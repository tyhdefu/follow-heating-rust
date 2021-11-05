pub mod gpio;
pub mod wiser;
pub mod temperatures;
pub mod dummy;

use std::sync::{Arc, Mutex};
use crate::TemperatureManager;
use crate::GPIOManager;
use crate::WiserManager;

pub struct IOBundle<T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager,
        W: WiserManager {
    temperature_manager: T,
    gpio: Option<G>,
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
            gpio: Some(gpio),
            wiser,
        }
    }

    pub fn temperature_manager(&self) -> &T {
        &self.temperature_manager
    }

    pub fn gpio(&mut self) -> &mut Option<G> {
        &mut self.gpio
    }

    pub fn wiser(&self) -> &W {
        &self.wiser
    }
}