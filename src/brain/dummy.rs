use tokio::runtime::Runtime;
use crate::brain::{Brain, BrainFailure};
use crate::io::IOBundle;
use crate::time::mytime::TimeProvider;

pub struct Dummy {
}

impl Dummy {
    pub fn new() -> Dummy {
        Dummy {}
    }
}

impl Brain for Dummy {
    fn run(&mut self, _runtime: &Runtime, io_bundle: &mut IOBundle, _time: &impl TimeProvider) -> Result<(), BrainFailure> {
        println!("Hello from brain");
        println!("Is heating on: {:?}", futures::executor::block_on(io_bundle.wiser().get_heating_on()));
        if let Some(off_time) = futures::executor::block_on(io_bundle.wiser().get_heating_turn_off_time()) {
            println!("Heating off time: {:?}", off_time);
        }
        Ok(())
    }

    fn reload_config(&mut self) {}
}