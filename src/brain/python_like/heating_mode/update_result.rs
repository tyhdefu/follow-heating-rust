use std::collections::HashMap;
use crate::brain::BrainFailure;
use crate::python_like::circulate_heat_pump::CirculateStatus;
use crate::python_like::heating_mode;
use crate::python_like::heating_mode::{HeatingMode, PossibleTemperatureContainer};
use crate::python_like::heatupto::HeatUpTo;
use crate::Sensor;
use crate::time::mytime::get_utc_time;

pub enum UpdateResult {
    Switch(HeatingMode),
    Error(BrainFailure)
}

pub struct TempRetrievalFailure<'a> {
    sensor: &'a Sensor,
}

impl<'a> TempRetrievalFailure<'a> {
    pub fn new(sensor: &'a Sensor) -> Self {
        Self {
            sensor
        }
    }

    fn report(&self) {
        eprintln!("Failed to retrieve sensor: {}, not doing anything.", failure.sensor);
    }

    fn turn_off_and_report(&self) -> Option<UpdateResult> {
        eprintln!("Failed to retrieve sensor: {}, turning off.", &sensor);
        Some(UpdateResult::Switch(HeatingMode::Off))
    }
}

trait AbortFlowResult<T> {
    fn ok_or_report(self) -> Option<T>;

    //fn or_turn_off(&self) -> Result<T, UpdateResult>;
}

impl<'a> AbortFlowResult<&'a f32> for Result<&'a f32, TempRetrievalFailure<'a>> {
    fn ok_or_report(self) -> Option<&'a f32> {
        if let Err(failure) = &self {
            failure.report();
        }
        self.ok()
    }

    //
    /*fn or_turn_off(&self) -> Result<&f32, UpdateResult> {
        self.map_err(|failure| {
            eprintln!("Failed to retrieve sensor: {}, turning off.", failure.sensor);
            UpdateResult::Switch(HeatingMode::Off)
        })
    }*/
}

impl<'a, T> AbortFlowResult<T> for Result<T, String>
    where T: PossibleTemperatureContainer {
    fn ok_or_report(self) -> Option<T> {
        epr
    }
}

pub trait TempResultContainer {
    fn try_retrieve<'a>(&'a self, sensor: &'a Sensor) -> Result<&'a f32, TempRetrievalFailure<'a>>;
}

impl<T> TempResultContainer for T
    where T: PossibleTemperatureContainer {
    fn try_retrieve<'a>(&'a self, sensor: &'a Sensor) -> Result<&'a f32, TempRetrievalFailure<'a>> {
        match self.get_sensor_temp(sensor) {
            None => {
                Err(TempRetrievalFailure::new(sensor))
            }
            Some(temp) => {
                Ok(temp)
            }
        }
    }
}

fn example_update(heating_on: bool, temps: HashMap<Sensor, f32>) -> Option<UpdateResult> {
    let temp = temps.try_retrieve(&Sensor::TKBT).ok_or_report()?;

    let temp2 = match temps.try_retrieve(&Sensor::TKBT) {
        Ok(temp) => temp,
        Err(err) => return err.turn_off_and_report()
    };
    None
}

fn example_off_update(heating_on: bool, temps: HashMap<Sensor, f32>) -> Option<UpdateResult> {
    /*let temps = get_temperatures();
    if let Err(err) = temps {
        eprintln!("Failed to retrieve temperatures {}. Not Switching on.", err);
        return Ok(None);
    }
    let temps = temps.unwrap();*/
    let temp = temps.try_retrieve(&Sensor::TKBT).ok_or_report()?;

    if !heating_on {
        // Make sure even if the wiser doesn't come on, that we heat up to a reasonable temperature overnight.
        let target = heating_mode::get_heatupto_temp(get_utc_time(), &config, *temp, false)?;
        println!("TKBT is {:.2}, which is below the minimum for this time. Heating up to {:.2}", temp, target.0.temp);
        let mode = HeatingMode::HeatUpTo(HeatUpTo::from_slot(target.0, target.1));
        return Some(UpdateResult::Switch(mode));
    }

    let (max_heating_hot_water, dist) = get_working_temp();
    if heating_mode::should_circulate(*temp, &temps, &max_heating_hot_water, &config)
        || (*temp > max_heating_hot_water.get_min() && dist.is_some() && dist.unwrap() < heating_mode::RELEASE_HEAT_FIRST_BELOW) {
        return Some(UpdateResult::Switch(HeatingMode::Circulate(CirculateStatus::Uninitialised)));
    }
    return heating_mode::heating_on_mode();

}

#[cfg(test)]
mod test {
    fn test_ez() {

    }
}