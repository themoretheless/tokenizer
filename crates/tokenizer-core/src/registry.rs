//! Static language registry for host discovery.

use crate::{capabilities::Capabilities, host::HostLanguage, language::LanguageId};

/// Why registering a language failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegisterError {
    EmptyId,
    DuplicateId { id: LanguageId },
    DuplicateAlias { alias: &'static str, id: LanguageId },
}

impl core::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyId => f.write_str("language id must not be empty"),
            Self::DuplicateId { id } => write!(f, "duplicate language id `{}`", id.as_str()),
            Self::DuplicateAlias { alias, id } => {
                write!(
                    f,
                    "alias `{alias}` collides with language `{}`",
                    id.as_str()
                )
            }
        }
    }
}

impl std::error::Error for RegisterError {}

/// Builder for a freeze-after-init language registry.
#[derive(Default)]
pub struct RegistryBuilder {
    engines: Vec<&'static dyn HostLanguage>,
}

impl RegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        engine: &'static dyn HostLanguage,
    ) -> Result<&mut Self, RegisterError> {
        let id = engine.id();
        if id.as_str().is_empty() {
            return Err(RegisterError::EmptyId);
        }
        if self.engines.iter().any(|existing| existing.id() == id) {
            return Err(RegisterError::DuplicateId { id });
        }
        for alias in engine.descriptor().aliases {
            if let Some(existing) = self
                .engines
                .iter()
                .find(|entry| entry.id().as_str() == *alias)
            {
                return Err(RegisterError::DuplicateAlias {
                    alias,
                    id: existing.id(),
                });
            }
        }
        self.engines.push(engine);
        Ok(self)
    }

    #[must_use]
    pub fn build(self) -> LanguageRegistry {
        LanguageRegistry {
            engines: self.engines,
        }
    }
}

/// Immutable registry of host language adapters.
#[derive(Clone)]
pub struct LanguageRegistry {
    engines: Vec<&'static dyn HostLanguage>,
}

impl LanguageRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            engines: Vec::new(),
        }
    }

    #[must_use]
    pub fn get(&self, id: LanguageId) -> Option<&'static dyn HostLanguage> {
        self.engines
            .iter()
            .copied()
            .find(|engine| engine.id() == id)
    }

    #[must_use]
    pub fn get_str(&self, id: &str) -> Option<&'static dyn HostLanguage> {
        self.engines
            .iter()
            .copied()
            .find(|engine| engine.id().as_str() == id || engine.descriptor().aliases.contains(&id))
    }

    /// First registered engine that claims the file extension (with or without dot).
    #[must_use]
    pub fn resolve_extension(&self, ext: &str) -> Option<&'static dyn HostLanguage> {
        let normalized = normalize_extension(ext);
        self.engines.iter().copied().find(|engine| {
            engine
                .descriptor()
                .extensions
                .iter()
                .any(|candidate| normalize_extension(candidate) == normalized)
        })
    }

    #[must_use]
    pub fn resolve_mime(&self, mime: &str) -> Option<&'static dyn HostLanguage> {
        let mime = mime.trim();
        self.engines.iter().copied().find(|engine| {
            engine
                .descriptor()
                .mime_types
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(mime))
        })
    }

    #[must_use]
    pub fn supports(&self, id: LanguageId, caps: Capabilities) -> bool {
        self.get(id)
            .is_some_and(|engine| engine.capabilities().contains(caps))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.engines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static dyn HostLanguage> + '_ {
        self.engines.iter().copied()
    }
}

fn normalize_extension(ext: &str) -> &str {
    ext.strip_prefix('.').unwrap_or(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        capabilities::Capabilities,
        host::{HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostTokenization},
        language::{DialectDescriptor, DialectId, LanguageDescriptor, LanguageId},
    };

    struct DummyJson;

    static DUMMY_DIALECTS: &[DialectDescriptor] = &[DialectDescriptor {
        id: DialectId::STRICT,
        display_name: "Strict",
        aliases: &[],
    }];

    static DUMMY_DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
        language: LanguageId::JSON,
        display_name: "JSON",
        dialects: DUMMY_DIALECTS,
        default_dialect: DialectId::STRICT,
        aliases: &["jsonc-file"],
        extensions: &[".json", "jsonc"],
        mime_types: &["application/json"],
        capabilities: Capabilities::JSON_FULL,
        engine_version: "0.0.0",
    };

    impl HostLanguage for DummyJson {
        fn descriptor(&self) -> &'static LanguageDescriptor {
            &DUMMY_DESCRIPTOR
        }

        fn lex(
            &self,
            _source: &str,
            _opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            Ok(HostTokenization {
                valid: true,
                ..HostTokenization::default()
            })
        }

        fn semantic_tokens(
            &self,
            source: &str,
            opts: &HostAnalysisOptions,
        ) -> Result<HostTokenization, HostError> {
            self.lex(source, opts)
        }

        fn diagnose(
            &self,
            _source: &str,
            _opts: &HostAnalysisOptions,
        ) -> Result<Vec<HostDiagnostic>, HostError> {
            Ok(Vec::new())
        }
    }

    static DUMMY: DummyJson = DummyJson;

    #[test]
    fn registers_and_resolves() {
        let mut builder = RegistryBuilder::new();
        builder.register(&DUMMY).unwrap();
        let registry = builder.build();
        assert!(registry.get(LanguageId::JSON).is_some());
        assert!(registry.get_str("jsonc-file").is_some());
        assert!(registry.resolve_extension("json").is_some());
        assert!(registry.resolve_extension(".jsonc").is_some());
        assert!(registry.resolve_mime("application/json").is_some());
        assert!(registry.supports(LanguageId::JSON, Capabilities::PARSE));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut builder = RegistryBuilder::new();
        builder.register(&DUMMY).unwrap();
        assert!(matches!(
            builder.register(&DUMMY),
            Err(RegisterError::DuplicateId { .. })
        ));
    }
}
