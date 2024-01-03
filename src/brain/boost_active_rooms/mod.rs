use crate::brain::boost_active_rooms::config::BoostActiveRoomsConfig;
use crate::brain::python_like::control::devices::Device;
use crate::io::wiser::hub::{WiserHub, WiserRoomData};
use crate::io::wiser::WiserManager;
use chrono::Duration as CDuration;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use log::{debug, info, trace, warn};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::Duration;

pub mod config;

const OUR_SET_POINT_ORIGINATOR: &str = "FollowHeatingBoostActiveRooms";

/// How long to boost for each time.
const BOOST_LENGTH_MINUTES: usize = 120;
/// How many minutes before the end we will renew the boost
const BOOST_RENEW_MINUTES: i64 = 90;

/// Wiser radiator boosts that have been applied in order to open a valve and create demand.
pub struct AppliedBoosts {
    // Boosts we applied so we can keep track of what was applied by us / not
    room_temps: HashMap<String, AppliedBoost>,
    // If we detected interference, leave the room alone for the given amount of time.
    leave_alone_until: HashMap<String, DateTime<Utc>>,
    /// Whether this is the first run, and hence we should take ownership of any boosts as they
    /// were probably us anyway.
    pub first_run: bool,
}

#[derive(Debug)]
pub struct AppliedBoost {
    temp_set: f32,
    end_time: DateTime<Utc>,
}

impl AppliedBoost {
    // The max amount that the wiser boost temperature and our set temperature can difference
    // before we decide that its not our boost.
    const ACCEPTABLE_DIFFERENCE: f32 = 0.1;
    /// Check that this applied boost is the same as the one currently observed
    /// on wiser.
    /// Done by checking end times.
    pub fn matches_wiser(&self, room: &WiserRoomData) -> bool {
        trace!("Room: {:?}, applied boost {:?}", room, self);
        room.get_override_timeout()
            .is_some_and(|timeout| timeout == self.end_time)
            && room
                .get_override_set_point()
                .is_some_and(|temp| (temp - self.temp_set).abs() < Self::ACCEPTABLE_DIFFERENCE)
    }
}

impl Display for AppliedBoost {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.1} ends {}",
            self.temp_set,
            self.end_time.to_rfc3339()
        )
    }
}

impl AppliedBoosts {
    pub fn new() -> Self {
        Self {
            room_temps: HashMap::new(),
            leave_alone_until: HashMap::new(),
            first_run: true,
        }
    }

    pub fn mark_applied(&mut self, room: String, temp_set: f32, end_time: DateTime<Utc>) {
        trace!(
            "Marking applied boost of {:.1} to {} ending {:?}",
            temp_set,
            room,
            end_time
        );
        self.room_temps
            .insert(room, AppliedBoost { temp_set, end_time });
    }

    pub fn clear_applied(&mut self, room: &str) {
        self.room_temps.remove(room);
    }

    pub fn get_applied_boost(&self, room_name: &str) -> Option<&AppliedBoost> {
        return self.room_temps.get(room_name);
    }

    pub fn mark_leave_alone_for(&mut self, room_name: String, until: DateTime<Utc>) {
        self.leave_alone_until.insert(room_name, until);
    }

    pub fn can_touch(&self, room_name: &str, now: &DateTime<Utc>) -> bool {
        match self.leave_alone_until.get(room_name) {
            Some(time) => now > time,
            None => true,
        }
    }
}

pub async fn update_boosted_rooms(
    state: &mut AppliedBoosts,
    config: &BoostActiveRoomsConfig,
    active_devices: Vec<Device>,
    wiser: &dyn WiserManager,
) -> Result<(), Box<dyn Error>> {
    // TODO: Should be extracted out and use TimeProvider
    let now = Utc::now();
    debug!(
        "Active Devices: {}",
        active_devices
            .iter()
            .map(|dev| dev.get_name())
            .sorted()
            .format(", ")
    );
    let mut room_boosts: HashMap<String, (Device, f32)> = HashMap::new();

    for part in config.get_parts() {
        if active_devices.contains(part.get_device()) {
            room_boosts
                .entry(part.get_room().to_owned())
                .and_modify(|(cur_dev, cur_change)| {
                    if part.get_increase() > *cur_change {
                        *cur_dev = part.get_device().to_owned();
                        *cur_change = part.get_increase();
                    }
                })
                .or_insert((part.get_device().to_owned(), part.get_increase()));
        }
    }

    let boost_str = room_boosts
        .iter()
        .sorted_by_key(|&(room, _)| room)
        .map(|(room, (device, change))| format!("{room} ({change:+.1} {device})"))
        .join(", ");

    debug!("To boost: {}", boost_str);

    let wiser_data = wiser.get_wiser_hub().get_room_data().await?;

    let mut ignored = Vec::new();

    for room in wiser_data.iter() {
        let room_name = room.get_name();
        if room_name.is_none() {
            warn!("Failed to get room name from id: {}", room.get_id());
            continue;
        }
        let room_name = room_name.unwrap();

        if !state.can_touch(room_name, &now) {
            ignored.push(room_name);
            continue;
        }

        match room_boosts.remove(room_name) {
            None => {
                if state.get_applied_boost(room_name).is_some() {
                    info!("Cancelling boost in room {}", room_name);
                    wiser
                        .get_wiser_hub()
                        .cancel_boost(room.get_id(), OUR_SET_POINT_ORIGINATOR.to_string())
                        .await?;
                }
                state.clear_applied(room_name);
            }
            Some((device, increase_by)) => {
                let should_set_to = room.get_scheduled_set_point() + increase_by;

                // If we've applied a boost, we need to check that its OUR boost before we touch it
                if let Some(applied_boost) = state.get_applied_boost(room_name) {
                    if !applied_boost.matches_wiser(room) {
                        let ignore_duration = match room.get_override_set_point() {
                            Some(_) => config.get_interfere_change_leave_alone_time(),
                            None => config.get_interfere_off_leave_alone_time(),
                        };
                        warn!("Current boost in {} does not match what we applied ({} vs actual {:.1} until {:?}). Assuming someone else set it and ignoring it for {:?}s", room_name, applied_boost, room.get_override_set_point().unwrap_or(-10.0), room.get_override_timeout(), ignore_duration.as_secs());
                        mark_interference(room_name, ignore_duration, now, state);
                        continue;
                    }
                    let temp = match room.get_override_set_point() {
                        None => {
                            warn!("Apparently there is no boost on {} -> maybe someone turned it off, doing nothing.", room_name);
                            continue;
                        }
                        Some(temp) => temp,
                    };

                    trace!(
                        "{}: Current boosted temp {:.1}, we applied {}",
                        room_name,
                        temp,
                        applied_boost
                    );
                    if (should_set_to - temp).abs() > 0.3 {
                        info!("Significant difference between what we applied and what we should be applying now, increasing.");
                        apply_boost(
                            room,
                            should_set_to,
                            room_name,
                            &device,
                            state,
                            wiser.get_wiser_hub(),
                        )
                        .await?;
                        continue;
                    }
                    let time_left = applied_boost.end_time - now;
                    trace!(
                        "{} has {}s of boost remaining",
                        room_name,
                        time_left.num_seconds()
                    );
                    if time_left < CDuration::minutes(BOOST_RENEW_MINUTES) {
                        info!(
                            "Less than {} minutes remaining on boost for room {}. Reapplying now.",
                            BOOST_RENEW_MINUTES, room_name
                        );
                        apply_boost(
                            room,
                            should_set_to,
                            room_name,
                            &device,
                            state,
                            wiser.get_wiser_hub(),
                        )
                        .await?;
                        continue;
                    }
                    continue;
                } else if room.get_override_timeout().is_none() || state.first_run {
                    if room.get_override_timeout().is_some() {
                        info!(
                            "Taking ownership of boost in {}, since we just started.",
                            room_name
                        );
                    }
                    // No boost and we haven't applied anything - just reapply.
                    apply_boost(
                        room,
                        should_set_to,
                        room_name,
                        &device,
                        state,
                        wiser.get_wiser_hub(),
                    )
                    .await?;
                    continue;
                } else {
                    let ignore_duration = config.get_interfere_change_leave_alone_time();
                    warn!("Wanted to apply boost to already boosted room {} but it already has a boost. Leaving alone for {}s", room_name, ignore_duration.as_secs());
                    mark_interference(room_name, ignore_duration, now, state);
                }
            }
        }
    }

    if !ignored.is_empty() {
        debug!(
            "Ignoring rooms: {} since they have been interfered with recently",
            ignored.iter().sorted().join(", ")
        );
        for room in ignored {
            // Don't warn about unapplied boosts to ignored rooms.
            room_boosts.remove(room);
        }
    }

    if !room_boosts.is_empty() {
        warn!(
            "Didn't apply room boosts: {:?} - Do the rooms exist?",
            room_boosts
        )
    }

    state.first_run = false;

    Ok(())
}

fn mark_interference(
    room_name: &str,
    ignore_duration: &Duration,
    now: DateTime<Utc>,
    state: &mut AppliedBoosts,
) {
    let chrono_duration = match CDuration::from_std(*ignore_duration) {
        Ok(duration) => duration,
        Err(e) => {
            warn!("Failed to convert std duration to chrono: {}", e);
            CDuration::hours(1)
        }
    };
    state.mark_leave_alone_for(room_name.to_owned(), now + chrono_duration);
    state.clear_applied(room_name);
}

async fn apply_boost(
    room: &WiserRoomData,
    set_to: f32,
    room_name: &str,
    device: &Device,
    state: &mut AppliedBoosts,
    wiser: &dyn WiserHub,
) -> Result<(), Box<dyn Error>> {
    info!(
        "Increasing set point in room {} to {:.1} due to device {} being active",
        room_name, set_to, device
    );
    let (temp, time) = wiser
        .set_boost(
            room.get_id(),
            BOOST_LENGTH_MINUTES,
            set_to,
            OUR_SET_POINT_ORIGINATOR.to_string(),
        )
        .await?;
    state.mark_applied(room_name.to_string(), temp, time);
    Ok(())
}
