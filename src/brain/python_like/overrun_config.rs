use std::cmp::Ordering;
use std::ops::{Range, RangeInclusive};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
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

    pub fn get_current_slot(&self, now: DateTime<Utc>) -> Option<OverrunBap> {
        self.slots.iter()
            .filter(|slot| slot.slot.contains(&now))
            .max_by(|slot1, slot2| slot1.temp.partial_cmp(&slot2.temp).unwrap_or(Ordering::Equal))
            .map(|slot| slot.clone())
    }
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
pub struct OverrunBap {
    slot: ZonedSlot,
    temp: f32,
}

impl OverrunBap {
    pub fn new(slot: ZonedSlot, temp: f32) -> Self {
        Self {
            slot,
            temp,
        }
    }

    pub fn get_slot(&self) -> &ZonedSlot {
        &self.slot
    }

    pub fn get_temp(&self) -> f32 {
        self.temp
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

        let expected = vec![
            OverrunBap {
                slot: ZonedSlot::Utc(utc_slot),
                temp: 32.8
            },
            OverrunBap {
                slot: ZonedSlot::Local(local_slot),
                temp: 27.3
            },

        ];
        assert_eq!(overrun_config.slots, expected);
    }
}