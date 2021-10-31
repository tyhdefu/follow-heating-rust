use std::collections::HashMap;
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState};

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

    fn set_pin(&mut self, pin: usize, state: &GPIOState) {
        self.map.insert(pin, state.clone());
    }

    fn get_pin(&self, pin: usize) -> GPIOState {
        self.map.get(&pin).map(|x| x.clone()).unwrap_or(GPIOState::HIGH)
    }
}