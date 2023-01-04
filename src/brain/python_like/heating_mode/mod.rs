use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::ops::{Add, DerefMut};
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use tokio::runtime::Runtime;
use crate::brain::{BrainFailure, CorrectiveActions};
use crate::brain::python_like::circulate_heat_pump::CirculateStatus;
use crate::brain::python_like::{cycling, FallbackWorkingRange, working_temp};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::working_temp::WorkingTemperatureRange;
use crate::{brain_fail, expect_available, HeatingControl, ImmersionHeaterControl};
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use crate::time::mytime::get_utc_time;
use crate::python_like::heatupto::HeatUpTo;
use crate::python_like::overrun_config::{OverrunConfig, TimeSlotView};
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

#[derive(Debug)]
pub struct TargetTemperature {
    sensor: Sensor,
    temp: f32,
}

impl TargetTemperature {
    pub fn new(sensor: Sensor, temp: f32) -> Self {
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
    last_successful_contact: Instant,
    fallback_working_range: FallbackWorkingRange,
    pub immersion_heater_on: bool,
    entered_state: Instant,
    last_wiser_state: bool,
}

impl SharedData {
    pub fn new(working_range: FallbackWorkingRange) -> Self {
        Self {
            last_successful_contact: Instant::now(),
            fallback_working_range: working_range,
            immersion_heater_on: false,
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
const TURNING_ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, false);
const ON_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, true);
const PRE_CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, false);
const CIRCULATE_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(false, true);
const HEAT_UP_TO_ENTRY_PREFERENCE: EntryPreferences = EntryPreferences::new(true, false);

const MIN_CIRCULATION_TEMP: f32 = 30.0;
const RELEASE_HEAT_FIRST_BELOW: f32 = 0.5;
const MIN_ON_RUNTIME: Duration = Duration::from_secs(6*60);

impl HeatingMode {

    fn get_temperatures_fn(temp_manager: &dyn TemperatureManager, runtime: &Runtime) -> Result<HashMap<Sensor, f32>, String>{
        let temps = temp_manager.retrieve_temperatures();
        let temps = runtime.block_on(temps);
        if temps.is_err() {
            eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
        }
        temps
    }

    fn get_working_temp_fn(fallback: &mut FallbackWorkingRange, wiser: &dyn WiserManager, config: &OverrunConfig, runtime: &Runtime) -> (WorkingTemperatureRange, Option<f32>) {
        working_temp::get_working_temperature_range_from_wiser_and_overrun(fallback,
                                                                           Self::get_wiser_data(wiser, runtime),
                                                                           config,
                                                                           get_utc_time())
    }

    fn get_wiser_data(wiser: &dyn WiserManager, rt: &Runtime) -> Result<WiserData, RetrieveDataError> {
        let wiser_data = rt.block_on(wiser.get_wiser_hub().get_data());
        if wiser_data.is_err() {
            eprintln!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
        }
        wiser_data
    }

    pub fn update(&mut self, shared_data: &mut SharedData, runtime: &Runtime,
                           config: &PythonBrainConfig, io_bundle: &mut IOBundle) -> Result<Option<HeatingMode>, BrainFailure> {
        fn heating_on_mode() -> Result<Option<HeatingMode>, BrainFailure> {
            return Ok(Some(HeatingMode::On(HeatingOnStatus::default())));
        }

        let heating_on_result = runtime.block_on(io_bundle.wiser().get_heating_on());

        // The wiser hub often doesn't respond. If this happens, carry on heating for a maximum of 1 hour.
        if heating_on_result.is_ok() {
            shared_data.last_successful_contact = Instant::now();
            let new = heating_on_result.unwrap();
            if shared_data.last_wiser_state != new {
                shared_data.last_wiser_state = new;
                println!("Wiser heating state changed to {}", if new {"On"} else {"Off"});
            }
        }

        let heating_on = heating_on_result.unwrap_or_else(|_e| {
            eprintln!("Wiser failed to provide whether the heating was on. Making our own guess.");
            if Instant::now() - shared_data.last_successful_contact > Duration::from_secs(60 * 60) {
                return false;
            }
            match self {
                HeatingMode::Off => false,
                HeatingMode::TurningOn(_) => true,
                HeatingMode::On(_) => true,
                HeatingMode::PreCirculate(_) => false,
                HeatingMode::Circulate(_) => true,
                HeatingMode::HeatUpTo(_) => false,
            }
        });

        let get_temperatures = || {
            Self::get_temperatures_fn(io_bundle.temperature_manager(), &runtime)
        };

        let mut get_working_temp = || {
            Self::get_working_temp_fn(&mut shared_data.fallback_working_range, io_bundle.wiser(), config.get_overrun_during(), &runtime)
        };

        match self {
            HeatingMode::Off => {
                let temps = get_temperatures();
                if let Err(err) = temps {
                    eprintln!("Failed to retrieve temperatures {}. Not Switching on.", err);
                    return Ok(None);
                }
                let temps = temps.unwrap();

                if !heating_on {
                    // Make sure even if the wiser doesn't come on, that we heat up to a reasonable temperature overnight.
                    let heatupto = get_heatup_while_off(get_utc_time(), &config.get_overrun_during(), &temps);
                    return Ok(heatupto);
                }

                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    let (max_heating_hot_water, dist) = get_working_temp();
                    if should_circulate(*temp, &max_heating_hot_water)
                        || (*temp > max_heating_hot_water.get_min() && dist.is_some() && dist.unwrap() < RELEASE_HEAT_FIRST_BELOW) {
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                    return Ok(Some(HeatingMode::TurningOn(Instant::now())));
                } else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
                }
            }
            HeatingMode::TurningOn(started) => {
                if !heating_on {
                    println!("Wiser turned off before waiting time period ended");
                    return Ok(Some(HeatingMode::Off));
                }
                if started.elapsed() > *config.get_hp_enable_time() {
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

                if !heating_on {
                    if let Some(mode) = get_overrun(get_utc_time(), &config.get_overrun_during(), &temps) {
                        println!("Overunning!.....");
                        return Ok(Some(mode));
                    }
                    let running_for = shared_data.get_entered_state().elapsed();
                    if running_for < MIN_ON_RUNTIME {
                        eprintln!("Warning: Carrying on until the 6 minute mark or 50C at the top.");
                        let remaining = MIN_ON_RUNTIME - running_for;
                        let end = get_utc_time().add(chrono::Duration::from_std(remaining).unwrap());
                        return Ok(Some(HeatingMode::HeatUpTo(HeatUpTo::from_time(TargetTemperature::new(Sensor::TKBT, 50.0), end))));
                    }
                    return Ok(Some(HeatingMode::Off));
                }

                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    println!("TKBT: {:.2}", temp);

                    let working_temp = get_working_temp().0;

                    if *temp > working_temp.get_max() {
                        return Ok(Some(HeatingMode::PreCirculate(Instant::now())));
                    }
                } else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures while on. Turning off. Returned sensors: {:?}", temps);
                    return Ok(Some(HeatingMode::Off));
                }
                if !&status.circulation_pump_on {
                    if let Some(temp) = temps.get(&Sensor::HPRT) {
                        if *temp > MIN_CIRCULATION_TEMP {
                            println!("Reached min circulation temp.");
                            let gpio = expect_available!(io_bundle.heating_control())?;
                            gpio.try_set_heat_circulation_pump(true)?;
                            status.circulation_pump_on = true;
                        }
                    }
                }
            }
            HeatingMode::PreCirculate(started) => {
                if !heating_on {
                    return Ok(Some(HeatingMode::Off));
                }
                // TODO: Check working range each time.

                if &started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
                    let temps = get_temperatures();
                    if temps.is_err() {
                        eprintln!("Failed to get temperatures, sleeping more and will keep checking.");
                        return Ok(None);
                    }
                    let temps = temps.unwrap();
                    if let Some(temp) = temps.get(&Sensor::TKBT) {
                        return if should_circulate(*temp, &get_working_temp().0) {
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
                match status {
                    CirculateStatus::Uninitialised => {
                        if !heating_on {
                            return Ok(Some(HeatingMode::Off));
                        }

                        let dispatched_gpio = io_bundle.dispatch_heating_control()
                            .map_err(|_| brain_fail!("Failed to dispatch gpio into circulation task", CorrectiveActions::unknown_heating()))?;
                        let task = cycling::start_task(runtime, dispatched_gpio, config.get_hp_circulation_config().clone());
                        *status = CirculateStatus::Active(task);
                        eprintln!("Had to initialise CirculateStatus during update.");
                        return Ok(None);
                    }
                    CirculateStatus::Active(_) => {
                        let mut stop_cycling = || {
                            let old_status = std::mem::replace(status, CirculateStatus::Uninitialised);
                            if let CirculateStatus::Active(active) = old_status {
                                *status = CirculateStatus::Stopping(active.terminate_soon(false));
                                Ok(())
                            } else {
                                return Err(brain_fail!("We just checked and it was active, so it should still be!", CorrectiveActions::unknown_heating()));
                            }
                        };

                        if !heating_on {
                            stop_cycling()?;
                            return Ok(None);
                        }

                        let temps = get_temperatures();
                        if let Err(err) = temps {
                            eprintln!("Failed to retrieve temperatures {}. Stopping cycling.", err);
                            stop_cycling()?;
                            return Ok(None);
                        }
                        let temps = temps.unwrap();

                        if let Some(temp) = temps.get(&Sensor::TKBT) {
                            println!("TKBT: {:.2}", temp);
                            if *temp < get_working_temp().0.get_min() {
                                stop_cycling()?;
                                return Ok(None);
                            }
                        }
                    }
                    CirculateStatus::Stopping(status) => {
                        if status.check_ready() {
                            let gpio = io_bundle.heating_control().rob_or_get_now()
                                .map_err(|_| brain_fail!("Couldn't retrieve control of gpio after cycling (in stopping update)", CorrectiveActions::unknown_heating()))?;
                            let left_on = gpio.try_get_heat_pump()?;

                            let temps = Self::get_temperatures_fn(io_bundle.temperature_manager(), &runtime);
                            if let Err(err) = temps {
                                eprintln!("Failed to retrieve temperatures {}. Turning off.", err);
                                return Ok(Some(HeatingMode::Off));
                            }
                            let temps = temps.unwrap();

                            if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                                println!("TKBT: {:.2}", tkbt);

                                return match (heating_on, left_on) {
                                    (true, true) => {
                                        heating_on_mode()
                                    }
                                    (false, true) => {
                                        if let Some(mode) = get_overrun(get_utc_time(), &config.get_overrun_during(), &temps) {
                                            return Ok(Some(mode));
                                        }
                                        Ok(Some(HeatingMode::Off))
                                    }
                                    (true, false) => {
                                        let working_temp = Self::get_working_temp_fn(&mut shared_data.fallback_working_range,
                                                                                     io_bundle.wiser(),
                                                                                     config.get_overrun_during(),
                                                                                        &runtime);
                                        if *tkbt < working_temp.0.get_min() {
                                            return Ok(Some(HeatingMode::PreCirculate(Instant::now())));
                                        }
                                        Ok(Some(HeatingMode::TurningOn(Instant::now())))
                                    }
                                    (false, false) => {
                                        Ok(Some(HeatingMode::Off))
                                    }
                                }
                            }
                            else {
                                eprintln!("No TKBT Temp!");
                                return Ok(Some(HeatingMode::Off));
                            }
                        } else if status.sent_terminate_request_time().elapsed() > Duration::from_secs(2) {
                            return Err(brain_fail!(format!("Didn't get back gpio from cycling task (Elapsed: {:?})", status.sent_terminate_request_time().elapsed()), CorrectiveActions::unknown_heating()));
                        }
                    }
                }
            }
            HeatingMode::HeatUpTo(target) => {
                if heating_on {
                    return heating_on_mode();
                }
                if target.has_expired(get_utc_time()) {
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = get_temperatures();
                if temps.is_err() {
                    eprintln!("Temperatures not available, stopping overrun {}", temps.unwrap_err());
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = temps.unwrap();
                println!("Target {:?} ({})", target.get_target(), target.get_expiry());
                if let Some(temp) = temps.get(target.get_target().get_target_sensor()) {
                    println!("{}: {:.2}", target.get_target().get_target_sensor(), temp);
                    if *temp > target.get_target().get_target_temp() {
                        println!("Reached target overrun temp.");
                        let next_overrun = get_overrun(get_utc_time(), &config.get_overrun_during(), &temps);
                        if next_overrun.is_some() {
                            println!("Another overrun to do before turning off");
                            return Ok(next_overrun);
                        }
                        return Ok(Some(HeatingMode::Off));
                    }
                } else {
                    eprintln!("Sensor {} targeted by overrun didn't have a temperature associated.", target.get_target().get_target_sensor());
                    return Ok(Some(HeatingMode::Off));
                }
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

        match &self {
            HeatingMode::Off => {}
            HeatingMode::TurningOn(_) => {
                let gpio = expect_available!(io_bundle.heating_control())?;
                if gpio.try_get_heat_pump()? {
                    eprintln!("Warning: Heat pump was already on when we entered TurningOn state - This is almost certainly a bug.");
                }
                else {
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

        let turn_off_hp_if_needed = |gpio: &mut dyn HeatingControl| {
            if !next_heating_mode.get_entry_preferences().allow_heat_pump_on {
                if gpio.try_get_heat_pump()? {
                    return gpio.try_set_heat_pump(false);
                }
            }
            Ok(())
        };

        let turn_off_circulation_pump_if_needed = |gpio: &mut dyn HeatingControl| {
            if !next_heating_mode.get_entry_preferences().allow_circulation_pump_on {
                if gpio.try_get_heat_circulation_pump()? {
                    return gpio.try_set_heat_circulation_pump(false);
                }
            }
            Ok(())
        };

        let turn_off_immersion_heater = |control: &mut dyn ImmersionHeaterControl| {
            if control.try_get_immersion_heater()? {
                return control.try_set_immersion_heater(false);
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

                if matches!(next_heating_mode, HeatingMode::Circulate(_) | HeatingMode::PreCirculate(_)) {
                    if io_bundle.misc_controls().try_get_immersion_heater()? {
                        println!("In circulate/precirculate but immersion heater on - turning off");
                        return io_bundle.misc_controls().try_set_immersion_heater(false);
                    }
                }
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
                Err(()) => Err(brain_fail!("Dispatchable was not available", CorrectiveActions::unknown_heating())),
                Ok(ok) => Ok(ok),
            }
        }
    }
}

fn expect_available_fn<T: ?Sized>(dispatchable: &mut Dispatchable<Box<T>>) -> Result<&mut T, ()> {
    if let Dispatchable::Available(available) = dispatchable {
        return Ok(available.deref_mut().borrow_mut());
    }
    return Err(());
}

fn get_overrun(datetime: DateTime<Utc>, config: &OverrunConfig, temps: &impl PossibleTemperatureContainer) -> Option<HeatingMode> {
    let view = get_overrun_temps(datetime, &config);
    if let Some(matching) = view.find_matching(temps) {
        return Some(HeatingMode::HeatUpTo(HeatUpTo::from_slot(TargetTemperature::new(matching.get_sensor().clone(), matching.get_temp()), matching.get_slot().clone())));
    }
    None
}

fn get_heatup_while_off(datetime: DateTime<Utc>, config: &OverrunConfig, temps: &impl PossibleTemperatureContainer) -> Option<HeatingMode> {
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

pub fn get_overrun_temps(datetime: DateTime<Utc>, config: &OverrunConfig) -> TimeSlotView {
    get_heatupto_temps(datetime, config, true)
}

pub fn get_heatupto_temps(datetime: DateTime<Utc>, config: &OverrunConfig, already_on: bool) -> TimeSlotView {
    config.get_current_slots(datetime, already_on)
}

fn should_circulate(tkbt: f32, range: &WorkingTemperatureRange) -> bool {
    println!("TKBT: {:.2}", tkbt);

    return tkbt > range.get_max();
}