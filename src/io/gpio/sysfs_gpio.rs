use std::collections::HashMap;
use sysfs_gpio::{Direction, Error, Pin};
use tokio::sync::mpsc::Sender;
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState, GPIOError, PinUpdate};

pub struct SysFsGPIO {
    gpios: HashMap<usize, Pin>,
    sender: Sender<PinUpdate>,
}

impl SysFsGPIO {
    pub fn new(sender: Sender<PinUpdate>) -> SysFsGPIO {
        SysFsGPIO {
            gpios: HashMap::new(),
            sender
        }
    }
}

impl GPIOManager for SysFsGPIO {
    fn setup(&mut self, pin_id: usize, mode: &GPIOMode) {
        println!("Setting up pin {}", pin_id);
        let pin = sysfs_gpio::Pin::new(pin_id as u64);
        let direction = match mode {
            GPIOMode::Input => Direction::In,
            GPIOMode::Output => Direction::High,
        };
        let direction_before = pin.get_direction()
            .expect("Expected to be able to read direction of pin");
        let already_at_mode = match direction_before {
            Direction::In => { matches!(mode, GPIOMode::Input) }
            Direction::Out => { matches!(mode, GPIOMode::Output) }
            Direction::High => { matches!(mode, GPIOMode::Output) }
            Direction::Low => { matches!(mode, GPIOMode::Output) }
        };
        if already_at_mode {
            self.gpios.insert(pin_id, pin);
            return;
        }
        println!("Actually having to set direction of pin {}", pin_id);
        pin.set_direction(direction)
            .expect("Expected to be able to set direction of pin");
        self.gpios.insert(pin_id, pin);
    }

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError> {
        //println!("Setting pin {} to {:?}", pin_id, state);
        let pin = self.gpios.get(&pin_id);
        if pin.is_none() {
            return Err(GPIOError::PinNotSetup);
        }
        let pin = pin.unwrap();
        let direction = pin.get_direction().map_err(|err| map_sysfs_err(err))?;
        if direction == Direction::In {
            return Err(GPIOError::PinInIncorrectMode {required_mode: GPIOMode::Output});
        }
        let bit_value = match state {
            GPIOState::HIGH => 1,
            GPIOState::LOW => 0,
        };
        let result = pin.set_value(bit_value)
            .map_err(|err| map_sysfs_err(err));

        if result.is_ok() {
            let send_result = self.sender.try_send(PinUpdate::new(pin_id, state.clone()));
            if send_result.is_err() {
                eprintln!("Error notifying sender of pin update {:?}", send_result);
            }
        }
        result
    }

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError> {
        let pin = self.gpios.get(&pin);
        if pin.is_none() {
            return Err(GPIOError::PinNotSetup);
        }
        let pin = pin.unwrap();
        let value = pin.get_value();
        value.map(|x| {
            match x {
                0 => GPIOState::LOW,
                1 => GPIOState::HIGH,
                _ => panic!("Breach of api contract / implementation")
            }
        }).map_err(|err| map_sysfs_err(err))
    }
}

fn map_sysfs_err(err: sysfs_gpio::Error) -> GPIOError {
    match err {
        Error::Io(err) => GPIOError::Io(err),
        Error::Unexpected(s) => GPIOError::Other(s),
        Error::InvalidPath(s) => GPIOError::Other(s),
        Error::Unsupported(s) => GPIOError::Other(s),
    }
}