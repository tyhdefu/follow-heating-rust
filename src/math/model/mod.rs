pub trait Model {
    fn get(&self, x: f32) -> f32;
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearModel {
    gradient: f32,
    y_intercept: f32,
}

impl LinearModel {

    pub fn from_points((x1, y1): (f32, f32), (x2, y2): (f32, f32)) -> Self {
        // y - y1 = m(x + x1)
        // So: y = mx - m(x1) + y1
        let delta_y = y2 - y1;
        let delta_x = x2 - x1;
        let gradient = delta_y / delta_x;
        Self {
            gradient,
            y_intercept: (gradient * -x1) + y1
        }
    }
}

impl Model for LinearModel {
    fn get(&self, x: f32) -> f32 {
        return x * self.gradient + self.y_intercept;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_linear() {
        let point1 = (12.0, 25.0);
        let point2 = (40.0, 62.0);
        let model = LinearModel::from_points(point1, point2);
        println!("Model {:?}", model);

        // Midpoint between any two points should always be on the line.
        let midpoint = point1.midpoint(&point2);
        assert_eq!(model.get(midpoint.0), midpoint.1);

        let midpoint2 = point1.midpoint(&midpoint);
        assert_eq!(model.get(midpoint2.0), midpoint2.1)
    }

    pub trait Midpoint {
        fn midpoint(&self, other: &Self) -> Self;
    }

    impl Midpoint for (f32, f32) {
        fn midpoint(&self, other: &Self) -> Self {
            ((self.0 + other.0) / 2.0, (self.1 + other.1) / 2.0)
        }
    }
}