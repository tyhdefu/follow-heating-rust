use std::time::Instant;

pub mod dummy;

pub trait WiserManager {
    fn get_heating_turn_off_time(&self) -> Option<Instant>;

    fn get_heating_on(&self) -> bool;
}