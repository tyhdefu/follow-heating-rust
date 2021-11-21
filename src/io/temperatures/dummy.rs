use std::cell::RefCell;
use std::collections::HashMap;
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::io::dummy::DummyIO;
    use crate::io::temperatures::dummy::{Dummy, ModifyState};
    use crate::io::temperatures::{Sensor, TemperatureManager};

    #[tokio::test]
    async fn starts_blank() {
        let (dummy, _sender) = Dummy::create();
        let temps = dummy.retrieve_temperatures().await
            .expect("Should retrieve temperatures");
        assert!(temps.is_empty(), "Expected no temperatures");
    }

    #[tokio::test]
    async fn set_single_temp() {
        let set_value = 37.2;
        let sensor = Sensor::TKRT;
        let (dummy, sender) = Dummy::create();
        sender.send(ModifyState::SetTemp(sensor.clone(), set_value))
            .expect("Should be able to send message");
        let temps = get_temps(&dummy).await;
        let mut expected = HashMap::new();
        expected.insert(sensor, set_value);
        assert_eq!(temps, expected, "Mismatch between set and received values.");
    }

    #[tokio::test]
    async fn set_multiple_temps() {
        let mut map = HashMap::new();
        map.insert(Sensor::TKEN, 39.3);
        map.insert(Sensor::HPFL, 23.3);
        map.insert(Sensor::TKBT, 18.1);
        map.insert(Sensor::HXIR, 14.5);
        let (dummy, sender) = Dummy::create();
        sender.send(ModifyState::SetTemps(map.clone()))
            .expect("Should be able to send message");
        let temps = get_temps(&dummy).await;
        assert_eq!(temps, map, "Expected ")
    }

    async fn get_temps(dummy: &Dummy) -> HashMap<Sensor, f32> {
        dummy.retrieve_temperatures().await
            .expect("Should retrieve temperatures")
    }
}