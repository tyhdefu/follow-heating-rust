use serde::Deserialize;
use std::fmt::{Debug, Formatter};
use chrono::{DateTime, Utc};
use crate::python_like::{CALIBRATION_ERROR, FallbackWorkingRange, MAX_ALLOWED_TEMPERATURE, UNKNOWN_ROOM};
use crate::python_like::heating_mode::get_overrun_temps;
use crate::python_like::overrun_config::OverrunConfig;
use crate::Sensor;
use crate::wiser::hub::{RetrieveDataError, WiserData};

#[derive(Clone, Deserialize, PartialEq)]
pub struct WorkingTemperatureRange {
    max: f32,
    min: f32,
}

impl WorkingTemperatureRange {
    pub fn from_delta(max: f32, delta: f32) -> Self {
        assert!(delta > 0.0);
        WorkingTemperatureRange {
            max,
            min: max - delta,
        }
    }

    pub fn from_min_max(min: f32, max: f32) -> Self {
        assert!(max > min, "Max should be greater than min.");
        WorkingTemperatureRange {
            max,
            min,
        }
    }

    pub fn get_max(&self) -> f32 {
        return self.max;
    }

    pub fn get_min(&self) -> f32 {
        return self.min;
    }

    pub fn modify_max(&mut self, new_max: f32) {
        assert!(self.min < new_max, "New max should be greater than min");
        self.max = new_max;
    }
}

impl Debug for WorkingTemperatureRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "WorkingTemperatureRange {{ min: {:.2} max: {:.2} }}", self.min, self.max)
    }
}

fn get_working_temperature(data: &WiserData) -> (WorkingTemperatureRange, f32) {
    let difference = data.get_rooms().iter()
        .filter(|room| room.get_temperature() > -10.0) // Low battery or something.
        .map(|room| (room.get_name().unwrap_or(UNKNOWN_ROOM), room.get_set_point().min(21.0) - room.get_temperature()))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((UNKNOWN_ROOM, 0.0));

    let range = get_working_temperature_from_max_difference(difference.1);

    if range.get_max() > MAX_ALLOWED_TEMPERATURE {
        eprintln!("Having to cap max temperature from {:.2} to {:.2}", range.max, MAX_ALLOWED_TEMPERATURE);
        let delta = range.get_max() - range.get_min();
        return (WorkingTemperatureRange::from_delta(MAX_ALLOWED_TEMPERATURE, delta), difference.1);
    }
    println!("Working Range {:?} (Room {})", range, difference.0);
    (range, difference.1)
}

fn get_working_temperature_from_max_difference(difference: f32) -> WorkingTemperatureRange {
    const DIFF_CAP: f32 = 2.5;
    const GRAPH_START_TEMP: f32 = 53.2 + CALIBRATION_ERROR;
    const MULTICAND: f32 = 10.0;
    const LEFT_SHIFT: f32 = 0.6;
    const BASE_RANGE_SIZE: f32 = 4.5;

    let capped_difference = difference.clamp(0.0, DIFF_CAP);
    println!("Difference: {:.2}, Capped: {:.2}", difference, capped_difference);
    let difference = capped_difference;
    let min = GRAPH_START_TEMP - (MULTICAND / (difference + LEFT_SHIFT));
    let max = min + BASE_RANGE_SIZE - difference;
    WorkingTemperatureRange::from_min_max(min, max)
}

pub fn get_working_temperature_range_from_wiser_data(fallback: &mut FallbackWorkingRange, result: Result<WiserData, RetrieveDataError>) -> (WorkingTemperatureRange, Option<f32>) {
    result.ok()
        .filter(|data| {
            let good_data = data.get_rooms().iter().any(|r| r.get_temperature() > -10.0);
            if !good_data {
                eprintln!("Bad data detected: no rooms with sensible temperatures");
                eprintln!("{:?}", data);
            }
            good_data
        })
        .map(|data| {
        let (working_range, max_dist) = get_working_temperature(&data);
        fallback.update(working_range.clone());
        (working_range, Some(max_dist))
    }).unwrap_or_else(|| (fallback.get_fallback().clone(), None))
}


/// Gets the working range, using wiser data and overrun configuration.
/// Returns the working temperature and maximum distance to heat in the rooms (in degrees)
pub fn get_working_temperature_range_from_wiser_and_overrun(fallback: &mut FallbackWorkingRange,
                                                            result: Result<WiserData, RetrieveDataError>,
                                                            overrun_config: &OverrunConfig,
                                                            time: DateTime<Utc>) -> (WorkingTemperatureRange, Option<f32>) {
    let (working_temp, max_dist) = get_working_temperature_range_from_wiser_data(fallback, result);

    let working_temp_from_overrun = get_working_temp_range_from_overrun(overrun_config, time);

    if let Some(overrun_range) = working_temp_from_overrun {
        if overrun_range.get_max() > working_temp.get_max() {
            println!("Expanding working range due to overrun: {:?}", overrun_range);
            return (overrun_range, max_dist);
        }
    }

    (working_temp, max_dist)
}

pub fn get_working_temp_range_from_overrun(overrun_config: &OverrunConfig,
                                           time: DateTime<Utc>) -> Option<WorkingTemperatureRange> {
    let view = get_overrun_temps(time, overrun_config);

    if let Some(tkbt_overruns) = view.get_applicable().get(&Sensor::TKBT) {
        if let Some(max_overrun) = tkbt_overruns.iter().filter(|a| a.get_temp().is_normal())
            .max_by(|a,b | a.get_temp().partial_cmp(&b.get_temp()).unwrap()) {

            let working = WorkingTemperatureRange::from_delta(max_overrun.get_temp(), 5.0);
            return Some(working);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use crate::brain::python_like::working_temp::get_working_temperature_from_max_difference;
    use crate::python_like::*;
    use crate::python_like::overrun_config::{OverrunBap, OverrunConfig};
    use crate::python_like::working_temp::{get_working_temp_range_from_overrun, get_working_temperature_range_from_wiser_and_overrun};
    use crate::Sensor;
    use crate::time::test_utils::{date, time, utc_time_slot};
    use crate::wiser::hub::RetrieveDataError;

    #[test]
    fn test_values() {
        //test_value(500.0, 50.0, 52.0);
        test_value(3.0, 50.0, 52.0);
        test_value(2.5, 50.0, 52.0);
        test_value(2.0, 49.4, 51.9);
        test_value(1.5, 48.4, 51.4);
        test_value(0.5, 44.1, 48.1);
        test_value(0.2, 40.7, 45.0);
        test_value(0.1, 38.9, 43.3);
        test_value(0.0, 36.5, 41.0);
    }

    fn test_value(temp_diff: f32, expect_min: f32, expect_max: f32) {
        const GIVE: f32 = 0.05;
        let expect_min = expect_min + CALIBRATION_ERROR;
        let expect_max = expect_max + CALIBRATION_ERROR;

        let range = get_working_temperature_from_max_difference(temp_diff);
        if !is_within_range(range.get_min(), expect_min, GIVE) {
            panic!("Min value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}", expect_min, range.get_min(), GIVE, temp_diff);
        }
        if !is_within_range(range.get_max(), expect_max, GIVE) {
            panic!("Max value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}", expect_min, range.get_max(), GIVE, temp_diff);
        }
    }

    fn is_within_range(check: f32, expect: f32, give: f32) -> bool {
        (check - expect).abs() < give
    }

    #[test]
    fn test_overrun_working_temp() {
        let day = date(2022, 03, 12);
        let slot = utc_time_slot(03, 00, 00,
                                                    04, 00, 00);

        let config = OverrunConfig::new(vec![
            OverrunBap::new(slot, 45.0, Sensor::TKBT),
        ]);
        let utc_time = Utc.from_utc_datetime(&NaiveDateTime::new(day, time(03, 30, 00)));
        let range = get_working_temp_range_from_overrun(&config, utc_time);

        let expected = WorkingTemperatureRange::from_delta(45.0, 5.0);
        assert_eq!(&range, &Some(expected.clone()), "overrun only");

        let mut fallback = FallbackWorkingRange::new(WorkingTemperatureRange::from_delta(41.0, 3.0));
        let (range, _dist) = get_working_temperature_range_from_wiser_and_overrun(&mut fallback, Err(RetrieveDataError::Other("...".to_owned())),
                                                                                 &config, utc_time);

        assert_eq!(range, expected, "overrun + wiser");
    }

    #[test]
    fn test_no_overrun() {
        let day = date(2022, 03, 12);
        let config = OverrunConfig::new(vec![]);

        let utc_time = Utc.from_utc_datetime(&NaiveDateTime::new(day, time(03, 30, 00)));

        let range = get_working_temp_range_from_overrun(&config, utc_time);
        assert_eq!(range, None);
    }
}
