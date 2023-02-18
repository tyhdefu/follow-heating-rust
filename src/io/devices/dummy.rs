use std::sync::{mpsc::Receiver, Mutex};

use chrono::{DateTime, Utc};

use crate::{io::{dummy::DummyIO, self}, brain::{python_like::control::devices::{ActiveDevices, Device}, BrainFailure}};

pub struct DummyActiveDevices {
    active_devices: Mutex<Vec<Device>>,
    rx: Mutex<Receiver<ActiveDevicesMessage>>,
}

impl DummyIO for DummyActiveDevices {
    type MessageType = ActiveDevicesMessage;
    type Config = ();

    fn new(receiver: Receiver<Self::MessageType>, _config: &Self::Config) -> Self {
        Self {
            active_devices: Mutex::new(vec![]),
            rx: Mutex::new(receiver),
        }
    }
}

pub enum ActiveDevicesMessage {
    SetActiveDevices(Vec<Device>),
}

impl ActiveDevices for DummyActiveDevices {
    fn get_active_devices(&mut self, _time: &DateTime<Utc>) -> Result<Vec<Device>, BrainFailure> {
        self.update_state();
        let guard = self.active_devices.lock().unwrap();
        Ok((&*guard).clone())
    }
}

impl DummyActiveDevices {
    pub fn update_state(&mut self) {
        let guard = self.rx.lock().unwrap();
        io::dummy::read_all(&*guard, |msg| {
            match msg {
                ActiveDevicesMessage::SetActiveDevices(devices) => *self.active_devices.lock().unwrap() = devices,
            }
        });
    }
}