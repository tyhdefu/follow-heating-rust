use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::python_like::working_temp::WorkingRange;
use crate::brain::python_like::MAX_ALLOWED_TEMPERATURE;
use crate::io::temperatures::Sensor;
use crate::time_util::mytime::TimeProvider;
use crate::{expect_available, BrainFailure, IOBundle, PythonBrainConfig};
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
        if heating.try_get_heat_pump()? != HeatPumpMode::DrainTank {
            heating.try_set_heat_pump(HeatPumpMode::DrainTank)?;
        }
        if !heating.try_get_heat_circulation_pump()? {
            heating.try_set_heat_circulation_pump(true)?;
        }
        Ok(())
    }

    fn update(
        &mut self,
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
        match find_working_temp_action(
            &temps,
            &range,
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
        ) {
            Ok(WorkingTempAction::Cool { circulate: true }) => Ok(Intention::KeepState),
            Ok(WorkingTempAction::Cool { circulate: false }) => {
                info!("TKBT too cold, would be heating the tank. ending circulation.");
                Ok(Intention::finish())
            }
            Ok(WorkingTempAction::Heat { allow_mixed: _ }) => {
                info!("Reached bottom of working range, ending circulation.");
                Ok(Intention::Finish)
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

/// What to do about the working temp in order to stay within the required range.
#[derive(PartialEq, Debug)]
pub enum WorkingTempAction {
    /// Heat up - we are below the top.
    Heat { allow_mixed: bool },
    /// Circulate (i.e. cool down)
    Cool { circulate: bool },
}

/// Forecasts what the Heat Exchanger temperature is likely to be soon based on the temperature of HXOR since
/// it will drop quickly if HXOR is low (and hence maybe we should go straight to On).
/// Returns the forecasted temperature, or the sensor that was missing.
pub fn find_working_temp_action(
    temps: &impl PossibleTemperatureContainer,
    range: &WorkingRange,
    config: &HeatPumpCirculationConfig,
    heat_direction: CurrentHeatDirection,
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
            hx_pct >= 1.0
                || get_tk_pct()? >= config.get_forecast_start_above_percent()
                    && get_tk_pct()? >= hx_pct
        }
    };

    if !should_cool {
        return Ok(WorkingTempAction::Heat {
            allow_mixed: hx_pct > config.mixed_forecast_above_percent(),
        });
    }

    Ok(WorkingTempAction::Cool {
        circulate: get_tk_pct()? >= hx_pct,
    })
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

    let avg_hx = (hxif + hxir) / 2.0;

    let adjusted_difference = (avg_hx - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);
    let adjusted_temp = (avg_hx - expected_drop).clamp(0.0, MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let hx_pct = (adjusted_temp - range.get_min()) / range_width;

    let info_msg = if hx_pct > 1.0 {
        "Above top".to_owned()
    } else if hx_pct < 0.0 {
        "Below bottom".to_owned()
    } else {
        match heat_direction {
            CurrentHeatDirection::None => format!(
                "{:.0}%, initial req. {:.0}%",
                hx_pct * 100.0,
                config.get_forecast_start_above_percent() * 100.0
            ),
            _ => format!("{:.0}%", hx_pct * 100.0),
        }
    };

    info!(
        "Avg. HXI: {:.2}, HXOR: {:.2}, HX Forecast temp: {:.2} ({})",
        avg_hx, hxor, adjusted_temp, info_msg,
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

    let adjusted_difference = (tkbt - hxor) - config.get_forecast_diff_offset();
    let expected_drop = adjusted_difference * config.get_forecast_diff_proportion();
    let expected_drop = expected_drop.clamp(0.0, 25.0);

    let adjusted_temp = (tkbt - expected_drop).clamp(0.0, MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let tk_pct = (adjusted_temp - range.get_min()) / range_width;

    let tk_pct_msg = if tk_pct > 1.0 {
        "Above top".to_owned()
    } else if tk_pct < 0.0 {
        "Below bottom".to_owned()
    } else {
        match heat_direction {
            CurrentHeatDirection::None => format!(
                "{:.0}% req. {:.0}%",
                tk_pct * 100.0,
                config.get_forecast_start_above_percent()
            ),
            _ => format!("{:.0}%", tk_pct * 100.0),
        }
    };

    info!(
        "TKBT: {:.2} TK Forecast for circulate: {:.2} ({})",
        tkbt, adjusted_temp, tk_pct_msg,
    );

    Ok(tk_pct)
}

#[cfg(test)]
mod test {
    use crate::brain::python_like::working_temp::WorkingTemperatureRange;

    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_none_heat_not_mixed() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 35.0);
        temps.insert(Sensor::HXIR, 35.0);
        temps.insert(Sensor::HXOR, 35.0);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: false }, action);

        Ok(())
    }

    #[test]
    fn test_none_heat_mixed() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 39.9);
        temps.insert(Sensor::HXIR, 39.9);
        temps.insert(Sensor::HXOR, 39.9);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: true }, action);

        Ok(())
    }

    #[test]
    fn test_none_heat_from_tank() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 25.0);
        temps.insert(Sensor::HXIR, 25.0);
        temps.insert(Sensor::HXOR, 25.0);
        temps.insert(Sensor::TKBT, 60.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
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
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::None,
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
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: false }, action);

        Ok(())
    }

    #[test]
    fn test_cool_using_tank_when_reach_top() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 40.5);
        temps.insert(Sensor::HXIR, 40.5);
        temps.insert(Sensor::HXOR, 40.5);
        temps.insert(Sensor::TKBT, 45.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        )?;

        assert_eq!(WorkingTempAction::Cool { circulate: true }, action);

        Ok(())
    }

    #[test]
    fn test_heat_when_hit_bottom() -> Result<(), Sensor> {
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 40.0));
        let mut temps = HashMap::new();

        temps.insert(Sensor::HXIF, 29.5);
        temps.insert(Sensor::HXIR, 29.5);
        temps.insert(Sensor::HXOR, 29.5);
        temps.insert(Sensor::TKBT, 20.0);

        let action = find_working_temp_action(
            &temps,
            &range,
            PythonBrainConfig::default().get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
        )?;

        assert_eq!(WorkingTempAction::Heat { allow_mixed: false }, action);

        Ok(())
    }
}
