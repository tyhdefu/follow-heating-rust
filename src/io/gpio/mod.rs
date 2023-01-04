pub mod dummy;
pub mod sysfs_gpio;
pub mod update_db_with_gpio;

#[derive(Clone, Debug)]
pub enum GPIOState {
    HIGH,
    LOW,
}

#[derive(Debug)]
pub enum GPIOMode {
    Input,
    Output,
}

#[derive(Debug)]
pub enum GPIOError {
    PinNotSetup,
    PinInIncorrectMode {required_mode: GPIOMode},
    Io(std::io::Error),
    Other(String),
}

pub trait GPIOManager {
    fn setup(&mut self, pin: usize, mode: &GPIOMode) -> Result<(), GPIOError>;

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError>;

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError>;
}

#[derive(Debug)]
pub struct PinUpdate {
    pin: usize,
    to: GPIOState,
}

impl PinUpdate {
    pub fn new(pin: usize, to: GPIOState) -> Self {
        PinUpdate {
            pin,
            to
        }
    }
}