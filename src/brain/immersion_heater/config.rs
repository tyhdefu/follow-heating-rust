use crate::brain::modes::heating_mode::PossibleTemperatureContainer;
use crate::io::temperatures::Sensor;
use crate::math::model::{LinearModel, Model};
use chrono::{NaiveTime, Timelike};
use log::error;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::ops::RangeInclusive;

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ImmersionHeaterModelConfig {
    parts: Vec<ImmersionHeaterModelPart>,
}

impl ImmersionHeaterModelConfig {
    #[cfg(test)]
    pub fn new(parts: Vec<ImmersionHeaterModelPart>) -> Self {
        Self { parts }
    }

    pub fn combine(&mut self, mut other: Self) {
        self.parts.append(&mut other.parts)
    }

    pub fn should_be_on(
        &self,
        temps: &impl PossibleTemperatureContainer,
        time: NaiveTime,
    ) -> Option<(Sensor, f32)> {
        let mut map: HashMap<Sensor, f32> = HashMap::new();

        for part in &self.parts {
            if let Some(recommended) = part.recommended_temp(time) {
                match temps.get_sensor_temp(part.get_sensor()) {
                    Some(temp) => {
                        if *temp < recommended {
                            map.entry(part.get_sensor().clone())
                                .and_modify(|cur_rec| {
                                    if recommended > *cur_rec {
                                        *cur_rec = recommended
                                    }
                                })
                                .or_insert(recommended);
                        }
                    }
                    None => error!(
                        "Missing sensor: {} when checking if immersion heater should be on",
                        part.sensor
                    ),
                }
            }
        }

        map.into_iter()
            .max_by(|(_sensor1, temp1), (_sensor2, temp2)| temp1.total_cmp(temp2))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImmersionHeaterModelPart {
    range: RangeInclusive<NaiveTime>,
    model: LinearModel,
    sensor: Sensor,
}

impl ImmersionHeaterModelPart {
    pub fn from_time_points(
        start: (NaiveTime, f32),
        end: (NaiveTime, f32),
        sensor: Sensor,
    ) -> Self {
        assert!(end.0 > start.0, "End should be after start");

        let start_sec = start.0.num_seconds_from_midnight();
        let end_sec = end.0.num_seconds_from_midnight();
        let model = LinearModel::from_points((start_sec as f32, start.1), (end_sec as f32, end.1));
        Self {
            range: start.0..=end.0,
            model,
            sensor,
        }
    }

    pub fn recommended_temp(&self, time: NaiveTime) -> Option<f32> {
        if !self.range.contains(&time) {
            return None;
        }
        let secs = time.num_seconds_from_midnight();
        Some(self.model.get(secs as f32))
    }

    pub fn get_sensor(&self) -> &Sensor {
        &self.sensor
    }
}

#[derive(Deserialize)]
struct ImmersionHeaterModelPartData {
    start: TimePoint,
    end: TimePoint,
    sensor: Sensor,
}

#[derive(Deserialize)]
struct TimePoint {
    time: NaiveTime,
    temp: f32,
}

impl<'de> Deserialize<'de> for ImmersionHeaterModelPart {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let data = ImmersionHeaterModelPartData::deserialize(deserializer)?;

        Ok(ImmersionHeaterModelPart::from_time_points(
            (data.start.time, data.start.temp),
            (data.end.time, data.end.temp),
            data.sensor,
        ))
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::brain::immersion_heater::config::{
        ImmersionHeaterModelConfig, ImmersionHeaterModelPart,
    };
    use crate::time_util::test_utils::time;

    #[test]
    fn check_basic() {
        let start = (time(01, 00, 00), 20.0);
        let end = (time(04, 30, 00), 50.0);
        let model = ImmersionHeaterModelPart::from_time_points(start, end, Sensor::TKBT);

        assert_eq!(
            model.recommended_temp(start.0),
            Some(start.1),
            "start should be preserved"
        );
        assert_eq!(
            model.recommended_temp(end.0),
            Some(end.1),
            "end should be preserved"
        );

        assert_eq!(
            model.recommended_temp(time(02, 45, 00)),
            Some(35.0),
            "midpoint should be correct"
        );
        assert_eq!(
            model.recommended_temp(time(03, 37, 30)),
            Some(42.5),
            "midpoint should be correct"
        );

        assert_eq!(
            model.recommended_temp(time(00, 13, 58)),
            None,
            "no immersion heater"
        );
        assert_eq!(
            model.recommended_temp(time(16, 37, 43)),
            None,
            "no immersion heater"
        );
        assert_eq!(
            model.recommended_temp(time(12, 21, 54)),
            None,
            "no immersion heater"
        );
    }

    #[test]
    fn check_complification() {
        let model = ImmersionHeaterModelConfig::new(vec![
            ImmersionHeaterModelPart::from_time_points(
                (time(00, 30, 00), 20.0),
                (time(01, 30, 00), 35.0),
                Sensor::TKTP,
            ),
            ImmersionHeaterModelPart::from_time_points(
                (time(01, 20, 00), 35.0),
                (time(04, 30, 00), 50.0),
                Sensor::TKBT,
            ),
            ImmersionHeaterModelPart::from_time_points(
                (time(03, 30, 00), 37.0),
                (time(04, 20, 00), 55.0),
                Sensor::TKBT,
            ),
        ]);

        let mut temps = HashMap::new();
        temps.insert(Sensor::TKBT, 13.0);
        temps.insert(Sensor::TKTP, 15.0);

        {
            let test1_time = time(01, 00, 00);
            assert_eq!(
                model.should_be_on(&temps, test1_time),
                Some((Sensor::TKTP, 27.5))
            );
        }
        {
            let test2_time = time(01, 25, 00);
            let (test2_sensor, test2_temp) = model.should_be_on(&temps, test2_time).unwrap();
            assert_eq!(test2_sensor, Sensor::TKBT);
            let test2_range = 35.3..35.5;
            assert!(
                test2_range.contains(&test2_temp),
                "temp not in range: {:?}, got: {:.2}",
                test2_range,
                test2_temp
            );
        }
        {
            let test3_time = time(04, 00, 00);
            let (test3_sensor, test3_temp) = model.should_be_on(&temps, test3_time).unwrap();
            assert_eq!(test3_sensor, Sensor::TKBT);
            let test3_range = 47.7..47.9;
            assert!(
                test3_range.contains(&test3_temp),
                "temp not in range: {:?}, got: {:.2}",
                test3_range,
                test3_temp
            );
        }
    }

    #[test]
    fn check_part_deserialization() {
        let config_str =
            std::fs::read_to_string("test/python_brain/immersion_heater/model_part.toml").unwrap();
        let model_part: ImmersionHeaterModelPart = toml::from_str(&config_str).unwrap();

        let start = (time(02, 10, 00), 30.0);
        let end = (time(04, 05, 00), 50.0);
        let expected = ImmersionHeaterModelPart::from_time_points(start, end, Sensor::TKBT);
        assert_eq!(model_part, expected);
    }

    #[test]
    fn check_deserialization() {
        let config_str =
            std::fs::read_to_string("test/python_brain/immersion_heater/model.toml").unwrap();
        let model: ImmersionHeaterModelConfig = toml::from_str(&config_str).unwrap();

        let parts = vec![
            ImmersionHeaterModelPart::from_time_points(
                (time(02, 10, 00), 30.0),
                (time(04, 05, 00), 50.0),
                Sensor::TKBT,
            ),
            ImmersionHeaterModelPart::from_time_points(
                (time(00, 30, 00), 25.6),
                (time(01, 30, 00), 50.3),
                Sensor::TKTP,
            ),
        ];

        assert_eq!(model.parts, parts);
    }
}

