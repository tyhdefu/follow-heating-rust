use chrono::{DateTime, Local, TimeZone, Utc};

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

pub struct DummyTimeProvider {
    utc_time: DateTime<Utc>,
}

impl DummyTimeProvider {
    pub fn new(utc_time: DateTime<Utc>) -> Self {
        Self {
            utc_time
        }
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
