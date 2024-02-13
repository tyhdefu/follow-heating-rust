use crate::brain::modes::heating_mode::PossibleTemperatureContainer;
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::config::working_temp_model::WorkingTempModelConfig;
use crate::io::temperatures::Sensor;
use crate::io::wiser::hub::WiserRoomData;
use crate::python_like::FallbackWorkingRange;
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

    WorkingRange::from_wiser(range, room)
}

fn get_working_temperature_from_max_difference(
    difference: f32,
    config: &WorkingTempModelConfig,
) -> (WorkingTemperatureRange, f32) {
    (
        WorkingTemperatureRange::from_min_max(
            config.min.get_temp_from_room_diff(difference),
            config.max.get_temp_from_room_diff(difference)
        ),
        difference,
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
    Heat { mixed_state: MixedState },
    /// Circulate (i.e. cool down)
    Cool { circulate: bool },
}

/// Whether in mixed mode i.e. whether heating and hot water both being heated
#[derive(PartialEq, Debug)]
pub enum MixedState {
    /// Both heating and hot water being heated
    MixedHeating,
    /// Heating boosted from hot water
    BoostedHeating,
    /// Not mixed
    NotMixed,
}

/// Forecasts what the Heat Exchanger temperature is likely to be soon based on the temperature of HXOR since
/// it will drop quickly if HXOR is low (and hence maybe we should go straight to On).
/// Returns the forecasted temperature, or the sensor that was missing.
pub fn find_working_temp_action(
    temps: &impl PossibleTemperatureContainer,
    range: &WorkingRange,
    config: &HeatPumpCirculationConfig,
    heat_direction: CurrentHeatDirection,
    mixed_state: Option<MixedState>,
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
        return Ok(WorkingTempAction::Heat { mixed_state: get_mixed_state(temps, config, mixed_state, hx_pct)? });
    }

    Ok(WorkingTempAction::Cool {
        circulate: temps.get_sensor_temp(&Sensor::TKBT).ok_or(Sensor::TKBT)? > temps.get_sensor_temp(&Sensor::HXOF).ok_or(Sensor::HXOF)?,
    })
}

fn get_mixed_state(
    temps:          &impl PossibleTemperatureContainer,
    config:         &HeatPumpCirculationConfig,
    mixed_state:    Option<MixedState>,
    hx_pct:         f32
) -> Result<MixedState, Sensor> {
    if let Some(mixed_state) = mixed_state {
        // Possible candidate for boosting. This is where the heat pump is on, but the values and pump speeds
        // are such that some the the water from HXRT enters the tank heat exchanger at TKRT, gets heated and
        // comes out at TKFL (flowing in the reverse of the normal direction) to join the flow from HPFL.
        // On an energy flow analysis this will provide a boost if TKFL > HXRT, but perhaps only by throttling
        // the flow through the heat pump which reduces its efficiency. A more conservative view is to only
        // boost if TKFL > HPFL which will directly enrich HXFL.
        // There remains the problem that TKFL and HPFL are only fully accurate for this purpose when water is
        // flowing through them and the sensors have had time to respond. Other cases are considered below:
        // * Tank has just been heated: TKFL will be a significant over-estimate of what would flow from
        // the tank, but it will be similar to HPFL so unlikely to be an issue. If nothing has happened since
        // the it is reasonable to expect TKFL to fall quicker than HPFL so also not an issue.
        // * Tank heated then switched to heating: TKFL will be an overestimate and HPFL will likely be
        // materially lower. This is a risk this will incorrectly trigger boosting for a short while.
        // * Tank not heated recently: Experience suggests that TKFL will be influenced by bleed across from
        // HPFL as much as the actual tank temperature. This will often pull it down causeing the boost to
        // fail to trigger when it should.
        // * Was recently circulating from tank: TKFL will be accurate, but HPFL could be too high if heat
        // has been retained in the system. This will soon be resolved when the heat pump switches on.
        // In summary, the threshold differency between TKFL and HPFL needs to be tunable. Hysteresis is
        // suspected not to be required, but might as well be provided.
        let tkfl = temps.get_sensor_temp(&Sensor::TKFL).ok_or(Sensor::TKFL)?;
        let hpfl = temps.get_sensor_temp(&Sensor::HPFL).ok_or(Sensor::HPFL)?;

        match mixed_state {
            MixedState::BoostedHeating => {
                if hx_pct < config.boost_mode.stop_heat_pct && tkfl - hpfl >= config.boost_mode.stop_tkfl_hpfl_diff {
                    return Ok(MixedState::BoostedHeating)
                }
            }
            MixedState::NotMixed | MixedState::MixedHeating => {
                if hx_pct < config.boost_mode.start_heat_pct && tkfl - hpfl >= config.boost_mode.start_tkfl_hpfl_diff {
                    return Ok(MixedState::BoostedHeating)
                }
            }
        }            

        match mixed_state {
            MixedState::MixedHeating => 
                if hx_pct > config.mixed_mode.stop_heat_pct {
                    return Ok(MixedState::MixedHeating)
                }
            MixedState::NotMixed | MixedState::BoostedHeating =>
                if hx_pct > config.mixed_mode.start_heat_pct {
                    return Ok(MixedState::MixedHeating)
                }
        }
    }

    return Ok(MixedState::NotMixed);
}

fn format_pct(pct: f32, required_pct: Option<f32>) -> String {
    if pct > 1.0 {
        "HI".to_owned()
    } else if pct < -0.995 {
        "LO".to_owned()
    } else {
        match required_pct {
            Some(required) => format!("{:.0}%, req. {:.0}%", pct * 100.0, required * 100.0),
            _ => format!("{:.0}%", pct * 100.0),
        }
    }
}

/// For anything above this the effect of the HXOR on the forecast will be ignored
/// in order to avoid accidentally overdriving the heat pump when the drop across
/// the heat exchanger is high
const HPRT_LO_LIMIT: f32 = 50.0;
const HPRT_HI_LIMIT: f32 = 54.0;

fn forecast_hx_pct(
    temps: &impl PossibleTemperatureContainer,
    config: &HeatPumpCirculationConfig,
    heat_direction: &CurrentHeatDirection,
    range: &WorkingRange,
) -> Result<f32, Sensor> {
    let hxif = temps.get_sensor_temp(&Sensor::HXIF).ok_or(Sensor::HXIF)?;
    let hxir = temps.get_sensor_temp(&Sensor::HXIR).ok_or(Sensor::HXIR)?;
    let hxor = temps.get_sensor_temp(&Sensor::HXOR).ok_or(Sensor::HXOR)?;
    let hprt = temps.get_sensor_temp(&Sensor::HPRT).ok_or(Sensor::HPRT)?;

    let hxia = (hxif + hxir) / 2.0;
    
    let adjusted_difference = (hxia - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let hxia_forecast_raw = hxia - expected_drop;

    let adjust_pct = ((hprt - HPRT_LO_LIMIT) / (HPRT_HI_LIMIT - HPRT_LO_LIMIT)).clamp(0.0, 1.0);
    let hxia_forecast = hxia_forecast_raw + (HPRT_HI_LIMIT - hxia_forecast_raw) * adjust_pct;

    let range_width = range.get_max() - range.get_min();

    let hx_pct = (hxia_forecast - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.get_forecast_start_above_percent()),
        _ => None,
    };

    debug!(
        "HXIA: {hxia:.2}, HXOR: {hxor:.2} => HXIA forecast: {hxia_forecast_raw:.2}/{hxia_forecast:.2} ({})",
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
    let hxia = tkbt - config.get_forecast_tkbt_hxia_drop();

    let adjusted_difference = (hxia - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let hxia_forecast = (hxia - expected_drop).clamp(0.0, 100.0);

    let range_width = range.get_max() - range.get_min();

    let tk_pct = (hxia_forecast - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.get_forecast_start_above_percent()),
        _ => None,
    };

    debug!(
        "TKBT: {tkbt:.2}, HXOR: {hxor:.2} => HXIA forecast: {hxia_forecast:.2} ({})",
        format_pct(tk_pct, required_pct),
    );

    Ok(tk_pct)
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use crate::brain::python_like::config::PythonBrainConfig;
    
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_none_heat_not_mixed1() -> Result<(), Sensor> {
        test_none_heat_not_mixed(Some(MixedState::MixedHeating))
    }
    #[test]
    fn test_none_heat_not_mixed2() -> Result<(), Sensor> {
        test_none_heat_not_mixed(Some(MixedState::NotMixed))
    }
    #[test]
    fn test_none_heat_not_mixed3() -> Result<(), Sensor> {
        test_none_heat_not_mixed(None)
    }
    
    fn test_none_heat_not_mixed(mixed_state: Option<MixedState>) -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 30.5);
        temps.insert(Sensor::HXIR, 30.5);
        temps.insert(Sensor::HXOR, 30.5);
        temps.insert(Sensor::TKBT, 20.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
            mixed_state,
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }

    #[test]
    fn test_none_heat_from_tank() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 25.0);
        temps.insert(Sensor::HXIR, 25.0);
        temps.insert(Sensor::HXOF, 25.0);
        temps.insert(Sensor::HXOR, 25.0);
        temps.insert(Sensor::TKBT, 60.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
            None,
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
        temps.insert(Sensor::HXOF, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
            None,
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
        temps.insert(Sensor::HXOF, 39.5);
        temps.insert(Sensor::HXOR, 39.5);
        temps.insert(Sensor::TKBT, 20.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
            None,
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
        temps.insert(Sensor::HXOF, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            None,
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

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::NotMixed),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating }, action);

        Ok(())
    }

    #[test]
    fn test_stay_in_mixed_at_high() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 39.5);
        temps.insert(Sensor::HXIR, 39.5);
        temps.insert(Sensor::HXOR, 39.5);
        temps.insert(Sensor::TKBT, 30.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating }, action);

        Ok(())
    }

    #[test]
    fn test_not_mixed_when_lower() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 35.0);
        temps.insert(Sensor::HXIR, 35.0);
        temps.insert(Sensor::HXOR, 35.0);
        temps.insert(Sensor::TKBT, 30.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::NotMixed),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }

    #[test]
    fn test_stay_in_mixed_when_lower() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 35.0);
        temps.insert(Sensor::HXIR, 35.0);
        temps.insert(Sensor::HXOR, 35.0);
        temps.insert(Sensor::TKBT, 30.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating }, action);

        Ok(())
    }

    #[test]
    fn test_exit_mixed_when_lower_still() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 31.0);
        temps.insert(Sensor::HXIR, 31.0);
        temps.insert(Sensor::HXOR, 31.0);
        temps.insert(Sensor::TKBT, 30.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }

    #[test]
    fn test_cool_using_tank_when_reach_top() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 40.5);
        temps.insert(Sensor::HXIR, 40.5);
        temps.insert(Sensor::HXOF, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 45.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            None,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: true }, action);

        Ok(())
    }

    #[test]
    fn test_heat_when_hit_bottom1() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 29.5);
        temps.insert(Sensor::HXIR, 29.5);
        temps.insert(Sensor::HXOF, 29.5);
        temps.insert(Sensor::HXOR, 29.5);
        temps.insert(Sensor::TKBT, 20.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
            Some(MixedState::MixedHeating),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }

    #[test]
    fn test_heat_when_hit_bottom2() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 29.5);
        temps.insert(Sensor::HXIR, 29.5);
        temps.insert(Sensor::HXOF, 29.5);
        temps.insert(Sensor::HXOR, 29.5);
        temps.insert(Sensor::TKBT, 20.0);

        temps.insert(Sensor::TKFL, 20.0);
        temps.insert(Sensor::HPFL, 30.0);
        temps.insert(Sensor::HPRT, 50.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
            Some(MixedState::NotMixed),
        )?;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }
}
