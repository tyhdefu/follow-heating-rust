use crate::time_util::timeslot::ZonedSlot;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};

pub fn time(hour: u32, minute: u32, second: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, second).expect(&format!(
        "Expected {:0>2}:{:0>2}:{:0>2} to be a valid date",
        hour, minute, second
    ))
}

pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect(&format!(
        "Expected {:0>4}-{:0>2}-{:0>2} to be a valid date",
        year, month, day
    ))
}

pub fn utc_datetime(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date(year, month, day).and_time(time(hour, minute, second)))
}

pub fn utc_time_slot(
    hour_start: u32,
    minute_start: u32,
    second_start: u32,
    hour_end: u32,
    minute_end: u32,
    second_end: u32,
) -> ZonedSlot {
    ZonedSlot::Utc(
        (time(hour_start, minute_start, second_start)..time(hour_end, minute_end, second_end))
            .into(),
    )
}

pub fn local_time_slot(
    hour_start: u32,
    minute_start: u32,
    second_start: u32,
    hour_end: u32,
    minute_end: u32,
    second_end: u32,
) -> ZonedSlot {
    ZonedSlot::Local(
        (time(hour_start, minute_start, second_start)..time(hour_end, minute_end, second_end))
            .into(),
    )
}

