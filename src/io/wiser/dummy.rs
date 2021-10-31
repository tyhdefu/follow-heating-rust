use std::time::Instant;
use crate::io::wiser::WiserManager;

pub struct Dummy {

}

impl Dummy {
    pub fn new() -> Dummy {
        Dummy {}
    }
}

impl WiserManager for Dummy {
    fn get_heating_turn_off_time(&self) -> Option<Instant> {
        None
    }

    fn get_heating_on(&self) -> bool {
        false
    }
}