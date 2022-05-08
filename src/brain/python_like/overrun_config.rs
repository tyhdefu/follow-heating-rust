use std::cmp::Ordering;
use std::ops::{Range, RangeInclusive};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use crate::python_like::heating_mode::TargetTemperature;
use crate::Sensor;
use crate::time::timeslot::ZonedSlot::Local;
use crate::time::timeslot::{TimeSlot, ZonedSlot};

#[derive(Deserialize, Clone)]
pub struct OverrunConfig {
    slots: Vec<OverrunBap>
}

impl OverrunConfig {
    pub fn new(slots: Vec<OverrunBap>) -> Self {
        Self {
            slots
        }
    }

    pub fn get_current_slot(&self, now: DateTime<Utc>, cur_temp: f32, currently_on: bool) -> Option<OverrunBap> {
        self.slots.iter()
            .filter(|slot| slot.slot.contains(&now))
            .filter(|slot| cur_temp < slot.temp)
            .filter(|slot| {
                if slot.min_temp.is_some() && slot.min_temp.unwrap() >= slot.temp {
                    eprintln!("Invalid slot, slot min temp must be greater than the slot target temp.");
                    return false;
                }
                return true;
            })
            .filter(|slot| currently_on || (slot.min_temp.is_some() && cur_temp < slot.min_temp.unwrap()))
            .max_by(|slot1, slot2| slot1.temp.partial_cmp(&slot2.temp).unwrap_or(Ordering::Equal))
            .map(|slot| slot.clone())
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
        assert_eq!(config.get_current_slot(time1, 12.5, true), Some(slot1.clone()), "Simple");
        assert_eq!(config.get_current_slot(time1, 12.5, false), None, "Not on so shouldn't do any overrun");
        assert_eq!(config.get_current_slot(time1, 40.0, true), None, "Too hot so shouldn't do any overrun");

        let slot_1_and_4_time = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(irrelevant_day, NaiveTime::from_hms(03, 32, 00)));
        assert_eq!(config.get_current_slot(slot_1_and_4_time, 12.5, true), Some(slot1.clone()), "Slot 1 because its hotter than Slot 4");
        assert_eq!(config.get_current_slot(slot_1_and_4_time, 12.5, false), Some(slot4.clone()), "Slot 4 because slot 1 only overruns, it won't switch on");
        assert_eq!(config.get_current_slot(slot_1_and_4_time, 30.0, true), Some(slot1.clone()), "Slot 1 because Slot 4 is below the current temp.");
        assert_eq!(config.get_current_slot(slot_1_and_4_time, 40.0, true), None, "Nothing because its too hot already");
    }
}