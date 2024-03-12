use serde::Deserialize;

/// Parameters for a signmoid temperature curve
/// See https://docs.google.com/spreadsheets/d/1W-7uisntqJJfkjusxofNv68s1fr1SONU1kiOftu9RHk/edit#gid=1222591046
#[derive(Deserialize, Clone, Debug, PartialEq)]
//#[serde(deny_unknown_fields)]
pub struct WorkingTempModelConfig {
    pub min: WorkingTempCurveConfig,
    pub max: WorkingTempCurveConfig,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct WorkingTempCurveConfig {
    pub sharpness:     f32,
    pub turning_point: f32,
    pub multiplier:    f32,
    pub offset:        f32,
}

impl WorkingTempCurveConfig {
    pub fn get_temp_from_room_diff(&self, room_diff: f32) -> f32 {
        self.multiplier /
        (1.0 + (-self.sharpness * (room_diff - self.turning_point)).exp())
        + self.offset
    }
}

impl Default for WorkingTempModelConfig {
    fn default() -> Self {
        Self {
            min: WorkingTempCurveConfig {
                sharpness:     1.90,
                turning_point: 0.50,
                multiplier:    24.0,
                offset:        23.3,
            },
            max: WorkingTempCurveConfig {
                sharpness:     1.90,
                turning_point: 0.35,
                multiplier:    18.7,
                offset:        31.2,
            },
        }
    }
}

#[cfg(test)]
pub mod test {
    use super::WorkingTempModelConfig;

    #[test]
    fn get_temp_from_room_diff() {
        let model = get_working_temp_model_test_data().min;

        assert_eq!((model.get_temp_from_room_diff(0.0) * 100.0) as u32, 2999);
        assert_eq!((model.get_temp_from_room_diff(1.0) * 100.0) as u32, 4060);
    }

    pub fn get_working_temp_model_test_data() -> WorkingTempModelConfig {
        WorkingTempModelConfig::default()
    }
}
