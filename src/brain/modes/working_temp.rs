use crate::brain::modes::heating_mode::HeatingMode;
use crate::brain::modes::pre_circulate::PreCirculateMode;
use crate::brain::{modes::heating_mode::PossibleTemperatureContainer, python_like::config::overrun_config::DhwBap};
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::config::working_temp_model::WorkingTempModelConfig;
use crate::io::temperatures::Sensor;
use crate::io::wiser::hub::WiserRoomData;
use crate::python_like::FallbackWorkingRange;
use crate::wiser::hub::RetrieveDataError;
use log::{debug, error, info};
use serde::Deserialize;
use std::fmt::{Debug, Display, Formatter};
use std::time::Duration;

const UNKNOWN_ROOM: &str = "Unknown";


// TODO: Make these hardcoded limits configurable in WorkingTempModelConfig.
// 17/1/2026 measured the HP external temperature sensor moving:
// 52 => 53 at HPRT 50.1
// 53 => 54 at HPRT 51.2 (just before 51.3) (during slow warmup) 
// But later say 53 => 54 at 52.2 during fast warmup - I suspect a loag
const HARD_HPFL_LIMIT: f32 = 59.4;
const HARD_HPRT_LIMIT: f32 = 52.1;

#[derive(Clone)]
pub struct WorkingRange {
    temp_range: WorkingTemperatureRange,
    room: Option<Room>,
}

impl WorkingRange {
    pub fn new(temp_range: WorkingTemperatureRange, room: Option<Room>) -> Self {
        Self { temp_range, room }
    }

    pub fn get_min(&self) -> f32 {
        self.temp_range.min
    }

    pub fn get_max(&self) -> f32 {
        self.temp_range.max
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
    name:              String,
    set_point:         f32,
    difference:        f32,
    capped_difference: f32,
}

impl Room {
    pub fn from_wiser(room: &WiserRoomData, capped_difference: f32) -> Self {
        Self {
            name:       room.get_name().unwrap_or("UNKNOWN").to_string(),
            set_point:  room.get_set_point(),
            difference: room.get_set_point().min(MAX_ROOM_TEMP) - room.get_temperature(),
            capped_difference
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
    #[cfg(test)]
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
// The range produced by this also capped, but the difference here is that the range
// will be lower / wider if e.g. current temp = 20.5 and target = 25
const MAX_ROOM_TEMP: f32 = 21.0;

pub fn get_working_temperature_range_from_wiser_data(
    fallback:     &mut FallbackWorkingRange,
    wiser_result: Result<Vec<WiserRoomData>, RetrieveDataError>,
    config:       &WorkingTempModelConfig,
) -> WorkingRange {
    match wiser_result {
        Ok(wiser_data) => {
            let most_heating_required = wiser_data
                .iter()
                .filter(|room| room.get_temperature() > -10.0) // Something causes low values (Low battery maybe)
                .map(|room| {
                    (
                        room,
                        room.get_set_point().min(MAX_ROOM_TEMP) - room.get_temperature(),
                    )
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
            ;

            if let Some((room, difference)) = most_heating_required {
                let range = WorkingTemperatureRange::from_min_max(
                    config.min.get_temp_from_room_diff(difference).clamp(0.0, HARD_HPRT_LIMIT - 4.0),
                    config.max.get_temp_from_room_diff(difference).clamp(5.0, HARD_HPRT_LIMIT)
                );

                debug!("Using {most_heating_required:?}");

                let result = WorkingRange::new(range, Some(Room::from_wiser(&room, difference)));
                fallback.update(&result);
                result
            }
            else {
                error!(target: "wiser", "Bad data detected: no rooms with sensible temperatures - using dummy:\n{wiser_data:?}");
                fallback.get_fallback(Duration::from_secs(20*60)).clone()
            }
        }
        Err(err) => {
            error!(target: "wiser", "Error getting data from wiser - using fallback:\n{err:?}");
            fallback.get_fallback(Duration::from_secs(30*60)).clone()
        }
    }
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
    /// Cool down OR heat up a little!!!
    Cool {
        /// Circulate from the tank to the radiators
        circulate: bool
    },
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
/// Returns what it things should happen next (which the caller must test for being a valid state transition),
/// PLUS a legacy indication of what action should be taken (or the sensor that was missing in case of error).
pub fn find_working_temp_action(
    temps:          &impl PossibleTemperatureContainer,
    range:          &WorkingRange,
    config:         &HeatPumpCirculationConfig,
    heat_direction: CurrentHeatDirection,
    mixed_state:    Option<MixedState>,
    dhw_slot:       Option<&DhwBap>,
    hp_duration:    Duration,
) -> Result<(Option<HeatingMode>, WorkingTempAction), Sensor> {
    let hx_pct = forecast_hx_pct(temps, config, &heat_direction, range)?;

    // Only cause 1 log if needed.
    let mut tk_pct_cached = None;
    let mut get_tk_pct = || -> Result<f32, Sensor> {
        if tk_pct_cached.is_none() {
            tk_pct_cached = Some(forecast_tk_pct(temps, config, &heat_direction, range)?);
        }
        Ok(tk_pct_cached.unwrap())
    };

    let lower_threshold = if hp_duration > Duration::from_secs(10*60) { 0.0 } else { -0.1 };
    let upper_threshold = if hp_duration > Duration::from_secs(40*60) {
        0.9
    }
    else if hp_duration > Duration::from_secs(16*60) {
        1.0
    }  
    else {
        1.2
    };

    let should_cool = match heat_direction {
        CurrentHeatDirection::Falling  => hx_pct >= lower_threshold,
        CurrentHeatDirection::Climbing => hx_pct >= upper_threshold
                                          // Backstop to catch low flow (HPFL high) or HPRT extension due to short duration
                                          || *temps.get_sensor_temp(&Sensor::HPFL).ok_or(Sensor::HPFL)? > HARD_HPFL_LIMIT
                                          || *temps.get_sensor_temp(&Sensor::HPRT).ok_or(Sensor::HPRT)? > HARD_HPRT_LIMIT,
        CurrentHeatDirection::None => {
            let tk_pct = get_tk_pct()?;

            // Happy to circulate first
            let hx_above_req = hx_pct >= config.forecast_start_above_percent;
            // Happy to drain from tank first
            let tk_above_req = tk_pct >= config.forecast_start_above_percent;

            hx_above_req || tk_above_req
        }
    };

    let (required_pct, used_tk) = match heat_direction {
        CurrentHeatDirection::None => (Some(config.forecast_start_above_percent), true),
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

    if should_cool {
        let tkbt = temps.get_sensor_temp(&Sensor::TKBT).ok_or(Sensor::TKBT)?;
        let hxof = temps.get_sensor_temp(&Sensor::HXOF).ok_or(Sensor::HXOF)?;
        // TODO: Not right: Should circulate only if hx_pct < 10 or so??
        // Also, temp check below should be handled by forecast_tk_pct
        let circulate = tkbt > hxof && (dhw_slot.is_none() || *tkbt > dhw_slot.unwrap().temps.min + 5.0);
        debug!("Considering might circulate. TKBT={tkbt}, HXOF={hxof}, dhw_slot={dhw_slot:?}, circulate={circulate}");
        if hx_pct < lower_threshold {
            Ok((None, WorkingTempAction::Heat { mixed_state: get_mixed_state(temps, config, mixed_state, hx_pct, dhw_slot, range)? }))
        }
        else {
            let rest_for = (hx_pct - lower_threshold) as f64 / (upper_threshold - lower_threshold) as f64;
            let rest_for = Duration::from_secs(
                ( (config.initial_hp_sleep.as_secs() as f64 * rest_for) as i64 - 20 as i64 ) // Reduce for equalise time
                .clamp(10, config.initial_hp_sleep.as_secs() as i64) as u64                  // Min pre-circulate time
            );
            // TODO: Equalise mode if less than 40 seconds or so
            Ok((Some(HeatingMode::PreCirculate(PreCirculateMode::new(rest_for))), WorkingTempAction::Cool { circulate: false }))
        }
    }
    else {
        Ok((None, WorkingTempAction::Heat { mixed_state: get_mixed_state(temps, config, mixed_state, hx_pct, dhw_slot, range)? }))
    }
}

fn get_mixed_state(
    temps:            &impl PossibleTemperatureContainer,
    config:           &HeatPumpCirculationConfig,
    mixed_state:      Option<MixedState>,
    hx_pct:           f32,
    dhw_slot:         Option<&DhwBap>,
    ch_working_range: &WorkingRange,
) -> Result<MixedState, Sensor> {
    if let Some(mixed_state) = mixed_state {
        if let Some(dhw_slot) = dhw_slot
            && let Some(room) = &ch_working_range.room
            && room.set_point >= 25.0
        {
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

            let temp = temps.get_sensor_temp(&dhw_slot.temps.sensor).ok_or(dhw_slot.temps.sensor.clone())?;
            let slot_margin = *temp - dhw_slot.temps.min;

            match mixed_state {
                MixedState::BoostedHeating => {
                    if hx_pct       <  config.boost_mode.stop_heat_pct &&
                       tkfl - hpfl  >= config.boost_mode.stop_tkfl_hpfl_diff &&
                       (slot_margin >  config.boost_mode.stop_slot_min_diff || room.set_point >= 30.0) {
                            return Ok(MixedState::BoostedHeating)
                    }
                }
                MixedState::NotMixed | MixedState::MixedHeating => {
                    if hx_pct       <  config.boost_mode.start_heat_pct &&
                       tkfl - hpfl  >= config.boost_mode.start_tkfl_hpfl_diff &&
                       (slot_margin >  config.boost_mode.start_slot_min_diff || room.set_point >= 30.0) {
                            return Ok(MixedState::BoostedHeating)
                    }
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
    if pct > 1.3 {
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
    
    let adjusted_difference = (hxia - hxor) - config.forecast_diff_offset;
    let expected_drop = adjusted_difference * config.forecast_diff_proportion;
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let hxia_forecast_raw = hxia - expected_drop;

    let hxia_forecast = merge_hprt_into_fhxia(hxia_forecast_raw, *hprt);
    
    let range_width = range.get_max() - range.get_min();

    let hx_pct = (hxia_forecast - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.forecast_start_above_percent),
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
    let hxia = tkbt - config.forecast_tkbt_hxia_drop;

    let adjusted_difference = (hxia - hxor) - config.forecast_diff_offset;
    let expected_drop = adjusted_difference * config.forecast_diff_proportion;
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let hxia_forecast = (hxia - expected_drop).clamp(0.0, 100.0);

    let range_width = range.get_max() - range.get_min();

    let tk_pct = (hxia_forecast - range.get_min()) / range_width;

    let required_pct = match heat_direction {
        CurrentHeatDirection::None => Some(config.forecast_start_above_percent),
        _ => None,
    };

    debug!(
        "TKBT: {tkbt:.2}, HXOR: {hxor:.2} => HXIA forecast: {hxia_forecast:.2} ({})",
        format_pct(tk_pct, required_pct),
    );

    Ok(tk_pct)
}

/// Gradually switch from using forecast HXI to using HPRT as temperatures get higher
/// This may result in higher or lower circulation temps, but either way it aligns the
/// top end of the range with the hard heatpump limit of 55deg HPRT.
fn merge_hprt_into_fhxia(fhxia: f32, hprt: f32) -> f32 {
    const HPRT_LO_LIMIT: f32 = HARD_HPRT_LIMIT - 3.5;
    const HPRT_HI_LIMIT: f32 = HARD_HPRT_LIMIT;

    // Either variables could be lower, but as either approach a maximum of 55 more emphasis needs
    // to be given to HPRT as ultimately this is what will cut off the heat pump. Also may be switching
    // from fhxia to hxoa at some point as a measure of output rather than effort.
    //let lower = fhxia.min(hprt);

    if hprt > HPRT_HI_LIMIT {
        hprt
    }
    else {
        let pct_hprt = ((hprt.max(hprt) - HPRT_LO_LIMIT) / (HPRT_HI_LIMIT - HPRT_LO_LIMIT)).clamp(0.0, 1.0);
        fhxia*(1.0-pct_hprt) + hprt*pct_hprt
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use crate::brain::python_like::config::PythonBrainConfig;
    
    use super::*;
    use std::{collections::HashMap, ops::Range};

    #[test]
    fn test_merge_hprt_into_fhxia_basic() {
        assert_eq!(merge_hprt_into_fhxia(50.0, 50.0), 50.0);
        assert_eq!(merge_hprt_into_fhxia(50.0, 55.0), 55.0);
        assert_eq!(merge_hprt_into_fhxia(40.0, 55.0), 55.0);
        assert_eq!(merge_hprt_into_fhxia(00.0, 55.0), 55.0);
    }

    #[test]
    fn test_merge_hprt_into_fhxia1() {
        // Must not bring fHXIA down at the top
        assert_range_float(merge_hprt_into_fhxia(15.0, 54.9), 53.9..55.3)
    }

    #[test]
    fn test_merge_hprt_into_fhxia2() {
        // Can only go a little over fHXIA near the top even in extreme circumstances
        assert_range_float(merge_hprt_into_fhxia(70.0, 54.9), 54.9..55.3);
    }

    #[test]
    fn test_merge_hprt_into_fhxia3() {
        assert_range_float(merge_hprt_into_fhxia(15.0, 56.0), 56.0..57.0);
    }

    #[test]
    fn test_merge_hprt_into_fhxia4() {
        assert_range_float(merge_hprt_into_fhxia(54.66, 52.1), 52.1..54.0);
    }

    fn assert_range_float<T>(value: T, range: Range<T>)
    where T: num_traits::Float + Display {
        if !value.is_zero() && !value.is_normal() {
            panic!("Abnormal number {}", value);
        }
        if !(range.start <= value && value < range.end) {
            panic!("Violation of {} <= {} < {}", range.start, value, range.end);
        }
    }

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::None,
            mixed_state,
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::None,
            None, None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::None,
            None, None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::None,
            None, None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            None, None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::NotMixed),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::NotMixed),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Climbing,
            None, None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Falling,
            Some(MixedState::MixedHeating),
            None,
        )?.1;

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
            &PythonBrainConfig::default().hp_circulation,
            CurrentHeatDirection::Falling,
            Some(MixedState::NotMixed),
            None,
        )?.1;

        assert_eq!(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }, action);

        Ok(())
    }
}
