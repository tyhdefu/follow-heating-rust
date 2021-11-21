use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub mod dummy;
pub mod hub;
pub mod dbhub;

#[async_trait]
pub trait WiserManager {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>>;

    async fn get_heating_on(&self) -> Result<bool, ()>;
}