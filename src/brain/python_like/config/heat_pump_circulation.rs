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
    /// TODO: Unused?
    #[serde_as(as = "DurationSeconds")]
    pub hp_pump_on_time: Duration,
    /// How long (in seconds) the heat pump should stay off before turning back on.
    /// TODO: Unused?
    #[serde_as(as = "DurationSeconds")]
    pub hp_pump_off_time: Duration,

    /// How long (in seconds) to sleep after going from On -> Circulation mode, and
    /// also, how long to stay in Equalise mode before giving up.
    #[serde_as(as = "DurationSeconds")]
    pub initial_hp_sleep: Duration,

    /// The temperature required on HXOR to go into pre circulate rather than directly to
    /// circulate.
    pub pre_circulate_temp_required: f32,

    /// The amount to subtract from the difference of TKBT and HXOR as the first step.
    pub forecast_diff_offset: f32,
    /// The proportion of the difference between TKBT and HXOR subtract from TKBT to make the
    /// forecasted temperature.
    pub forecast_diff_proportion: f32,

    /// The percentage i.e 0.33 that it needs to be above the bottom when first starting.
    pub forecast_start_above_percent: f32,

    /// The steady-state drop between TKBT (Tank Bottom) and HXIA (Heat Exchanger Input Average)
    pub forecast_tkbt_hxia_drop: f32,

    /// The threshold of the forecast heat exchanger temperature needs to be in the working
    /// range in order to go into a mixed heating mode (if there is demand for hot water)
    pub mixed_mode: MixedModeConfig,

    /// When to enter boost mode whereby the heat pump is on and the heating is boosted
    /// by taking heat from the hot water tank
    pub boost_mode: BoostModeConfig,

    /// How long to sample draining the tank to see whether it is effective.
    #[serde_as(as = "DurationSeconds")]
    pub sample_tank_time: Duration,
}

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct MixedModeConfig {
    pub start_heat_pct: f32,
    pub stop_heat_pct:  f32,
}

#[serde_as]
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct BoostModeConfig {
    /// The maximum percentage within the heating range to start boosting
    /// Lower than this the rooms will heat up sufficiently quickly without boosting
    pub start_heat_pct:       f32,

    /// The maximum percentage within the heating range to continue boosting once started
    /// If too close to start_heat_pct then this will result in excessive valve movement
    /// to little effect.
    pub stop_heat_pct:        f32,

    /// The minimum difference between the TKFL and HPFL to start boosting
    /// Lower than this would achieve very little.
    pub start_tkfl_hpfl_diff: f32,

    /// The minimum difference between the TKFL and HPFL to continue boosting once started
    /// If too close to start_tkfl_hpfl_diff then this will result in excessive valve movement
    /// to little effect.
    pub stop_tkfl_hpfl_diff:  f32,

    /// The minimum difference between the configured minimum tank temperature for the current
    /// slot to start boosting
    pub start_slot_min_diff: f32,

    /// The minimum difference between the configured minimum tank temperature for the current
    /// slot to continue boosting once started
    /// If too close to start_slot_min_diff then this will result in excessive valve movement
    /// to little effect.
    pub stop_slot_min_diff:  f32,
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
            boost_mode: BoostModeConfig {
                start_heat_pct:       0.00,
                stop_heat_pct:        0.10,
                start_tkfl_hpfl_diff: 2.0,
                stop_tkfl_hpfl_diff:  1.0,
                start_slot_min_diff:  3.5,
                stop_slot_min_diff:   1.5,
            },
            sample_tank_time: Duration::from_secs(30),
        }
    }
}
