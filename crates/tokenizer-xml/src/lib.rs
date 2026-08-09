//! Full XML engine: markup lex + element AST + host adapters.

use themoretheless_tokenizer_core::{
    Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostTokenization,
    LanguageDescriptor, LanguageId, MarkupParse, FULL_CAPS, markup_to_host, parse_markup,
    require_default_dialect,
};

#[must_use]
pub fn parse(source: &str) -> MarkupParse<'_> {
    parse_markup(source)
}

#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
    language: LanguageId::XML,
    display_name: "XML",
    dialects: themoretheless_tokenizer_core::DEFAULT_DIALECTS,
    default_dialect: themoretheless_tokenizer_core::DialectId::DEFAULT,
    aliases: &[],
    extensions: &[".xml", ".xsl", ".svg"],
    mime_types: &["application/xml", "text/xml"],
    capabilities: FULL_CAPS,
    engine_version: env!("CARGO_PKG_VERSION"),
};

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(markup_to_host(&parse(source)))
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
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(validate(source)
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root() {
        let p = parse("<root a=\"1\"/>");
        assert!(p.lexed.is_lossless("<root a=\"1\"/>"));
    }
}
