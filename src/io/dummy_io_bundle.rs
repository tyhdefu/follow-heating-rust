use std::{net::Ipv4Addr, sync::mpsc::Sender};

use crate::config::WiserConfig;

use super::{IOBundle, dummy::{DummyAllOutputs, DummyIO}, wiser, temperatures, devices::dummy::{DummyActiveDevices, ActiveDevicesMessage}};

pub struct DummyIOBundleHandle {
    wiser_handle: Sender<wiser::dummy::ModifyState>,
    temp_handle: Sender<temperatures::dummy::ModifyState>,
    active_devices_handle: Sender<ActiveDevicesMessage>,
}

impl DummyIOBundleHandle {
    pub fn send_wiser(&mut self, msg: wiser::dummy::ModifyState) {
        self.wiser_handle.send(msg).unwrap();
    }

    pub fn send_temps(&mut self, msg: temperatures::dummy::ModifyState) {
        self.temp_handle.send(msg).unwrap();
    }

    pub fn send_devices(&mut self, msg: ActiveDevicesMessage) {
        self.active_devices_handle.send(msg).unwrap();
    }
}

pub fn new_dummy_io() -> (IOBundle, DummyIOBundleHandle) {
    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::UNSPECIFIED.into(), String::new()));
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