use log::{debug, info};
use crate::brain::BrainFailure;
use crate::brain::immersion_heater::config::ImmersionHeaterModelConfig;
use crate::brain::python_like::control::misc_control::ImmersionHeaterControl;
use crate::brain::modes::heating_mode::PossibleTemperatureContainer;
use crate::time_util::mytime::TimeProvider;

pub mod config;

pub fn follow_ih_model(time_provider: &impl TimeProvider,
                       temps: &impl PossibleTemperatureContainer,
                       immersion_heater_control: &mut dyn ImmersionHeaterControl,
                       model: &ImmersionHeaterModelConfig,
) -> Result<(), BrainFailure> {
    let currently_on = immersion_heater_control.try_get_immersion_heater()?;
    let recommendation = model.should_be_on(temps, time_provider.get_local_time().time());
    if let Some((sensor, recommend_temp)) = recommendation {
        debug!("Hope for temp {}: {:.2}, currently {:.2} at this time", sensor, recommend_temp, temps.get_sensor_temp(&sensor).copied().unwrap_or(-10000.0));
        if !currently_on {
            info!("Turning on immersion heater");
            immersion_heater_control.try_set_immersion_heater(true)?;
        }
    } else if currently_on {
        info!("Turning off immersion heater");
        immersion_heater_control.try_set_immersion_heater(false)?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use chrono::{TimeZone, Utc};
    use crate::Sensor;
    use crate::brain::immersion_heater::config::ImmersionHeaterModelPart;
    use crate::time_util::test_utils::{date, time};
    use crate::brain::python_like::control::misc_control::MiscControls;
    use crate::io::dummy::DummyAllOutputs;
    use crate::time_util::mytime::DummyTimeProvider;
    use super::*;

    #[test]
    fn check_blank_does_nothing() {
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 40.0);
        temps.insert(Sensor::TKBT, 20.0);

        let model = ImmersionHeaterModelConfig::new(vec![]);

        let mut dummy = DummyAllOutputs::default();
        let datetime = Utc.from_utc_datetime(&date(2022, 10, 03).and_time(time(02, 30, 00)));
        let time_provider = DummyTimeProvider::new(datetime);
        follow_ih_model(&time_provider, &temps, dummy.as_ih(), &model).unwrap();

        assert!(!dummy.try_get_immersion_heater().unwrap(), "Immersion heater should have been turned on.");
    }

    #[test]
    fn check_ih_model_follow() {
        let model_part = ImmersionHeaterModelPart::from_time_points(
            (time(00, 30, 00), 30.0),
            (time(04, 30, 00), 38.0),
            Sensor::TKBT,
        );
        let model = ImmersionHeaterModelConfig::new(vec![model_part]);
        let datetime = Utc.from_utc_datetime(&date(2022, 01, 18).and_time(time(02, 30, 00)));
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 40.0);
        temps.insert(Sensor::TKBT, 32.0);

        let mut dummy = DummyAllOutputs::default();
        let time_provider = DummyTimeProvider::new(datetime);

        follow_ih_model(&time_provider, &temps, dummy.as_ih(), &model).unwrap();

        assert!(dummy.try_get_immersion_heater().unwrap(), "Immersion heater should have been turned on.");
    }
}
