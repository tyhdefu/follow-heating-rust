use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use chrono::{DateTime, Duration, Utc};
use log::{debug, info, warn};
use crate::brain::boost_active_rooms::config::BoostActiveRoomsConfig;
use crate::brain::python_like::control::devices::Device;
use crate::io::wiser::hub::{WiserHub, WiserRoomData};
use crate::io::wiser::WiserManager;
use itertools::Itertools;

pub mod config;

const OUR_SET_POINT_ORIGINATOR: &str = "FollowHeatingBoostActiveRooms";

pub struct AppliedBoosts {
    room_temps: HashMap<String, AppliedBoost>,
}

#[derive(Debug)]
pub struct AppliedBoost {
    temp_set: f32,
    end_time: DateTime<Utc>,
}

impl AppliedBoost {
    /// Check that this applied boost is the same as the one currently observed
    /// on wiser.
    /// Done by checking end times.
    pub fn matches_wiser(&self, room: &WiserRoomData) -> bool {
        if let Some(timeout) = room.get_override_timeout() {
            return timeout == self.end_time;
        }
        false
    }
}

impl Display for AppliedBoost {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1} ends {}", self.temp_set, self.end_time.to_rfc3339())
    }
}

impl AppliedBoosts {
    pub fn new() -> Self {
        Self {
            room_temps: HashMap::new(),
        }
    }

    pub fn mark_applied(&mut self, room: String, temp_set: f32, end_time: DateTime<Utc>) {
        self.room_temps.insert(room, AppliedBoost { temp_set, end_time});
    }

    pub fn clear_applied(&mut self, room: &str) {
        self.room_temps.remove(room);
    }

    pub fn get_applied_boost(&self, room_name: &str) -> Option<&AppliedBoost> {
        return self.room_temps.get(room_name);
    }
}

pub async fn update_boosted_rooms(state: &mut AppliedBoosts, config: &BoostActiveRoomsConfig, active_devices: Vec<Device>, wiser: &dyn WiserManager) -> Result<(), Box<dyn Error>>{
    debug!("Active Devices: {}", active_devices.iter().map(|dev| dev.get_name()).sorted().format(", "));
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
        debug!("Room: {} should be boosted by {} due to device {}", room, change, device);
    }

    let wiser_data = wiser.get_wiser_hub().get_data().await?;

    for room in wiser_data.get_rooms() {
        let room_name = room.get_name();
        if room_name.is_none() {
            warn!("Failed to get room name from id: {}", room.get_id());
            continue;
        }
        let room_name = room_name.unwrap();

        match room_boosts.remove(room_name) {
            None => {
                if state.get_applied_boost(room_name).is_some() {
                    info!("Cancelling boost in room {}", room_name);
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
                if let Some(applied_boost) = state.get_applied_boost(room_name) {
                    if applied_boost.matches_wiser(room) {
                        warn!("Current boost in {} does not match what we applied ({}). Assuming someone else set it and ignoring.", room_name, applied_boost);
                        continue;
                    }
                    debug!("We have already applied a matching boost to {}", room_name);
                    let temp = match room.get_override_set_point() {
                        None => {
                            warn!("But apparently there is no boost -> maybe someone turned it off, doing nothing.");
                            continue;
                        }
                        Some(temp) => temp,
                    };

                    debug!("Current boosted temp {:.1}, we applied {}", temp, applied_boost);
                    if (should_set_to - temp).abs() > 0.3 {
                        info!("Significant difference between what we applied and what we should be applying now, increasing.");
                        apply_boost(room, should_set_to, room_name, &device, state, wiser.get_wiser_hub()).await?;
                        continue;
                    }
                    if applied_boost.end_time < Utc::now() - Duration::seconds(2*60) {
                        info!("Less than two minutes remaining on boost for room {}. Reapplying now.", room_name);
                        apply_boost(room, should_set_to, room_name, &device, state, wiser.get_wiser_hub()).await?;
                        continue;
                    }
                    continue;
                }
                debug!("No record of applying boost to {} - not touching it.", room_name);
            }
        }
    }

    if !room_boosts.is_empty() {
        warn!("Didn't apply room boosts: {:?} - Do the rooms exist?", room_boosts)
    }

    Ok(())
}

const BOOST_LENGTH_MINUTES: usize = 30;

async fn apply_boost(room: &WiserRoomData, set_to: f32,
                     room_name: &str,
                     device: &Device,
                     state: &mut AppliedBoosts,
                     wiser: &dyn WiserHub) -> Result<(), Box<dyn Error>> {
    info!("Increasing set point in room {} to {:.1} due to device {} being active", room_name, set_to, device);
    let time = wiser.set_boost(room.get_id(), BOOST_LENGTH_MINUTES, set_to, OUR_SET_POINT_ORIGINATOR.to_string()).await?;
    state.mark_applied(room_name.to_string(), set_to, time);
    Ok(())
}
