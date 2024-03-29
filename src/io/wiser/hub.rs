use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Client, Method, Request};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::time::Duration;

#[async_trait]
pub trait WiserHub {
    async fn get_data(&self) -> Result<WiserData, RetrieveDataError>;

    async fn get_room_data(&self) -> Result<Vec<WiserRoomData>, RetrieveDataError>;

    async fn cancel_boost(
        &self,
        room_id: usize,
        originator: String,
    ) -> Result<(), Box<dyn std::error::Error>>;

    async fn set_boost(
        &self,
        room_id: usize,
        duration_minutes: usize,
        temp: f32,
        originator: String,
    ) -> Result<(f32, DateTime<Utc>), Box<dyn std::error::Error>>;
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
        IpWiserHub { ip, secret }
    }
}

#[async_trait]
impl WiserHub for IpWiserHub {
    async fn get_data(&self) -> Result<WiserData, RetrieveDataError> {
        match self.get_data_raw(GrabData::All).await {
            Ok(s) => serde_json::from_str(&s).map_err(|json_err| RetrieveDataError::Json(json_err)),
            Err(network_err) => Err(RetrieveDataError::Network(network_err)),
        }
    }

    async fn get_room_data(&self) -> Result<Vec<WiserRoomData>, RetrieveDataError> {
        match self.get_data_raw(GrabData::Room).await {
            Ok(s) => serde_json::from_str(&s).map_err(|json_err| RetrieveDataError::Json(json_err)),
            Err(network_err) => Err(RetrieveDataError::Network(network_err)),
        }
    }

    async fn cancel_boost(
        &self,
        room_id: usize,
        originator: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request_payload = WiserRequest::RequestOverride(RequestOverride::cancel(originator));
        let request_payload = serde_json::to_string(&request_payload)?;

        let client = Client::new();
        let mut request = self.new_request(
            &client,
            Method::PATCH,
            &format!("data/domain/Room/{}", room_id),
        )?;
        *request.body_mut() = Some(request_payload.into());

        let response = client.execute(request).await?;

        if let Err(e) = response.error_for_status_ref() {
            return Err(format!(
                "Got response: {:?}. Body '{}'",
                e.status(),
                response.text().await?.as_str()
            )
            .into());
        }

        Ok(())
    }

    /// Sets a boost on room id (should be gotten from get_data
    /// Returns when the boost will expire.
    async fn set_boost(
        &self,
        room_id: usize,
        duration_minutes: usize,
        temp: f32,
        originator: String,
    ) -> Result<(f32, DateTime<Utc>), Box<dyn std::error::Error>> {
        let request_payload =
            WiserRequest::RequestOverride(RequestOverride::new(duration_minutes, temp, originator));
        let request_payload = serde_json::to_string(&request_payload)?;

        let client = Client::new();
        let mut request = self.new_request(
            &client,
            Method::PATCH,
            &format!("data/domain/Room/{}", room_id),
        )?;
        *request.body_mut() = Some(request_payload.into());

        let response = client.execute(request).await?;

        if let Err(e) = response.error_for_status_ref() {
            return Err(format!(
                "Got response: {:?}. Body '{}'",
                e.status(),
                response.text().await?.as_str()
            )
            .into());
        }

        let new_room_data: WiserRoomData = response.json().await?;

        let timeout = new_room_data.get_override_timeout().ok_or_else(|| {
            format!(
                "No override timeout on received new room data, probably didn't apply: {:?}",
                new_room_data
            )
        })?;

        let temp = new_room_data.get_override_set_point().ok_or_else(|| {
            format!(
                "No override temp on received new room data, probably didn't apply: {:?}",
                new_room_data
            )
        })?;

        Ok((temp, timeout))
    }
}

impl IpWiserHub {
    fn new_request(
        &self,
        client: &Client,
        method: Method,
        location: &str,
    ) -> Result<Request, reqwest::Error> {
        client
            .request(method, format!("http://{}/{}/", self.ip, location))
            .header("SECRET", &self.secret)
            .header("Content-Type", "application/json;charset=UTF-8")
            .timeout(Duration::from_secs(3))
            .build()
    }

    async fn get_data_raw(&self, select: GrabData) -> Result<String, reqwest::Error> {
        let client = Client::new();

        let extension = match select {
            GrabData::All => "",
            GrabData::System => "System/",
            GrabData::Room => "Room/",
        };

        let s = format!("data/domain/{}", extension);

        let request = self.new_request(&client, Method::GET, &s)?;

        return client.execute(request).await?.text().await;
    }
}

enum GrabData {
    /// Get all the data, including all the schedule data
    All,
    /// Get data about system, e.g whether heating is on or off.
    System,
    /// Get data about temperature and current set points of rooms.
    Room,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct WiserData {
    system: WiserDataSystem,
    room: Vec<WiserRoomData>,
}

impl WiserData {
    pub fn new(system: WiserDataSystem, room: Vec<WiserRoomData>) -> Self {
        Self { system, room }
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
    unix_time: u64,
}

impl WiserDataSystem {
    pub fn new(unix_time: u64) -> Self {
        Self { unix_time }
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
    override_timeout_unix_time: Option<i64>,
    #[serde(alias = "OverrideSetpoint")]
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
        override_timeout_unix_time: Option<i64>,
        override_set_point: Option<i32>,
        setpoint_origin: String,
        calculated_temperature: i32,
        current_set_point: i32,
        name: Option<String>,
    ) -> Self {
        Self {
            id,
            override_type,
            override_timeout_unix_time,
            override_set_point,
            setpoint_origin,
            calculated_temperature,
            current_set_point,
            scheduled_set_point: current_set_point,
            name,
        }
    }

    pub fn get_id(&self) -> usize {
        self.id
    }

    pub fn get_override_timeout(&self) -> Option<DateTime<Utc>> {
        self.override_timeout_unix_time
            .map(|secs| Utc.timestamp_opt(secs, 0).single())
            .flatten()
    }

    pub fn get_setpoint_origin(&self) -> &str {
        &self.setpoint_origin
    }

    pub fn get_override_set_point(&self) -> Option<f32> {
        self.override_set_point
            .map(|set_point| (set_point as f32) / 10.0)
    }

    pub fn get_set_point(&self) -> f32 {
        return (self.current_set_point as f32) / 10.0;
    }

    pub fn get_scheduled_set_point(&self) -> f32 {
        return (self.scheduled_set_point as f32) / 10.0;
    }

    pub fn get_temperature(&self) -> f32 {
        return (self.calculated_temperature as f32) / 10.0;
    }

    pub fn get_name(&self) -> Option<&str> {
        self.name.as_ref().map(|s| s.as_str())
    }
}

// Externally tagged.
#[derive(Serialize, Debug)]
enum WiserRequest {
    RequestOverride(RequestOverride),
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
            originator,
        }
    }

    pub fn cancel(originator: String) -> Self {
        Self {
            wiser_type: "None".to_owned(),
            duration_minutes: 0,
            set_point: 0,
            originator: originator.to_owned(),
        }
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

