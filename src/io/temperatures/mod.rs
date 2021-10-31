use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use async_trait::async_trait;

pub mod database;
pub mod dummy;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Sensor {
    TKTP,
    TKEN,
    TKEX,
    TKBT,
    HPFL,
    HPRT,
    TKFL,
    TKRT,
    HXOF,
    HXOR,
    HXIF,
    HXIR,
    Other(SensorId)
}

impl Display for Sensor {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Sensor::Other(id) = &self {
            return write!(f, "{}", id.id);
        }
        write!(f, "{:?}", &self)
    }
}

impl From<String> for Sensor {
    fn from(s: String) -> Self {
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "tktp" => Sensor::TKTP,
            "tken" => Sensor::TKEN,
            "tkex" => Sensor::TKEX,
            "tkbt" => Sensor::TKBT,
            "hprt" => Sensor::HPRT,
            "hpfl" => Sensor::HPFL,
            "tkfl" => Sensor::TKFL,
            "tkrt" => Sensor::TKRT,
            "hxof" => Sensor::HXOF,
            "hxor" => Sensor::HXOR,
            "hxif" => Sensor::HXIF,
            "hxir" => Sensor::HXIR,
            _ => Sensor::Other(SensorId::new(lower)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SensorId {
    id: String
}

impl SensorId {
    /// If looking to construct a sensor, use Sensor::from()
    fn new(id: String) -> SensorId {
        assert!(id.is_ascii(), "Id must be ascii");
        SensorId {
            id: id.to_ascii_lowercase()
        }
    }
}

impl Display for SensorId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}


#[async_trait]
pub trait TemperatureManager {
    async fn retrieve_sensors(&mut self) -> Result<(), String>;

    async fn retrieve_temperatures(&self) -> Result<HashMap<Sensor, f32>, String>;
}

#[cfg(test)]
mod tests {
    use crate::io::temperatures::Sensor::Other;
    use super::*;

    #[test]
    fn sanity() {
        let sensors = [Sensor::TKTP, Sensor::TKEN,
            Sensor::TKEX, Sensor::TKBT,
            Sensor::HPFL, Sensor::HPRT,
            Sensor::TKFL, Sensor::TKRT,
            Sensor::HXOF, Sensor::HXOR, Sensor::HXIF, Sensor::HXIR,
            Other(SensorId::new("dumb_sensor".to_owned()))];
        for sensor in sensors {
            let same_sensor = sensor.to_string().into();
            assert_eq!(sensor, same_sensor, "Expected sensor '{}' to transform back into itself.", sensor.to_string());
        }
    }
}