use serde::Deserialize;

/// A graph of 1/-x where x is difference
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkingTempModelConfig {
    /// The maximum value that lower bound of the working range can be.
    /// (The horizontal asymptote)
    graph_max_lower_temp: f32,
    /// The y-axis stretch factor
    /// (affects the steepness of the graph)
    multiplicand: f32,
    /// The left shift of the graph,
    /// (chops of the very steep bit of 1/-x
    left_shift: f32,
    /// The maximum difference
    /// (chops of the very flat bit of the graph)
    /// Must be less than base range size
    difference_cap: f32,
    /// The base range of max - min, gets capped difference subtracted from it,
    /// causing the range to be tightened at higher temperatures
    /// Must be greater than difference cap
    base_range_size: f32,
}

impl WorkingTempModelConfig {
    pub fn get_max_lower_temp(&self) -> f32 {
        self.graph_max_lower_temp
    }

    pub fn get_multiplicand(&self) -> f32 {
        self.multiplicand
    }

    pub fn get_left_shift(&self) -> f32 {
        self.left_shift
    }

    pub fn get_difference_cap(&self) -> f32 {
        self.difference_cap
    }

    pub fn get_base_range_size(&self) -> f32 {
        self.base_range_size
    }
}

impl Default for WorkingTempModelConfig {
    fn default() -> Self {
        Self {
            graph_max_lower_temp: 53.2,
            multiplicand: 10.0,
            left_shift: 0.6,
            difference_cap: 2.5,
            base_range_size: 4.5
        }
    }
}
