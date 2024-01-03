use crate::WiserHub;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub mod dbhub;
pub mod dummy;
pub mod filehub;
pub mod hub;

#[async_trait]
pub trait WiserManager {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>>;

    async fn get_heating_on(&self) -> Result<bool, ()>;

    fn get_wiser_hub(&self) -> &dyn WiserHub;
}

