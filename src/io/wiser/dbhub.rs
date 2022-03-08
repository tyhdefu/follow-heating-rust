use std::net::IpAddr;
use chrono::{DateTime, Utc};
use sqlx::MySqlPool;
use crate::io::wiser::hub::{WiserData, IpWiserHub};
use crate::io::wiser::WiserManager;
use async_trait::async_trait;
use crate::wiser::hub::WiserHub;

const HEATING_STATE_DB_ID: u32 = 17;

pub struct DBAndHub {
    hub: IpWiserHub,
    conn: MySqlPool,
}

impl DBAndHub {
    pub fn new(conn: MySqlPool, wiser_ip: IpAddr, wiser_secret: String) -> Self {
        DBAndHub {
            hub: IpWiserHub::new(wiser_ip, wiser_secret),
            conn,
        }
    }
}

#[async_trait]
impl WiserManager for DBAndHub {
    async fn get_heating_turn_off_time(&self) -> Option<DateTime<Utc>> {
        let data = self.hub.get_data().await;
        if let Err(e) = data {
            println!("Error retrieving hub data: {:?}", e);
            return None;
        }
        let data = data.unwrap();
        get_turn_off_time(&data)
    }

    async fn get_heating_on(&self) -> Result<bool,()> {
        let result = sqlx::query!("SELECT raw_value FROM reading WHERE sensor_id=? ORDER BY `id` DESC LIMIT 1", HEATING_STATE_DB_ID)
            .fetch_one(&self.conn).await;
        if result.is_err() {
            println!("{}", result.unwrap_err());
            return Err(())
        }
        match result.unwrap().raw_value {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(())
        }
    }

    fn get_wiser_hub(&self) -> &dyn WiserHub {
        &self.hub
    }
}

fn get_turn_off_time(data: &WiserData) -> Option<DateTime<Utc>> {
    data.get_rooms().iter()
        .filter_map(|room| room.get_override_timeout())
        .max()
}