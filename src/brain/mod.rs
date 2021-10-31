use crate::io::gpio::GPIOManager;
use crate::io::IOBundle;
use crate::io::temperatures::TemperatureManager;
use crate::io::wiser::WiserManager;

pub mod dummy;

pub trait Brain {
    fn run<T, G, W>(&mut self, io_bundle: &mut IOBundle<T,G,W>)
        where
            T: TemperatureManager,
            G: GPIOManager,
            W: WiserManager;
}