use std::collections::HashMap;
use chrono::Utc;
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState, GPIOError};

pub struct Dummy {
    map: HashMap<usize, GPIOState>
}

impl GPIOManager for Dummy {
    fn setup(&mut self, _pin: usize, _mode: &GPIOMode) -> Result<(), GPIOError> {
        Ok(())
    }

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError> {
        println!("{} Setting pin {} to {:?}", Utc::now().format("%H:%M:%S"), pin_id, state);
        self.map.insert(pin_id, state.clone());
        Ok(())
    }

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError> {
        Ok(self.map.get(&pin).map(|x| x.clone()).unwrap_or(GPIOState::HIGH))
    }
}