//! Indexed triangle meshes with finite positions and validated indices.
use std::fmt::Write;
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
}
impl Mesh {
    pub fn new(vertices: Vec<[f64; 3]>, triangles: Vec<[usize; 3]>) -> Result<Self, &'static str> {
        if vertices.iter().flatten().any(|x| !x.is_finite()) {
            return Err("Mesh vertices must be finite");
        }
        if triangles.iter().any(|t| {
            t.iter().any(|i| *i >= vertices.len()) || t[0] == t[1] || t[1] == t[2] || t[0] == t[2]
        }) {
            return Err("Invalid triangle indices");
        }
        Ok(Self {
            vertices,
            triangles,
        })
    }
    pub fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }
    pub fn triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }
    pub fn to_obj(&self) -> String {
        let mut text = String::new();
        self.write_obj(&mut text)
            .expect("Writing to String cannot fail");
        text
    }
    /// Stream OBJ into a caller-owned writer. Stops immediately on writer failure.
    pub fn write_obj(&self, output: &mut impl Write) -> std::fmt::Result {
        for v in &self.vertices {
            writeln!(output, "v {} {} {}", v[0], v[1], v[2])?;
        }
        for t in &self.triangles {
            writeln!(output, "f {} {} {}", t[0] + 1, t[1] + 1, t[2] + 1)?;
        }
        Ok(())
    }
}
