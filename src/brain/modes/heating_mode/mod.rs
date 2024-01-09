use crate::brain::modes::circulate::{CirculateMode, WorkingTempAction};
use crate::brain::modes::heat_up_to::HeatUpTo;
use crate::brain::modes::off::OffMode;
use crate::brain::modes::on::OnMode;
use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::python_like::{working_temp, FallbackWorkingRange};
use crate::brain::BrainFailure;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::Sensor;
use crate::io::wiser::hub::WiserRoomData;
use crate::io::wiser::WiserManager;
use crate::io::IOBundle;
use crate::python_like::config::overrun_config::{OverrunConfig, TimeSlotView};
use crate::python_like::working_temp::WorkingRange;
use crate::time_util::mytime::TimeProvider;
use crate::wiser::hub::RetrieveDataError;
use crate::{expect_available, HeatingControl};
use chrono::{DateTime, Utc};
use log::{debug, error, info, trace, warn};
use serde::Deserialize;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::time::Instant;
use tokio::runtime::Runtime;

use super::circulate::{find_working_temp_action, CurrentHeatDirection};
use super::mixed::MixedMode;
use super::pre_circulate::PreCirculateMode;
use super::try_circulate::TryCirculateMode;
use super::turning_on::TurningOnMode;

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

/// Normally we opt for every state to clean up after themselves immediately,
/// but if these preferences allow passing the burden of making sure these
/// things are in the correct state, then the previous state is allowed
/// to pass a them without shutting down these things.
#[derive(Clone)]
pub struct EntryPreferences {
    allow_heat_pump_on: bool,
    allow_circulation_pump_on: bool,
}

impl EntryPreferences {
    pub const fn new(allow_heat_pump_on: bool, allow_circulation_pump_on: bool) -> Self {
        Self {
            allow_heat_pump_on,
            allow_circulation_pump_on,
        }
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
    /// Circulate for a short time in order to get a good temperature reading
    TryCirculate(TryCirculateMode),
    /// Turn off the heat pump and run through tank until we reach the bottom of the working
    /// temperature.
    Circulate(CirculateMode),
    /// Heat the hot water up to a certain temperature.
    HeatUpTo(HeatUpTo),
}

const OFF_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, false);
const TURNING_ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const PRE_CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, false);
const TRY_CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, true);
const CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const MIXED_MODE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const HEAT_UP_TO_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, false);

pub fn get_working_temp_fn(
    fallback: &mut FallbackWorkingRange,
    wiser: &dyn WiserManager,
    config: &PythonBrainConfig,
    runtime: &Runtime,
) -> WorkingRange {
    working_temp::get_working_temperature_range_from_wiser_data(
        fallback,
        get_wiser_room_data(wiser, runtime),
        config.get_working_temp_model(),
    )
}

fn get_wiser_room_data(
    wiser: &dyn WiserManager,
    rt: &Runtime,
) -> Result<Vec<WiserRoomData>, RetrieveDataError> {
    let wiser_data = rt.block_on(wiser.get_wiser_hub().get_room_data());
    if wiser_data.is_err() {
        error!(target: "wiser", "Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
    }
    wiser_data
}

impl HeatingMode {
    pub fn off() -> Self {
        HeatingMode::Off(OffMode::default())
    }

    pub fn update(
        &mut self,
        _shared_data: &mut SharedData,
        rt: &Runtime,
        config: &PythonBrainConfig,
        io_bundle: &mut IOBundle,
        info_cache: &mut InfoCache,
        time_provider: &impl TimeProvider,
    ) -> Result<Option<HeatingMode>, BrainFailure> {
        let intention = match self {
            HeatingMode::Off(mode) => {
                mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::TurningOn(mode) => {
                mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::On(mode) => {
                mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::PreCirculate(mode) => {
                mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::Circulate(status) => {
                status.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::HeatUpTo(target) => {
                target.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::Mixed(mixed_mode) => {
                mixed_mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
            HeatingMode::TryCirculate(mode) => {
                mode.update(rt, config, info_cache, io_bundle, time_provider)?
            }
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

    pub fn enter(
        &mut self,
        config: &PythonBrainConfig,
        runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        // Check entry preferences:
        {
            let gpio = expect_available!(io_bundle.heating_control())?;
            if !self.get_entry_preferences().allow_heat_pump_on
                && gpio.try_get_heat_pump()? != HeatPumpMode::Off
            {
                warn!("Had to turn off heat pump upon entering state.");
                gpio.try_set_heat_pump(HeatPumpMode::Off)?;
            }

            if !self.get_entry_preferences().allow_circulation_pump_on
                && gpio.try_get_heat_circulation_pump()?
            {
                warn!("Had to turn off circulation pump upon entering state");
                gpio.try_set_heat_circulation_pump(false)?;
            }
        }

        match self {
            HeatingMode::Off(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::TurningOn(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::On(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::PreCirculate(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::Circulate(status) => status.enter(config, runtime, io_bundle)?,
            HeatingMode::HeatUpTo(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::Mixed(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::TryCirculate(mode) => mode.enter(config, runtime, io_bundle)?,
        }

        Ok(())
    }

    pub fn exit_to(
        self,
        next_heating_mode: &HeatingMode,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let turn_off_hp_if_needed = |control: &mut dyn HeatingControl| {
            if !next_heating_mode.get_entry_preferences().allow_heat_pump_on
                && control.try_get_heat_pump()? != HeatPumpMode::Off
            {
                return control.try_set_heat_pump(HeatPumpMode::Off);
            }
            Ok(())
        };

        let turn_off_circulation_pump_if_needed = |control: &mut dyn HeatingControl| {
            if !next_heating_mode
                .get_entry_preferences()
                .allow_circulation_pump_on
                && control.try_get_heat_circulation_pump()?
            {
                return control.try_set_heat_circulation_pump(false);
            }
            Ok(())
        };

        match self {
            HeatingMode::Off(_) => {} // Off is off, nothing hot to potentially pass here.
            _ => {
                let heating_control = expect_available!(io_bundle.heating_control())?;
                turn_off_hp_if_needed(heating_control)?;
                turn_off_circulation_pump_if_needed(heating_control)?;
            }
        }
        Ok(())
    }

    pub fn transition_to(
        &mut self,
        to: HeatingMode,
        config: &PythonBrainConfig,
        rt: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let old = std::mem::replace(self, to);
        old.exit_to(self, io_bundle)?;
        self.enter(config, rt, io_bundle)
    }

    pub fn get_entry_preferences(&self) -> &EntryPreferences {
        match self {
            HeatingMode::Off(_) => &OFF_ENTRY_PREFERENCE,
            HeatingMode::TurningOn(_) => &TURNING_ON_ENTRY_PREFERENCE,
            HeatingMode::On(_) => &ON_ENTRY_PREFERENCE,
            HeatingMode::Circulate(_) => &CIRCULATE_ENTRY_PREFERENCE,
            HeatingMode::HeatUpTo(_) => &HEAT_UP_TO_ENTRY_PREFERENCE,
            HeatingMode::PreCirculate(_) => &PRE_CIRCULATE_ENTRY_PREFERENCE,
            HeatingMode::Mixed(_) => &MIXED_MODE_ENTRY_PREFERENCE,
            HeatingMode::TryCirculate(_) => &TRY_CIRCULATE_ENTRY_PREFERENCE,
        }
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

pub fn find_overrun(
    datetime: &DateTime<Utc>,
    config: &OverrunConfig,
    temps: &impl PossibleTemperatureContainer,
) -> Option<HeatingMode> {
    let view = get_overrun_temps(datetime, config);
    debug!("Current overrun time slots: {:?}. Time: {}", view, datetime);
    if let Some(matching) = view.find_matching(temps) {
        return Some(HeatingMode::HeatUpTo(HeatUpTo::from_overrun(matching)));
    }
    None
}

fn get_heatup_while_off(
    datetime: &DateTime<Utc>,
    config: &OverrunConfig,
    temps: &impl PossibleTemperatureContainer,
) -> Option<HeatingMode> {
    let view = get_heatupto_temps(datetime, config, false);
    let matching = view.find_matching(temps);
    if let Some(bap) = matching {
        if let Some(t) = temps.get_sensor_temp(bap.get_sensor()) {
            info!(
                "{} is {:.2} which is below the minimum for this time. (From {:?})",
                bap.get_sensor(),
                t,
                bap
            );
        } else {
            error!("Failed to retrieve sensor {} from temperatures when we really should have been able to.", bap.get_sensor())
        }
        return Some(HeatingMode::HeatUpTo(HeatUpTo::from_overrun(bap)));
    }
    None
}

pub fn get_overrun_temps<'a>(
    datetime: &DateTime<Utc>,
    config: &'a OverrunConfig,
) -> TimeSlotView<'a> {
    get_heatupto_temps(datetime, config, true)
}

pub fn get_heatupto_temps<'a>(
    datetime: &DateTime<Utc>,
    config: &'a OverrunConfig,
    already_on: bool,
) -> TimeSlotView<'a> {
    config.get_current_slots(datetime, already_on)
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
    let heating_state = info_cache.heating_state();
    let hp_on = heating_control.try_get_heat_pump()?.is_hp_on();
    let cp_on = heating_control.try_get_heat_circulation_pump()?;
    debug!(
        "Finished mode, now figuring out where to go. HP mode: {:?}, Wiser: {}, CP on: {}",
        hp_on, heating_state, cp_on
    );
    match (heating_state.is_on(), hp_on) {
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

            let working_temp_action = find_working_temp_action(
                &temps,
                &working_temp,
                config.get_hp_circulation_config(),
                CurrentHeatDirection::Climbing,
            );

            match working_temp_action {
                Ok(WorkingTempAction::Heat { allow_mixed }) => {
                    if allow_mixed {
                        let view = get_overrun_temps(now, config.get_overrun_during());
                        if let Some(overrun) = view.find_matching(&temps) {
                            debug!("Applicable overrun: {overrun} while heating is nearly at top of working range. Will use mixed mode.");
                            return Ok(Some(HeatingMode::Mixed(MixedMode::from_overrun(
                                overrun.clone(),
                            ))));
                        }
                    }
                    Ok(Some(HeatingMode::On(OnMode::create(cp_on))))
                }
                Ok(WorkingTempAction::Cool { circulate }) => {
                    if let Some(overrun) = find_overrun(now, config.get_overrun_during(), &temps) {
                        debug!(
                            "Overrun: {:?} would apply, going into overrun instead of circulating.",
                            overrun
                        );
                        return Ok(Some(overrun));
                    }

                    if !circulate {
                        info!("Avoiding circulate but going into pre-circulate before deciding what to do");
                        return Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::start())));
                    }

                    let hxor = match temps.get_sensor_temp(&Sensor::HXOR) {
                        Some(temp) => temp,
                        None => {
                            error!("Missing HXOR sensor - turning off");
                            return Ok(Some(HeatingMode::off()));
                        }
                    };

                    if *hxor
                        > config
                            .get_hp_circulation_config()
                            .get_pre_circulate_temp_required()
                    {
                        info!("Hot enough to pre-circulate straight away");
                        return Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::start())));
                    }

                    Ok(Some(HeatingMode::TryCirculate(TryCirculateMode::start())))
                }
                Err(missing_sensor) => {
                    error!(
                                "Could not determine whether to circulate due to missing sensor: {}. Turning off.",
                                missing_sensor
                            );
                    Ok(Some(HeatingMode::off()))
                }
            }
        }
        // WISER OFF, HP ON
        (false, true) => {
            // Look for overrun otherwise turn off.
            let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
            if let Err(err) = temps {
                error!("Failed to retrieve temperatures: '{}', turning off", err);
                return Ok(Some(HeatingMode::off()));
            }

            if let Some(mode) = find_overrun(now, config.get_overrun_during(), &temps.unwrap()) {
                return Ok(Some(mode));
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
                config.get_hp_circulation_config(),
                CurrentHeatDirection::None,
            ) {
                Ok(WorkingTempAction::Heat { allow_mixed: _ }) => {
                    info!("Call for heat: turning on");
                    Ok(Some(HeatingMode::TurningOn(TurningOnMode::new(
                        Instant::now(),
                    ))))
                }
                Ok(WorkingTempAction::Cool { circulate: true }) => {
                    info!("Circulation recommended - will try.");
                    Ok(Some(HeatingMode::TryCirculate(TryCirculateMode::new(
                        Instant::now(),
                    ))))
                }
                Ok(WorkingTempAction::Cool { circulate: false }) => {
                    info!("Idle recommended, doing pre-circulate");
                    Ok(Some(HeatingMode::PreCirculate(PreCirculateMode::start())))
                }
                Err(missing_sensor) => {
                    error!("Missing sensor: {}", missing_sensor);
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
