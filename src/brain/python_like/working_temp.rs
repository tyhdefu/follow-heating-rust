use crate::brain::python_like::config::working_temp_model::WorkingTempModelConfig;
use crate::io::wiser::hub::WiserRoomData;
use crate::python_like::{FallbackWorkingRange, MAX_ALLOWED_TEMPERATURE, UNKNOWN_ROOM};
use crate::wiser::hub::RetrieveDataError;
use log::error;
use serde::Deserialize;
use std::fmt::{Debug, Display, Formatter};

#[derive(Clone)]
pub struct WorkingRange {
    temp_range: WorkingTemperatureRange,

    room: Option<Room>,
}

impl WorkingRange {
    pub fn from_wiser(temp_range: WorkingTemperatureRange, room: Room) -> Self {
        Self {
            temp_range: temp_range.clone(),
            room: Some(room),
        }
    }

    pub fn from_temp_only(temp_range: WorkingTemperatureRange) -> Self {
        Self {
            temp_range: temp_range.clone(),
            room: None,
        }
    }

    pub fn get_min(&self) -> f32 {
        self.temp_range.get_min()
    }

    pub fn get_max(&self) -> f32 {
        self.temp_range.get_max()
    }

    pub fn get_temperature_range(&self) -> &WorkingTemperatureRange {
        &self.temp_range
    }

    pub fn get_room(&self) -> Option<&Room> {
        self.room.as_ref()
    }
}

impl Display for WorkingRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Room ")?;
        match &self.room {
            None => write!(f, "N/A: ",)?,
            Some(room) => {
                write!(f, "{} (diff: {:.1}", room.name, room.difference)?;
                if room.capped_difference != room.difference {
                    write!(f, ", cap: {:.1}", room.capped_difference)?;
                }
                write!(f, "); ")?;
            }
        }
        write!(
            f,
            "Working Range {:.2}-{:.2}",
            self.get_min(),
            self.get_max()
        )?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct Room {
    name: String,
    difference: f32,
    capped_difference: f32,
}

impl Room {
    pub fn of(name: String, difference: f32, capped_difference: f32) -> Self {
        Self {
            name,
            difference,
            capped_difference,
        }
    }

    pub fn get_difference(&self) -> f32 {
        self.capped_difference
    }
}

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
        WorkingTemperatureRange { max, min }
    }

    pub fn get_max(&self) -> f32 {
        self.max
    }

    pub fn get_min(&self) -> f32 {
        self.min
    }
}

impl Debug for WorkingTemperatureRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WorkingTemperatureRange {{ min: {:.2} max: {:.2} }}",
            self.min, self.max
        )
    }
}

impl Display for WorkingTemperatureRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}-{:.2}", self.min, self.max)
    }
}

fn get_working_temperature(
    data: &[WiserRoomData],
    working_temp_config: &WorkingTempModelConfig,
) -> WorkingRange {
    let difference = data
        .iter()
        .filter(|room| room.get_temperature() > -10.0) // Low battery or something.
        .map(|room| {
            (
                room.get_name().unwrap_or(UNKNOWN_ROOM),
                room.get_set_point().min(21.0) - room.get_temperature(),
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((UNKNOWN_ROOM, 0.0));

    let (range, capped_difference) =
        get_working_temperature_from_max_difference(difference.1, working_temp_config);

    let room = Room::of(difference.0.to_owned(), difference.1, capped_difference);

    if range.get_max() > MAX_ALLOWED_TEMPERATURE {
        error!(
            "Having to cap max temperature from {:.2} to {:.2}",
            range.max, MAX_ALLOWED_TEMPERATURE
        );
        let delta = range.get_max() - range.get_min();
        let temp_range = WorkingTemperatureRange::from_delta(MAX_ALLOWED_TEMPERATURE, delta);
        return WorkingRange::from_wiser(temp_range, room);
    }
    WorkingRange::from_wiser(range, room)
}

fn get_working_temperature_from_max_difference(
    difference: f32,
    config: &WorkingTempModelConfig,
) -> (WorkingTemperatureRange, f32) {
    let capped_difference = difference.clamp(0.0, config.get_difference_cap());
    let difference = capped_difference;
    let min = config.get_max_lower_temp()
        - (config.get_multiplicand() / (difference + config.get_left_shift()));
    let max = min + config.get_base_range_size() - difference;
    (
        WorkingTemperatureRange::from_min_max(min, max),
        capped_difference,
    )
}

pub fn get_working_temperature_range_from_wiser_data(
    fallback: &mut FallbackWorkingRange,
    result: Result<Vec<WiserRoomData>, RetrieveDataError>,
    working_temp_config: &WorkingTempModelConfig,
) -> WorkingRange {
    result
        .ok()
        .filter(|data| {
            let good_data = data.iter().any(|r| r.get_temperature() > -10.0);
            if !good_data {
                error!(target: "wiser", "Bad data detected: no rooms with sensible temperatures");
                error!(target: "wiser", "{:?}", data);
            }
            good_data
        })
        .map(|data| {
            let working_range = get_working_temperature(&data, working_temp_config);
            fallback.update(working_range.get_temperature_range().clone());
            working_range
        })
        .unwrap_or_else(|| WorkingRange::from_temp_only(fallback.get_fallback().clone()))
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod tests {
    use crate::brain::python_like::config::working_temp_model::WorkingTempModelConfig;
    use crate::brain::python_like::working_temp::get_working_temperature_from_max_difference;

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
        let expect_min = expect_min;
        let expect_max = expect_max;

        let (range, _capped) = get_working_temperature_from_max_difference(
            temp_diff,
            &WorkingTempModelConfig::default(),
        );
        if !is_within_range(range.get_min(), expect_min, GIVE) {
            panic!(
                "Min value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}",
                expect_min,
                range.get_min(),
                GIVE,
                temp_diff
            );
        }
        if !is_within_range(range.get_max(), expect_max, GIVE) {
            panic!(
                "Max value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}",
                expect_min,
                range.get_max(),
                GIVE,
                temp_diff
            );
        }
    }

    fn is_within_range(check: f32, expect: f32, give: f32) -> bool {
        (check - expect).abs() < give
    }
}
