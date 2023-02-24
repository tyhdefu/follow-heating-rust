use std::collections::HashMap;
use std::error::Error;
use chrono::{DateTime, Utc};
use crate::brain::BrainFailure;
use crate::brain::python_like::config::boost_active::BoostActiveRoomsConfig;
use crate::brain::python_like::control::devices::Device;
use crate::io::wiser::hub::{FROM_SCHEDULE_ORIGIN, WiserHub, WiserRoomData};
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

    /// Check if we have applied a boost to the room and whether
    /// the end time matches what we set.
    pub fn have_we_applied_any_boost_to(&self, room: &WiserRoomData) -> bool {
        if let Some(room_name) = room.get_name() {
            if let Some((_we_set, time_set)) = self.room_temps.get(room_name) {
                return room.get_override_timeout().is_some() && room.get_override_timeout().as_ref().unwrap() == time_set;
            }
        }
        return false;
    }

    pub fn get_applied_boost_temp(&self, room: &WiserRoomData) -> Option<f32> {
        if let Some(room_name) = room.get_name() {
            if let Some((we_set, time_set)) = self.room_temps.get(room_name) {
                return Some(*we_set);
            }
        }
        return None;
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
                if state.have_we_applied_any_boost_to(room) {
                    println!("Cancelling boost in room {}", room_name);
                    wiser.get_wiser_hub().cancel_boost(room.get_id(), OUR_SET_POINT_ORIGINATOR.to_string()).await?;
                }
                state.clear_applied(room_name);
            }
            Some((device, increase_by)) => {
                let should_set_to = room.get_scheduled_set_point() + increase_by;

                // If no boost - we can easily apply it.
                if room.get_override_timeout().is_none() {
                    apply_boost(room, should_set_to, room_name, &device, state, wiser.get_wiser_hub()).await?;
                    continue;
                }

                // If we've applied a boost, we need to check that its OUR boost if we increase it.
                if state.have_we_applied_any_boost_to(room) {
                    println!("We have already applied a matching boost to {}", room_name);
                    let temp = match room.get_override_set_point() {
                        None => {
                            println!("But apparently there is no boost -> maybe someone turned it off, doing nothing.");
                            continue;
                        }
                        Some(temp) => temp,
                    };
                    let we_applied_temp = state.get_applied_boost_temp(room);
                    match we_applied_temp {
                        None => eprintln!("Apparently we didn't apply any temp as it turns out...?"),
                        Some(we_applied_temp) => {
                            println!("Current boosted temp {:.1}, we applied {}", temp, we_applied_temp);
                            if (should_set_to - temp).abs() > 0.3 {
                                println!("Significant difference between what we applied and what we should be applying now, increasing.");
                                apply_boost(room, should_set_to, room_name, &device, state, wiser.get_wiser_hub()).await?
                            }
                        }
                    }
                    continue;
                }
                println!("Looks the currently applied boost in room {} was not by us - not touching it.", room_name);
            }
        }
    }

    if !room_boosts.is_empty() {
        println!("Didn't apply room boosts: {:?} - Do the rooms exist?", room_boosts)
    }

    Ok(())
}

const BOOST_LENGTH_MINUTES: usize = 30;

async fn apply_boost(room: &WiserRoomData, set_to: f32,
                     room_name: &str,
                     device: &Device,
                     state: &mut AppliedBoosts,
                     wiser: &dyn WiserHub) -> Result<(), Box<dyn Error>> {
    println!("Increasing set point in room {} to {:.1} due to device {} being active", room_name, set_to, device);
    let time = wiser.set_boost(room.get_id(), BOOST_LENGTH_MINUTES, set_to, OUR_SET_POINT_ORIGINATOR.to_string()).await?;
    state.mark_applied(room_name.to_string(), set_to, time);
    Ok(())
}