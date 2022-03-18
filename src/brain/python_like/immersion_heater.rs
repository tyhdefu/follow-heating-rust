use std::ops::RangeInclusive;
use chrono::{NaiveTime, Timelike};
use crate::math::model::{LinearModel, Model};

#[derive(Clone, Debug)]
pub struct ImmersionHeaterModel {
    range: RangeInclusive<NaiveTime>,
    model: LinearModel,
}

impl ImmersionHeaterModel {
    pub fn from_time_points(start: (NaiveTime, f32), end: (NaiveTime, f32)) -> Self {
        assert!(end.0 >= start.0, "End should be after start");

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
}