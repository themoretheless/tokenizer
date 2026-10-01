//! Portable 2D polygon values. Coordinates use a mathematical upward Y axis.
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    points: Vec<[f64; 2]>,
}
impl Polygon {
    pub fn new(points: Vec<[f64; 2]>) -> Result<Self, &'static str> {
        if points.len() < 3 || points.iter().flatten().any(|x| !x.is_finite()) {
            return Err("Polygon requires at least three finite 2D points");
        }
        Ok(Self { points })
    }
    pub fn points(&self) -> &[[f64; 2]] {
        &self.points
    }
    pub fn translated(&self, offset: [f64; 2]) -> Result<Self, &'static str> {
        Self::new(
            self.points
                .iter()
                .map(|p| [p[0] + offset[0], p[1] + offset[1]])
                .collect(),
        )
    }
    pub fn rotated(&self, radians: f64) -> Result<Self, &'static str> {
        let (sin, cos) = radians.sin_cos();
        Self::new(
            self.points
                .iter()
                .map(|p| [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos])
                .collect(),
        )
    }
    /// Export a filled polygon with a padded viewBox. Self-intersections use even-odd fill.
    pub fn to_svg(&self) -> Result<String, &'static str> {
        let mut svg = String::new();
        self.write_svg(&mut svg)?;
        Ok(svg)
    }
    /// Stream SVG into a caller-owned writer, stopping on its first error.
    /// Bounds are validated before writing any output.
    pub fn write_svg(&self, svg: &mut impl Write) -> Result<(), &'static str> {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for p in &self.points {
            for axis in 0..2 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
        let extent = (high[0] - low[0]).max(high[1] - low[1]);
        let padding = (extent * 0.05).max(1.0);
        let width = high[0] - low[0] + 2.0 * padding;
        let height = high[1] - low[1] + 2.0 * padding;
        let x = low[0] - padding;
        let y = -high[1] - padding;
        if ![width, height, x, y].iter().all(|n| n.is_finite()) {
            return Err("SVG bounds overflow");
        }
        write!(svg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x} {y} {width} {height}\">\n<polygon fill=\"#5b6ee1\" fill-rule=\"evenodd\" points=\""
        ).map_err(|_| "SVG output write failed")?;
        for p in &self.points {
            write!(svg, "{},{} ", p[0], -p[1]).map_err(|_| "SVG output write failed")?;
        }
        svg.write_str("\"/>\n</svg>\n")
            .map_err(|_| "SVG output write failed")
    }
}
