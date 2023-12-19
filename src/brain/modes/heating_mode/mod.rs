use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::time::Instant;
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use serde::Deserialize;
use tokio::runtime::Runtime;
use crate::brain::{BrainFailure, CorrectiveActions};
use crate::brain::modes::circulate::CirculateStatus;
use crate::brain::python_like::{FallbackWorkingRange, working_temp};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::working_temp::WorkingTemperatureRange;
use crate::{brain_fail, expect_available, HeatingControl};
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use crate::time_util::mytime::{RealTimeProvider, TimeProvider};
use crate::brain::modes::heat_up_to::HeatUpTo;
use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
use crate::brain::modes::off::OffMode;
use crate::brain::modes::on::OnMode;
use crate::io::wiser::hub::WiserRoomData;
use crate::python_like::config::overrun_config::{OverrunConfig, TimeSlotView};
use crate::python_like::working_temp::WorkingRange;
use crate::wiser::hub::RetrieveDataError;

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
        Self {
            sensor,
            temp,
        }
    }

    pub fn get_target_sensor(&self) -> &Sensor {
        &self.sensor
    }

    pub fn get_target_temp(&self) -> f32 {
        self.temp
    }

    pub fn try_has_reached<T: PossibleTemperatureContainer>(&self, temperature_container: &T) -> Option<bool> {
        temperature_container.get_sensor_temp(self.get_target_sensor()).map(|temp| *temp >= self.get_target_temp())
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
    TurningOn(Instant),
    /// Heat pump fully on, circulation pump also going.
    On(OnMode),
    /// Let heat dissipate slightly out of radiators before circulating
    PreCirculate(Instant),
    /// Turn the heat pump on and off in a timed and controlled manner in order to run its pump
    /// but not causing the heating (signified by the fan) to come on.
    Circulate(CirculateStatus),
    /// Heat the hot water up to a certain temperature.
    HeatUpTo(HeatUpTo),
}

const OFF_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, false);
const TURNING_ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const PRE_CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, false);
const CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, true);
const HEAT_UP_TO_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, false);

// TODO: Configurate these.
const RELEASE_HEAT_FIRST_BELOW: f32 = 0.5;

pub fn get_working_temp_fn(fallback: &mut FallbackWorkingRange,
                           wiser: &dyn WiserManager,
                           config: &PythonBrainConfig,
                           runtime: &Runtime,
                           time: &impl TimeProvider,
) -> WorkingRange {
    working_temp::get_working_temperature_range_from_wiser_and_overrun(fallback,
                                                                       get_wiser_room_data(wiser, runtime),
                                                                       config.get_overrun_during(),
                                                                       config.get_working_temp_model(),
                                                                       time.get_utc_time())
}

fn get_wiser_room_data(wiser: &dyn WiserManager, rt: &Runtime) -> Result<Vec<WiserRoomData>, RetrieveDataError> {
    let wiser_data = rt.block_on(wiser.get_wiser_hub().get_room_data());
    if wiser_data.is_err() {
        error!(target: "wiser", "Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
    }
    wiser_data
}

impl HeatingMode {
    pub(crate) fn off() -> HeatingMode {
        Self::Off(OffMode::default())
    }

    fn get_temperatures_fn(temp_manager: &dyn TemperatureManager, runtime: &Runtime) -> Result<HashMap<Sensor, f32>, String> {
        let temps = temp_manager.retrieve_temperatures();
        let temps = runtime.block_on(temps);
        if temps.is_err() {
            error!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
        }
        temps
    }

    pub fn update(&mut self, shared_data: &mut SharedData, runtime: &Runtime,
                  config: &PythonBrainConfig, io_bundle: &mut IOBundle, info_cache: &mut InfoCache, time_provider: &impl TimeProvider) -> Result<Option<HeatingMode>, BrainFailure> {
        fn heating_on_mode() -> Result<Option<HeatingMode>, BrainFailure> {
            return Ok(Some(HeatingMode::On(OnMode::default())));
        }

        let get_temperatures = || {
            Self::get_temperatures_fn(io_bundle.temperature_manager(), &runtime)
        };

        match self {
            HeatingMode::Off(mode) => {
                let intention = mode.update(shared_data, runtime, config, info_cache, io_bundle, time_provider)?;
                return handle_intention(intention, info_cache, io_bundle, config, runtime, &time_provider.get_utc_time());
            }
            HeatingMode::TurningOn(started) => {
                if !info_cache.heating_on() {
                    info!("Wiser turned off before waiting time period ended");
                    return Ok(Some(HeatingMode::off()));
                }
                let temps = get_temperatures();
                if let Err(s) = temps {
                    error!("Failed to retrieve temperatures '{}'. Cancelling TurningOn", s);
                    return Ok(Some(HeatingMode::off()));
                }
                let temps = temps.unwrap();

                if let Some(temp) = temps.get(&Sensor::HPRT) {
                    let heating_control = expect_available!(io_bundle.heating_control())?;
                    if *temp > config.get_temp_before_circulate() && !heating_control.try_get_heat_circulation_pump()? {
                        info!("Reached min circulation temperature while turning on, turning on circulation pump.");
                        heating_control.try_set_heat_circulation_pump(true)?
                    }
                }

                if started.elapsed() > *config.get_hp_enable_time() {
                    if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                        let working_temp = info_cache.get_working_temp_range();
                        if should_circulate(*tkbt, &working_temp.get_temperature_range()) {
                            info!("Aborting turn on and instead circulating.");
                            return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                        }
                    }
                    info!("Heat pump is now fully on.");
                    return heating_on_mode();
                }
            }
            HeatingMode::On(mode) => {
                let intention = mode.update(shared_data, runtime, config, info_cache, io_bundle, time_provider)?;
                return handle_intention(intention, info_cache, io_bundle, config, runtime, &time_provider.get_utc_time());
            }
            HeatingMode::PreCirculate(started) => {
                if !info_cache.heating_on() {
                    return Ok(Some(HeatingMode::off()));
                }
                let working_temp = info_cache.get_working_temp_range();
                // TODO: Check working range each time.

                if &started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
                    let temps = get_temperatures();
                    if temps.is_err() {
                        error!("Failed to get temperatures, sleeping more and will keep checking.");
                        return Ok(None);
                    }
                    let temps = temps.unwrap();
                    if let Some(temp) = temps.get(&Sensor::TKBT) {
                        return if should_circulate(*temp, &working_temp.get_temperature_range()) {
                            Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)))
                        } else {
                            info!("Conditions no longer say we should circulate, turning on fully.");
                            Ok(Some(HeatingMode::TurningOn(Instant::now())))
                        };
                    } else {
                        error!("Failed to get TKBT temperature, sleeping more and will keep checking.");
                    }
                }
            }
            HeatingMode::Circulate(status) => {
                let intention = status.update(shared_data, runtime, config, info_cache, io_bundle, time_provider)?;
                return handle_intention(intention, info_cache, io_bundle, config, runtime, &time_provider.get_utc_time());
            }
            HeatingMode::HeatUpTo(target) => {
                let intention = target.update(shared_data, runtime, config, info_cache, io_bundle, time_provider)?;
                return handle_intention(intention, info_cache, io_bundle, config, runtime, &time_provider.get_utc_time());
            }
        };

        Ok(None)
    }

    pub fn enter(&mut self, config: &PythonBrainConfig, runtime: &Runtime, io_bundle: &mut IOBundle) -> Result<(), BrainFailure> {
        fn ensure_hp_on(gpio: &mut dyn HeatingControl) -> Result<(), BrainFailure> {
            if !gpio.try_get_heat_pump()? {
                gpio.try_set_heat_pump(true)?;
            }
            Ok(())
        }

        // Check entry preferences:
        {
            let gpio = expect_available!(io_bundle.heating_control())?;
            if !self.get_entry_preferences().allow_heat_pump_on {
                if gpio.try_get_heat_pump()? {
                    warn!("Had to turn off heat pump upon entering state.");
                    gpio.try_set_heat_pump(false)?;
                }
            }
            if !self.get_entry_preferences().allow_circulation_pump_on {
                if gpio.try_get_heat_circulation_pump()? {
                    warn!("Had to turn off circulation pump upon entering state");
                    gpio.try_set_heat_circulation_pump(false)?;
                }
            }
        }


        match self {
            HeatingMode::Off(mode) => {
                mode.enter(config, runtime, io_bundle)?;
            }
            HeatingMode::TurningOn(_) => {
                let gpio = expect_available!(io_bundle.heating_control())?;
                if gpio.try_get_heat_pump()? {
                    warn!("Warning: Heat pump was already on when we entered TurningOn state - This is almost certainly a bug.");
                } else {
                    gpio.try_set_heat_pump(true)?;
                }
            }
            HeatingMode::On(mode) => mode.enter(config, runtime, io_bundle)?,
            HeatingMode::PreCirculate(_) => {
                info!("Waiting {}s before starting to circulate", config.get_hp_circulation_config().get_initial_hp_sleep().as_secs());
            }
            HeatingMode::Circulate(status) => status.enter(config, runtime, io_bundle)?,
            HeatingMode::HeatUpTo(mode) => {
                mode.enter(config, runtime, io_bundle)?;
            }
        }

        Ok(())
    }

    pub fn exit_to(self, next_heating_mode: &HeatingMode, io_bundle: &mut IOBundle) -> Result<(), BrainFailure> {
        let turn_off_hp_if_needed = |control: &mut dyn HeatingControl| {
            if !next_heating_mode.get_entry_preferences().allow_heat_pump_on
                && control.try_get_heat_pump()? {
                return control.try_set_heat_pump(false);
            }
            Ok(())
        };

        let turn_off_circulation_pump_if_needed = |control: &mut dyn HeatingControl| {
            if !next_heating_mode.get_entry_preferences().allow_circulation_pump_on
                && control.try_get_heat_circulation_pump()? {
                return control.try_set_heat_circulation_pump(false);
            }
            Ok(())
        };

        match self {
            HeatingMode::Off(_) => {} // Off is off, nothing hot to potentially pass here.
            HeatingMode::Circulate(status) => {
                match status {
                    CirculateStatus::Uninitialised => {}
                    CirculateStatus::Active(_active) => {
                        return Err(brain_fail!("Can't go straight from active circulating to another state", CorrectiveActions::unknown_heating()));
                    }
                    CirculateStatus::Stopping(mut stopping) => {
                        if !stopping.check_ready() {
                            return Err(brain_fail!("Cannot change mode yet, haven't finished stopping circulating.", CorrectiveActions::unknown_heating()));
                        }
                        io_bundle.heating_control().rob_or_get_now()
                            .map_err(|_| brain_fail!("Couldn't retrieve control of gpio after cycling", CorrectiveActions::unknown_heating()))?;
                    }
                }
                let heating_control = expect_available!(io_bundle.heating_control())?;
                turn_off_hp_if_needed(heating_control)?;
                turn_off_circulation_pump_if_needed(heating_control)?;
            }
            _ => {
                let heating_control = expect_available!(io_bundle.heating_control())?;
                turn_off_hp_if_needed(heating_control)?;
                turn_off_circulation_pump_if_needed(heating_control)?;
            }
        }
        Ok(())
    }

    pub fn transition_to(&mut self, to: HeatingMode, config: &PythonBrainConfig, rt: &Runtime, io_bundle: &mut IOBundle) -> Result<(), BrainFailure> {
        let old = std::mem::replace(self, to);
        old.exit_to(&self, io_bundle)?;
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
        }
    }
}

#[macro_export]
macro_rules! expect_available {
    ($dispatchable:expr) => {
        {
            match expect_available_fn($dispatchable) {
                None => Err(brain_fail!("Dispatchable was not available", CorrectiveActions::unknown_heating())),
                Some(x) => Ok(x),
            }
        }
    }
}

pub fn expect_available_fn<T: ?Sized>(dispatchable: &mut Dispatchable<Box<T>>) -> Option<&mut T> {
    if let Dispatchable::Available(available) = dispatchable {
        return Some(available.deref_mut().borrow_mut());
    }
    None
}

pub fn find_overrun(datetime: &DateTime<Utc>, config: &OverrunConfig, temps: &impl PossibleTemperatureContainer) -> Option<HeatingMode> {
    let view = get_overrun_temps(datetime, &config);
    debug!("Current overrun time slots: {:?}. Time: {}", view, datetime);
    if let Some(matching) = view.find_matching(temps) {
        return Some(HeatingMode::HeatUpTo(HeatUpTo::from_slot(TargetTemperature::new(matching.get_sensor().clone(), matching.get_temp()), matching.get_slot().clone())));
    }
    None
}

fn get_heatup_while_off(datetime: &DateTime<Utc>, config: &OverrunConfig, temps: &impl PossibleTemperatureContainer) -> Option<HeatingMode> {
    let view = get_heatupto_temps(datetime, config, false);
    let matching = view.find_matching(temps);
    if let Some(bap) = matching {
        if let Some(t) = temps.get_sensor_temp(bap.get_sensor()) {
            info!("{} is {:.2} which is below the minimum for this time. (From {:?})", bap.get_sensor(), t, bap);
        } else {
            error!("Failed to retrieve sensor {} from temperatures when we really should have been able to.", bap.get_sensor())
        }
        return Some(HeatingMode::HeatUpTo(HeatUpTo::from_slot(
            TargetTemperature::new(bap.get_sensor().clone(), bap.get_temp()),
            bap.get_slot().clone(),
        )));
    }
    None
}

pub fn get_overrun_temps<'a>(datetime: &DateTime<Utc>, config: &'a OverrunConfig) -> TimeSlotView<'a> {
    get_heatupto_temps(datetime, config, true)
}

pub fn get_heatupto_temps<'a>(datetime: &DateTime<Utc>, config: &'a OverrunConfig, already_on: bool) -> TimeSlotView<'a> {
    config.get_current_slots(datetime, already_on)
}

fn should_circulate(tkbt: f32, range: &WorkingTemperatureRange) -> bool {
    debug!("TKBT: {:.2}", tkbt);

    return tkbt > range.get_max();
}

pub fn handle_intention(intention: Intention, info_cache: &mut InfoCache,
                        io_bundle: &mut IOBundle,
                        config: &PythonBrainConfig, rt: &Runtime, now: &DateTime<Utc>) -> Result<Option<HeatingMode>, BrainFailure> {
    debug!("Intention: {:?}", intention);
    match intention {
        Intention::KeepState => Ok(None),
        Intention::SwitchForce(mode) => Ok(Some(mode)),
        Intention::FinishMode => {
            let heating_control = expect_available!(io_bundle.heating_control())?;
            let heating_state = info_cache.heating_state();
            let hp_on = heating_control.try_get_heat_pump()?;
            let cp_on = heating_control.try_get_heat_circulation_pump()?;
            info!("Finished mode, now figuring out where to go. HP on: {}, Wiser: {}, CP on: {}", hp_on, heating_state, cp_on);
            match (heating_state.is_on(), hp_on) {
                // WISER: ON, HP: OFF
                (true, true) => {
                    let working_temp = info_cache.get_working_temp_range();

                    let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
                        Ok(temps) => temps,
                        Err(err) => {
                            error!("Failed to get temperatures, turning off: {}", err);
                            return Ok(Some(HeatingMode::off()));
                        }
                    };

                    let tkbt = match temps.get(&Sensor::TKBT) {
                        None => {
                            error!("Failed to get tkbt, will turn off.");
                            return Ok(Some(HeatingMode::off()));
                        }
                        Some(tkbt) => tkbt,
                    };

                    if tkbt > &working_temp.get_max() {
                        debug!("TKBT above working temp max: {:.2} > {:.2}", tkbt, working_temp.get_max());
                        // Think about circulating if no overrun.
                        if let Some(overrun) = find_overrun(&now, &config.get_overrun_during(), &temps) {
                            debug!("Overrun: {:?} would apply, still go into On mode.", overrun);
                            return Ok(Some(HeatingMode::On(OnMode::new(cp_on))));
                        }
                        // TODO: Check if we're hot enough to be pre-circulating immediately.
                        return Ok(Some(HeatingMode::PreCirculate(Instant::now())));
                    }
                    return Ok(Some(HeatingMode::On(OnMode::new(cp_on))));
                }
                // WISER OFF, HP ON
                (false, true) => {
                    // Look for overrun otherwise turn off.
                    let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
                    if let Err(err) = temps {
                        error!("Failed to retrieve temperatures: '{}', turning off", err);
                        return Ok(Some(HeatingMode::off()));
                    }

                    if let Some(mode) = find_overrun(&now, &config.get_overrun_during(), &temps.unwrap()) {
                        return Ok(Some(mode));
                    }
                    Ok(Some(HeatingMode::off()))
                }
                // WISER ON, HP OFF
                (true, false) => {
                    // Turn on.
                    let working_temp = info_cache.get_working_temp_range();
                    let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
                        Err(err) => {
                            error!("Failed to retrieve temperatures: '{}', turning off", err);
                            return Ok(Some(HeatingMode::off()));
                        },
                        Ok(temps) => temps,
                    };

                    let tkbt = match temps.get_sensor_temp(&Sensor::TKBT) {
                        None => {
                            error!("Failed to retrieve get tkbt, turning off");
                            return Ok(Some(HeatingMode::off()));
                        }
                        Some(tkbt) => tkbt,
                    };

                    if should_circulate(*tkbt, working_temp.get_temperature_range()) {
                        info!("Above max working temperature (TKBT: {:.2}) so going straight to circulate", tkbt);
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }

                    if *tkbt > working_temp.get_min() && working_temp.get_room().is_some() && working_temp.get_room().unwrap().get_difference() < RELEASE_HEAT_FIRST_BELOW {
                        info!("Small amount of heating needed and above working temp minimum (TKBT: {:.2}) so going straight to circulate", tkbt);
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                    Ok(Some(HeatingMode::TurningOn(Instant::now())))
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

                    if let Some(overrun) = get_heatup_while_off(&now, &config.get_overrun_during(), &temps) {
                        debug!("Found overrun: {:?}.", overrun);
                        return Ok(Some(overrun));
                    }
                    Ok(Some(HeatingMode::off()))
                }
            }
        }
    }
}