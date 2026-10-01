use themoretheless_tokenizer_rush::{Polygon, Value, evaluate};

#[test]
fn procedural_polygon_transforms_and_exports() {
    let source = "range(0, 360, 90) | map(a => vec2(cos(deg(a)), sin(deg(a))) * 2) | polygon | rotate(deg(90)) | translate(vec2(10, 20))";
    let Value::Polygon(polygon) = evaluate(source, 1000).unwrap() else {
        panic!()
    };
    for (actual, expected) in
        polygon
            .points()
            .iter()
            .zip([[10.0, 22.0], [8.0, 20.0], [10.0, 18.0], [12.0, 20.0]])
    {
        for axis in 0..2 {
            assert!((actual[axis] - expected[axis]).abs() < 1e-10);
        }
    }
    let svg = polygon.to_svg().unwrap();
    assert!(svg.contains("viewBox=\"7 -23 6 6\""));
    assert!(svg.contains("<polygon"));
    assert!(svg.contains("10,-22"));
}

#[test]
fn malformed_geometry_is_rejected() {
    for source in [
        "polygon([])",
        "polygon([vec3(1,2,3), vec3(1,2,3), vec3(1,2,3)])",
        "polygon([vec2(0,0),vec2(1,0),vec2(0,1)]) | translate(vec3(1,2,3))",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
    assert!(Polygon::new(vec![[f64::NAN, 0.0]; 3]).is_err());
    let huge = Polygon::new(vec![[-f64::MAX, 0.0], [f64::MAX, 0.0], [0.0, 1.0]]).unwrap();
    assert!(huge.to_svg().is_err());
}

#[test]
fn matrices_compose_right_to_left_and_distinguish_directions() {
    let source = "const m = translation(vec3(10, 20, 30)) * scaling(vec3(2, 3, 4))\ntransform_point(m, vec3(1, 1, 1))";
    assert_eq!(
        evaluate(source, 200).unwrap(),
        Value::Vector(vec![12., 23., 34.])
    );
    assert_eq!(
        evaluate(
            "transform_direction(translation(vec3(10,20,30)), vec3(1,2,3))",
            100
        )
        .unwrap(),
        Value::Vector(vec![1., 2., 3.])
    );
    let Value::Vector(v) =
        evaluate("transform_point(rotation_z(deg(90)), vec3(1,0,0))", 100).unwrap()
    else {
        panic!()
    };
    assert!(v[0].abs() < 1e-12 && (v[1] - 1.).abs() < 1e-12);
    for source in [
        "translation(vec2(1,2))",
        "transform_point(identity(), vec2(1,2))",
        "scaling(vec3(1e308,1,1)) * scaling(vec3(1e308,1,1))",
    ] {
        assert!(evaluate(source, 100).is_err());
    }
}

#[test]
fn rotations_are_right_handed_and_preserve_lengths() {
    for (source, expected) in [
        (
            "transform_direction(rotation_x(deg(90)), vec3(0,1,0))",
            [0., 0., 1.],
        ),
        (
            "transform_direction(rotation_y(deg(90)), vec3(0,0,1))",
            [1., 0., 0.],
        ),
        (
            "transform_direction(rotation_z(deg(90)), vec3(1,0,0))",
            [0., 1., 0.],
        ),
    ] {
        let Value::Vector(v) = evaluate(source, 100).unwrap() else {
            panic!()
        };
        assert!(v.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-12));
    }
    let source = "fn spin(angle: number) -> mat4 { return rotation_x(angle) * rotation_y(angle) * rotation_z(angle) }\nlength(transform_direction(spin(0.73), vec3(2,3,6)))";
    let Value::Number(length) = evaluate(source, 300).unwrap() else {
        panic!()
    };
    assert!((length - 7.).abs() < 1e-12);
    assert!(evaluate("const m: mat4 = vec4(1,2,3,4)", 100).is_err());
}

#[test]
fn quaternion_composition_matches_matrix_composition() {
    let source = "const q: quat = axis_angle(vec3(1,0,0), 0.7) * axis_angle(vec3(0,0,2), 0.3)\n(transform_point(rotation_matrix(q), vec3(2,3,4)), transform_point(rotation_x(0.7) * rotation_z(0.3), vec3(2,3,4)))";
    let Value::Tuple(values) = evaluate(source, 500).unwrap() else {
        panic!()
    };
    let (Value::Vector(a), Value::Vector(b)) = (&values[0], &values[1]) else {
        panic!()
    };
    assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-12));
    assert!(evaluate("axis_angle(vec3(0,0,0), 1)", 100).is_err());
    assert!(evaluate("axis_angle(vec2(1,0), 1)", 100).is_err());
}

#[test]
fn slerp_uses_shortest_arc_and_handles_near_identical_rotations() {
    for (end, t, expected) in [
        (90., 0.5, 45_f64),
        (350., 0.5, -5.),
        (0.000001, 0.5, 0.0000005),
        (90., 0., 0.),
        (90., 1., 90.),
    ] {
        let source = format!(
            "let a = axis_angle(vec3(0,0,1), 0)\nlet b = axis_angle(vec3(0,0,1), deg({end}))\ntransform_point(rotation_matrix(slerp(a,b,{t})),vec3(1,0,0))"
        );
        let Value::Vector(v) = evaluate(&source, 200).unwrap() else {
            panic!()
        };
        assert!((v[0] - expected.to_radians().cos()).abs() < 1e-10);
        assert!((v[1] - expected.to_radians().sin()).abs() < 1e-10);
    }
    assert!(evaluate("let q = axis_angle(vec3(1,0,0),0)\nslerp(q,q,2)", 100).is_err());
}

#[test]
fn formula_surface_has_expected_vertices_winding_and_obj_indices() {
    let Value::Mesh(mesh) =
        evaluate("grid_mesh([0, 1], [0, 1], (x,y) => vec3(x,y,x+y))", 200).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        mesh.vertices(),
        &[[0., 0., 0.], [1., 0., 1.], [0., 1., 1.], [1., 1., 2.]]
    );
    assert_eq!(mesh.triangles(), &[[0, 1, 3], [0, 3, 2]]);
    assert!(mesh.to_obj().contains("f 1 2 4\nf 1 4 3"));
    for source in [
        "grid_mesh([0], [0,1], (x,y) => vec3(x,y,0))",
        "grid_mesh([0,1], [0,1], (x,y) => vec2(x,y))",
    ] {
        assert!(evaluate(source, 200).is_err());
    }
    assert!(
        evaluate(
            "grid_mesh(range(0,10),range(0,10),(x,y) => vec3(x,y,0))",
            50
        )
        .is_err()
    );
}

#[test]
fn mesh_transforms_preserve_topology_and_original_value() {
    let source = "let original = grid_mesh([0,1],[0,1],(x,y) => vec3(x,y,0))\nlet moved = original | transform(translation(vec3(2,3,4)) * scaling(vec3(2,2,2)))\n(original, moved)";
    let Value::Tuple(values) = evaluate(source, 500).unwrap() else {
        panic!()
    };
    let (Value::Mesh(original), Value::Mesh(moved)) = (&values[0], &values[1]) else {
        panic!()
    };
    assert_eq!(original.vertices()[0], [0., 0., 0.]);
    assert_eq!(
        moved.vertices(),
        &[[2., 3., 4.], [4., 3., 4.], [2., 5., 4.], [4., 5., 4.]]
    );
    assert_eq!(original.triangles(), moved.triangles());
    assert!(
        evaluate(
            "grid_mesh([0,2],[0,1],(x,y) => vec3(x,y,0)) | transform(scaling(vec3(1e308,1,1)))",
            500
        )
        .is_err()
    );
}

#[test]
fn explicit_angles_preserve_units_and_validate_contracts() {
    let source = "fn rotate_point(a: angle) -> vec3 { return transform_point(rotation_z(a),vec3(1,0,0)) }\nrotate_point(degrees(45) * 2)";
    let Value::Vector(v) = evaluate(source, 200).unwrap() else {
        panic!()
    };
    assert!(v[0].abs() < 1e-12 && (v[1] - 1.).abs() < 1e-12);
    assert_eq!(
        evaluate("sin(degrees(90))", 100).unwrap(),
        Value::Number(1.)
    );
    assert!(evaluate("const a: angle = 90", 100).is_err());
    assert!(evaluate("degrees(90) + 1", 100).is_err());
    assert_eq!(
        evaluate("radians(1) + radians(2)", 100).unwrap(),
        Value::Angle(3.)
    );
}

#[test]
fn indexed_mesh_constructor_validates_topology() {
    use themoretheless_tokenizer_rush::{Value, evaluate};
    let points = "[vec3(0,0,0), vec3(1,0,0), vec3(0,1,0)]";
    let source = format!("mesh({points}, [[0,1,2]])");
    let Value::Mesh(mesh) = evaluate(&source, 200).unwrap() else {
        panic!()
    };
    assert_eq!(mesh.triangles(), &[[0, 1, 2]]);
    assert_eq!(mesh.vertices()[1], [1., 0., 0.]);
    for faces in [
        "[[0,1,3]]",
        "[[0,0,1]]",
        "[[0,1]]",
        "[[0,1,1.5]]",
        "[[-1,1,2]]",
        "[[0,true,2]]",
    ] {
        assert!(
            evaluate(&format!("mesh({points}, {faces})"), 200).is_err(),
            "{faces}"
        );
    }
    assert!(evaluate("mesh([vec2(0,0)], [])", 100).is_err());
}

#[test]
fn mesh_data_supports_functional_reconstruction_without_mutating_source() {
    use themoretheless_tokenizer_rush::{Value, evaluate};
    let source = "const original = mesh([vec3(0,0,0),vec3(1,0,0),vec3(0,1,0)], [[0,1,2]])\nconst changed = mesh(original.vertices | map(v => v + vec3(0,0,2)), original.triangles)\n(original.vertices[0].z, changed.vertices[0].z, changed.triangles)";
    assert_eq!(
        evaluate(source, 500).unwrap(),
        Value::Tuple(vec![
            Value::Number(0.),
            Value::Number(2.),
            Value::List(vec![Value::List(vec![
                Value::Number(0.),
                Value::Number(1.),
                Value::Number(2.)
            ])])
        ])
    );
    assert!(
        evaluate("mesh([], []).missing", 100)
            .unwrap_err()
            .message
            .contains("Unknown mesh field")
    );
}

#[test]
fn normalization_and_rotation_axes_handle_extreme_finite_magnitudes() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program, Quaternion, Value};
    let program = Program::compile("normalize(vec3(size,size,size))").unwrap();
    for size in [f64::MAX, f64::MIN_POSITIVE, f64::from_bits(1)] {
        let Value::Vector(v) = program
            .run(100, &CancellationToken::default(), &[("size", size)])
            .unwrap()
        else {
            panic!()
        };
        for component in v {
            assert!((component - 1.0 / 3.0_f64.sqrt()).abs() < 1e-15);
        }
        let rotation = Quaternion::axis_angle([size, size, size], 1.0).unwrap();
        let reference = Quaternion::axis_angle([1., 1., 1.], 1.0).unwrap();
        for (a, b) in rotation
            .components()
            .into_iter()
            .zip(reference.components())
        {
            assert!((a - b).abs() < 1e-15);
        }
    }
    assert!(Quaternion::axis_angle([f64::NAN, 1., 0.], 1.).is_none());
    assert!(Quaternion::axis_angle([0., 0., 0.], 1.).is_none());
}

#[test]
fn smoothstep_handles_extreme_edges_and_clamps_before_subtraction() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};
    let program = Program::compile("smoothstep(low, high, x)").unwrap();
    for (low, high, x, expected) in [
        (-f64::MAX, f64::MAX, 0., 0.5),
        (-f64::MAX, f64::MAX, -f64::MAX, 0.),
        (-f64::MAX, f64::MAX, f64::MAX, 1.),
        (0., 1., -f64::MAX, 0.),
        (-1., 0., f64::MAX, 1.),
        (0., f64::from_bits(2), f64::from_bits(1), 0.5),
    ] {
        assert_eq!(
            program
                .run(
                    100,
                    &CancellationToken::default(),
                    &[("low", low), ("high", high), ("x", x)]
                )
                .unwrap(),
            Value::Number(expected)
        );
    }
}

#[test]
fn streaming_exports_match_strings_and_stop_on_writer_failure() {
    use themoretheless_tokenizer_rush::Mesh;
    struct FailAfter {
        remaining: usize,
        failed: bool,
    }
    impl std::fmt::Write for FailAfter {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            assert!(!self.failed, "Exporter continued after writer failure");
            if text.len() > self.remaining {
                self.failed = true;
                return Err(std::fmt::Error);
            }
            self.remaining -= text.len();
            Ok(())
        }
    }
    let polygon = Polygon::new(vec![[0., 0.], [1., 0.], [0., 1.]]).unwrap();
    let mesh = Mesh::new(
        vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        vec![[0, 1, 2]],
    )
    .unwrap();
    let mut text = String::new();
    polygon.write_svg(&mut text).unwrap();
    assert_eq!(text, polygon.to_svg().unwrap());
    text.clear();
    mesh.write_obj(&mut text).unwrap();
    assert_eq!(text, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    assert_eq!(text, mesh.to_obj());
    for remaining in [0, 10, 20] {
        let mut writer = FailAfter {
            remaining,
            failed: false,
        };
        assert_eq!(
            polygon.write_svg(&mut writer),
            Err("SVG output write failed")
        );
        assert!(writer.failed);
        let mut writer = FailAfter {
            remaining,
            failed: false,
        };
        assert!(mesh.write_obj(&mut writer).is_err());
        assert!(writer.failed);
    }
    let invalid = Polygon::new(vec![[-f64::MAX, 0.], [f64::MAX, 0.], [0., 1.]]).unwrap();
    let mut writer = FailAfter {
        remaining: 0,
        failed: false,
    };
    assert_eq!(invalid.write_svg(&mut writer), Err("SVG bounds overflow"));
    assert!(
        !writer.failed,
        "Invalid bounds must be rejected before writing"
    );
}

#[test]
fn unary_vectors_preserve_dimensions_and_negate_components() {
    for (source, expected) in [
        ("-vec2(1,-2)", vec![-1., 2.]),
        ("+vec3(1,-2,3)", vec![1., -2., 3.]),
        ("-(-vec4(1,2,3,4))", vec![1., 2., 3., 4.]),
        ("let v = vec2(1,2); let neg = -v; v", vec![1., 2.]),
    ] {
        assert_eq!(
            evaluate(source, 1000).unwrap(),
            Value::Vector(expected),
            "{source}"
        );
    }
}
