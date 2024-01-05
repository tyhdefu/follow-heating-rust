use chrono::{Duration, Utc};
use std::sync::mpsc::Sender;

use crate::config::WiserConfig;

use super::{
    devices::dummy::{ActiveDevicesMessage, DummyActiveDevices},
    dummy::{DummyAllOutputs, DummyIO},
    temperatures::{self, Sensor},
    wiser, IOBundle,
};

pub struct DummyIOBundleHandle {
    wiser_handle: Sender<wiser::dummy::ModifyState>,
    temp_handle: Sender<temperatures::dummy::ModifyState>,
    active_devices_handle: Sender<ActiveDevicesMessage>,
}

impl DummyIOBundleHandle {
    pub fn send_wiser(&mut self, msg: wiser::dummy::ModifyState) {
        self.wiser_handle.send(msg).unwrap();
    }

    pub fn send_wiser_on(&mut self, on: bool) {
        let message = match on {
            true => wiser::dummy::ModifyState::SetHeatingOffTime(Utc::now() + Duration::days(5)),
            false => wiser::dummy::ModifyState::TurnOffHeating,
        };
        self.wiser_handle.send(message).unwrap()
    }

    pub fn send_temps(&mut self, msg: temperatures::dummy::ModifyState) {
        self.temp_handle.send(msg).unwrap();
    }

    pub fn send_temp(&mut self, sensor: Sensor, temp: f32) {
        self.send_temps(temperatures::dummy::ModifyState::SetTemp(sensor, temp))
    }

    pub fn send_devices(&mut self, msg: ActiveDevicesMessage) {
        self.active_devices_handle.send(msg).unwrap();
    }
}

pub fn new_dummy_io() -> (IOBundle, DummyIOBundleHandle) {
    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::fake());
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create(&());
    let (active_devices, active_devices_handle) = DummyActiveDevices::create(&());

    let io_bundle = IOBundle::new(
        temp_manager,
        heating_control,
        misc_control,
        wiser,
        active_devices,
    );

    let handle = DummyIOBundleHandle {
        wiser_handle,
        temp_handle,
        active_devices_handle,
    };

    (io_bundle, handle)
}

