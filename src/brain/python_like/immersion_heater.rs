use std::ops::RangeInclusive;
use chrono::{NaiveTime, Timelike};
use serde::{Deserialize, Deserializer};
use crate::math::model::{LinearModel, Model};

#[derive(Clone, Debug, PartialEq)]
pub struct ImmersionHeaterModel {
    range: RangeInclusive<NaiveTime>,
    model: LinearModel,
}

impl ImmersionHeaterModel {
    pub fn from_time_points(start: (NaiveTime, f32), end: (NaiveTime, f32)) -> Self {
        assert!(end.0 > start.0, "End should be after start");

        let start_sec = start.0.num_seconds_from_midnight();
        let end_sec = end.0.num_seconds_from_midnight();
        let model = LinearModel::from_points((start_sec as f32, start.1), (end_sec as f32, end.1));
        Self {
            range: start.0..=end.0,
            model,
        }
    }

    pub fn recommended_temp(&self, time: NaiveTime) -> Option<f32> {
        if !self.range.contains(&time) {
            return None;
        }
        let secs = time.num_seconds_from_midnight();
        Some(self.model.get(secs as f32))
    }
}

#[derive(Deserialize)]
struct TimePoints {
    start: TimePoint,
    end: TimePoint,
}

#[derive(Deserialize)]
struct TimePoint {
    time: NaiveTime,
    temp: f32,
}

impl<'de> Deserialize<'de> for ImmersionHeaterModel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
        let time_points = TimePoints::deserialize(deserializer)?;
        Ok(ImmersionHeaterModel::from_time_points((time_points.start.time, time_points.start.temp),
                                                  (time_points.end.time, time_points.end.temp)))
    }
}

#[cfg(test)]
mod test {
    use chrono::NaiveTime;
    use super::*;

    #[test]
    fn check_basic() {
        let start = (NaiveTime::from_hms(01, 00, 00), 20.0);
        let end = (NaiveTime::from_hms(04, 30, 00), 50.0);
        let model = ImmersionHeaterModel::from_time_points(start, end);

        assert_eq!(model.recommended_temp(start.0), Some(start.1), "start should be preserved");
        assert_eq!(model.recommended_temp(end.0), Some(end.1), "end should be preserved");

        assert_eq!(model.recommended_temp(NaiveTime::from_hms(02, 45, 00)), Some(35.0), "midpoint should be correct");
        assert_eq!(model.recommended_temp(NaiveTime::from_hms(03, 37, 30)), Some(42.5), "midpoint should be correct");

        assert_eq!(model.recommended_temp(NaiveTime::from_hms(00, 13, 58)), None, "no immersion heater");
        assert_eq!(model.recommended_temp(NaiveTime::from_hms(16, 37, 43)), None, "no immersion heater");
        assert_eq!(model.recommended_temp(NaiveTime::from_hms(12, 21, 54)), None, "no immersion heater");
    }

    #[test]
    fn check_deserialization() {
        let config_str = std::fs::read_to_string("test/immersion_heater/basic_model.toml").unwrap();
        let model: ImmersionHeaterModel = toml::from_str(&config_str).unwrap();

        let start = (NaiveTime::from_hms(02, 10, 00), 30.0);
        let end = (NaiveTime::from_hms(04, 05, 00), 50.0);
        let expected = ImmersionHeaterModel::from_time_points(start, end);
        assert_eq!(model, expected);
    }
}