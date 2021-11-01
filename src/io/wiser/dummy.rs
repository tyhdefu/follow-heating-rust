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

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use super::*;

    #[tokio::test]
    async fn dummy_starts_off() {
        let (wiser, sender) = Dummy::create();
        assert_eq!(wiser.get_heating_on(), false, "Dummy should start off");
        assert_eq!(wiser.get_heating_turn_off_time(), None, "Dummy should start with empty off time since it starts off")
    }

    #[tokio::test]
    async fn dummy_turn_on() {
        let off_time = Instant::now() + Duration::from_secs(1234);
        let (wiser, sender) = get_on_dummy_with_off_time(off_time);
        assert_eq!(wiser.get_heating_on(), true, "Should now be on");
        assert_eq!(wiser.get_heating_turn_off_time(), Some(off_time), "Should have the same off time as what was set.");

        let (wiser, sender) = get_on_dummy_with_off_time(off_time);
        assert_eq!(wiser.get_heating_turn_off_time(), Some(off_time), "Getting off time should act the same even when called first");
        assert_eq!(wiser.get_heating_on(), true, "Getting whether heating is on should act the same even when called second");
    }

    fn get_on_dummy_with_off_time(off_time: Instant) -> (Dummy, Sender<ModifyState>) {
        let (wiser, sender) = Dummy::create();
        sender.send(ModifyState::SetHeatingOffTime(off_time.clone()))
            .expect("Should be able to send message");
        return (wiser, sender);
    }
}