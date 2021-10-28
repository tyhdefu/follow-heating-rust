pub mod cdev;

enum GPIOState {
    HIGH,
    LOW,
}

enum GPIOMode {
    Input,
    Output,
}

trait GPIOManager {
    fn setup(&mut self, pin: usize, mode: &GPIOMode);

    fn set_pin(&mut self, pin: usize, state: &GPIOState);

    fn get_pin(&self, pin: usize) -> GPIOState;
}