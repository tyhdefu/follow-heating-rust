use std::fmt::{Display, Formatter, write};
use std::ops::Range;
use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq, Clone)]
pub struct TimeSlot {
    start: NaiveTime,
    end: NaiveTime,
}

impl TimeSlot {
    pub fn contains(&self, time: &NaiveTime) -> bool {
        return if self.start < self.end {
            self.start <= *time && *time <= self.end
        } else {
            *time > self.start || *time <= self.end
        }
    }
}

impl From<Range<NaiveTime>> for TimeSlot {
    fn from(range: Range<NaiveTime>) -> Self {
        Self {
            start: range.start,
            end: range.end
        }
    }
}

impl Display for TimeSlot {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

#[derive(Deserialize, PartialEq, Debug, Clone)]
#[serde(tag = "type")]
pub enum ZonedSlot {
    Utc(TimeSlot),
    Local(TimeSlot),
}

impl ZonedSlot {
    pub fn contains(&self, now: &DateTime<Utc>) -> bool {
        return match self {
            ZonedSlot::Utc(slot) => slot.contains(&now.time()),
            ZonedSlot::Local(slot) => slot.contains(&Local::from_utc_datetime(&Local, &now.naive_utc()).time()),
        }
    }
}

impl Display for ZonedSlot {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        return match self {
            ZonedSlot::Utc(slot) => write!(f, "{} UTC", slot),
            ZonedSlot::Local(slot) => write!(f, "{} Local Time", slot)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
    use super::*;

    fn timeslot_of(start: (u32, u32, u32), end: (u32, u32, u32)) -> TimeSlot {
        (NaiveTime::from_hms(start.0, start.1, start.2)..NaiveTime::from_hms(end.0, end.1, end.2)).into()
    }

    fn timeslot_contains_time_fn(slot: &TimeSlot, h: u32, m: u32, s: u32) -> bool {
        let time = NaiveTime::from_hms(h, m, s);
        slot.contains(&time)
    }

    fn zoned_utc_timeslot_contains_time_fn(slot: &ZonedSlot, date: NaiveDate, h: u32, m: u32, s: u32) -> bool {
        let time = NaiveTime::from_hms(h, m, s);
        let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(date, time));
        slot.contains(&datetime)
    }

    fn zoned_local_timeslot_contains_time_fn(slot: &ZonedSlot, date: NaiveDate, h: u32, m: u32, s: u32) -> bool {
        let time = NaiveTime::from_hms(h, m, s);
        let local_datetime = Local::from_local_datetime(&Local, &NaiveDateTime::new(date, time)).unwrap();
        let utc_datetime = Utc::from_utc_datetime(&Utc, &local_datetime.naive_utc());
        slot.contains(&utc_datetime)
    }

    #[test]
    fn test_timeslot_during_day() {
        let slot: TimeSlot = timeslot_of((12, 13, 00),
                                         (15,52,12));
        let date = NaiveDate::from_ymd(2021, 04, 16);

        let slot_contains_time = |h, m, s| {
            timeslot_contains_time_fn(&slot, h, m, s)
        };

        assert!(slot_contains_time(13, 20, 55), "Slot should contain time");
        assert!(!slot_contains_time(18, 00, 00), "Slot should not contain time");
        assert!(!slot_contains_time(18, 00, 00), "Slot should not contain time");

        // Same scenarios should apply to UTC zoned when supplied with utc.
        let zoned_time_slot = ZonedSlot::Utc(slot.clone());

        let zoned_slot_contains_time = |h, m, s| {
            zoned_utc_timeslot_contains_time_fn(&zoned_time_slot, date, h, m, s)
        };

        assert!(zoned_slot_contains_time(13, 20, 55), "Zoned Slot should contain time");
        assert!(!zoned_slot_contains_time(18, 00, 00), "Zoned Slot should not contain time");
        assert!(!zoned_slot_contains_time(18, 00, 00), "Zoned Slot should not contain time");
    }

    #[test]
    fn test_timeslot_overnight() {
        let slot: TimeSlot = timeslot_of((22, 55, 32),
                                         (04, 26, 26));
        let date = NaiveDate::from_ymd(2021, 04, 16);

        let slot_contains_time = |h, m, s| {
            timeslot_contains_time_fn(&slot, h, m, s)
        };

        assert!(slot_contains_time(02, 00, 00), "Slot should contain time");
        assert!(slot_contains_time(23, 20, 55), "Slot should contain time");
        assert!(!slot_contains_time(13, 00, 00), "Slot should not contain time");

        // Same scenarios should apply to UTC zoned when supplied with utc.
        let zoned_time_slot = ZonedSlot::Utc(slot.clone());

        let zoned_slot_contains_time = |h, m, s| {
            zoned_utc_timeslot_contains_time_fn(&zoned_time_slot, date, h, m, s)
        };

        assert!(zoned_slot_contains_time(02, 00, 00), "Slot should contain time");
        assert!(zoned_slot_contains_time(23, 20, 55), "Slot should contain time");
        assert!(!zoned_slot_contains_time(13, 00, 00), "Slot should not contain time");
    }

    #[test]
    fn test_zoned_local_timeslot_during_day() {
        let zoned_time_slot = ZonedSlot::Local(timeslot_of((12, 13, 00),
                                                         (15,52,12)));
        let date = NaiveDate::from_ymd(2021, 04, 16);

        let zoned_slot_contains_time = |h, m, s| {
            zoned_local_timeslot_contains_time_fn(&zoned_time_slot, date, h, m, s)
        };

        assert!(zoned_slot_contains_time(13, 20, 55), "Slot should contain time");
        assert!(!zoned_slot_contains_time(18, 00, 00), "Slot should not contain time");
        assert!(!zoned_slot_contains_time(18, 00, 00), "Slot should not contain time");

        assert!(!zoned_slot_contains_time(11, 30, 00), "Slot should not contain time");
        assert!(zoned_slot_contains_time(13, 30, 00), "Slot should not contain time");
    }

    #[test]
    fn test_zoned_local_timeslot_overnight() {
        let zoned_time_slot = ZonedSlot::Local(timeslot_of((22, 55, 32),
                                                           (04, 26, 26)));
        let date = NaiveDate::from_ymd(2021, 04, 16);

        let zoned_slot_contains_time = |h, m, s| {
            zoned_local_timeslot_contains_time_fn(&zoned_time_slot, date, h, m, s)
        };

        assert!(zoned_slot_contains_time(02, 00, 00), "Slot should contain time");
        assert!(zoned_slot_contains_time(23, 20, 55), "Slot should contain time");
        assert!(!zoned_slot_contains_time(13, 00, 00), "Slot should not contain time");

        assert!(!zoned_slot_contains_time(22, 30, 00), "Slot should not contain time");
        assert!(zoned_slot_contains_time(23, 30, 00), "Slot should not contain time");
    }

    #[test]
    fn manual_check() {
        std::env::set_var("TZ", "GB");
        let zoned_time_slot = ZonedSlot::Local(timeslot_of((22, 55, 32),
                                                           (04, 26, 26)));
        let bst_date = NaiveDate::from_ymd(2021, 04, 16); // BST
        let contained = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(bst_date, NaiveTime::from_hms(22,30,32)));
        assert!(zoned_time_slot.contains(&contained));


        let gmt_date = NaiveDate::from_ymd(2021, 01, 16); // GMT
        let not_contained = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(gmt_date, NaiveTime::from_hms(22,30,32)));
        assert!(!zoned_time_slot.contains(&not_contained));
    }
}