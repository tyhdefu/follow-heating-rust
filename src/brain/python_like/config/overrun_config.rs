use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use itertools::Itertools;
use log::{debug, error, info, trace};
use crate::python_like::modes::heating_mode::PossibleTemperatureContainer;
use crate::Sensor;
use crate::time_util::timeslot::ZonedSlot;

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct OverrunConfig {
    slots: Vec<OverrunBap>,
}

impl OverrunConfig {
    #[cfg(test)]
    pub fn new(slots: Vec<OverrunBap>) -> Self {
        Self {
            slots
        }
    }

    pub fn combine(&mut self, mut other: OverrunConfig) {
        self.slots.append(&mut other.slots);
    }

    pub fn get_current_slots(&self, now: &DateTime<Utc>, currently_on: bool) -> TimeSlotView {
        trace!("All slots (currently on: {}): {}", currently_on, self.slots.iter().map(|s| format!("{{ {} }}", s)).join(", "));
        let map: HashMap<Sensor, Vec<_>> = self.slots.iter()
            .filter(|slot| slot.slot.contains(now))
            .filter(|slot| {
                if slot.min_temp.is_some() && slot.min_temp.unwrap() >= slot.temp {
                    error!("Invalid slot, slot min temp must be greater than the slot target temp.");
                    return false;
                }
                return true;
            })
            .filter(|slot| currently_on || slot.min_temp.is_some())
            .map(|slot| (slot.sensor.clone(), slot))
            .into_group_map();

        TimeSlotView {
            applicable: map,
            already_on: currently_on,
        }
    }
}

pub const OVERRUN_LOG_TARGET: &str = "overrun";

#[derive(Debug)]
pub struct TimeSlotView<'a> {
    applicable: HashMap<Sensor, Vec<&'a OverrunBap>>,
    already_on: bool,
}

impl<'a> TimeSlotView<'a> {
    pub fn get_applicable(&self) -> &HashMap<Sensor, Vec<&'a OverrunBap>> {
        &self.applicable
    }

    // TODO: This logic should not be in *this* file, as well as the tests that use it.
    pub fn find_matching<T: PossibleTemperatureContainer>(&self, temps: &T) -> Option<&OverrunBap> {
        for (sensor, baps) in &self.applicable {
            if let Some(temp) = temps.get_sensor_temp(sensor) {
                for bap in baps {
                    debug!(target: OVERRUN_LOG_TARGET, "Checking overrun for {}. Current temp {:.2}. Overrun config: {}", sensor, temp, bap);
                    if !self.already_on {
                        if bap.min_temp.is_none() {
                            error!(target: OVERRUN_LOG_TARGET, "runtime assertion error: bap should have a min temp if its put in a already_on TimeSlotView!");
                            continue;
                        }
                        if *temp > bap.min_temp.unwrap() {
                            continue; // Doesn't match
                        }
                    }
                    if *temp < bap.temp {
                        info!(target: OVERRUN_LOG_TARGET, "Found matching overrun {}", bap);
                        return Some(*bap);
                    }
                }
            }
            else {
                error!(target: OVERRUN_LOG_TARGET, "Potentially missing sensor: {}", sensor);
            }
        }
        None
    }
}

/// A boost applicable at a certain time of day.
#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct OverrunBap {
    /// The time slot during which this is applicable.
    slot: ZonedSlot,
    /// The temperature to reach
    temp: f32,
    /// The sensor to reach the temperature
    sensor: Sensor,
    /// The minimum allowed temperature during this slot.
    /// If the temperature is below this, then the heating will come on automatically.
    /// If this is not set, it will act as an overrun only.
    min_temp: Option<f32>,
}

impl OverrunBap {
    #[cfg(test)]
    pub fn new(slot: ZonedSlot, temp: f32, sensor: Sensor) -> Self {
        Self {
            slot,
            temp,
            sensor,
            min_temp: None,
        }
    }

    #[cfg(test)]
    pub fn new_with_min(slot: ZonedSlot, temp: f32, sensor: Sensor, min_temp: f32) -> Self {
        assert!(min_temp < temp, "min_temp should be less than temp");
        Self {
            slot,
            temp,
            sensor,
            min_temp: Some(min_temp),
        }
    }

    pub fn get_slot(&self) -> &ZonedSlot {
        &self.slot
    }

    pub fn get_temp(&self) -> f32 {
        self.temp
    }

    pub fn get_sensor(&self) -> &Sensor {
        &self.sensor
    }

    pub fn get_min_temp(&self) -> &Option<f32> {
        &self.min_temp
    }
}

impl Display for OverrunBap {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Overrun {}: up to {} (min {:?}, During {})", self.sensor, self.temp, self.min_temp, self.slot)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDateTime, TimeZone};
    use crate::time_util::test_utils::{date, time};
    use super::*;

    #[test]
    fn test_deserialize() {
        let config_str = std::fs::read_to_string("test/python_brain/overrun_config/basic.toml").expect("Failed to read config file.");
        let overrun_config: OverrunConfig = toml::from_str(&config_str).expect("Failed to deserialize config");

        let utc_slot = (time(03, 02, 05)..time(07, 03, 09)).into();
        let local_slot = (time(12, 45, 31)..time(14, 55, 01)).into();
        let local_slot2 = (time(09, 37, 31)..time(11, 15, 26)).into();

        let expected = vec![
            OverrunBap::new(ZonedSlot::Utc(utc_slot), 32.8, Sensor::TKTP),
            OverrunBap::new(ZonedSlot::Local(local_slot), 27.3, Sensor::TKBT),
            OverrunBap::new_with_min(ZonedSlot::Local(local_slot2), 45.0, Sensor::TKTP, 30.0),
        ];
        assert_eq!(overrun_config.slots, expected);
    }

    fn mk_map(bap: &OverrunBap) -> HashMap<Sensor, Vec<&OverrunBap>> {
        let mut map = HashMap::new();
        map.insert(bap.get_sensor().clone(), vec![bap]);
        map
    }

    #[test]
    fn test_get_slot() {
        let utc_slot1 = (time(03, 02, 05)..time(07, 03, 09)).into();
        let utc_slot2 = (time(12, 45, 31)..time(14, 55, 01)).into();
        let utc_slot3 = (time(09, 37, 31)..time(11, 15, 26)).into();
        let utc_slot4 = (time(02, 00, 00)..time(04, 30, 00)).into();

        let slot1 = OverrunBap::new(ZonedSlot::Utc(utc_slot1), 32.8, Sensor::TKTP);
        let slot2 = OverrunBap::new(ZonedSlot::Utc(utc_slot2), 27.3, Sensor::TKTP);
        let slot3 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot3), 45.0, Sensor::TKTP, 30.0);
        let slot4 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot4), 29.5, Sensor::TKTP, 25.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone(), slot3.clone(), slot4.clone()]);

        let irrelevant_day = date(2022, 04, 18);
        let time1 = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, time(06, 23, 00)));

        assert_eq!(config.get_current_slots(&time1, true).get_applicable(), &mk_map(&slot1), "Simple");
        assert_eq!(config.get_current_slots(&time1, false).get_applicable(), &HashMap::new(), "Not on so shouldn't do any overrun");

        let slot_1_and_4_time = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, time(03, 32, 00)));
        //assert_eq!(config.get_current_slots(slot_1_and_4_time, true).get_applicable(), &mk_map(&slot1), "Slot 1 because its hotter than Slot 4"); // No longer applicable because it returns both, not the best one.
        assert_eq!(config.get_current_slots(&slot_1_and_4_time, false).get_applicable(), &mk_map(&slot4), "Slot 4 because slot 1 only overruns, it won't switch on");
    }

    #[test]
    fn test_overlapping_min_temp() {
        let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(date(2022, 08, 19),
                                                                        time(04, 15, 00)));

        let utc_slot1 = (time(04, 00, 00)..time(04, 30, 00)).into();
        let utc_slot2 = (time(03, 00, 00)..time(04, 30, 00)).into();
        let slot1 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot1), 43.6, Sensor::TKTP, 40.5);
        let slot2 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot2), 41.6, Sensor::TKTP, 36.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);


        let view = config.get_current_slots(&datetime, false);
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 38.0); // A temp below the higher min temp.
        assert_eq!(view.find_matching(&temps), Some(&slot1));
    }

    #[test]
    fn test_disjoint_annoying() {
        let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(date(2022, 08, 19),
                                                                        time(04, 15, 00)));

        let utc_slot1 = (time(04, 00, 00)..time(04, 30, 00)).into();
        let utc_slot2 = (time(03, 00, 00)..time(04, 30, 00)).into();

        let slot1 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot1), 35.0, Sensor::TKBT, 33.0);
        let slot2 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot2), 43.0, Sensor::TKBT, 37.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);

        let current_slot_map = config.get_current_slots(&datetime, false);
        println!("Current slot view {:?}", current_slot_map);

        let current_tkbt_temp = 36.0; // Example of tkbt temp that should cause it to turn on due to slot2.

        let mut temps = HashMap::new();
        temps.insert(Sensor::TKBT, current_tkbt_temp);
        let bap = current_slot_map.find_matching(&temps)
            .expect("Should have a bap.");

        assert_eq!(bap, &slot2);
    }
}