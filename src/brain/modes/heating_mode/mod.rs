use crate::brain::modes::circulate::CirculateMode;
use crate::brain::modes::dhw_only::DhwOnlyMode;
use crate::brain::modes::off::OffMode;
use crate::brain::modes::on::OnMode;
use crate::brain::modes::working_temp::{
    find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState,
};
use crate::brain::modes::equalise::EqualiseMode;
use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::python_like::FallbackWorkingRange;
use crate::brain::BrainFailure;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::Sensor;
use crate::io::wiser::hub::WiserRoomData;
use crate::io::wiser::WiserManager;
use crate::io::IOBundle;
use crate::python_like::config::overrun_config::OverrunConfig;
use crate::time_util::mytime::TimeProvider;
use crate::wiser::hub::RetrieveDataError;
use chrono::{DateTime, Utc};
use log::{debug, error, info, trace, warn};
use serde::Deserialize;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::time::{Instant, Duration};
use tokio::runtime::Runtime;

use super::mixed::MixedMode;
use super::pre_circulate::PreCirculateMode;
use super::try_circulate::TryCirculateMode;
use super::turning_on::TurningOnMode;
use super::working_temp::{self, WorkingRange};

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test;

pub trait PossibleTemperatureContainer {
    fn get_sensor_temp(&self, sensor: &Sensor) -> Option<&f32>;
}

impl PossibleTemperatureContainer for HashMap<Sensor, f32> {
    fn get_sensor_temp(&self, sensor: &Sensor) -> Option<&f32> {
        self.get(sensor)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TargetTemperature {
    sensor: Sensor,
    temp: f32,
}

impl TargetTemperature {
    pub const fn new(sensor: Sensor, temp: f32) -> Self {
        Self { sensor, temp }
    }

    pub fn get_target_sensor(&self) -> &Sensor {
        &self.sensor
    }

    pub fn get_target_temp(&self) -> f32 {
        self.temp
    }
}

impl Display for TargetTemperature {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.temp, self.sensor)
    }
}

/// Data that is used shared between multiple states.
pub struct SharedData {
    pub last_successful_contact: Instant,
    pub fallback_working_range: FallbackWorkingRange,
    pub entered_state: Instant,
    pub last_wiser_state: HeatingState,
}

impl SharedData {
    pub fn new(working_range: FallbackWorkingRange) -> Self {
        Self {
            last_successful_contact: Instant::now(),
            fallback_working_range: working_range,
            entered_state: Instant::now(),
            last_wiser_state: HeatingState::OFF,
        }
    }

    pub fn notify_entered_state(&mut self) {
        self.entered_state = Instant::now();
    }

    pub fn get_entered_state(&self) -> Instant {
        self.entered_state
    }

    pub fn get_fallback_working_range(&mut self) -> &mut FallbackWorkingRange {
        &mut self.fallback_working_range
    }
}

#[derive(Debug, PartialEq)]
pub enum HeatingMode {
    /// Everything off
    Off(OffMode),
    /// Heat pump turning on, pump going but no heating is happening.
    TurningOn(TurningOnMode),
    /// Heat pump fully on, circulation pump also going.
    On(OnMode),
    /// Both heating and hot water.
    Mixed(MixedMode),
    /// First step in chain PreCirculate -> TryCirculate -> Circulate
    /// Let heat dissipate slightly out of radiators before circulating
    PreCirculate(PreCirculateMode),
    /// Circulate the heating but not the tank to get a good reading
    Equalise(EqualiseMode),
    /// Circulate for a short time in order to get a good temperature reading
    TryCirculate(TryCirculateMode),
    /// Turn off the heat pump and run through tank until we reach the bottom of the working
    /// temperature.
    Circulate(CirculateMode),
    /// Heat the hot water up to a certain temperature.
    DhwOnly(DhwOnlyMode),
}

pub fn get_working_temp_fn(
    fallback: &mut FallbackWorkingRange,
    wiser: &dyn WiserManager,
    config: &PythonBrainConfig,
    runtime: &Runtime,
) -> WorkingRange {
    working_temp::get_working_temperature_range_from_wiser_data(
        fallback,
        get_wiser_room_data(wiser, runtime),
        &config.working_temp_model,
    )
}

fn get_wiser_room_data(
    wiser: &dyn WiserManager,
    rt: &Runtime,
) -> Result<Vec<WiserRoomData>, RetrieveDataError> {
    let wiser_data = rt.block_on(wiser.get_wiser_hub().get_room_data());
    if wiser_data.is_err() {
        warn!(target: "wiser", "Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
    }
    wiser_data
}

impl HeatingMode {
    pub fn off() -> Self {
        HeatingMode::Off(OffMode::default())
    }

    pub fn update(
        &mut self,
        _shared_data:  &mut SharedData,
        rt:            &Runtime,
        config:        &PythonBrainConfig,
        io_bundle:     &mut IOBundle,
        info_cache:    &mut InfoCache,
        time_provider: &impl TimeProvider,
    ) -> Result<Option<HeatingMode>, BrainFailure> {
        let intention = match self {
            HeatingMode::Off(mode)          => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::TurningOn(mode)    => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::On(mode)           => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::PreCirculate(mode) => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::Equalise(mode)     => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::Circulate(mode)    => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::DhwOnly(mode)      => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::Mixed(mode)        => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
            HeatingMode::TryCirculate(mode) => mode.update(rt, config, info_cache, io_bundle, time_provider)?,
        };

        handle_intention(
            intention,
            info_cache,
            io_bundle,
            config,
            rt,
            &time_provider.get_utc_time(),
        )
    }

    pub fn transition_to(
        &mut self,
        _from:     &Option<HeatingMode>,
        config:    &PythonBrainConfig,
        runtime:   &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        // Could check for exit actions here...

        match self {
            HeatingMode::Off(mode)          => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::TurningOn(mode)    => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::On(mode)           => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::Equalise(mode)     => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::PreCirculate(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::Circulate(mode)    => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::DhwOnly(mode)      => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::Mixed(mode)        => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::TryCirculate(mode) => mode.enter(config, runtime, io_bundle)?,
        }

        Ok(())
    }
}

#[macro_export]
macro_rules! expect_available {
    ($dispatchable:expr) => {{
        match $crate::brain::modes::heating_mode::expect_available_fn($dispatchable) {
            None => Err($crate::brain_fail!(
                "Dispatchable was not available",
                $crate::brain::CorrectiveActions::unknown_heating()
            )),
            Some(x) => Ok(x),
        }
    }};
}

pub fn expect_available_fn<T: ?Sized>(dispatchable: &mut Dispatchable<Box<T>>) -> Option<&mut T> {
    if let Dispatchable::Available(available) = dispatchable {
        return Some(available.deref_mut().borrow_mut());
    }
    None
}

fn get_heatup_while_off(
    datetime: &DateTime<Utc>,
    config: &OverrunConfig,
    temps: &impl PossibleTemperatureContainer,
) -> Option<HeatingMode> {
    let slot = config.find_matching_slot(datetime, temps, |temps, temp| temp <= temps.min && temp < temps.max);
    if let Some(bap) = slot {
        if let Some(t) = temps.get_sensor_temp(&bap.temps.sensor) {
            info!(
                "{} is {:.2} which is below the minimum for this time. (From {:?})",
                bap.temps.sensor,
                t,
                bap
            );
        } else {
            error!("Failed to retrieve sensor {} from temperatures when we really should have been able to.", bap.temps.sensor)
        }
        return Some(HeatingMode::DhwOnly(DhwOnlyMode::new()));
    }
    None
}

pub fn handle_intention(
    intention: Intention,
    info_cache: &mut InfoCache,
    io_bundle: &mut IOBundle,
    config: &PythonBrainConfig,
    rt: &Runtime,
    now: &DateTime<Utc>,
) -> Result<Option<HeatingMode>, BrainFailure> {
    trace!("Intention: {:?}", intention);
    match intention {
        Intention::KeepState => Ok(None),
        Intention::SwitchForce(mode) => {
            debug!("Force switching to mode: {:?}", mode);
            Ok(Some(mode))
        }
        Intention::Finish => handle_finish_mode(info_cache, io_bundle, config, rt, now),
        Intention::YieldHeatUps => {
            // Check for heat ups.
            let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
                Ok(temps) => temps,
                Err(e) => {
                    error!("Failed to get temperatures to check for overruns: {}, but might be ok in the current mode, not changing.", e);
                    return Ok(None);
                }
            };
            Ok(get_heatup_while_off(
                now,
                config.get_overrun_during(),
                &temps,
            ))
        }
    }
}

pub fn handle_finish_mode(
    info_cache: &mut InfoCache,
    io_bundle: &mut IOBundle,
    config: &PythonBrainConfig,
    rt: &Runtime,
    now: &DateTime<Utc>,
) -> Result<Option<HeatingMode>, BrainFailure> {
    let heating_control = expect_available!(io_bundle.heating_control())?;
    let wiser_state = info_cache.heating_state();
    let (hp_on, hp_duration) = heating_control.get_heat_pump_on_with_time()?;
    let cp_on = heating_control.get_circulation_pump()?.0;
    debug!(
        "Finished mode. HP on: {:?}, Wiser: {}, CP on: {}",
        hp_on, wiser_state, cp_on
    );
    match (wiser_state.is_on(), hp_on) {
        // WISER: ON, HP: ON
        (true, true) => {
            let working_temp = info_cache.get_working_temp_range();

            let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
                Ok(temps) => temps,
                Err(err) => {
                    error!("Failed to get temperatures, turning off: {}", err);
                    return Ok(Some(HeatingMode::off()));
                }
            };

            if let Some(heatupto) = get_heatup_while_off(now, config.get_overrun_during(), &temps) {
                info!("Below minimum for a HeatUpTo, entering despite wiser calling for heat.");
                return Ok(Some(heatupto));
            }

            let mixed_mode = match expect_available!(io_bundle.heating_control())?.try_get_heat_pump()? {
                HeatPumpMode::BoostedHeating => Some(MixedState::BoostedHeating),
                HeatPumpMode::MostlyHotWater => Some(MixedState::MixedHeating),
                HeatPumpMode::HeatingOnly    => Some(MixedState::NotMixed),
                _ => None
            };

            let working_temp_action = find_working_temp_action(
                &temps,
                &working_temp,
                &config,
                CurrentHeatDirection::Climbing,
                mixed_mode,
                None,
                hp_duration
            );

            let heating_mode = match working_temp_action {
                Ok((_, WorkingTempAction::Heat { mixed_state })) => {
                    if matches!(mixed_state, MixedState::MixedHeating) {
                        // Use "extra" when considering MixedMode
                        let slot = config.get_overrun_during().find_matching_slot(now, &temps,
                            |temps, temp| temp < temps.extra.unwrap_or(temps.max));
                        if let Some(overrun) = slot {
                            debug!("Applicable overrun: {overrun} while heating is nearly at top of working range. Will use mixed mode.");
                            return Ok(Some(HeatingMode::Mixed(MixedMode::new())));
                        }
                    }
                    Ok(Some(HeatingMode::On(OnMode::create(cp_on))))
                }
                Ok((heating_mode, WorkingTempAction::Cool { circulate })) => {
                    let slot = config.get_overrun_during().find_matching_slot(now, &temps,
                        |temps, temp| temp < temps.max);
                    if let Some(slot) = slot {
                        debug!("Overrun: {slot:?} would apply, going into overrun instead of circulating.");
                        return Ok(Some(HeatingMode::DhwOnly(DhwOnlyMode::new())));
                    }

                    if !circulate {
                        info!("Avoiding circulate but going into pre-circulate before deciding what to do");
                        if let pre_circulate @ Some(HeatingMode::PreCirculate(_)) = heating_mode {
                            return Ok(pre_circulate);
                        }
                        else { 
                            warn!("Legacy code path");
                            return Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::new(config.hp_circulation.initial_hp_sleep))));
                        }
                    }

                    let hxor = match temps.get_sensor_temp(&Sensor::HXOR) {
                        Some(temp) => temp,
                        None => {
                            error!("Missing HXOR sensor - turning off");
                            return Ok(Some(HeatingMode::off()));
                        }
                    };

                    if *hxor > config.hp_circulation.pre_circulate_temp_required
                    {
                        info!("Hot enough to pre-circulate straight away");
                        if let pre_circulate @ Some(HeatingMode::PreCirculate(_)) = heating_mode {
                            return Ok(pre_circulate);
                        }
                        else { 
                            warn!("Legacy code path");
                            return Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::new(config.hp_circulation.initial_hp_sleep))));
                        }
                    }

                    Ok(Some(HeatingMode::TryCirculate(TryCirculateMode::start())))
                }
                Err(missing_sensor) => {
                    error!("Could not determine whether to circulate due to missing sensor: {missing_sensor}. Turning off.");
                    Ok(Some(HeatingMode::off()))
                }
            };
            heating_mode
        }
        // WISER OFF, HP ON
        (false, true) => {
            // Look for overrun otherwise turn off.
            let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
            if let Err(err) = temps {
                error!("Failed to retrieve temperatures: '{}', turning off", err);
                return Ok(Some(HeatingMode::off()));
            }

            let slot = config.get_overrun_during().find_matching_slot(now, &temps.unwrap(),
                |temps, temp| temp < temps.max || (hp_duration < Duration::from_secs(60 * 10) && temp < temps.extra.unwrap_or(temps.max))
            );
            if let Some(_) = slot {
                return Ok(Some(HeatingMode::DhwOnly(DhwOnlyMode::new())));
            }
            Ok(Some(HeatingMode::off()))
        }
        // WISER ON, HP OFF
        (true, false) => {
            let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
            if let Err(err) = temps {
                error!("Failed to retrieve temperatures: {}, staying off", err);
                return Ok(Some(HeatingMode::off()));
            }
            match find_working_temp_action(
                &temps.unwrap(),
                &info_cache.get_working_temp_range(),
                &config,
                CurrentHeatDirection::None,
                None, None,
                hp_duration,
            ) {
                Ok((_, WorkingTempAction::Heat { .. })) => {
                    info!("Call for heat: turning on");
                    Ok(Some(HeatingMode::TurningOn(TurningOnMode::new(
                        Instant::now(),
                    ))))
                }
                Ok((_, WorkingTempAction::Cool { circulate: true })) => {
                    info!("Circulation recommended - will try.");
                    Ok(Some(HeatingMode::TryCirculate(TryCirculateMode::new(
                        Instant::now(),
                    ))))
                }
                Ok((heating_mode, WorkingTempAction::Cool { circulate: false })) => {
                    info!("TKBT too cold, would be heating the tank. Idle recommended, doing pre-circulate");
                    if let pre_circulate @ Some(HeatingMode::PreCirculate(_)) = heating_mode {
                        Ok(pre_circulate)
                    }
                    else { 
                        warn!("Legacy code path");
                        Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::new(config.hp_circulation.initial_hp_sleep))))
                    }
                }
                Err(missing_sensor) => {
                    error!("Missing sensor: {missing_sensor}");
                    Ok(Some(HeatingMode::off()))
                }
            }
        }
        // WISER OFF, HP OFF
        (false, false) => {
            // Check if should go into HeatUpTo.
            let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
                Ok(temps) => temps,
                Err(err) => {
                    error!("Failed to get temperatures, turning off: {}", err);
                    return Ok(Some(HeatingMode::off()));
                }
            };

            if let Some(overrun) = get_heatup_while_off(now, config.get_overrun_during(), &temps) {
                debug!("Found overrun: {:?}.", overrun);
                return Ok(Some(overrun));
            }
            Ok(Some(HeatingMode::off()))
        }
    }
}
