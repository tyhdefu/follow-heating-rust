use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::{Add, DerefMut, Sub};
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::runtime::Runtime;
use crate::brain::{BrainFailure, CorrectiveActions};
use crate::brain::python_like::modes::circulate::CirculateStatus;
use crate::brain::python_like::{cycling, FallbackWorkingRange, working_temp};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::working_temp::WorkingTemperatureRange;
use crate::{brain_fail, expect_available, HeatingControl};
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use crate::time::mytime::{RealTimeProvider, TimeProvider};
use crate::brain::python_like::modes::heat_up_to::HeatUpTo;
use crate::brain::python_like::modes::intention::ChangeState;
use crate::python_like::modes::{InfoCache, Intention, Mode};
use crate::python_like::config::overrun_config::{OverrunConfig, TimeSlotView};
use crate::python_like::working_temp::WorkingRange;
use crate::wiser::hub::{RetrieveDataError, WiserData};

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

pub struct SharedData {
    pub last_successful_contact: Instant,
    pub fallback_working_range: FallbackWorkingRange,
    pub entered_state: Instant,
    pub last_wiser_state: bool,
}

impl SharedData {
    pub fn new(working_range: FallbackWorkingRange) -> Self {
        Self {
            last_successful_contact: Instant::now(),
            fallback_working_range: working_range,
            entered_state: Instant::now(),
            last_wiser_state: false,
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

#[derive(Debug)]
pub struct HeatingOnStatus {
    circulation_pump_on: bool,
}

impl Default for HeatingOnStatus {
    fn default() -> Self {
        Self {
            circulation_pump_on: false,
        }
    }
}

#[derive(Debug)]
pub enum HeatingMode {
    Off,
    TurningOn(Instant),
    On(HeatingOnStatus),
    PreCirculate(Instant),
    Circulate(CirculateStatus),
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
                                                                       get_wiser_data(wiser, runtime),
                                                                       config.get_overrun_during(),
                                                                       config.get_working_temp_model(),
                                                                       time.get_utc_time())
}

fn get_wiser_data(wiser: &dyn WiserManager, rt: &Runtime) -> Result<WiserData, RetrieveDataError> {
    let wiser_data = rt.block_on(wiser.get_wiser_hub().get_data());
    if wiser_data.is_err() {
        eprintln!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
    }
    wiser_data
}

impl HeatingMode {
    fn get_temperatures_fn(temp_manager: &dyn TemperatureManager, runtime: &Runtime) -> Result<HashMap<Sensor, f32>, String> {
        let temps = temp_manager.retrieve_temperatures();
        let temps = runtime.block_on(temps);
        if temps.is_err() {
            eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
        }
        temps
    }

    pub fn update(&mut self, shared_data: &mut SharedData, runtime: &Runtime,
                  config: &PythonBrainConfig, io_bundle: &mut IOBundle, info_cache: &mut InfoCache) -> Result<Option<HeatingMode>, BrainFailure> {
        fn heating_on_mode() -> Result<Option<HeatingMode>, BrainFailure> {
            return Ok(Some(HeatingMode::On(HeatingOnStatus::default())));
        }

        let get_temperatures = || {
            Self::get_temperatures_fn(io_bundle.temperature_manager(), &runtime)
        };

        let time_provider = RealTimeProvider::default();

        match self {
            HeatingMode::Off => {
                let temps = get_temperatures();
                if let Err(err) = temps {
                    eprintln!("Failed to retrieve temperatures {}. Not Switching on.", err);
                    return Ok(None);
                }
                let temps = temps.unwrap();

                if !info_cache.heating_on() {
                    // Make sure even if the wiser doesn't come on, that we heat up to a reasonable temperature overnight.
                    let heatupto = get_heatup_while_off(&time_provider.get_utc_time(), &config.get_overrun_during(), &temps);
                    return Ok(heatupto);
                }

                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    let working_range = info_cache.get_working_temp_range();
                    if should_circulate(*temp, working_range.get_temperature_range()) {
                        println!("Above max working temperature (TKBT: {:.2}) so going straight to circulate", temp);
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                    if *temp > working_range.get_min() && working_range.get_room().is_some() && working_range.get_room().unwrap().get_difference() < RELEASE_HEAT_FIRST_BELOW {
                        println!("Small amount of heating needed and above working temp minimum (TKBT: {:.2}) so going straight to circulate", temp);
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                    return Ok(Some(HeatingMode::TurningOn(Instant::now())));
                } else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
                }
            }
            HeatingMode::TurningOn(started) => {
                if !info_cache.heating_on() {
                    println!("Wiser turned off before waiting time period ended");
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = get_temperatures();
                if let Err(s) = temps {
                    eprintln!("Failed to retrieve temperatures '{}'. Cancelling TurningOn", s);
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = temps.unwrap();

                if let Some(temp) = temps.get(&Sensor::HPRT) {
                    let heating_control = expect_available!(io_bundle.heating_control())?;
                    if *temp > config.get_temp_before_circulate() && !heating_control.try_get_heat_circulation_pump()? {
                        println!("Reached min circulation temperature while turning on, turning on circulation pump.");
                        heating_control.try_set_heat_circulation_pump(true)?
                    }
                }

                if started.elapsed() > *config.get_hp_enable_time() {
                    if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                        let working_temp = info_cache.get_working_temp_range();
                        if should_circulate(*tkbt, &working_temp.get_temperature_range()) {
                            println!("Aborting turn on and instead circulating.");
                            return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                        }
                    }
                    println!("Heat pump is now fully on.");
                    return heating_on_mode();
                }
            }
            HeatingMode::On(status) => {
                let temps = get_temperatures();
                if let Err(err) = temps {
                    eprintln!("Failed to retrieve temperatures {}. Turning off.", err);
                    return Ok(Some(HeatingMode::Off)); // TODO: A bit more tolerance here, although i don't think its ever been an issue.
                }
                let temps = temps.unwrap();

                if !info_cache.heating_on() {
                    if let Some(mode) = get_overrun(&time_provider.get_utc_time(), &config.get_overrun_during(), &temps) {
                        println!("Overunning!.....");
                        return Ok(Some(mode));
                    }
                    let running_for = shared_data.get_entered_state().elapsed();
                    let min_runtime = config.get_min_hp_runtime();
                    if running_for < *min_runtime.get_min_runtime() {
                        eprintln!("Warning: Carrying on until the {} second mark or safety cut off: {}", min_runtime.get_min_runtime().as_secs(), min_runtime.get_safety_cut_off());
                        let remaining = min_runtime.get_min_runtime().clone() - running_for;
                        let end = time_provider.get_utc_time().add(chrono::Duration::from_std(remaining).unwrap());
                        return Ok(Some(HeatingMode::HeatUpTo(HeatUpTo::from_time(min_runtime.get_safety_cut_off().clone(), end))));
                    }
                    return Ok(Some(HeatingMode::Off));
                }

                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    println!("TKBT: {:.2}", temp);

                    let working_temp = info_cache.get_working_temp_range();

                    if *temp > working_temp.get_max() {
                        return Ok(Some(HeatingMode::PreCirculate(Instant::now())));
                    }
                } else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures while on. Turning off. Returned sensors: {:?}", temps);
                    return Ok(Some(HeatingMode::Off));
                }
                if !&status.circulation_pump_on {
                    if let Some(temp) = temps.get(&Sensor::HPRT) {
                        if *temp > config.get_temp_before_circulate() {
                            println!("Reached min circulation temp.");
                            let gpio = expect_available!(io_bundle.heating_control())?;
                            gpio.try_set_heat_circulation_pump(true)?;
                            status.circulation_pump_on = true;
                        }
                    }
                }
            }
            HeatingMode::PreCirculate(started) => {
                if !info_cache.heating_on() {
                    return Ok(Some(HeatingMode::Off));
                }
                let working_temp = info_cache.get_working_temp_range();
                // TODO: Check working range each time.

                if &started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
                    let temps = get_temperatures();
                    if temps.is_err() {
                        eprintln!("Failed to get temperatures, sleeping more and will keep checking.");
                        return Ok(None);
                    }
                    let temps = temps.unwrap();
                    if let Some(temp) = temps.get(&Sensor::TKBT) {
                        return if should_circulate(*temp, &working_temp.get_temperature_range()) {
                            Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)))
                        } else {
                            println!("Conditions no longer say we should circulate, turning on fully.");
                            Ok(Some(HeatingMode::TurningOn(Instant::now())))
                        };
                    } else {
                        eprintln!("Failed to get TKBT temperature, sleeping more and will keep checking.");
                    }
                }
            }
            HeatingMode::Circulate(status) => {
                let intention = status.update(shared_data, runtime, config, info_cache, io_bundle, &time_provider)?;
                return handle_intention(intention, info_cache, io_bundle, config, runtime, &time_provider.get_utc_time());
            }
            HeatingMode::HeatUpTo(target) => {
                let intention = target.update(shared_data, runtime, config, info_cache, io_bundle, &time_provider)?;
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
                    println!("Had to turn off heat pump upon entering state.");
                    gpio.try_set_heat_pump(false)?;
                }
            }
            if !self.get_entry_preferences().allow_circulation_pump_on {
                if gpio.try_get_heat_circulation_pump()? {
                    println!("Had to turn off circulation pump upon entering state");
                    gpio.try_set_heat_circulation_pump(false)?;
                }
            }
        }


        match &self {
            HeatingMode::Off => {}
            HeatingMode::TurningOn(_) => {
                let gpio = expect_available!(io_bundle.heating_control())?;
                if gpio.try_get_heat_pump()? {
                    eprintln!("Warning: Heat pump was already on when we entered TurningOn state - This is almost certainly a bug.");
                } else {
                    gpio.try_set_heat_pump(true)?;
                }
            }
            HeatingMode::On(_) => {
                let gpio = expect_available!(io_bundle.heating_control())?;
                ensure_hp_on(gpio)?;
            }
            HeatingMode::PreCirculate(_) => {
                println!("Waiting {}s before starting to circulate", config.get_hp_circulation_config().get_initial_hp_sleep().as_secs());
            }
            HeatingMode::Circulate(status) => {
                if let CirculateStatus::Uninitialised = status {
                    let dispatched_gpio = io_bundle.dispatch_heating_control()
                        .map_err(|_| brain_fail!("Failed to dispatch gpio into circulation task", CorrectiveActions::unknown_heating()))?;
                    let task = cycling::start_task(runtime, dispatched_gpio, config.get_hp_circulation_config().clone());
                    *self = HeatingMode::Circulate(CirculateStatus::Active(task));
                }
            }
            HeatingMode::HeatUpTo(_) => {
                let gpio = expect_available!(io_bundle.heating_control())?;
                ensure_hp_on(gpio)?;
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
            HeatingMode::Off => {} // Off is off, nothing hot to potentially pass here.
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
            HeatingMode::Off => &OFF_ENTRY_PREFERENCE,
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

fn expect_available_fn<T: ?Sized>(dispatchable: &mut Dispatchable<Box<T>>) -> Option<&mut T> {
    if let Dispatchable::Available(available) = dispatchable {
        return Some(available.deref_mut().borrow_mut());
    }
    None
}

pub fn get_overrun(datetime: &DateTime<Utc>, config: &OverrunConfig, temps: &impl PossibleTemperatureContainer) -> Option<HeatingMode> {
    let view = get_overrun_temps(datetime, &config);
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
            println!("{} is {:.2} which is below the minimum for this time. (From {:?})", bap.get_sensor(), t, bap);
        } else {
            eprintln!("Failed to retrieve sensor {} from temperatures when we really should have been able to.", bap.get_sensor())
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
    println!("TKBT: {:.2}", tkbt);

    return tkbt > range.get_max();
}

pub fn handle_intention(intention: Intention, info_cache: &mut InfoCache,
                        io_bundle: &mut IOBundle,
                        config: &PythonBrainConfig, rt: &Runtime, now: &DateTime<Utc>) -> Result<Option<HeatingMode>, BrainFailure> {
    println!("Intention: {:?}", intention);
    match intention {
        Intention::KeepState => Ok(None),
        Intention::SwitchForce(mode) => Ok(Some(mode)),
        Intention::Change(change) => {
            match change {
                ChangeState::BeginCirculating => {
                    // TODO: Check if we should just circulate immediately.
                    Ok(Some(HeatingMode::PreCirculate(Instant::now())))
                }
            }
        }
        Intention::FinishMode => {
            let heating_control = expect_available!(io_bundle.heating_control())?;
            let heating_on = info_cache.heating_on();
            let hp_on = heating_control.try_get_heat_pump()?;
            match (heating_on, hp_on) {
                (true, true) => {
                    Ok(Some(HeatingMode::On(HeatingOnStatus::default())))
                }
                (false, true) => {
                    // Look for overrun otherwise turn off.
                    let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
                    if let Err(err) = temps {
                        eprintln!("Failed to retrieve temperatures: '{}', turning off", err);
                        return Ok(Some(HeatingMode::Off));
                    }
                    if let Some(mode) = get_overrun(&now, &config.get_overrun_during(), &temps.unwrap()) {
                        return Ok(Some(mode));
                    }
                    Ok(Some(HeatingMode::Off))
                }
                (true, false) => {
                    let working_temp = info_cache.get_working_temp_range();
                    let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
                    if let Err(err) = temps {
                        eprintln!("Failed to retrieve temperatures: '{}', turning off", err);
                        return Ok(Some(HeatingMode::Off));
                    }
                    if let Some(tkbt) = temps.unwrap().get_sensor_temp(&Sensor::TKBT) {
                        if *tkbt > working_temp.get_max() {
                            println!("TKBT: {:.2} above working temp max ({:.2})", tkbt, working_temp.get_max());
                            return Ok(Some(HeatingMode::PreCirculate(Instant::now())));
                        }
                    } else {
                        eprintln!("Failed to retrieve get tkbt, turning off");
                        return Ok(Some(HeatingMode::Off));
                    }
                    Ok(Some(HeatingMode::TurningOn(Instant::now())))
                }
                (false, false) => {
                    Ok(Some(HeatingMode::Off))
                }
            }
        }
    }
}