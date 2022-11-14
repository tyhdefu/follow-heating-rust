use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use crate::time::timeslot::ZonedSlot;

pub fn time(hour: u32, minute: u32, second: u32) -> NaiveTime {
    NaiveTime::from_hms(hour, minute, second)
}

pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd(year, month, day)
}

pub fn utc_time_slot(hour_start: u32, minute_start: u32, second_start: u32, hour_end: u32, minute_end: u32, second_end: u32) -> ZonedSlot {
    ZonedSlot::Utc((time(hour_start, minute_start, second_start)..time(hour_end, minute_end, second_end)).into())
}