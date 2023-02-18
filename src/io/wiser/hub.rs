use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::time::Duration;
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::{Client, Method, Request};
use serde::{Deserialize, Serialize};
use async_trait::async_trait;

#[async_trait]
pub trait WiserHub {
    async fn get_data_raw(&self) -> Result<String, reqwest::Error>;

    async fn get_data(&self) -> Result<WiserData, RetrieveDataError>;

    async fn cancel_boost(&self, room_id: usize, originator: String) -> Result<(), Box<dyn std::error::Error>>;

    async fn set_boost(&self, room_id: usize, duration_minutes: usize, temp: f32, originator: String) -> Result<(), Box<dyn std::error::Error>>;
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

impl Display for RetrieveDataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self {
            RetrieveDataError::Network(e) => write!(f, "Network Error: {}", e),
            RetrieveDataError::Json(e) => write!(f, "Deserialization Error: {}", e),
            RetrieveDataError::Other(e) => write!(f, "Unknown Error: {}", e),
        }
    }
}

impl std::error::Error for RetrieveDataError {}

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
        let client = Client::new();

        let request = self.new_request(&client, Method::GET, "data/domain/")?;

        return client.execute(request).await?.text().await;
    }

    async fn get_data(&self) -> Result<WiserData, RetrieveDataError> {
        match self.get_data_raw().await {
            Ok(s) => serde_json::from_str(&s).map_err(|json_err| RetrieveDataError::Json(json_err)),
            Err(network_err) => Err(RetrieveDataError::Network(network_err))
        }
    }

    async fn cancel_boost(&self, room_id: usize, originator: String) -> Result<(), Box<dyn std::error::Error>> {
        let request_payload = RequestOverride::cancel(originator);
        let request_payload = serde_json::to_string(&request_payload)?;

        let client = Client::new();
        let mut request = self.new_request(&client, Method::POST, &format!("data/domain/Room/{}", room_id))?;
        *request.body_mut() = Some(request_payload.into());

        client.execute(request).await?;
        Ok(())
    }

    async fn set_boost(&self, room_id: usize, duration_minutes: usize, temp: f32, originator: String) -> Result<(), Box<dyn std::error::Error>> {
        let request_payload = RequestOverride::new(duration_minutes, temp, originator);
        let request_payload = serde_json::to_string(&request_payload)?;

        let client = Client::new();
        let mut request = self.new_request(&client, Method::POST, &format!("data/domain/Room/{}", room_id))?;
        *request.body_mut() = Some(request_payload.into());

        client.execute(request).await?;
        Ok(())
    }
}

impl IpWiserHub {
    fn new_request(&self, client: &Client, method: Method, location: &str) -> Result<Request, reqwest::Error> {
        client.request(method, format!("http://{}/{}/", self.ip, location))
            .header("SECRET", &self.secret)
            .header("Content-Type", "application/json;charset=UTF-8")
            .timeout(Duration::from_secs(3))
            .build()
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct WiserData {
    system: WiserDataSystem,
    room: Vec<WiserRoomData>
}

impl WiserData {
    pub fn new(system: WiserDataSystem, room: Vec<WiserRoomData>) -> Self {
        Self {
            system,
            room
        }
    }
}

impl WiserData {
    pub fn get_system(&self) -> &WiserDataSystem {
        &self.system
    }

    pub fn get_rooms(&self) -> &Vec<WiserRoomData> {
        &self.room
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct WiserDataSystem {
    unix_time: u64
}

impl WiserDataSystem {
    pub fn new(unix_time: u64) -> Self {
        Self {
            unix_time
        }
    }
}

impl WiserDataSystem {
    pub fn get_unix_time(&self) -> u64 {
        self.unix_time
    }
}

pub const FROM_SCHEDULE_ORIGIN: &str = "FromSchedule";

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct WiserRoomData {
    #[serde(alias = "id")] // This is not pascal case for some reason, unlike every other field.
    id: usize,
    override_type: Option<String>,
    override_timeout_unix: Option<i64>,
    override_set_point: Option<i32>,
    setpoint_origin: String,
    calculated_temperature: i32,
    current_set_point: i32,
    scheduled_set_point: i32,
    name: Option<String>,
}

impl WiserRoomData {
    pub fn new(
        id: usize,
        override_type: Option<String>,
        override_timeout_unix: Option<i64>,
        override_set_point: Option<i32>,
        setpoint_origin: String,
        calculated_temperature: i32,
        current_set_point: i32,
        name: Option<String>) -> Self {
        Self {
            id,
            override_type,
            override_timeout_unix,
            override_set_point,
            setpoint_origin,
            calculated_temperature,
            current_set_point,
            scheduled_set_point: current_set_point,
            name
        }
    }

    pub fn get_id(&self) -> usize {
        self.id
    }

    pub fn get_override_timeout(&self) -> Option<DateTime<Utc>> {
        self.override_timeout_unix.map(|secs| {
            chrono::DateTime::from_utc(NaiveDateTime::from_timestamp(secs, 0), Utc)
        })
    }

    pub fn get_setpoint_origin(&self) -> &str {
        &self.setpoint_origin
    }

    pub fn get_override_set_point(&self) -> Option<f32> {
        self.override_set_point.map(|set_point| (set_point as f32) / 10.0)
    }

    pub fn get_set_point(&self) -> f32 {
        return (self.current_set_point as f32) / 10.0
    }

    pub fn get_scheduled_set_point(&self) -> f32 {
        return (self.scheduled_set_point as f32) / 10.0
    }


    pub fn get_temperature(&self) -> f32 {
        return (self.calculated_temperature as f32) / 10.0
    }

    pub fn get_name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| s.as_str())
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct RequestOverride {
    #[serde(rename = "Type")]
    wiser_type: String,
    duration_minutes: usize,
    // In 10x Celsius
    set_point: i32,
    originator: String,
}

impl RequestOverride {
    pub fn new(duration_minutes: usize, set_point: f32, originator: String) -> Self {
        Self {
            wiser_type: "Manual".to_owned(),
            duration_minutes,
            set_point: (set_point * 10.0) as i32,
            originator
        }
    }

    pub fn cancel(originator: String) -> Self {
        Self::new(0, 0.0, originator)
    }
}

#[cfg(test)]
mod tests {
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