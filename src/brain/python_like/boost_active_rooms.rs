use std::collections::HashMap;
use std::error::Error;
use chrono::{DateTime, Utc};
use crate::brain::python_like::config::boost_active::BoostActiveRoomsConfig;
use crate::brain::python_like::control::devices::Device;
use crate::io::wiser::hub::{FROM_SCHEDULE_ORIGIN, WiserRoomData};
use crate::io::wiser::WiserManager;

const OUR_SET_POINT_ORIGINATOR: &str = "FollowHeatingBoostActiveRooms";

pub struct AppliedBoosts {
    room_temps: HashMap<String, (f32, DateTime<Utc>)>,
}

impl AppliedBoosts {
    pub fn new() -> Self {
        Self {
            room_temps: HashMap::new(),
        }
    }

    pub fn mark_applied(&mut self, room: String, temp: f32, datetime: DateTime<Utc>) {
        self.room_temps.insert(room, (temp, datetime));
    }

    pub fn clear_applied(&mut self, room: &str) {
        self.room_temps.remove(room);
    }

    pub fn have_we_applied(&self, room: &WiserRoomData) -> bool {
        if let Some(room_name) = room.get_name() {
            if let Some((we_set, time_set)) = self.room_temps.get(room_name) {
                if room.get_override_set_point().is_some()
                    && (we_set - room.get_override_set_point().unwrap()).abs() > 0.3 {
                    return false;
                }
                return room.get_override_timeout().is_some() && room.get_override_timeout().as_ref().unwrap() == time_set;
            }
        }
        return false;
    }
}

pub async fn update_boosted_rooms(state: &mut AppliedBoosts, config: &BoostActiveRoomsConfig, active_devices: Vec<Device>, wiser: &dyn WiserManager) -> Result<(), Box<dyn Error>>{

    let mut room_boosts: HashMap<String, (Device, f32)> = HashMap::new();

    for part in config.get_parts() {
        if active_devices.contains(part.get_device()) {
            room_boosts.entry(part.get_room().to_owned())
                .and_modify(|(cur_dev, cur_change)| {
                    if part.get_increase() > *cur_change {
                        *cur_dev = part.get_device().to_owned();
                        *cur_change = part.get_increase();
                    }
                })
                .or_insert((part.get_device().to_owned(), part.get_increase()));
        }
    }

    for (room, (device, change)) in &room_boosts {
        println!("Room: {} should be boosted by {} due to device {}", room, change, device);
    }

    let wiser_data = wiser.get_wiser_hub().get_data().await?;

    for room in wiser_data.get_rooms() {
        let room_name = room.get_name();
        if room_name.is_none() {
            continue;
        }
        let room_name = room_name.unwrap();

        match room_boosts.remove(room_name) {
            None => {
                if state.have_we_applied(room) {
                    println!("Cancelling boost in room {}", room_name);
                    wiser.get_wiser_hub().cancel_boost(room.get_id(), OUR_SET_POINT_ORIGINATOR.to_string()).await?;
                }
                state.clear_applied(room_name);
            }
            Some((device, increase_by)) => {
                let should_set_to = room.get_scheduled_set_point() + increase_by;
                if room.get_override_timeout().is_none()
                    || (state.have_we_applied(room) && room.get_override_set_point().filter(|temp| temp - 0.3 > should_set_to).is_some()) {
                    println!("Increasing set point in room {} due to device {} being active", room_name, device);
                    let time = wiser.get_wiser_hub().set_boost(room.get_id(), 30, should_set_to, OUR_SET_POINT_ORIGINATOR.to_string()).await?;
                    state.mark_applied(room_name.to_string(), should_set_to, time);
                }
            }
        }
    }

    if !room_boosts.is_empty() {
        println!("Didn't apply room boosts: {:?} - Do the rooms exist?", room_boosts)
    }

    Ok(())
}