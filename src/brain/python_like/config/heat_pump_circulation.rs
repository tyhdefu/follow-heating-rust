use serde::Deserialize;
use serde_with::serde_as;
use serde_with::DurationSeconds;
use std::time::Duration;

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HeatPumpCirculationConfig {
    /// How long (in seconds) the heat pump should stay on for before turning off
    /// (Should be less than the time it takes for it to turn on)
    #[serde_as(as = "DurationSeconds")]
    hp_pump_on_time: Duration,
    /// How long (in seconds) the heat pump should stay off before turning back on.
    #[serde_as(as = "DurationSeconds")]
    hp_pump_off_time: Duration,

    /// How long (in seconds) to sleep after going from On -> Circulation mode, and
    /// also, how long to stay in Equalise mode before giving up.
    #[serde_as(as = "DurationSeconds")]
    initial_hp_sleep: Duration,

    /// The temperature required on HXOR to go into pre circulate rather than directly to
    /// circulate.
    pre_circulate_temp_required: f32,

    /// The amount to subtract from the difference of TKBT and HXOR as the first step.
    forecast_diff_offset: f32,
    /// The proportion of the difference between TKBT and HXOR subtract from TKBT to make the
    /// forecasted temperature.
    forecast_diff_proportion: f32,

    /// The percentage i.e 0.33 that it needs to be above the bottom when first starting.
    forecast_start_above_percent: f32,

    /// The steady-state drop between TKBT (Tank Bottom) and HXIA (Heat Exchanger Input Average)
    forecast_tkbt_hxia_drop: f32,

    /// The threshold of the forecast heat exchanger temperature needs to be in the working
    /// range in order to go into a mixed heating mode (if there is demand for hot water)
    mixed_mode: MixedModeConfig,

    /// How long to sample draining the tank to see whether it is effective.
    #[serde_as(as = "DurationSeconds")]
    sample_tank_time: Duration,
}

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct MixedModeConfig {
    pub start_heat_pct: f32,
    pub stop_heat_pct: f32,
}

impl HeatPumpCirculationConfig {
    pub fn get_initial_hp_sleep(&self) -> &Duration {
        &self.initial_hp_sleep
    }

    pub fn get_pre_circulate_temp_required(&self) -> f32 {
        self.pre_circulate_temp_required
    }

    pub fn get_forecast_diff_offset(&self) -> f32 {
        self.forecast_diff_offset
    }

    pub fn get_forecast_diff_proportion(&self) -> f32 {
        self.forecast_diff_proportion
    }

    pub fn get_forecast_start_above_percent(&self) -> f32 {
        self.forecast_start_above_percent
    }

    pub fn get_forecast_tkbt_hxia_drop(&self) -> f32 {
        self.forecast_tkbt_hxia_drop
    }
    pub fn mixed_mode(&self) -> &MixedModeConfig {
        &self.mixed_mode
    }
    pub fn sample_tank_time(&self) -> &Duration {
        &self.sample_tank_time
    }
}

impl Default for HeatPumpCirculationConfig {
    fn default() -> Self {
        Self {
            hp_pump_on_time: Duration::from_secs(70),
            hp_pump_off_time: Duration::from_secs(30),
            initial_hp_sleep: Duration::from_secs(5 * 60),
            forecast_diff_offset: 5.0,
            forecast_diff_proportion: 0.33,
            forecast_start_above_percent: 0.10,
            forecast_tkbt_hxia_drop: 3.0,
            pre_circulate_temp_required: 35.0,
            mixed_mode: MixedModeConfig {
                start_heat_pct: 0.70,
                stop_heat_pct: 0.30,
            },
            sample_tank_time: Duration::from_secs(30),
        }
    }
}
