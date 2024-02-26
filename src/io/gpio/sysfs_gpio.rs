use crate::io::gpio::{GPIOError, GPIOManager, GPIOMode, GPIOState, PinUpdate};
use log::{debug, error, trace, warn};
use std::{collections::HashMap, thread::sleep, time::Duration};
use sysfs_gpio::{Direction, Error, Pin};
use tokio::sync::mpsc::Sender;

pub struct SysFsGPIO {
    gpios: HashMap<usize, Pin>,
    sender: Sender<PinUpdate>,
}

impl SysFsGPIO {
    pub fn new(sender: Sender<PinUpdate>) -> SysFsGPIO {
        SysFsGPIO {
            gpios: HashMap::new(),
            sender,
        }
    }
}

impl GPIOManager for SysFsGPIO {
    fn setup(&mut self, pin_id: usize, mode: &GPIOMode) -> Result<(), GPIOError> {
        debug!("Setting up pin {}", pin_id);
        let pin = sysfs_gpio::Pin::new(pin_id as u64);
        let direction = match mode {
            GPIOMode::Input => Direction::In,
            GPIOMode::Output => Direction::High,
        };
        pin.export()?;
        let direction_before = pin
            .get_direction()
            .expect("Expected to be able to read direction of pin");
        let already_at_mode = match direction_before {
            Direction::In => {
                matches!(mode, GPIOMode::Input)
            }
            Direction::Out => {
                matches!(mode, GPIOMode::Output)
            }
            Direction::High => {
                matches!(mode, GPIOMode::Output)
            }
            Direction::Low => {
                matches!(mode, GPIOMode::Output)
            }
        };
        if already_at_mode {
            self.gpios.insert(pin_id, pin);
            return Ok(());
        }

        const MAX_ATTEMPTS: usize = 5;
        let mut attempt = 0;
        while let Err(e) = pin.set_direction(direction) {
            warn!(
                "Failed to set direction of pin {} - Attempt {}",
                pin_id, attempt
            );
            if attempt >= MAX_ATTEMPTS {
                return Err(e.into());
            }
            attempt += 1;
            sleep(Duration::from_millis(400));
        }

        warn!("Set direction of pin {} on attempt {}", pin_id, attempt);
        self.gpios.insert(pin_id, pin);
        Ok(())
    }

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError> {
        trace!("Setting pin {} to {:?}", pin_id, state);
        let pin = self.gpios.get(&pin_id);
        if pin.is_none() {
            return Err(GPIOError::PinNotSetup);
        }
        let pin = pin.unwrap();
        let direction = pin.get_direction()?;
        if direction == Direction::In {
            return Err(GPIOError::PinInIncorrectMode {
                required_mode: GPIOMode::Output,
            });
        }
        let current_state = self.get_pin(pin_id)?;
        if current_state == *state {
            trace!("Pin {} was already {:?}", pin_id, state);
            return Ok(());
        }
        let bit_value = match state {
            GPIOState::High => 1,
            GPIOState::Low => 0,
        };
        let result = pin.set_value(bit_value);

        if result.is_ok() {
            let send_result = self.sender.try_send(PinUpdate::new(pin_id, state.clone()));
            if send_result.is_err() {
                error!("Error notifying sender of pin update {:?}", send_result);
            }
        }
        result.map_err(|err| err.into())
    }

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError> {
        let pin = self.gpios.get(&pin);
        if pin.is_none() {
            return Err(GPIOError::PinNotSetup);
        }
        let pin = pin.unwrap();
        let value = pin.get_value();
        Ok(value.map(|x| match x {
            0 => GPIOState::Low,
            1 => GPIOState::High,
            _ => panic!("Breach of api contract / implementation"),
        })?)
    }
}

impl From<sysfs_gpio::Error> for GPIOError {
    fn from(err: sysfs_gpio::Error) -> Self {
        match err {
            Error::Io(err) => GPIOError::Io(err),
            Error::Unexpected(s) => GPIOError::Other(s),
            Error::InvalidPath(s) => GPIOError::Other(s),
            Error::Unsupported(s) => GPIOError::Other(s),
        }
    }
}
