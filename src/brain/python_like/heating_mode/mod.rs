use std::borrow::BorrowMut;
use std::time::{Duration, Instant};
use chrono::{DateTime, Local, LocalResult, NaiveDateTime, NaiveTime, Timelike, TimeZone, Utc};
use futures::executor::enter;
use futures::FutureExt;
use tokio::runtime::Runtime;
use crate::brain::{BrainFailure, CorrectiveActions, python_like};
use crate::brain::python_like::circulate_heat_pump::{CirculateHeatPumpOnlyTaskHandle, CirculateStatus};
use crate::brain::python_like::{cycling, FallbackWorkingRange, HEAT_CIRCULATION_PUMP, HEAT_PUMP_RELAY, PythonBrainConfig};
use crate::io::gpio::{GPIOManager, GPIOState};
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use crate::mytime::{get_local_time, get_utc_time};

#[cfg(test)]
mod test;

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
}

#[derive(Debug)]
pub struct HeatUpTo {
    target: TargetTemperature,
    expire: DateTime<Utc>,
}

impl HeatUpTo {
    pub fn new(target: TargetTemperature, expire: DateTime<Utc>) -> Self {
        Self {
            target,
            expire,
        }
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
}

impl SharedData {
    pub fn new(working_range: FallbackWorkingRange) -> Self {
        Self {
            last_successful_contact: Instant::now(),
            fallback_working_range: working_range,
            immersion_heater_on: false,
        }
    }
}

#[derive(Debug)]
pub struct HeatingOnStatus {
    circulation_pump_on: bool
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
    On(HeatingOnStatus),
    Circulate(CirculateStatus),
    HeatUpTo(HeatUpTo),
}

const OFF_ENTRY_PREFERENCE:             EntryPreferences = EntryPreferences::new(false, false);
const ON_ENTRY_PREFERENCE:              EntryPreferences = EntryPreferences::new(true, true);
const CIRCULATE_ENTRY_PREFERENCE:       EntryPreferences = EntryPreferences::new(false, true);
const HEAT_UP_TO_ENTRY_PREFERENCE:      EntryPreferences = EntryPreferences::new(true, false);

const MIN_CIRCULATION_TEMP: f32 = 30.0;

impl HeatingMode {
    pub fn update<T,G,W>(&mut self, shared_data: &mut SharedData, runtime: &Runtime,
                         config: &PythonBrainConfig, io_bundle: &mut IOBundle<T, G, W>) -> Result<Option<HeatingMode>, BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: GPIOManager + Send + 'static {

        fn heating_on_mode() -> Result<Option<HeatingMode>, BrainFailure> {
            return Ok(Some(HeatingMode::On(HeatingOnStatus::default())));
        }

        let heating_on_result = runtime.block_on(io_bundle.wiser().get_heating_on());

        // The wiser hub often doesn't respond. If this happens, carry on heating for a maximum of 1 hour.
        if heating_on_result.is_ok() {
            shared_data.last_successful_contact = Instant::now();
        }

        let heating_on = heating_on_result.unwrap_or_else(|e| {
            eprintln!("Wiser failed to provide whether the heating was on. Making our own guess.");
            if Instant::now() - shared_data.last_successful_contact > Duration::from_secs(60*60) {
                return false;
            }
            match self {
                HeatingMode::Off => false,
                HeatingMode::On(_) => true,
                HeatingMode::Circulate(_) => true,
                HeatingMode::HeatUpTo(_) => true,
            }
        });

        let get_wiser_data = |wiser: &W| {
            let wiser_data = runtime.block_on(wiser.get_wiser_hub().get_data());
            if wiser_data.is_err() {
                println!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
            }
            wiser_data
        };

        let get_temperatures = || {
            let temps = io_bundle.temperature_manager().retrieve_temperatures();
            let temps = runtime.block_on(temps);
            if temps.is_err() {
                println!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
            }
            temps
        };

        let mut get_working_temp = || {
            python_like::get_working_temperature_range_from_wiser_data(&mut shared_data.fallback_working_range, get_wiser_data(io_bundle.wiser()))
        };

        match self {
            HeatingMode::Off => {
                if !heating_on {
                    return Ok(None);
                }
                let temps = get_temperatures();
                if let Err(err) = temps {
                    eprintln!("Failed to retrieve temperatures {}. Not Switching on.", err);
                    return Ok(None);
                }
                let temps = temps.unwrap();
                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    let max_heating_hot_water = get_working_temp();
                    if *temp > max_heating_hot_water.get_max() {
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                    return heating_on_mode();
                }
                else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
                }
            }
            HeatingMode::On(status) => {
                if !heating_on {
                    if let Some(mode) = get_overrun(get_local_time(), config) {
                        println!("Overunning!.....");
                        return Ok(Some(mode));
                    }
                    return Ok(Some(HeatingMode::Off));
                }

                let temps = get_temperatures();
                if let Err(err) = temps {
                    eprintln!("Failed to retrieve temperatures {}. Turning off.", err);
                    return Ok(Some(HeatingMode::Off)); // TODO: A bit more tolerance here, although i don't think its ever been an issue.
                }
                let temps = temps.unwrap();

                if let Some(temp) = temps.get(&Sensor::TKBT)  {
                    if *temp > get_working_temp().get_max() {
                        return Ok(Some(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
                    }
                } else {
                    eprintln!("No TKBT returned when we tried to retrieve temperatures while on. Turning off. Returned sensors: {:?}", temps);
                    return Ok(Some(HeatingMode::Off))
                }
                if !&status.circulation_pump_on {
                    if let Some(temp) = temps.get(&Sensor::HPRT) {
                        if *temp > MIN_CIRCULATION_TEMP {
                            let gpio = expect_gpio_available(io_bundle.gpio())?;
                            gpio.set_pin(python_like::HEAT_CIRCULATION_PUMP, &GPIOState::LOW)
                                .map_err(|_| BrainFailure::new("Failed to turn on heat circulation pump".to_owned(), CorrectiveActions::unknown_gpio()))?;
                            status.circulation_pump_on = true;
                        }
                    }
                }
            }
            HeatingMode::Circulate(status) => {
                match status {
                    CirculateStatus::Uninitialised => {
                        if !heating_on {
                            return Ok(Some(HeatingMode::Off));
                        }

                        let dispatched_gpio = io_bundle.dispatch_gpio()
                            .map_err(|_| BrainFailure::new("Failed to dispatch gpio into circulation task".to_owned(), CorrectiveActions::unknown_gpio()))?;
                        let task = cycling::start_task(runtime, dispatched_gpio, config.clone(), config.initial_heat_pump_cycling_sleep);
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
                            }
                            else {
                                return Err(BrainFailure::new("We just checked and it was active, so it should still be!".to_owned(), CorrectiveActions::unknown_gpio()));
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
                            if *temp < get_working_temp().get_min() {
                                stop_cycling()?;
                                return Ok(None);
                            }
                        }
                    }
                    CirculateStatus::Stopping(status) => {
                        if status.check_ready() {
                            if !heating_on {
                                if let Some(mode) = get_overrun(get_local_time(), config) {
                                    return Ok(Some(mode));
                                }
                                return Ok(Some(HeatingMode::Off));
                            }

                            let temps = get_temperatures();
                            if let Err(err) = temps {
                                eprintln!("Failed to retrieve temperatures {}. Turning off.", err);
                                return Ok(Some(HeatingMode::Off));
                            }
                            let temps = temps.unwrap();

                            if let Some(temp) = temps.get(&Sensor::TKBT) {
                                if *temp < get_working_temp().get_min() {
                                    return heating_on_mode();
                                }
                            }
                        }
                        else if *status.sent_terminate_request_time() + Duration::from_secs(2) > Instant::now() {
                            return Err(BrainFailure::new("Didn't get back gpio from cycling task".to_owned(), CorrectiveActions::unknown_gpio()));
                        }
                    }
                }
            }
            HeatingMode::HeatUpTo(target) => {
                if heating_on {
                    return heating_on_mode();
                }
                if get_utc_time() > target.expire {
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = get_temperatures();
                if temps.is_err() {
                    eprintln!("Temperatures not available, stopping overrun {}", temps.unwrap_err());
                    return Ok(Some(HeatingMode::Off));
                }
                let temps = temps.unwrap();
                if let Some(temp) = temps.get(&target.target.sensor) {
                    if *temp > target.target.get_target_temp() {
                        println!("Reached target overrun temp.");
                        return Ok(Some(HeatingMode::Off));
                    }
                }
                else {
                    eprintln!("Sensor {} targeted by overrun didn't have a temperature associated.", target.target.sensor);
                    return Ok(Some(HeatingMode::Off))
                }
            }
        };

        Ok(None)
    }

    pub fn enter<T,G,W>(&mut self, config: &PythonBrainConfig, runtime: &Runtime, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: GPIOManager + Send + 'static {

        fn ensure_turned_on<G>(gpio: &mut G, pin: usize) -> Result<(), BrainFailure>
            where G: GPIOManager + Send + 'static {
            let state = gpio.get_pin(pin)
                .map_err(|err| BrainFailure::new(format!("Failed to get pin status for pin {}", pin), CorrectiveActions::unknown_gpio()))?;
            if let GPIOState::HIGH = state {
                gpio.set_pin(pin, &GPIOState::LOW)
                    .map_err(|err| BrainFailure::new(format!("Failed to turn on pin {}", pin), CorrectiveActions::unknown_gpio()))?;
            }
            Ok(())
        }

        match &self {
            HeatingMode::Off => {}
            HeatingMode::On(_) => {
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                ensure_turned_on(gpio, HEAT_PUMP_RELAY)?;
                //ensure_turned_on(gpio, HEAT_CIRCULATION_PUMP)?;
            }
            HeatingMode::Circulate(status) => {
                if let CirculateStatus::Uninitialised = status {
                    let dispatched_gpio = io_bundle.dispatch_gpio()
                        .map_err(|_| BrainFailure::new("Failed to dispatch gpio into circulation task".to_owned(), CorrectiveActions::unknown_gpio()))?;
                    let task = cycling::start_task(runtime, dispatched_gpio, config.clone(), config.initial_heat_pump_cycling_sleep);
                    *self = HeatingMode::Circulate(CirculateStatus::Active(task));
                }
            }
            HeatingMode::HeatUpTo(_) => {
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                ensure_turned_on(gpio, HEAT_PUMP_RELAY)?;
            }
        }

        Ok(())
    }

    pub fn exit_to<T,G,W>(self, next_heating_mode: &HeatingMode, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: GPIOManager {

        fn turn_off_gpio_if_needed<G>(gpio: &mut G, pin: usize, next_heating_mode: &HeatingMode) -> Result<(), BrainFailure>
            where G: GPIOManager {
            let state = gpio.get_pin(pin)
                .map_err(|err| BrainFailure::new(format!("Failed to get state of pin {} {:?}", pin, err), CorrectiveActions::unknown_gpio()))?;

            if let GPIOState::LOW = state {
                gpio.set_pin(pin, &GPIOState::HIGH)
                    .map_err(|err| BrainFailure::new(format!("Failed to turn off pin {} {:?}", pin, err), CorrectiveActions::unknown_gpio()))?;
            }
            Ok(())
        }

        let turn_off_hp_if_needed = |gpio: &mut G| {
            if !next_heating_mode.get_entry_preferences().allow_heat_pump_on {
                return turn_off_gpio_if_needed(gpio, HEAT_PUMP_RELAY, next_heating_mode);
            }
            Ok(())
        };

        let turn_off_circulation_pump_if_needed = |gpio: &mut G| {
            if !next_heating_mode.get_entry_preferences().allow_circulation_pump_on {
                return turn_off_gpio_if_needed(gpio, HEAT_CIRCULATION_PUMP, next_heating_mode);
            }
            Ok(())
        };

        let turn_off_immersion_heater_if_needed = |gpio: &mut G| {
            turn_off_gpio_if_needed(gpio, python_like::IMMERSION_HEATER, next_heating_mode)
        };

        match self {
            HeatingMode::Off => {} // Off is off, nothing hot to potentially pass here.
            HeatingMode::Circulate(status) => {
                match status {
                    CirculateStatus::Uninitialised => {}
                    CirculateStatus::Active(active) => {
                        return Err(BrainFailure::new("Can't go straight from active circulating to another state".to_owned(), CorrectiveActions::unknown_gpio()));
                    }
                    CirculateStatus::Stopping(mut stopping) => {
                        if !stopping.check_ready() {
                            return Err(BrainFailure::new("Cannot change mode yet, haven't finished stopping circulating.".to_owned(), CorrectiveActions::unknown_gpio()));
                        }
                        io_bundle.gpio().rob_or_get_now()
                            .map_err(|_| BrainFailure::new("Couldn't retrieve control of gpio after cycling".to_owned(), CorrectiveActions::unknown_gpio()))?;
                    }
                }
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                turn_off_hp_if_needed(&mut gpio)?;
                turn_off_circulation_pump_if_needed(&mut gpio)?;
            }
            _ => {
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                turn_off_hp_if_needed(&mut gpio)?;
                turn_off_circulation_pump_if_needed(&mut gpio)?;

                if let HeatingMode::Circulate(_) = next_heating_mode {
                    turn_off_immersion_heater_if_needed(&mut gpio)?;
                }
            }
        }
        Ok(())
    }

    pub fn transition_to<T,G,W>(&mut self, to: HeatingMode, config: &PythonBrainConfig, rt: &Runtime, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, G: GPIOManager + std::marker::Send + 'static, W: WiserManager {
        let old = std::mem::replace(self, to);
        old.exit_to(&self, io_bundle)?;
        self.enter(config, rt, io_bundle)
    }

    pub fn get_entry_preferences(&self) -> &EntryPreferences {
        match self {
            HeatingMode::Off => &OFF_ENTRY_PREFERENCE,
            HeatingMode::On(_) => &ON_ENTRY_PREFERENCE,
            HeatingMode::Circulate(_) => &CIRCULATE_ENTRY_PREFERENCE,
            HeatingMode::HeatUpTo(_) => &HEAT_UP_TO_ENTRY_PREFERENCE,
        }
    }
}

fn expect_gpio_available<T: GPIOManager>(dispatchable: &mut Dispatchable<T>) -> Result<&mut T, BrainFailure> {
    if let Dispatchable::Available(gpio) = dispatchable {
        return Ok(&mut *gpio);
    }
    return Err(BrainFailure::new("GPIO was not available".to_owned(), CorrectiveActions::unknown_gpio()));
}

fn get_overrun(datetime: DateTime<Local>, config: &PythonBrainConfig) -> Option<HeatingMode> {
    let time = datetime.time();

    let range1 = NaiveTime::from_hms(01, 00, 00)..NaiveTime::from_hms(04, 30, 00);

    if range1.contains(&time) {
        let result = datetime.date().and_time(range1.end);
        return result.map(|expire| HeatingMode::HeatUpTo(HeatUpTo::new(TargetTemperature::new(Sensor::TKBT, 50.0), Utc::from_utc_datetime(&Utc, &expire.naive_utc()))));
    }

    let range2 = NaiveTime::from_hms(12, 00, 00)..NaiveTime::from_hms(14, 50, 00);
    if range2.contains(&time) {
        let result = datetime.date().and_time(range2.end);
        return result.map(|expire| HeatingMode::HeatUpTo(HeatUpTo::new(TargetTemperature::new(Sensor::TKBT, 46.0), Utc::from_utc_datetime(&Utc, &expire.naive_utc()))));
    }

    return None;


    /*config.overrun_during.iter().find(|range| range.contains(&time))
        .map(|range| {
            let target_temp = TargetTemperature::new(Sensor::TKBT, config.heat_up_to_during_optimal_time);

            println!("Naive localtime {:?}", range.end);
            let result = local.date().and_time(range.end);
            println!("Converted localdatetime {:?}", result);

            result.map(|expire| HeatingMode::HeatUpTo(HeatUpTo::new(target_temp, Utc::from_utc_datetime(&Utc, &expire.naive_utc()))))
        })
        .flatten()*/
}