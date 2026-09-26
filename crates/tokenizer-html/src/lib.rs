//! Full HTML engine: markup lex + element AST + host adapters.

use themoretheless_tokenizer_core::{
    Capabilities, Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, MarkupFlavor, MarkupParse, markup_to_host,
    parse_markup_as, require_default_dialect,
};

#[must_use]
pub fn parse(source: &str) -> MarkupParse<'_> {
    parse_markup_as(source, MarkupFlavor::Html5)
}

#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
    language: LanguageId::HTML,
    display_name: "HTML",
    dialects: themoretheless_tokenizer_core::DEFAULT_DIALECTS,
    default_dialect: themoretheless_tokenizer_core::DialectId::DEFAULT,
    aliases: &[],
    extensions: &[".html", ".htm"],
    mime_types: &["text/html"],
    // The markup AST in `core::markup_full` really does reject a mismatched or
    // unclosed element, and under `MarkupFlavor::Html5` it stays quiet on the
    // shapes HTML allows: void elements, omitted end tags, `<` inside a
    // `<script>` body. The shared fullkit parser behind the wave languages does
    // not claim either half.
    capabilities: Capabilities::LEX
        .union(Capabilities::PARSE)
        .union(Capabilities::VALIDATE),
    engine_version: env!("CARGO_PKG_VERSION"),
};

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(markup_to_host(&parse(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        // SEMANTIC dropped: identical to syntax (measurement-driven capability honesty).
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
    fn parse_div() {
        let p = parse("<div>hi</div>");
        assert!(p.lexed.is_lossless("<div>hi</div>"));
        assert!(!p.doc.roots.is_empty());
    }
}
