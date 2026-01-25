use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::{Client, Method, Request};
use serde::{Deserialize, Serialize};
use core::fmt;
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
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct WiserRoomData {
    #[serde(alias = "id")] // This is not pascal case for some reason, unlike every other field.
    id:                     usize,
    // manual_set_point          - ?Temperature when "Follow Schedule" unticked from dropdown on room screen?
    override_type:          Option<String>,
    override_timeout_unix_time: Option<i64>,
    #[serde(alias = "OverrideSetpoint")]
    override_set_point:     Option<i32>,
    // schedule_id               - Schedule being followed
    // heating_rate              - ?Always 1200?
    // smart_valve_ids
    name:                   Option<String>,
    // mode                      - "Auto"
    // demand_type               - "Modulating"
    // window_detection_active   - true/false
    calculated_temperature: i32,
    current_set_point:      i32,
    /// ?How far the value is open?
    percentage_demand:      i32,
    // control_output_state      - "Off"
    // window_state              - "Closed" (null if window_detection_average = false)
    /// FromBoost, FromSchedule
    setpoint_origin:        String,
    // displayed_set_point
    scheduled_set_point:    i32,
    // rounded_alexa_temperature  - rounded to nearest 0.5degC
    // effective_mode             - same as mode
    // percentage_demand_for_itrv - same as percentage_demand
    // control_direction          - always "Heat"
    // heating_type               - "HydronicRadiator"
}

impl Display for WiserRoomData {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:<15}: {}/{} Vlv{:>3}",
            OptionalString(&self.name),
            OptionalTemp(&Some(self.calculated_temperature)),
            OptionalTemp(&Some(self.current_set_point)),
            self.percentage_demand,
        )?;

        let diff = (self.calculated_temperature - self.current_set_point) as f32 / 10.0;
        if diff > -1000.0 && diff < 0.029 {
            write!(f, "({diff:0<+4.1})")?;
        }
        else {
            write!(f, "      ")?;
        }

        write!(f, " {:<14}", self.setpoint_origin)?;

        if self.override_set_point.is_some()
            || self.override_type.is_some()
            || self.scheduled_set_point != self.current_set_point
        {
            write!(f, " ({:0<4.1} due to {:<10} until {:?} then {:0<4.1})",
                OptionalTemp(&self.override_set_point),
                OptionalString(&self.override_type),
                self.get_override_timeout(),
                OptionalTemp(&Some(self.scheduled_set_point))
            )?;
        }

        Ok(())
    }
}

pub struct OptionalTemp<'a>(&'a Option<i32>);

impl Display for OptionalTemp<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some(temp) = self.0 {
            if *temp == -32768 {
                write!(f, "UnKn")
            }
            else if *temp == -200 {
                write!(f, "UnSt")
            }
            else {
               write!(f, "{:0>4.1}", *temp as f32 / 10.0)
            }
        }
        else {
            write!(f, "None")
        }
    }
}

pub struct OptionalString<'a>(&'a Option<String>);

impl Display for OptionalString<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let text = if let Some(str) = self.0 { &str } else { "<None>" };
            
        let width = f.width().unwrap_or(0);
        let align = f.align().unwrap_or(fmt::Alignment::Left);

        match align {
            fmt::Alignment::Left   => write!(f, "{text:<width$}"),
            fmt::Alignment::Right  => write!(f, "{text:>width$}"),
            fmt::Alignment::Center => write!(f, "{text:^width$}"),
        }
    }
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
            percentage_demand: 0,
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

