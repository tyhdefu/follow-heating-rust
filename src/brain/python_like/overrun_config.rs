use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::{Deref, Range, RangeInclusive};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use itertools::Itertools;
use crate::python_like::heating_mode::{PossibleTemperatureContainer, TargetTemperature};
use crate::Sensor;
use crate::time::timeslot::ZonedSlot::Local;
use crate::time::timeslot::{TimeSlot, ZonedSlot};

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct OverrunConfig {
    slots: Vec<OverrunBap>,
}

impl OverrunConfig {
    pub fn new(slots: Vec<OverrunBap>) -> Self {
        Self {
            slots
        }
    }

    pub fn get_current_slots(&self, now: DateTime<Utc>, currently_on: bool) -> TimeSlotView {
        let map: HashMap<Sensor, Vec<_>> = self.slots.iter()
            .filter(|slot| slot.slot.contains(&now))
            .filter(|slot| {
                if slot.min_temp.is_some() && slot.min_temp.unwrap() >= slot.temp {
                    eprintln!("Invalid slot, slot min temp must be greater than the slot target temp.");
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
                    println!("Checking overrun for {}. Current temp {}. Overrun config: {:?}", sensor, temp, bap);
                    if !self.already_on {
                        if bap.min_temp.is_none() {
                            eprintln!("runtime assertion error: bap should have a min temp if its put in a already_on TiemSlotView!");
                            continue;
                        }
                        if *temp > bap.min_temp.unwrap() {
                            continue; // Doesn't match
                        }
                    }
                    if *temp < bap.temp {
                        println!("Found matching overrun {:?}", bap);
                        return Some(*bap);
                    }
                }
            }
            else {
                eprintln!("Potentially missing sensor: {}", sensor);
            }
        }
        None
    }
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
pub struct OverrunBap {
    slot: ZonedSlot,
    temp: f32,
    sensor: Sensor,
    min_temp: Option<f32>,
}

impl OverrunBap {
    pub fn new(slot: ZonedSlot, temp: f32, sensor: Sensor) -> Self {
        Self {
            slot,
            temp,
            sensor,
            min_temp: None,
        }
    }

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

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use chrono::{DurationRound, Local, NaiveDateTime, TimeZone};
    use super::*;

    #[test]
    fn test_deserialize() {
        let config_str = std::fs::read_to_string("test/overrun_config/basic.toml").expect("Failed to read config file.");
        let overrun_config: OverrunConfig = toml::from_str(&config_str).expect("Failed to deserialize config");

        let utc_slot = (NaiveTime::from_hms(03, 02, 05)..NaiveTime::from_hms(07, 03, 09)).into();
        let local_slot = (NaiveTime::from_hms(12, 45, 31)..NaiveTime::from_hms(14, 55, 01)).into();
        let local_slot2 = (NaiveTime::from_hms(09, 37, 31)..NaiveTime::from_hms(11, 15, 26)).into();

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
        let utc_slot1 = (NaiveTime::from_hms(03, 02, 05)..NaiveTime::from_hms(07, 03, 09)).into();
        let utc_slot2 = (NaiveTime::from_hms(12, 45, 31)..NaiveTime::from_hms(14, 55, 01)).into();
        let utc_slot3 = (NaiveTime::from_hms(09, 37, 31)..NaiveTime::from_hms(11, 15, 26)).into();
        let utc_slot4 = (NaiveTime::from_hms(02, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into();

        let slot1 = OverrunBap::new(ZonedSlot::Utc(utc_slot1), 32.8, Sensor::TKTP);
        let slot2 = OverrunBap::new(ZonedSlot::Utc(utc_slot2), 27.3, Sensor::TKTP);
        let slot3 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot3), 45.0, Sensor::TKTP, 30.0);
        let slot4 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot4), 29.5, Sensor::TKTP, 25.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone(), slot3.clone(), slot4.clone()]);

        let irrelevant_day = NaiveDate::from_ymd(2022, 04, 18);
        let time1 = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, NaiveTime::from_hms(06, 23, 00)));

        assert_eq!(config.get_current_slots(time1, true).get_applicable(), &mk_map(&slot1), "Simple");
        assert_eq!(config.get_current_slots(time1, false).get_applicable(), &HashMap::new(), "Not on so shouldn't do any overrun");

        let slot_1_and_4_time = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, NaiveTime::from_hms(03, 32, 00)));
        //assert_eq!(config.get_current_slots(slot_1_and_4_time, true).get_applicable(), &mk_map(&slot1), "Slot 1 because its hotter than Slot 4"); // No longer applicable because it returns both, not the best one.
        assert_eq!(config.get_current_slots(slot_1_and_4_time, false).get_applicable(), &mk_map(&slot4), "Slot 4 because slot 1 only overruns, it won't switch on");
    }

    #[test]
    fn test_overlapping_min_temp() {
        let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(NaiveDate::from_ymd(2022, 08, 19),
                                                                        NaiveTime::from_hms(04, 15, 00)));

        let utc_slot1 = (NaiveTime::from_hms(04, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into();
        let utc_slot2 = (NaiveTime::from_hms(03, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into();
        let slot1 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot1), 43.6, Sensor::TKTP, 40.5);
        let slot2 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot2), 41.6, Sensor::TKTP, 36.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);


        let view = config.get_current_slots(datetime, false);
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 38.0); // A temp below the higher min temp.
        assert_eq!(view.find_matching(&temps), Some(&slot1));
    }

    #[test]
    fn test_disjoint_annoying() {
        let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(NaiveDate::from_ymd(2022, 08, 19),
                                                                        NaiveTime::from_hms(04, 15, 00)));

        let utc_slot1 = (NaiveTime::from_hms(04, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into();
        let utc_slot2 = (NaiveTime::from_hms(03, 00, 00)..NaiveTime::from_hms(04, 30, 00)).into();

        let slot1 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot1), 35.0, Sensor::TKBT, 33.0);
        let slot2 = OverrunBap::new_with_min(ZonedSlot::Utc(utc_slot2), 43.0, Sensor::TKBT, 37.0);

        let config = OverrunConfig::new(vec![slot1.clone(), slot2.clone()]);

        let current_slot_map = config.get_current_slots(datetime, false);
        println!("Current slot view {:?}", current_slot_map);

        let current_tkbt_temp = 36.0; // Example of tkbt temp that should cause it to turn on due to slot2.

        let mut temps = HashMap::new();
        temps.insert(Sensor::TKBT, current_tkbt_temp);
        let bap = current_slot_map.find_matching(&temps)
            .expect("Should have a bap.");

        assert_eq!(bap, &slot2);
    }
}