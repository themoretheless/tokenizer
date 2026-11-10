//! Descriptor and dialect helpers for language plugins.

use crate::{
    Capabilities, DialectDescriptor, DialectId, HostAnalysisOptions, HostError, LanguageDescriptor,
    LanguageId,
};

/// Build a default dialect list with only `default`.
pub const DEFAULT_DIALECTS: &[DialectDescriptor] = &[DialectDescriptor {
    id: DialectId::DEFAULT,
    display_name: "Default",
    aliases: &[],
}];

/// Highlight-only capabilities (lex + validate via diagnostics).
pub const HIGHLIGHT_CAPS: Capabilities = Capabilities::LEX.union(Capabilities::VALIDATE);

/// Full pipeline capabilities (`LEX | PARSE | SEMANTIC`).
pub const FULL_CAPS: Capabilities = crate::fullkit::FULL_ENGINE_CAPS;

/// Resolve dialect or reject; accepts empty/default.
pub fn require_default_dialect(
    descriptor: &LanguageDescriptor,
    raw: &str,
) -> Result<DialectId, HostError> {
    descriptor
        .resolve_dialect(raw)
        .ok_or_else(|| HostError::UnknownDialect {
            dialect: raw.to_owned(),
        })
}

/// Validate both the requested dialect and the input byte budget against `descriptor`.
pub fn check_host_entry(
    descriptor: &LanguageDescriptor,
    source: &str,
    opts: &HostAnalysisOptions,
) -> Result<DialectId, HostError> {
    let dialect = require_default_dialect(descriptor, opts.dialect.as_ref())?;
    if opts.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: opts.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    Ok(dialect)
}

/// Descriptor builder for highlight-only plugins.
#[must_use]
pub const fn highlight_descriptor(
    language: LanguageId,
    display_name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    mime_types: &'static [&'static str],
    engine_version: &'static str,
) -> LanguageDescriptor {
    language_descriptor(
        language,
        display_name,
        aliases,
        extensions,
        mime_types,
        engine_version,
        HIGHLIGHT_CAPS,
    )
}

/// Descriptor for full engine plugins.
#[must_use]
pub const fn full_descriptor(
    language: LanguageId,
    display_name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    mime_types: &'static [&'static str],
    engine_version: &'static str,
) -> LanguageDescriptor {
    language_descriptor(
        language,
        display_name,
        aliases,
        extensions,
        mime_types,
        engine_version,
        FULL_CAPS,
    )
}

#[must_use]
pub const fn language_descriptor(
    language: LanguageId,
    display_name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    mime_types: &'static [&'static str],
    engine_version: &'static str,
    capabilities: Capabilities,
) -> LanguageDescriptor {
    LanguageDescriptor {
        language,
        display_name,
        dialects: DEFAULT_DIALECTS,
        default_dialect: DialectId::DEFAULT,
        aliases,
        extensions,
        mime_types,
        capabilities,
        engine_version,
    }
}
