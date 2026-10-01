//! Row-major matrices acting on column vectors; A * B applies B first.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix4(pub(crate) [[f64; 4]; 4]);
impl Matrix4 {
    pub fn identity() -> Self {
        Self([
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn rows(&self) -> &[[f64; 4]; 4] {
        &self.0
    }
    pub(crate) fn multiply(&self, other: &Self) -> Option<Self> {
        let mut rows = [[0.; 4]; 4];
        for (i, row) in rows.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.0[i][k] * other.0[k][j]).sum();
            }
        }
        if rows.iter().flatten().all(|n| n.is_finite()) {
            Some(Self(rows))
        } else {
            None
        }
    }
    pub(crate) fn apply(&self, vector: &[f64], point: bool) -> Option<Vec<f64>> {
        if vector.len() != 3 {
            return None;
        }
        let input = [vector[0], vector[1], vector[2], if point { 1. } else { 0. }];
        let mut output = [0.; 4];
        for (i, value) in output.iter_mut().enumerate() {
            *value = self.0[i].iter().zip(input).map(|(a, b)| a * b).sum();
        }
        if !output.iter().all(|n| n.is_finite()) || (point && output[3] == 0.) {
            return None;
        }
        let divisor = if point { output[3] } else { 1. };
        let result: Vec<_> = output[..3].iter().map(|x| x / divisor).collect();
        result.iter().all(|n| n.is_finite()).then_some(result)
    }
}
