use std::collections::HashMap;
use std::error::Error;
use crate::brain::python_like::config::boost_active::BoostActiveRoomsConfig;
use crate::brain::python_like::control::devices::Device;
use crate::io::wiser::hub::FROM_SCHEDULE_ORIGIN;
use crate::io::wiser::WiserManager;

const OUR_SET_POINT_ORIGIN: &str = "FollowHeatingBoostActiveRooms";

pub async fn update_boosted_rooms(config: &BoostActiveRoomsConfig, active_devices: Vec<Device>, wiser: &dyn WiserManager) -> Result<(), Box<dyn Error>>{

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
                if room.get_setpoint_origin() == OUR_SET_POINT_ORIGIN {
                    // Cancel boost.
                    println!("Cancelling boost in room {}", room_name);
                    wiser.get_wiser_hub().cancel_boost(room.get_id(), OUR_SET_POINT_ORIGIN.to_string()).await?;
                }
            }
            Some((device, increase_by)) => {
                let origin = room.get_setpoint_origin();
                if origin != FROM_SCHEDULE_ORIGIN && origin != OUR_SET_POINT_ORIGIN {
                    println!("Not touching room {}, set point origin is {}", room_name, origin);
                    continue;
                }
                let should_set_to = room.get_scheduled_set_point() + increase_by;
                if (should_set_to - 0.3) > room.get_set_point() {
                    println!("Increasing set point in room {} due to device {} being active", room_name, device);
                    wiser.get_wiser_hub().set_boost(room.get_id(), 30, should_set_to, OUR_SET_POINT_ORIGIN.to_string()).await?;
                }
            }
        }
    }

    if !room_boosts.is_empty() {
        println!("Didn't apply room boosts: {:?} - Do the rooms exist?", room_boosts)
    }

    Ok(())
}