use crate::brain::modes::heating_mode::PossibleTemperatureContainer;
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::config::working_temp_model::WorkingTempModelConfig;
use crate::io::temperatures::Sensor;
use crate::io::wiser::hub::WiserRoomData;
use crate::python_like::{FallbackWorkingRange, MAX_ALLOWED_TEMPERATURE};
use crate::wiser::hub::RetrieveDataError;
use log::{debug, error, info};
use serde::Deserialize;
use std::fmt::{Debug, Display, Formatter};

const UNKNOWN_ROOM: &str = "Unknown";

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

// Cap the maximum room temperature in order to make boosts at high temperatures
// increase just the time rather than the temperature
const MAX_ROOM_TEMP: f32 = 21.0;

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
                room.get_set_point().min(MAX_ROOM_TEMP) - room.get_temperature(),
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

/// Which way we are currently travelling within the working range.
pub enum CurrentHeatDirection {
    /// Just started up. Fine to go either up or down.
    None,
    /// Already climbing / temperature rising, only start circulating once we hit the top.
    Climbing,
    /// Already falling (circulating already), only stop circulating once we hit the bottom.
    Falling,
}

/// What to do about the working temp in order to stay within the required range.
#[derive(PartialEq, Debug)]
pub enum WorkingTempAction {
    /// Heat up - we are below the top.
    Heat { allow_mixed: bool },
    /// Circulate (i.e. cool down)
    Cool { circulate: bool },
}

/// Forecasts what the Heat Exchanger temperature is likely to be soon based on the temperature of HXOR since
/// it will drop quickly if HXOR is low (and hence maybe we should go straight to On).
/// Returns the forecasted temperature, or the sensor that was missing.
pub fn find_working_temp_action(
    temps: &impl PossibleTemperatureContainer,
    range: &WorkingRange,
    config: &HeatPumpCirculationConfig,
    heat_direction: CurrentHeatDirection,
) -> Result<WorkingTempAction, Sensor> {
    let hx_pct = forecast_hx_pct(temps, config, &heat_direction, range)?;

    // Only cause 1 log if needed.
    let mut tk_pct_cached = None;
    let mut get_tk_pct = || -> Result<f32, Sensor> {
        if tk_pct_cached.is_none() {
            tk_pct_cached = Some(forecast_tk_pct(temps, config, &heat_direction, range)?);
        }
        Ok(tk_pct_cached.unwrap())
    };

    let should_cool = match heat_direction {
        CurrentHeatDirection::Falling => hx_pct >= 0.0,
        CurrentHeatDirection::Climbing => hx_pct >= 1.0,
        CurrentHeatDirection::None => {
            let tk_pct = get_tk_pct()?;

            // Happy to circulate first
            let hx_above_req = hx_pct >= config.get_forecast_start_above_percent();
            // Happy to drain from tank first
            let tk_above_req = tk_pct >= config.get_forecast_start_above_percent();

            hx_above_req || tk_above_req
        }
    };

    let (required_pct, used_tk) = match heat_direction {
        CurrentHeatDirection::None => (Some(config.get_forecast_start_above_percent()), true),
        _ => (None, false),
    };
    if should_cool || used_tk {
        info!(
            "HX Forecast ({}), TK Forecast ({})",
            format_pct(hx_pct, required_pct),
            format_pct(get_tk_pct()?, required_pct)
        )
    } else {
        info!("HX Forecast ({})", format_pct(hx_pct, required_pct))
    }

    if !should_cool {
        return Ok(WorkingTempAction::Heat {
            allow_mixed: hx_pct > config.mixed_forecast_above_percent(),
        });
    }

    Ok(WorkingTempAction::Cool {
        circulate: get_tk_pct()? >= hx_pct,
    })
}

fn format_pct(pct: f32, required_pct: Option<f32>) -> String {
    if pct > 1.0 {
        "Above top".to_owned()
    } else if pct < 0.0 {
        "Below bottom".to_owned()
    } else {
        match required_pct {
            Some(required) => format!("{:.0}%, req. {:.0}%", pct * 100.0, required * 100.0),
            _ => format!("{:.0}%", pct * 100.0),
        }
    }
}

fn forecast_hx_pct(
    temps: &impl PossibleTemperatureContainer,
    config: &HeatPumpCirculationConfig,
    heat_direction: &CurrentHeatDirection,
    range: &WorkingRange,
) -> Result<f32, Sensor> {
    let hxif = temps.get_sensor_temp(&Sensor::HXIF).ok_or(Sensor::HXIF)?;
    let hxir = temps.get_sensor_temp(&Sensor::HXIR).ok_or(Sensor::HXIR)?;
    let hxor = temps.get_sensor_temp(&Sensor::HXOR).ok_or(Sensor::HXOR)?;

    let avg_hx = (hxif + hxir) / 2.0;

    let adjusted_difference = (avg_hx - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let adjusted_temp = (avg_hx - expected_drop).clamp(0.0, MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let hx_pct = (adjusted_temp - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.get_forecast_start_above_percent()),
        _ => None,
    };

    debug!(
        "Avg. HXI: {:.2}, HXOR: {:.2}, HX Forecast temp: {:.2} ({})",
        avg_hx,
        hxor,
        adjusted_temp,
        format_pct(hx_pct, required_pct),
    );

    Ok(hx_pct)
}

fn forecast_tk_pct(
    temps: &impl PossibleTemperatureContainer,
    config: &HeatPumpCirculationConfig,
    heat_direction: &CurrentHeatDirection,
    range: &WorkingRange,
) -> Result<f32, Sensor> {
    let tkbt = temps.get_sensor_temp(&Sensor::TKBT).ok_or(Sensor::TKBT)?;
    let hxor = temps.get_sensor_temp(&Sensor::HXOR).ok_or(Sensor::HXOR)?;

    let adjusted_difference = (tkbt - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);

    let adjusted_temp = (tkbt - expected_drop).clamp(0.0, MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let tk_pct = (adjusted_temp - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.get_forecast_start_above_percent()),
        _ => None,
    };

    debug!(
        "TKBT: {:.2} TK Forecast for circulate: {:.2} ({})",
        tkbt,
        adjusted_temp,
        format_pct(tk_pct, required_pct),
    );

    Ok(tk_pct)
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use crate::brain::python_like::config::{
        working_temp_model::WorkingTempModelConfig, PythonBrainConfig,
    };

    use super::*;
    use std::collections::HashMap;

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

    #[test]
    fn test_none_heat_not_mixed() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 30.5);
        temps.insert(Sensor::HXIR, 30.5);
        temps.insert(Sensor::HXOR, 30.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: false }, action);

        Ok(())
    }

    #[test]
    fn test_none_heat_from_tank() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 25.0);
        temps.insert(Sensor::HXIR, 25.0);
        temps.insert(Sensor::HXOR, 25.0);
        temps.insert(Sensor::TKBT, 60.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: true }, action);

        Ok(())
    }

    #[test]
    fn test_none_refuse_circulate() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 40.5);
        temps.insert(Sensor::HXIR, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: false }, action);

        Ok(())
    }

    #[test]
    fn test_none_idle_when_tank_cold_but_hx_warm() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 39.5);
        temps.insert(Sensor::HXIR, 39.5);
        temps.insert(Sensor::HXOR, 39.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: false }, action);

        Ok(())
    }

    #[test]
    fn test_cool_using_idle_when_reach_top() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 40.5);
        temps.insert(Sensor::HXIR, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: false }, action);

        Ok(())
    }

    #[test]
    fn test_mixed_when_reach_high_in_range() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 39.5);
        temps.insert(Sensor::HXIR, 39.5);
        temps.insert(Sensor::HXOR, 39.5);
        temps.insert(Sensor::TKBT, 30.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: true }, action);

        Ok(())
    }

    #[test]
    fn test_cool_using_tank_when_reach_top() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 40.5);
        temps.insert(Sensor::HXIR, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 45.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: true }, action);

        Ok(())
    }

    #[test]
    fn test_heat_when_hit_bottom() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 29.5);
        temps.insert(Sensor::HXIR, 29.5);
        temps.insert(Sensor::HXOR, 29.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: false }, action);

        Ok(())
    }
}
