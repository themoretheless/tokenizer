//! Static builtin catalog, type contracts, and numeric literal parsers shared by
//! both the static analyzer/editor frontend and the execution runtime.

use themoretheless_tokenizer_core::Span;

/// Pure collection and math operations available in the initial Rush environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Int,
    Float,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    Degrees,
    Radians,
    Some,
    None,
    Ok,
    Err,
    Promote,
    Random,
    Noise,
    GridMesh,
    Mesh,
    Transform,
    Slerp,
    AxisAngle,
    RotationMatrix,
    Zip,
    Identity,
    Translation,
    Scaling,
    RotationX,
    RotationY,
    RotationZ,
    TransformPoint,
    TransformDirection,
    Polygon,
    Translate,
    Rotate,
    Range,
    RangeIter,
    Iter,
    Collect,
    Cross,
    Lerp,
    Clamp,
    Smoothstep,
    Vec2,
    Vec3,
    Vec4,
    Dot,
    Length,
    Normalize,
    Sin,
    Cos,
    Sqrt,
    Deg,
    Map,
    FlatMap,
    Assert,
    Len,
    Get,
    Any,
    All,
    Filter,
    Fold,
    GroupBy,
    FoldBy,
    Arena,
    ArenaRun,
    ArenaStats,
    ArenaReset,
    TaskAll,
    TaskRace,
    JsonParse,
    JsonStringify,
    Trim,
    TrimStart,
    TrimEnd,
    Split,
    Join,
    StartsWith,
    EndsWith,
    Contains,
    Replace,
    ToLower,
    ToUpper,
}

impl Builtin {
    /// Callback argument index and number of values supplied per invocation.
    pub fn callback_arities(self) -> &'static [(usize, usize)] {
        match self {
            Self::Map | Self::Filter | Self::FlatMap | Self::Any | Self::All | Self::GroupBy => {
                &[(1, 1)]
            }
            Self::Fold => &[(2, 2)],
            Self::FoldBy => &[(1, 1), (3, 2)],
            Self::GridMesh => &[(2, 2)],
            Self::ArenaRun => &[(1, 0)],
            _ => &[],
        }
    }

    /// Inclusive accepted argument-count range, shared by runtime and analysis.
    pub fn arity(self) -> std::ops::RangeInclusive<usize> {
        let (min, max) = match self {
            Self::None | Self::Identity => (0, 0),
            Self::Int
            | Self::Float
            | Self::BitNot
            | Self::Degrees
            | Self::Iter
            | Self::Len
            | Self::Promote
            | Self::Radians
            | Self::Some
            | Self::Ok
            | Self::Err
            | Self::RotationMatrix
            | Self::Translation
            | Self::Scaling
            | Self::RotationX
            | Self::RotationY
            | Self::RotationZ
            | Self::Polygon
            | Self::Length
            | Self::Normalize
            | Self::Sin
            | Self::Cos
            | Self::Sqrt
            | Self::JsonParse
            | Self::JsonStringify
            | Self::Deg
            | Self::Trim
            | Self::TrimStart
            | Self::TrimEnd
            | Self::ToLower
            | Self::ToUpper => (1, 1),
            Self::Range | Self::RangeIter => (2, 3),
            Self::Assert => (1, 2),
            Self::Arena => (1, 4),
            Self::ArenaStats | Self::ArenaReset | Self::TaskAll | Self::TaskRace => (1, 1),
            Self::ArenaRun => (2, 2),
            Self::GridMesh
            | Self::Slerp
            | Self::Lerp
            | Self::Clamp
            | Self::Smoothstep
            | Self::Vec3
            | Self::Replace
            | Self::Fold => (3, 3),
            Self::Vec4 | Self::FoldBy => (4, 4),
            Self::BitAnd
            | Self::BitOr
            | Self::BitXor
            | Self::Shl
            | Self::Shr
            | Self::Random
            | Self::Collect
            | Self::Any
            | Self::All
            | Self::Get
            | Self::Mesh
            | Self::Noise
            | Self::Transform
            | Self::AxisAngle
            | Self::Zip
            | Self::TransformPoint
            | Self::TransformDirection
            | Self::Translate
            | Self::Rotate
            | Self::Cross
            | Self::Vec2
            | Self::Dot
            | Self::Map
            | Self::GroupBy
            | Self::FlatMap
            | Self::Split
            | Self::Join
            | Self::StartsWith
            | Self::EndsWith
            | Self::Contains
            | Self::Filter => (2, 2),
        };
        min..=max
    }
}

/// The runtime registration table, shared with editor name validation.
pub fn builtin_catalog() -> &'static [(&'static str, Builtin)] {
    &[
        ("int", Builtin::Int),
        ("float", Builtin::Float),
        ("bit_and", Builtin::BitAnd),
        ("bit_or", Builtin::BitOr),
        ("bit_xor", Builtin::BitXor),
        ("bit_not", Builtin::BitNot),
        ("shl", Builtin::Shl),
        ("shr", Builtin::Shr),
        ("assert", Builtin::Assert),
        ("json_parse", Builtin::JsonParse),
        ("json_stringify", Builtin::JsonStringify),
        ("trim", Builtin::Trim),
        ("trim_start", Builtin::TrimStart),
        ("trim_end", Builtin::TrimEnd),
        ("split", Builtin::Split),
        ("join", Builtin::Join),
        ("starts_with", Builtin::StartsWith),
        ("ends_with", Builtin::EndsWith),
        ("contains", Builtin::Contains),
        ("replace", Builtin::Replace),
        ("to_lower", Builtin::ToLower),
        ("to_upper", Builtin::ToUpper),
        ("len", Builtin::Len),
        ("get", Builtin::Get),
        ("any", Builtin::Any),
        ("all", Builtin::All),
        ("degrees", Builtin::Degrees),
        ("radians", Builtin::Radians),
        ("Some", Builtin::Some),
        ("None", Builtin::None),
        ("Ok", Builtin::Ok),
        ("Err", Builtin::Err),
        ("random", Builtin::Random),
        ("noise", Builtin::Noise),
        ("transform", Builtin::Transform),
        ("grid_mesh", Builtin::GridMesh),
        ("mesh", Builtin::Mesh),
        ("slerp", Builtin::Slerp),
        ("axis_angle", Builtin::AxisAngle),
        ("rotation_matrix", Builtin::RotationMatrix),
        ("identity", Builtin::Identity),
        ("promote", Builtin::Promote),
        ("arena", Builtin::Arena),
        ("arena_run", Builtin::ArenaRun),
        ("arena_stats", Builtin::ArenaStats),
        ("arena_reset", Builtin::ArenaReset),
        ("task_all", Builtin::TaskAll),
        ("task_race", Builtin::TaskRace),
        ("translation", Builtin::Translation),
        ("scaling", Builtin::Scaling),
        ("rotation_x", Builtin::RotationX),
        ("rotation_y", Builtin::RotationY),
        ("rotation_z", Builtin::RotationZ),
        ("transform_point", Builtin::TransformPoint),
        ("transform_direction", Builtin::TransformDirection),
        ("polygon", Builtin::Polygon),
        ("translate", Builtin::Translate),
        ("rotate", Builtin::Rotate),
        ("zip", Builtin::Zip),
        ("range", Builtin::Range),
        ("range_iter", Builtin::RangeIter),
        ("iter", Builtin::Iter),
        ("collect", Builtin::Collect),
        ("cross", Builtin::Cross),
        ("lerp", Builtin::Lerp),
        ("clamp", Builtin::Clamp),
        ("smoothstep", Builtin::Smoothstep),
        ("vec2", Builtin::Vec2),
        ("vec3", Builtin::Vec3),
        ("vec4", Builtin::Vec4),
        ("dot", Builtin::Dot),
        ("length", Builtin::Length),
        ("normalize", Builtin::Normalize),
        ("sin", Builtin::Sin),
        ("cos", Builtin::Cos),
        ("sqrt", Builtin::Sqrt),
        ("deg", Builtin::Deg),
        ("map", Builtin::Map),
        ("flat_map", Builtin::FlatMap),
        ("filter", Builtin::Filter),
        ("fold", Builtin::Fold),
        ("group_by", Builtin::GroupBy),
        ("fold_by", Builtin::FoldBy),
    ]
}

/// Fast O(1) compiled jump-table lookup for builtin names.
#[inline]
pub fn lookup_builtin(name: &str) -> Option<Builtin> {
    Some(match name {
        "int" => Builtin::Int,
        "float" => Builtin::Float,
        "bit_and" => Builtin::BitAnd,
        "bit_or" => Builtin::BitOr,
        "bit_xor" => Builtin::BitXor,
        "bit_not" => Builtin::BitNot,
        "shl" => Builtin::Shl,
        "shr" => Builtin::Shr,
        "assert" => Builtin::Assert,
        "json_parse" => Builtin::JsonParse,
        "json_stringify" => Builtin::JsonStringify,
        "trim" => Builtin::Trim,
        "trim_start" => Builtin::TrimStart,
        "trim_end" => Builtin::TrimEnd,
        "split" => Builtin::Split,
        "join" => Builtin::Join,
        "starts_with" => Builtin::StartsWith,
        "ends_with" => Builtin::EndsWith,
        "contains" => Builtin::Contains,
        "replace" => Builtin::Replace,
        "to_lower" => Builtin::ToLower,
        "to_upper" => Builtin::ToUpper,
        "len" => Builtin::Len,
        "get" => Builtin::Get,
        "any" => Builtin::Any,
        "all" => Builtin::All,
        "degrees" => Builtin::Degrees,
        "radians" => Builtin::Radians,
        "Some" => Builtin::Some,
        "None" => Builtin::None,
        "Ok" => Builtin::Ok,
        "Err" => Builtin::Err,
        "random" => Builtin::Random,
        "noise" => Builtin::Noise,
        "transform" => Builtin::Transform,
        "grid_mesh" => Builtin::GridMesh,
        "mesh" => Builtin::Mesh,
        "slerp" => Builtin::Slerp,
        "axis_angle" => Builtin::AxisAngle,
        "rotation_matrix" => Builtin::RotationMatrix,
        "identity" => Builtin::Identity,
        "promote" => Builtin::Promote,
        "arena" => Builtin::Arena,
        "arena_run" => Builtin::ArenaRun,
        "arena_stats" => Builtin::ArenaStats,
        "arena_reset" => Builtin::ArenaReset,
        "task_all" => Builtin::TaskAll,
        "task_race" => Builtin::TaskRace,
        "translation" => Builtin::Translation,
        "scaling" => Builtin::Scaling,
        "rotation_x" => Builtin::RotationX,
        "rotation_y" => Builtin::RotationY,
        "rotation_z" => Builtin::RotationZ,
        "transform_point" => Builtin::TransformPoint,
        "transform_direction" => Builtin::TransformDirection,
        "polygon" => Builtin::Polygon,
        "translate" => Builtin::Translate,
        "rotate" => Builtin::Rotate,
        "zip" => Builtin::Zip,
        "range" => Builtin::Range,
        "range_iter" => Builtin::RangeIter,
        "iter" => Builtin::Iter,
        "collect" => Builtin::Collect,
        "cross" => Builtin::Cross,
        "lerp" => Builtin::Lerp,
        "clamp" => Builtin::Clamp,
        "smoothstep" => Builtin::Smoothstep,
        "vec2" => Builtin::Vec2,
        "vec3" => Builtin::Vec3,
        "vec4" => Builtin::Vec4,
        "dot" => Builtin::Dot,
        "length" => Builtin::Length,
        "normalize" => Builtin::Normalize,
        "sin" => Builtin::Sin,
        "cos" => Builtin::Cos,
        "sqrt" => Builtin::Sqrt,
        "deg" => Builtin::Deg,
        "map" => Builtin::Map,
        "flat_map" => Builtin::FlatMap,
        "filter" => Builtin::Filter,
        "fold" => Builtin::Fold,
        "group_by" => Builtin::GroupBy,
        "fold_by" => Builtin::FoldBy,
        _ => return None,
    })
}

/// Type contracts for Rush values and host-registered signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
    Function(Vec<ValueType>, Box<ValueType>),
    User(String),
    HostObject(&'static str),
    Angle,
    Option(Box<ValueType>),
    Result(Box<ValueType>, Box<ValueType>),
    Tuple(Vec<ValueType>),
    Mesh,
    Quaternion,
    String,
    Matrix4,
    Int,
    Number,
    Bool,
    Vector(usize),
    Polygon,
    List(Box<ValueType>),
    Sequence,
    Null,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnnotationError {
    pub(crate) span: Span,
    pub(crate) name: String,
}

impl ValueType {
    pub(crate) fn parse_annotation(ty: &crate::Type<'_>) -> Result<Self, AnnotationError> {
        Self::parse_annotation_with(ty, &|_| None)
    }

    pub(crate) fn parse_annotation_with(
        ty: &crate::Type<'_>,
        resolve: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self, AnnotationError> {
        let qualified = ty.qualified_name();
        if ty.arguments.is_empty()
            && let Some(name) = resolve(&qualified)
        {
            return Ok(Self::User(name));
        }
        let primitive = match qualified.as_ref() {
            "angle" => Some(Self::Angle),
            "int" | "i64" => Some(Self::Int),
            "number" | "f64" | "float" => Some(Self::Number),
            "bool" => Some(Self::Bool),
            "vec2" => Some(Self::Vector(2)),
            "vec3" => Some(Self::Vector(3)),
            "vec4" => Some(Self::Vector(4)),
            "polygon" => Some(Self::Polygon),
            "str" | "string" => Some(Self::String),
            "quat" => Some(Self::Quaternion),
            "mesh" => Some(Self::Mesh),
            "mat4" => Some(Self::Matrix4),
            "null" => Some(Self::Null),
            "sequence" => Some(Self::Sequence),
            _ => None,
        };
        if ty.arguments.is_empty()
            && let Some(primitive) = primitive
        {
            return Ok(primitive);
        }
        if ty.name.text == "Fn"
            && ty.arguments.len() == 2
            && let Self::Tuple(parameters) = Self::parse_annotation_with(&ty.arguments[0], resolve)?
        {
            return Ok(Self::Function(
                parameters,
                Box::new(Self::parse_annotation_with(&ty.arguments[1], resolve)?),
            ));
        }
        if ty.name.text == "Option" && ty.arguments.len() == 1 {
            return Ok(Self::Option(Box::new(Self::parse_annotation_with(
                &ty.arguments[0],
                resolve,
            )?)));
        }
        if ty.name.text == "Result" && ty.arguments.len() == 2 {
            return Ok(Self::Result(
                Box::new(Self::parse_annotation_with(&ty.arguments[0], resolve)?),
                Box::new(Self::parse_annotation_with(&ty.arguments[1], resolve)?),
            ));
        }
        if ty.name.text == "tuple" {
            return Ok(Self::Tuple(
                ty.arguments
                    .iter()
                    .map(|ty| Self::parse_annotation_with(ty, resolve))
                    .collect::<Result<Vec<_>, _>>()?,
            ));
        }
        if ty.name.text == "list" && ty.arguments.len() == 1 {
            return Ok(Self::List(Box::new(Self::parse_annotation_with(
                &ty.arguments[0],
                resolve,
            )?)));
        }
        Err(AnnotationError {
            span: ty.name.span,
            name: ty.name.text.to_owned(),
        })
    }
}

#[inline]
pub(crate) fn is_int_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    (bytes.len() >= 2
        && bytes[0] == b'0'
        && matches!(bytes[1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        || text.ends_with('i')
        || text.ends_with("i64")
}

pub(crate) fn parse_int_u128(text: &str) -> Option<u128> {
    let body = text
        .strip_suffix("i64")
        .or_else(|| text.strip_suffix('i'))
        .unwrap_or(text);
    let (radix, digits) =
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16u128, rest)
        } else if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2u128, rest)
        } else if let Some(rest) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            (8u128, rest)
        } else {
            (10u128, body)
        };
    let mut acc = 0u128;
    let mut has_digit = false;
    for b in digits.bytes() {
        if b == b'_' {
            continue;
        }
        let d = (b as char).to_digit(radix as u32)? as u128;
        has_digit = true;
        acc = acc.checked_mul(radix)?.checked_add(d)?;
    }
    has_digit.then_some(acc)
}

pub(crate) fn parse_int_literal(text: &str) -> Option<i64> {
    i64::try_from(parse_int_u128(text)?).ok()
}

#[cfg(feature = "runtime")]
pub(crate) fn parse_neg_int_literal(text: &str) -> Option<i64> {
    let mag = parse_int_u128(text)?;
    if mag == (i64::MAX as u128) + 1 {
        Some(i64::MIN)
    } else {
        i64::try_from(mag).ok()?.checked_neg()
    }
}

#[cfg(feature = "runtime")]
pub(crate) fn parse_signed_int_str(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('-') {
        parse_neg_int_literal(rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        parse_int_literal(rest)
    } else {
        parse_int_literal(trimmed)
    }
}

pub(crate) fn parse_f64_literal(text: &str) -> Option<f64> {
    if text.as_bytes().contains(&b'_') {
        let cleaned: String = text.chars().filter(|&c| c != '_').collect();
        cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
    } else {
        text.parse::<f64>().ok().filter(|n| n.is_finite())
    }
}
