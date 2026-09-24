//! The language-vs-format axis and the curated engine presets.
//!
//! Engines split by *family*, not by delivery wave: `sql` ships in `wave2`
//! beside data formats but is a query language, while `top20` mixes it with
//! `python`. These tables are the single source of truth for that split. The
//! Cargo `top20`/`next20` features stay hand-written because `Cargo.toml`
//! cannot read Rust consts.

use crate::language::LanguageId;

/// What an engine tokenizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// A programming or query language: statements, expressions, scope.
    Language,
    /// A data, config, or markup document: structure carries the meaning.
    Format,
}

impl Family {
    pub const LANGUAGE: Self = Self::Language;
    pub const FORMAT: Self = Self::Format;

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::Format => "format",
        }
    }

    #[must_use]
    pub const fn is_format(self) -> bool {
        matches!(self, Self::Format)
    }

    /// Family of a registered engine. Query languages (`sql`, `mongo`,
    /// `graphql`) are languages; markup and config documents are formats.
    #[must_use]
    pub fn of(id: LanguageId) -> Self {
        match id {
            LanguageId::JSON
            | LanguageId::URL
            | LanguageId::XML
            | LanguageId::HTML
            | LanguageId::CSS
            | LanguageId::YAML
            | LanguageId::TOML
            | LanguageId::MARKDOWN => Self::Format,
            _ => Self::Language,
        }
    }
}

/// Data, config, and markup formats (Cargo feature `formats`).
pub const FORMAT_IDS: &[LanguageId] = &[
    LanguageId::JSON,
    LanguageId::YAML,
    LanguageId::TOML,
    LanguageId::URL,
    LanguageId::XML,
    LanguageId::HTML,
    LanguageId::CSS,
    LanguageId::MARKDOWN,
];

/// TIOBE-style popular programming languages (Cargo feature `top20`).
pub const TOP20_IDS: &[LanguageId] = &[
    LanguageId::PYTHON,
    LanguageId::C,
    LanguageId::CPP,
    LanguageId::JAVA,
    LanguageId::CSHARP,
    LanguageId::JAVASCRIPT,
    LanguageId::VISUALBASIC,
    LanguageId::SQL,
    LanguageId::GO,
    LanguageId::FORTRAN,
    LanguageId::MATLAB,
    LanguageId::PHP,
    LanguageId::RUST,
    LanguageId::R,
    LanguageId::RUBY,
    LanguageId::KOTLIN,
    LanguageId::SWIFT,
    LanguageId::TYPESCRIPT,
    LanguageId::DELPHI,
    LanguageId::ASSEMBLY,
];

/// Popular languages after `top20` (Cargo feature `next20` / `wave7`).
pub const NEXT20_IDS: &[LanguageId] = &[
    LanguageId::GROOVY,
    LanguageId::HASKELL,
    LanguageId::ELIXIR,
    LanguageId::ERLANG,
    LanguageId::CLOJURE,
    LanguageId::FSHARP,
    LanguageId::OCAML,
    LanguageId::LISP,
    LanguageId::SCHEME,
    LanguageId::SOLIDITY,
    LanguageId::ZIG,
    LanguageId::NIM,
    LanguageId::DLANG,
    LanguageId::COBOL,
    LanguageId::ADA,
    LanguageId::PROLOG,
    LanguageId::ABAP,
    LanguageId::VHDL,
    LanguageId::VERILOG,
    LanguageId::GRAPHQL,
];

/// A named engine set a host can offer as a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Preset(pub &'static str);

impl Preset {
    pub const TOP20: Self = Self("top20");
    pub const NEXT20: Self = Self("next20");
    pub const FORMATS: Self = Self("formats");

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Members of this preset, independent of which features are enabled.
    #[must_use]
    pub fn ids(self) -> &'static [LanguageId] {
        match self {
            Self::TOP20 => TOP20_IDS,
            Self::NEXT20 => NEXT20_IDS,
            _ => FORMAT_IDS,
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::TOP20, Self::NEXT20, Self::FORMATS]
    }
}

impl core::fmt::Display for Preset {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// Presets naming a given engine, in [`Preset::all`] order.
#[must_use]
pub fn presets_of(id: LanguageId) -> &'static [Preset] {
    static NONE: &[Preset] = &[];
    static TOP20: &[Preset] = &[Preset::TOP20];
    static NEXT20: &[Preset] = &[Preset::NEXT20];
    static FORMATS: &[Preset] = &[Preset::FORMATS];

    if FORMAT_IDS.contains(&id) {
        FORMATS
    } else if TOP20_IDS.contains(&id) {
        TOP20
    } else if NEXT20_IDS.contains(&id) {
        NEXT20
    } else {
        NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_presets_hold_twenty_each() {
        assert_eq!(TOP20_IDS.len(), 20);
        assert_eq!(NEXT20_IDS.len(), 20);
    }

    #[test]
    fn presets_do_not_overlap() {
        for id in TOP20_IDS {
            assert!(!NEXT20_IDS.contains(id), "{id} in both language sets");
            assert!(
                !FORMAT_IDS.contains(id),
                "{id} is both a language and a format"
            );
        }
    }

    #[test]
    fn every_preset_id_is_well_formed() {
        for preset in Preset::all() {
            for id in preset.ids() {
                assert!(id.is_well_formed(), "{} of {preset}", id.as_str());
            }
        }
    }

    #[test]
    fn formats_are_the_only_format_family() {
        for id in FORMAT_IDS {
            assert_eq!(Family::of(*id), Family::FORMAT, "{}", id.as_str());
        }
        assert_eq!(Family::of(LanguageId::SQL), Family::LANGUAGE);
        assert_eq!(Family::of(LanguageId::GRAPHQL), Family::LANGUAGE);
    }

    #[test]
    fn family_is_disjoint_from_delivery_waves() {
        // A "not scripting means format" rule would misfile these.
        assert_eq!(presets_of(LanguageId::SQL), &[Preset::TOP20]);
        assert_eq!(presets_of(LanguageId::GRAPHQL), &[Preset::NEXT20]);
        assert_eq!(presets_of(LanguageId::CSS), &[Preset::FORMATS]);
        assert_eq!(presets_of(LanguageId::PYTHON), &[Preset::TOP20]);
        assert!(presets_of(LanguageId::BASH).is_empty());
    }
}
