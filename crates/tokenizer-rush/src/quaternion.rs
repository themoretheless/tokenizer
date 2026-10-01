//! Unit quaternions representing right-handed rotations.
#[derive(Clone, Debug, PartialEq)]
pub struct Quaternion([f64; 4]);
impl Quaternion {
    pub fn axis_angle(axis: [f64; 3], radians: f64) -> Option<Self> {
        let scale = axis.iter().fold(0.0_f64, |n, x| n.max(x.abs()));
        if axis.iter().any(|x| !x.is_finite()) || scale == 0. || !radians.is_finite() {
            return None;
        }
        let axis = axis.map(|x| x / scale);
        let length = axis.iter().fold(0.0_f64, |n, x| n.hypot(*x));
        let (sin, cos) = (radians / 2.).sin_cos();
        Self::unit([
            axis[0] / length * sin,
            axis[1] / length * sin,
            axis[2] / length * sin,
            cos,
        ])
    }
    fn unit(mut values: [f64; 4]) -> Option<Self> {
        let length = values.iter().fold(0.0_f64, |n, x| n.hypot(*x));
        if !length.is_finite() || length == 0. {
            return None;
        }
        for value in &mut values {
            *value /= length;
        }
        Some(Self(values))
    }
    pub fn components(&self) -> [f64; 4] {
        self.0
    }
    pub fn compose(&self, other: &Self) -> Self {
        let [x, y, z, w] = self.0;
        let [a, b, c, d] = other.0;
        Self::unit([
            w * a + x * d + y * c - z * b,
            w * b - x * c + y * d + z * a,
            w * c + x * b - y * a + z * d,
            w * d - x * a - y * b - z * c,
        ])
        .expect("product of unit quaternions is nonzero and finite")
    }
    /// Interpolate along the shortest arc, with a normalized linear limit nearby.
    pub fn slerp(&self, other: &Self, t: f64) -> Option<Self> {
        if !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let mut end = other.0;
        let mut dot: f64 = self.0.iter().zip(end).map(|(a, b)| a * b).sum();
        if dot < 0.0 {
            for component in &mut end {
                *component = -*component;
            }
            dot = -dot;
        }
        let (left, right) = if dot > 0.9995 {
            (1.0 - t, t)
        } else {
            let angle = dot.clamp(-1.0, 1.0).acos();
            (
                ((1.0 - t) * angle).sin() / angle.sin(),
                (t * angle).sin() / angle.sin(),
            )
        };
        Self::unit(std::array::from_fn(|i| self.0[i] * left + end[i] * right))
    }
    pub fn matrix(&self) -> crate::Matrix4 {
        let [x, y, z, w] = self.0;
        crate::Matrix4([
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y - z * w),
                2. * (x * z + y * w),
                0.,
            ],
            [
                2. * (x * y + z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z - x * w),
                0.,
            ],
            [
                2. * (x * z - y * w),
                2. * (y * z + x * w),
                1. - 2. * (x * x + y * y),
                0.,
            ],
            [0., 0., 0., 1.],
        ])
    }
}
