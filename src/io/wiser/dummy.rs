use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Instant;
use crate::io;
use crate::io::dummy::DummyIO;
use crate::io::wiser::WiserManager;

pub enum ModifyState {
    SetHeatingOffTime(Instant),
    TurnOffHeating,
}

pub struct Dummy {
    receiver: Receiver<ModifyState>,
    heating_off_time: RefCell<Option<Instant>>,
}

impl WiserManager for Dummy {
    fn get_heating_turn_off_time(&self) -> Option<Instant> {
        self.update_state();
        self.heating_off_time.borrow().clone()
    }

    fn get_heating_on(&self) -> bool {
        self.update_state();
        self.heating_off_time.borrow().is_some()
    }
}

impl DummyIO for Dummy {
    type MessageType = ModifyState;

    fn new(receiver: Receiver<Self::MessageType>) -> Self {
        Dummy {
            receiver,
            heating_off_time: RefCell::new(None),
        }
    }
}

impl Dummy {
    fn update_state(&self) {
        io::dummy::read_all(&self.receiver, |message| {
            match message {
                ModifyState::SetHeatingOffTime(when) => self.heating_off_time.replace(Some(when)),
                ModifyState::TurnOffHeating => self.heating_off_time.replace(None),
            };
        })
    }
}