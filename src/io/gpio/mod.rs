pub mod cdev;
pub mod dummy;

#[derive(Clone)]
pub enum GPIOState {
    HIGH,
    LOW,
}

pub enum GPIOMode {
    Input,
    Output,
}

pub trait GPIOManager {
    fn setup(&mut self, pin: usize, mode: &GPIOMode);

    fn set_pin(&mut self, pin: usize, state: &GPIOState);

    fn get_pin(&self, pin: usize) -> GPIOState;
}