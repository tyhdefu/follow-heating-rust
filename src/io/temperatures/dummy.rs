use std::borrow::{Borrow, BorrowMut};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use crate::io;
use crate::io::dummy::DummyIO;
use crate::io::temperatures::{Sensor, TemperatureManager};
use async_trait::async_trait;

pub enum ModifyState {
    SetTemp(Sensor, f32),
    SetTemps(HashMap<Sensor, f32>)
}

pub struct Dummy {
    receiver: Mutex<Receiver<ModifyState>>,
    temps: Mutex<RefCell<HashMap<Sensor, f32>>>,
}

#[async_trait]
impl TemperatureManager for Dummy {

    async fn retrieve_sensors(&mut self) -> Result<(), String> {
        Ok(())
    }

    async fn retrieve_temperatures(&self) -> Result<HashMap<Sensor, f32>, String> {
        self.update_state();
        let guard = self.temps.lock()
            .unwrap();
        let map = (*guard).borrow().clone();
        Ok(map)
    }
}

impl DummyIO for Dummy {
    type MessageType = ModifyState;

    fn new(receiver: Receiver<Self::MessageType>) -> Self {
        Dummy {
            receiver: Mutex::new(receiver),
            temps: Mutex::new(RefCell::new(HashMap::new())),
        }
    }
}

impl Dummy {
    fn update_state(&self) {
        let guard = self.receiver.lock().unwrap();
        io::dummy::read_all(&*guard, |message| {
            match message {
                ModifyState::SetTemp(sensor, temp) => { (*self.temps.lock().unwrap()).borrow_mut().insert(sensor, temp); },
                ModifyState::SetTemps(temps) => {self.temps.lock().unwrap().replace(temps);},
            };
        })
    }
}