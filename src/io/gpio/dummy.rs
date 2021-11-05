use std::collections::HashMap;
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState, GPIOError};

pub struct Dummy {
    map: HashMap<usize, GPIOState>
}

impl Dummy {
    pub fn new() -> Dummy {
        Dummy {
            map: HashMap::new(),
        }
    }
}

impl GPIOManager for Dummy {
    fn setup(&mut self, pin: usize, mode: &GPIOMode) {}

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError> {
        self.map.insert(pin_id, state.clone());
        Ok(())
    }

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError> {
        Ok(self.map.get(&pin).map(|x| x.clone()).unwrap_or(GPIOState::HIGH))
    }
}