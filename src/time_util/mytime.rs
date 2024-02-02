use chrono::{DateTime, Duration, Local, TimeZone, Utc};

#[cfg(test)]
use super::timeslot::ZonedSlot;

pub trait TimeProvider {
    fn get_utc_time(&self) -> DateTime<Utc>;

    fn get_local_time(&self) -> DateTime<Local>;
}

#[derive(Default)]
pub struct RealTimeProvider {}

impl TimeProvider for RealTimeProvider {
    fn get_utc_time(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn get_local_time(&self) -> DateTime<Local> {
        Local::now()
    }
}

#[derive(Debug)]
pub struct DummyTimeProvider {
    utc_time: DateTime<Utc>,
}

impl DummyTimeProvider {
    pub fn new(utc_time: DateTime<Utc>) -> Self {
        Self { utc_time }
    }

    #[cfg(test)]
    pub fn in_slot(slot: &ZonedSlot) -> Self {
        use chrono::NaiveDate;
        use log::info;

        let bst_date = &NaiveDate::from_ymd_opt(2023, 6, 10).unwrap();

        let utc_time = match slot {
            ZonedSlot::Utc(slot) => {
                let within_time = slot.get_start() + Duration::seconds(30);
                if within_time > slot.get_end() {
                    panic!("Timeslot too short to get time within: {}", slot);
                }
                Utc.from_utc_datetime(&bst_date.and_time(within_time))
            }
            ZonedSlot::Local(slot) => {
                let within_time = slot.get_start() + Duration::seconds(30);
                if within_time > slot.get_end() {
                    panic!("Timeslot too short to get time within: {}", slot);
                }
                Utc.from_local_datetime(&bst_date.and_time(within_time))
                    .single()
                    .expect("Cannot decide which result to use")
            }
        };

        assert!(
            slot.contains(&utc_time),
            "Slot {} does not contain time: {}",
            slot,
            utc_time
        );

        info!("Selecting {} as the time within {}", utc_time, slot);
        Self { utc_time }
    }

    /// Change the time returned by this dummy time provider.
    pub fn set(&mut self, utc_time: DateTime<Utc>) {
        self.utc_time = utc_time;
    }

    /// Move the time returned by this dummy time provider forward by the given duration
    pub fn advance(&mut self, duration: Duration) {
        self.utc_time += duration;
    }
}

impl TimeProvider for DummyTimeProvider {
    fn get_utc_time(&self) -> DateTime<Utc> {
        self.utc_time.clone()
    }

    fn get_local_time(&self) -> DateTime<Local> {
        Local.from_utc_datetime(&self.utc_time.naive_utc())
    }
}
