pub mod dummy;
pub mod sysfs_gpio;

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
    fn setup(&mut self, pin: usize, mode: &GPIOMode);

    fn set_pin(&mut self, pin_id: usize, state: &GPIOState) -> Result<(), GPIOError>;

    fn get_pin(&self, pin: usize) -> Result<GPIOState, GPIOError>;
}