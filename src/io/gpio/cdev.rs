use std::collections::HashMap;
use gpio_cdev::{Chip, LineHandle};
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState};

const HANDLE_NAME: &str = "follow-heating-rust";

struct CDevGPIO {
    held_gpios: HashMap<u32, LineHandle>,
    chip: Chip
}

impl CDevGPIO {
    pub fn new() -> CDevGPIO {
        CDevGPIO {
            chip: Chip::new("/dev/gpiochip0").expect("Expected to be able to create chip instance."),
        }
    }
}

impl GPIOManager for CDevGPIO {
    fn setup(&mut self, pin: usize, mode: &GPIOMode) {
        self.chip.get_line(pin as u32).expect("").request()
        todo!()
    }

    fn set_pin(&mut self, pin: usize, state: &GPIOState) {
        todo!()
    }

    fn get_pin(&self, pin: usize) -> GPIOState {
        todo!()
    }
}