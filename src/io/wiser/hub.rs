use std::future::Future;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::{Client};
use serde::{Serialize, Deserialize};
use async_trait::async_trait;

#[async_trait]
pub trait WiserHub {
    async fn get_data_raw(&self) -> Result<String, reqwest::Error>;

    async fn get_data(&self) -> Result<WiserData, RetrieveDataError>;
}

pub struct IpWiserHub {
    ip: IpAddr,
    secret: String,
}

#[derive(Debug)]
pub enum RetrieveDataError {
    Network(reqwest::Error),
    Json(serde_json::Error),
    Other(String),
}

impl IpWiserHub {
    pub fn new(ip: IpAddr, secret: String) -> Self {
        IpWiserHub {
            ip,
            secret,
        }
    }
}

#[async_trait]
impl WiserHub for IpWiserHub {
    async fn get_data_raw(&self) -> Result<String, reqwest::Error> {
        let url = format!("http://{}/data/domain/", self.ip);
        let client = Client::new();

        let request = client.get(url)
            .header("SECRET", &self.secret)
            .header("Content-Type", "application/json;charset=UTF-8")
            .timeout(Duration::from_secs(3))
            .build()?;
        return client.execute(request).await?.text().await;
    }

    async fn get_data(&self) -> Result<WiserData, RetrieveDataError> {
        match self.get_data_raw().await {
            Ok(s) => serde_json::from_str(&s).map_err(|json_err| RetrieveDataError::Json(json_err)),
            Err(network_err) => Err(RetrieveDataError::Network(network_err))
        }
    }

}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct WiserData {
    system: WiserDataSystem,
    room: Vec<WiserRoomData>
}

impl WiserData {
    pub fn get_system(&self) -> &WiserDataSystem {
        &self.system
    }

    pub fn get_rooms(&self) -> &Vec<WiserRoomData> {
        &self.room
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct WiserDataSystem {
    unix_time: u64
}

impl WiserDataSystem {
    pub fn get_unix_time(&self) -> u64 {
        self.unix_time
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct WiserRoomData {
    #[serde(alias = "id")] // This is not pascal case for some reason, unlike every other field.
    id: usize,
    override_type: Option<String>,
    override_timeout_unix: Option<i64>,
    override_set_point: Option<i32>,
    calculated_temperature: i32,
    current_set_point: i32,
    name: Option<String>,
}

impl WiserRoomData {
    pub fn get_id(&self) -> usize {
        self.id
    }

    pub fn get_override_timeout(&self) -> Option<DateTime<Utc>> {
        self.override_timeout_unix.map(|secs| {
            chrono::DateTime::from_utc(NaiveDateTime::from_timestamp(secs, 0), Utc)
        })
    }

    pub fn get_set_point(&self) -> f32 {
        return (self.current_set_point as f32) / 10.0
    }

    pub fn get_temperature(&self) -> f32 {
        return (self.calculated_temperature as f32) / 10.0
    }

    pub fn get_name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use super::*;
    use std::fs;

    #[tokio::test]
    pub async fn test_deserialization() {
        let json = fs::read_to_string("test/test_wiser_output.json").unwrap();
        let data: WiserData = serde_json::from_str(&json).unwrap();
        assert_eq!(data.system.unix_time, 1637331300);
        assert_eq!(data.room.len(), 8);
    }
}