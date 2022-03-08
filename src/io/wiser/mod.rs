use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::io::wiser::hub::IpWiserHub;
use crate::WiserHub;

pub mod dummy;
pub mod hub;
pub mod dbhub;

#[async_trait]
pub trait WiserManager {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>>;

    async fn get_heating_on(&self) -> Result<bool, ()>;

    fn get_wiser_hub(&self) -> &dyn WiserHub;
}