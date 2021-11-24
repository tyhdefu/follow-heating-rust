use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver};
use crate::io;
use crate::io::dummy::DummyIO;
use crate::io::wiser::WiserManager;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::io::wiser::hub::WiserHub;

pub enum ModifyState {
    SetHeatingOffTime(DateTime<Utc>),
    TurnOffHeating,
}

pub struct Dummy {
    receiver: Mutex<Receiver<ModifyState>>,
    heating_off_time: Mutex<RefCell<Option<DateTime<Utc>>>>,
}

#[async_trait]
impl WiserManager for Dummy {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>> {
        self.update_state();
        (*self.heating_off_time.lock().unwrap()).borrow().clone()
    }

    async fn get_heating_on(&self) -> Result<bool,()> {
        self.update_state();
        Ok((*self.heating_off_time.lock().unwrap()).borrow().is_some())
    }

    fn get_wiser_hub(&self) -> &WiserHub {
        todo!()
    }
}

impl DummyIO for Dummy {
    type MessageType = ModifyState;

    fn new(receiver: Receiver<Self::MessageType>) -> Self {
        Dummy {
            receiver: Mutex::new(receiver),
            heating_off_time: Mutex::new(RefCell::new(None)),
        }
    }
}

impl Dummy {
    fn update_state(&self) {
        let guard = self.receiver.lock().unwrap();
        io::dummy::read_all(&*guard, |message| {
            match message {
                ModifyState::SetHeatingOffTime(when) => self.heating_off_time.lock().unwrap().borrow_mut().replace(Some(when)),
                ModifyState::TurnOffHeating => self.heating_off_time.lock().unwrap().borrow_mut().replace(None),
            };
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Add;
    use std::sync::mpsc::Sender;
    use std::time::Duration;
    use super::*;

    #[tokio::test]
    async fn dummy_starts_off() {
        let (wiser, _sender) = Dummy::create();
        assert_eq!(wiser.get_heating_on().await, Ok(false), "Dummy should start off");
        assert_eq!(wiser.get_heating_turn_off_time().await, None, "Dummy should start with empty off time since it starts off")
    }

    #[tokio::test]
    async fn dummy_turn_on() {
        let off_time = Utc::now() + chrono::Duration::seconds(1234);
        let (wiser, _sender) = get_on_dummy_with_off_time(off_time);
        assert_eq!(wiser.get_heating_on().await, Ok(true), "Should now be on");
        assert_eq!(wiser.get_heating_turn_off_time().await, Some(off_time), "Should have the same off time as what was set.");

        let (wiser, _sender) = get_on_dummy_with_off_time(off_time);
        assert_eq!(wiser.get_heating_turn_off_time().await, Some(off_time), "Getting off time should act the same even when called first");
        assert_eq!(wiser.get_heating_on().await, Ok(true), "Getting whether heating is on should act the same even when called second");
    }

    fn get_on_dummy_with_off_time(off_time: DateTime<Utc>) -> (Dummy, Sender<ModifyState>) {
        let (wiser, sender) = Dummy::create();
        sender.send(ModifyState::SetHeatingOffTime(off_time.clone()))
            .expect("Should be able to send message");
        return (wiser, sender);
    }
}