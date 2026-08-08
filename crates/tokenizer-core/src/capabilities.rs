//! Capability flags advertised by language engines.

use crate::language::LanguageId;

/// Bitset of engine layers a plugin implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Capabilities(u32);

impl Capabilities {
    pub const EMPTY: Self = Self(0);
    pub const LEX: Self = Self(1 << 0);
    pub const PARSE: Self = Self(1 << 1);
    pub const SEMANTIC: Self = Self(1 << 2);
    pub const CST: Self = Self(1 << 3);
    pub const NAVIGATE: Self = Self(1 << 4);
    pub const VISITOR: Self = Self(1 << 5);
    pub const VALIDATE: Self = Self(1 << 6);

    /// Full JSON-class engine surface.
    pub const JSON_FULL: Self = Self(
        Self::LEX.0
            | Self::PARSE.0
            | Self::SEMANTIC.0
            | Self::CST.0
            | Self::NAVIGATE.0
            | Self::VISITOR.0
            | Self::VALIDATE.0,
    );

    /// URL today: lossless highlight tokens + structural validation.
    pub const URL_TODAY: Self = Self(Self::LEX.0 | Self::VALIDATE.0);

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Close implied prerequisites (`PARSE ⇒ LEX`, `SEMANTIC ⇒ PARSE`, …).
    #[must_use]
    pub const fn close_prerequisites(self) -> Self {
        let mut bits = self.0;
        if bits & Self::VISITOR.0 != 0 {
            bits |= Self::PARSE.0;
        }
        if bits & Self::NAVIGATE.0 != 0 {
            bits |= Self::PARSE.0;
        }
        if bits & Self::CST.0 != 0 {
            bits |= Self::PARSE.0;
        }
        if bits & Self::SEMANTIC.0 != 0 {
            bits |= Self::PARSE.0;
        }
        if bits & Self::PARSE.0 != 0 {
            bits |= Self::LEX.0;
        }
        Self(bits)
    }
}

impl core::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Named capability for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    Lex,
    Parse,
    Semantic,
    Cst,
    Navigate,
    Visitor,
    Validate,
}

impl Capability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lex => "lex",
            Self::Parse => "parse",
            Self::Semantic => "semantic",
            Self::Cst => "cst",
            Self::Navigate => "navigate",
            Self::Visitor => "visitor",
            Self::Validate => "validate",
        }
    }

    #[must_use]
    pub const fn bits(self) -> Capabilities {
        match self {
            Self::Lex => Capabilities::LEX,
            Self::Parse => Capabilities::PARSE,
            Self::Semantic => Capabilities::SEMANTIC,
            Self::Cst => Capabilities::CST,
            Self::Navigate => Capabilities::NAVIGATE,
            Self::Visitor => Capabilities::VISITOR,
            Self::Validate => Capabilities::VALIDATE,
        }
    }
}

/// Requested capability set is not advertised by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityError {
    pub language_id: LanguageId,
    pub required: Capabilities,
    pub available: Capabilities,
}

impl CapabilityError {
    #[must_use]
    pub const fn missing(self) -> Capabilities {
        self.required.difference(self.available)
    }
}

impl core::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "language `{}` lacks capabilities bits {:#x} (available {:#x})",
            self.language_id.as_str(),
            self.missing().bits(),
            self.available.bits()
        )
    }
}

impl std::error::Error for CapabilityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerequisites_close_parse_and_lex() {
        let caps = Capabilities::NAVIGATE.close_prerequisites();
        assert!(caps.contains(Capabilities::NAVIGATE));
        assert!(caps.contains(Capabilities::PARSE));
        assert!(caps.contains(Capabilities::LEX));
    }

    #[test]
    fn validate_does_not_require_parse() {
        let caps = Capabilities::VALIDATE.close_prerequisites();
        assert!(caps.contains(Capabilities::VALIDATE));
        assert!(!caps.contains(Capabilities::PARSE));
    }
}
