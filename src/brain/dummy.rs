use crate::brain::Brain;
use crate::io::gpio::GPIOManager;
use crate::io::IOBundle;
use crate::io::temperatures::TemperatureManager;
use crate::io::wiser::WiserManager;

pub struct Dummy {

}

impl Dummy {
    pub fn new() -> Dummy {
        Dummy {}
    }
}

impl Brain for Dummy {
    fn run<T, G, W>(&mut self, io_bundle: &mut IOBundle<T,G,W>) where T: TemperatureManager, G: GPIOManager, W: WiserManager {
        println!("Hello from brain");
    }
}