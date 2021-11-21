use tokio::runtime::Runtime;
use crate::brain::{Brain, BrainFailure};
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
    fn run<T, G, W>(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle<T,G,W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: GPIOManager {
        println!("Hello from brain");
        println!("Is heating on: {:?}", futures::executor::block_on(io_bundle.wiser().get_heating_on()));
        if let Some(off_time) = futures::executor::block_on(io_bundle.wiser().get_heating_turn_off_time()) {
            println!("Heating off time: {:?}", off_time);
        }
        Ok(())
    }
}