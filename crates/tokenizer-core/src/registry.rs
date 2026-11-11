//! Static language registry for host discovery.

use std::cmp::Ordering;

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
        let engines = self.engines.into_boxed_slice();
        let mut by_key = Vec::with_capacity(engines.len() * 2);
        let mut by_ext = Vec::with_capacity(engines.len() * 2);
        let mut by_mime = Vec::with_capacity(engines.len());

        for (idx, engine) in engines.iter().enumerate() {
            let desc = engine.descriptor();
            by_key.push((desc.language.as_str(), idx));
            for &alias in desc.aliases {
                by_key.push((alias, idx));
            }
            for &ext in desc.extensions {
                by_ext.push((normalize_extension(ext), idx));
            }
            for &mime in desc.mime_types {
                by_mime.push((mime.trim(), idx));
            }
        }

        // Exact-byte key index: primary IDs never collide, and on duplicate aliases
        // first-registered wins (`idx` ascending).
        by_key.sort_by(|(ka, ia), (kb, ib)| ka.cmp(kb).then_with(|| ia.cmp(ib)));
        by_key.dedup_by(|a, b| a.0 == b.0);

        // Extension index: exact match after stripping a leading `.`; first-registered wins.
        by_ext.sort_by(|(ea, ia), (eb, ib)| ea.cmp(eb).then_with(|| ia.cmp(ib)));
        by_ext.dedup_by(|a, b| a.0 == b.0);

        // MIME index: ASCII case-insensitive order; first-registered wins.
        by_mime.sort_by(|(ma, ia), (mb, ib)| {
            cmp_ascii_case_insensitive(ma, mb).then_with(|| ia.cmp(ib))
        });
        by_mime.dedup_by(|a, b| a.0.eq_ignore_ascii_case(b.0));

        LanguageRegistry {
            engines,
            by_key: by_key.into_boxed_slice(),
            by_ext: by_ext.into_boxed_slice(),
            by_mime: by_mime.into_boxed_slice(),
        }
    }
}

/// Immutable, pre-indexed registry of host language adapters (`O(log N)` lookups).
#[derive(Clone)]
pub struct LanguageRegistry {
    engines: Box<[&'static dyn HostLanguage]>,
    by_key: Box<[(&'static str, usize)]>,
    by_ext: Box<[(&'static str, usize)]>,
    by_mime: Box<[(&'static str, usize)]>,
}

impl LanguageRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            engines: Box::new([]),
            by_key: Box::new([]),
            by_ext: Box::new([]),
            by_mime: Box::new([]),
        }
    }

    #[must_use]
    pub fn get(&self, id: LanguageId) -> Option<&'static dyn HostLanguage> {
        let target = id.as_str();
        let idx = self
            .by_key
            .binary_search_by_key(&target, |&(key, _)| key)
            .ok()?;
        let engine = self.engines[self.by_key[idx].1];
        (engine.id() == id).then_some(engine)
    }

    #[must_use]
    pub fn get_str(&self, id: &str) -> Option<&'static dyn HostLanguage> {
        let idx = self
            .by_key
            .binary_search_by_key(&id, |&(key, _)| key)
            .ok()?;
        Some(self.engines[self.by_key[idx].1])
    }

    /// First registered engine that claims the file extension (with or without dot).
    #[must_use]
    pub fn resolve_extension(&self, ext: &str) -> Option<&'static dyn HostLanguage> {
        let normalized = normalize_extension(ext);
        let idx = self
            .by_ext
            .binary_search_by_key(&normalized, |&(key, _)| key)
            .ok()?;
        Some(self.engines[self.by_ext[idx].1])
    }

    #[must_use]
    pub fn resolve_mime(&self, mime: &str) -> Option<&'static dyn HostLanguage> {
        let mime = mime.trim();
        let idx = self
            .by_mime
            .binary_search_by(|&(candidate, _)| cmp_ascii_case_insensitive(candidate, mime))
            .ok()?;
        Some(self.engines[self.by_mime[idx].1])
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

#[inline]
fn normalize_extension(ext: &str) -> &str {
    ext.strip_prefix('.').unwrap_or(ext)
}

#[inline]
fn cmp_ascii_case_insensitive(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let min_len = ab.len().min(bb.len());
    let mut i = 0;
    while i < min_len {
        let ca = ab[i].to_ascii_lowercase();
        let cb = bb[i].to_ascii_lowercase();
        if ca != cb {
            return ca.cmp(&cb);
        }
        i += 1;
    }
    ab.len().cmp(&bb.len())
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
        assert!(registry.get(LanguageId("jsonc-file")).is_none());
        assert!(registry.get_str("jsonc-file").is_some());
        assert!(registry.resolve_extension("json").is_some());
        assert!(registry.resolve_extension(".jsonc").is_some());
        assert!(registry.resolve_mime("application/json").is_some());
        assert!(registry.resolve_mime("  APPLICATION/JSON ").is_some());
        assert!(registry.supports(LanguageId::JSON, Capabilities::PARSE));
    }

    #[test]
    fn first_registered_wins_on_shared_alias_extension_and_mime() {
        struct SecondUrl;
        static SECOND_DESC: LanguageDescriptor = LanguageDescriptor {
            language: LanguageId::URL,
            display_name: "Second",
            dialects: DUMMY_DIALECTS,
            default_dialect: DialectId::STRICT,
            aliases: &["jsonc-file"],
            extensions: &[".json", ".url"],
            mime_types: &["APPLICATION/JSON", "text/uri-list"],
            capabilities: Capabilities::LEX,
            engine_version: "0.0.0",
        };
        impl HostLanguage for SecondUrl {
            fn descriptor(&self) -> &'static LanguageDescriptor {
                &SECOND_DESC
            }
            fn lex(
                &self,
                _source: &str,
                _opts: &HostAnalysisOptions,
            ) -> Result<HostTokenization, HostError> {
                Ok(HostTokenization::default())
            }
        }
        static SECOND: SecondUrl = SecondUrl;

        let mut builder = RegistryBuilder::new();
        builder.register(&DUMMY).unwrap();
        builder.register(&SECOND).unwrap();
        let registry = builder.build();

        assert_eq!(
            registry.get_str("jsonc-file").unwrap().id(),
            LanguageId::JSON
        );
        assert_eq!(
            registry.resolve_extension(".json").unwrap().id(),
            LanguageId::JSON
        );
        assert_eq!(
            registry.resolve_extension("url").unwrap().id(),
            LanguageId::URL
        );
        assert_eq!(
            registry.resolve_mime("application/json").unwrap().id(),
            LanguageId::JSON
        );
        assert_eq!(
            registry.resolve_mime("TEXT/URI-LIST").unwrap().id(),
            LanguageId::URL
        );
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
