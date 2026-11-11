//! Dialect profile definition for [`super::FullEngine`].

/// Declarative lexical and structural profile for a shared-pipeline language.
#[derive(Debug, Clone, Copy)]
pub struct FullProfile {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    /// Additional language-specific function-introducing keywords (beyond built-in `fn`, `def`, `function`, `func`, `fun`, `sub`, `procedure`).
    pub fn_keywords: &'static [&'static str],
    /// Additional language-specific type/module-introducing keywords (beyond built-in `class`, `struct`, `interface`, `trait`, `type`, `enum`, `module`, `package`).
    pub type_keywords: &'static [&'static str],
    /// Additional language-specific binding/declaration keywords (beyond built-in `let`, `var`, `const`, `val`, `mut`, `final`, `dim`, `local`, `my`).
    pub decl_keywords: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub hash_line_comment: bool,
    /// Allow `$` in identifiers.
    pub dollar_ident: bool,
    /// Treat paired backticks as an opaque quoted span (SQL identifiers, shell commands).
    pub backtick_strings: bool,
    /// Python-style triple quotes.
    pub triple_strings: bool,
    /// `#` comments / colon-introduced suite blocks (Python, Nim, Mojo style).
    pub soft_indent_blocks: bool,
}

impl FullProfile {
    pub const DEFAULT: Self = Self {
        keywords: &[],
        types: &[],
        fn_keywords: &[],
        type_keywords: &[],
        decl_keywords: &[],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    };
}

impl Default for FullProfile {
    fn default() -> Self {
        Self::DEFAULT
    }
}
