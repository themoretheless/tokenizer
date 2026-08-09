//! Language and dialect identity.

use crate::capabilities::Capabilities;

/// Stable language identifier (`json`, `python`, `javascript`, …).
///
/// Convention: lowercase ASCII `[a-z][a-z0-9-]*`, full words preferred
/// (`javascript` not `js`, `csharp` not `c#`, `cpp` not `c++`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanguageId(pub &'static str);

impl LanguageId {
    pub const JSON: Self = Self("json");
    pub const URL: Self = Self("url");
    pub const XML: Self = Self("xml");
    pub const HTML: Self = Self("html");
    pub const CSS: Self = Self("css");
    pub const YAML: Self = Self("yaml");
    pub const TOML: Self = Self("toml");
    pub const MARKDOWN: Self = Self("markdown");
    pub const SQL: Self = Self("sql");
    pub const MONGO: Self = Self("mongo");
    pub const BASH: Self = Self("bash");
    pub const POWERSHELL: Self = Self("powershell");
    pub const JAVASCRIPT: Self = Self("javascript");
    pub const TYPESCRIPT: Self = Self("typescript");
    pub const PYTHON: Self = Self("python");
    pub const JAVA: Self = Self("java");
    pub const CSHARP: Self = Self("csharp");
    pub const GO: Self = Self("go");
    pub const PHP: Self = Self("php");
    pub const RUBY: Self = Self("ruby");
    pub const C: Self = Self("c");
    pub const CPP: Self = Self("cpp");
    pub const RUST: Self = Self("rust");
    pub const KOTLIN: Self = Self("kotlin");
    pub const SWIFT: Self = Self("swift");
    pub const DART: Self = Self("dart");
    pub const R: Self = Self("r");
    pub const VISUALBASIC: Self = Self("visualbasic");
    pub const FORTRAN: Self = Self("fortran");
    pub const MATLAB: Self = Self("matlab");
    pub const DELPHI: Self = Self("delphi");
    pub const SCALA: Self = Self("scala");
    pub const LUA: Self = Self("lua");
    pub const PERL: Self = Self("perl");
    pub const OBJECTIVEC: Self = Self("objectivec");
    pub const JULIA: Self = Self("julia");
    pub const ASSEMBLY: Self = Self("assembly");

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Returns true when the id matches the naming convention.
    #[must_use]
    pub fn is_well_formed(self) -> bool {
        is_well_formed_id(self.0)
    }
}

impl core::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// Dialect within one language engine (`strict`, `jsonc`, `tsx`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DialectId(pub &'static str);

impl DialectId {
    pub const DEFAULT: Self = Self("default");
    pub const STRICT: Self = Self("strict");
    pub const JSONC: Self = Self("jsonc");

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl core::fmt::Display for DialectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// Language plus optional dialect (`json` or `json/jsonc` on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanguageKey {
    pub language: LanguageId,
    pub dialect: DialectId,
}

impl LanguageKey {
    #[must_use]
    pub const fn new(language: LanguageId, dialect: DialectId) -> Self {
        Self { language, dialect }
    }

    #[must_use]
    pub const fn language_only(language: LanguageId) -> Self {
        Self {
            language,
            dialect: DialectId::DEFAULT,
        }
    }
}

/// Metadata for one dialect of a language engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialectDescriptor {
    pub id: DialectId,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
}

/// Static description of a language plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageDescriptor {
    pub language: LanguageId,
    pub display_name: &'static str,
    pub dialects: &'static [DialectDescriptor],
    pub default_dialect: DialectId,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub mime_types: &'static [&'static str],
    pub capabilities: Capabilities,
    pub engine_version: &'static str,
}

impl LanguageDescriptor {
    #[must_use]
    pub fn id(&self) -> LanguageId {
        self.language
    }

    #[must_use]
    pub fn supports_dialect(&self, dialect: DialectId) -> bool {
        if dialect == self.default_dialect || dialect == DialectId::DEFAULT {
            return true;
        }
        self.dialects
            .iter()
            .any(|entry| entry.id == dialect || entry.aliases.contains(&dialect.0))
    }

    /// Resolve a dialect string to a known dialect, or `None`.
    #[must_use]
    pub fn resolve_dialect(&self, raw: &str) -> Option<DialectId> {
        if raw.is_empty() || raw == "default" {
            return Some(self.default_dialect);
        }
        if raw == self.default_dialect.0 {
            return Some(self.default_dialect);
        }
        for entry in self.dialects {
            if entry.id.0 == raw || entry.aliases.contains(&raw) {
                return Some(entry.id);
            }
        }
        None
    }
}

fn is_well_formed_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capabilities;

    #[test]
    fn language_ids_follow_convention() {
        assert!(LanguageId::JSON.is_well_formed());
        assert!(LanguageId("javascript").is_well_formed());
        assert!(!LanguageId("C#").is_well_formed());
        assert!(!LanguageId("").is_well_formed());
    }

    #[test]
    fn dialect_resolution() {
        static DIALECTS: &[DialectDescriptor] = &[
            DialectDescriptor {
                id: DialectId::STRICT,
                display_name: "Strict JSON",
                aliases: &[],
            },
            DialectDescriptor {
                id: DialectId::JSONC,
                display_name: "JSONC",
                aliases: &["json-with-comments"],
            },
        ];
        let descriptor = LanguageDescriptor {
            language: LanguageId::JSON,
            display_name: "JSON",
            dialects: DIALECTS,
            default_dialect: DialectId::STRICT,
            aliases: &[],
            extensions: &[".json"],
            mime_types: &["application/json"],
            capabilities: Capabilities::JSON_FULL,
            engine_version: "0.4.0",
        };
        assert_eq!(descriptor.resolve_dialect("jsonc"), Some(DialectId::JSONC));
        assert_eq!(
            descriptor.resolve_dialect("json-with-comments"),
            Some(DialectId::JSONC)
        );
        assert_eq!(descriptor.resolve_dialect(""), Some(DialectId::STRICT));
        assert_eq!(descriptor.resolve_dialect("nope"), None);
    }
}
