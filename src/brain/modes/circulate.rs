use crate::brain::modes::heating_mode::expect_available_fn;
use crate::brain::modes::heating_mode::SharedData;
use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::python_like::working_temp::WorkingRange;
use crate::brain::python_like::MAX_ALLOWED_TEMPERATURE;
use crate::io::temperatures::Sensor;
use crate::time_util::mytime::TimeProvider;
use crate::{
    brain_fail, expect_available, BrainFailure, CorrectiveActions, IOBundle, PythonBrainConfig,
};
use core::option::Option::{None, Some};
use log::{error, info};
use tokio::runtime::Runtime;

use super::heating_mode::PossibleTemperatureContainer;

#[derive(Debug, PartialEq, Default)]
pub struct CirculateMode {}

impl Mode for CirculateMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.try_set_heat_pump(HeatPumpMode::DrainTank)?;
        heating.try_set_heat_circulation_pump(true)?;
        Ok(())
    }

    fn update(
        &mut self,
        _shared_data: &mut SharedData,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            return Ok(Intention::finish());
        }
        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(temps) => temps,
            Err(e) => {
                error!("Failed to retrieve temperatures: {} - Turning off.", e);
                return Ok(Intention::off_now());
            }
        };
        let range = info_cache.get_working_temp_range();
        match should_circulate_using_forecast(
            &temps,
            &range,
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
        ) {
            Ok(true) => Ok(Intention::KeepState),
            Ok(false) => {
                info!("Reached bottom of working range, ending circulation.");
                Ok(Intention::FinishMode)
            }
            Err(missing_sensor) => {
                error!(
                    "Could not check whether to circulate due to missing sensor: {} - turning off.",
                    missing_sensor
                );
                Ok(Intention::off_now())
            }
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

/// Forecasts what the Heat Exchanger temperature is likely to be soon based on the temperature of HXOR since
/// it will drop quickly if HXOR is low (and hence maybe we should go straight to On).
/// Returns the forecasted temperature, or the sensor that was missing.
pub fn should_circulate_using_forecast(
    temps: &impl PossibleTemperatureContainer,
    range: &WorkingRange,
    config: &HeatPumpCirculationConfig,
    current_circulate_state: CurrentHeatDirection,
) -> Result<bool, Sensor> {
    let hxif = temps.get_sensor_temp(&Sensor::HXIF).ok_or(Sensor::HXIF)?;
    let hxir = temps.get_sensor_temp(&Sensor::HXIR).ok_or(Sensor::HXIR)?;
    let hxor = temps.get_sensor_temp(&Sensor::HXOR).ok_or(Sensor::HXOR)?;

    let avg_hx = (hxif + hxir) / 2.0;

    let adjusted_difference = (avg_hx - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let adjusted_temp = (avg_hx - expected_drop).clamp(0.0, MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let pct = (adjusted_temp - range.get_min()) / range_width;

    let info_msg = if pct > 1.0 {
        "Above top".to_owned()
    } else if pct < 0.0 {
        "Below bottom".to_owned()
    } else {
        match current_circulate_state {
            CurrentHeatDirection::None => format!(
                "{:.0}%, initial req. {:.0}%",
                pct * 100.0,
                config.get_forecast_start_above_percent() * 100.0
            ),
            _ => format!("{:.0}%", pct * 100.0),
        }
    };

    info!(
        "Avg. HXI: {:.2}, HXOR: {:.2}, Forecast temp: {:.2} ({})",
        avg_hx, hxor, adjusted_temp, info_msg,
    );

    Ok(match current_circulate_state {
        CurrentHeatDirection::Falling => pct >= 0.0,
        CurrentHeatDirection::Climbing => pct >= 1.0,
        CurrentHeatDirection::None => pct > config.get_forecast_start_above_percent(),
    })
}
