# Plugin API design: multi-language syntax engines

**Status:** design contract (synthesized from ~40 plan-agent API slices + existing crate surface)  
**Package today:** `themoretheless-tokenizer` (single crate: JSON full engine + URL highlight/validate)  
**Target:** workspace of `tokenizer-core` + one crate per language + thin facade

This document is the API source of truth for the plugin architecture. It is design only; no implementation is implied beyond what already ships.

---

## 1. Locked decisions

| Topic | Choice |
|-------|--------|
| Plugin mechanism | **Cargo crates + features** only (no `dlopen` / `.so` / `libloading`) |
| Core deps | **Zero runtime crates.io deps** on `tokenizer-core` and language engines |
| Spans | **UTF-8 half-open byte offsets** inside all engines |
| Tokens | **Lossless** full streams (trivia retained; `concat(token texts) == source`) |
| Engine model | **Typed `LanguageEngine` per language crate** + **object-safe `HostLanguage` dyn facade** |
| Depth target | **JSON-level** (lex → parse → borrowing AST → semantic → CST / navigate / visitor) when capabilities allow |
| Incomplete engines | Advertise **capability flags**; missing layers return `CapabilityError`, never silent partial success |
| Legacy paths | **Re-export** root `tokenize_json` / `JsonTokenizer` / coarse `TokenKind` and `json::*` / `url::*` via facade |
| Encoding | Engines accept **valid UTF-8 `&str` only**; decode / non-UTF-8 at host preparer boundary |
| Multi-file | **Out of scope** for engines (single document); host owns `DocumentId` aggregation |
| Streaming / incremental | **Deferred**; full-buffer relex/reparse is the v1 contract |
| Family helpers | Optional later (`markup`, `c-family` scanners); rule-of-three, not mega-crates |

---

## 2. Workspace layout

```text
tokenizer/                                 # repo root = published facade themoretheless-tokenizer
  Cargo.toml                               # workspace + facade package
  crates/
    tokenizer-core/                        # themoretheless-tokenizer-core
    tokenizer-json/
    tokenizer-url/
    tokenizer-<language_id>/               # added per delivery wave PR
  wasm/
  playground/
  docs/plugin-api-design.md                # this file
```

- **Core** has no language crate dependencies.
- **Language crates** depend only on core (plus optional family utility crates).
- **Facade** (`themoretheless-tokenizer`) optional-depends on languages via features and re-exports stable paths.
- **No empty stub crates** for future languages until their PR lands.

Crate/package naming:

| Role | Package name (crates.io) | Rust crate name |
|------|--------------------------|-----------------|
| Core | `themoretheless-tokenizer-core` | `themoretheless_tokenizer_core` |
| Language | `themoretheless-tokenizer-<id>` | `themoretheless_tokenizer_<id>` |
| Facade | `themoretheless-tokenizer` | `themoretheless_tokenizer` |

Internal monorepo dirs may use short names (`crates/tokenizer-json`). Public features use the language id (`json`, `url`, `python`).

---

## 3. Core modules and key types

### 3.1 Module map (`tokenizer-core`)

```text
tokenizer_core
  span          // Span
  source        // LineIndex, LineColumn, ColumnEncoding, PositionError
  diagnostic    // Severity, Diagnostic, DiagnosticKind mapping helpers
  language      // LanguageId, DialectId, LanguageKey, LanguageDescriptor
  capabilities  // Capabilities bitset, Capability, CapabilityError
  limits        // InputLimits, LimitExceeded
  lex           // RawSyntaxKind, TokenFlags, LexToken, Lexed, lossless verify
  parse         // RecoveryMode, InputCompleteness, ParseConfig, ParseStatus (shared knobs)
  semantic      // TokenLayer, ThemeLegend, StandardSemanticClass (host-facing)
  syntax        // NodeId, TokenId, SyntaxElement, TextEdit, apply_edits, SyntaxSnapshot
  navigate      // Navigate, NodePath, NavigationError
  visit         // VisitControl, VisitOutcome
  host          // HostLanguage, host DTOs, HostError
  registry      // RegistryBuilder, LanguageRegistry, RegisterError
  lossless      // verify_spans / LosslessViolation
  scan          // optional: next_char_boundary, line_break_end, JSON string scan (rule-of-three)
```

### 3.2 Span and positions

```text
// UTF-8 half-open byte range. Only in-engine position unit.
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self;
    pub const fn len(self) -> usize;
    pub const fn is_empty(self) -> bool;
    pub const fn range(self) -> Range<usize>;
    pub const fn contains(self, offset: usize) -> bool;  // start <= offset < end
    pub const fn cover(self, other: Self) -> Self;
    pub fn is_valid_for(self, source: &str) -> bool;     // char boundaries + bounds
    pub fn slice(self, source: &str) -> Option<&str>;
}

pub enum ColumnEncoding {
    Utf8Bytes,
    UnicodeScalars,
    Utf16CodeUnits,
}

pub struct LineColumn {
    pub line: u32,    // 0-based
    pub column: u32,  // 0-based within encoding
}

pub enum PositionError {
    OffsetOutOfBounds,
    NotCharBoundary,
    InsideCrLf,
    LineOutOfBounds,
    ColumnOutOfBounds,
}

pub struct LineIndex<'source> { /* private */ }

impl<'source> LineIndex<'source> {
    pub fn new(source: &'source str) -> Self;
    pub fn source(&self) -> &'source str;
    pub fn line_count(&self) -> usize;
    pub fn line_column(&self, offset: usize, encoding: ColumnEncoding)
        -> Result<LineColumn, PositionError>;
    pub fn offset(&self, position: LineColumn, encoding: ColumnEncoding)
        -> Result<usize, PositionError>;
    pub fn span_to_range(&self, span: Span, encoding: ColumnEncoding)
        -> Result<(LineColumn, LineColumn), PositionError>;
}
```

**Invariants:** engines store only `Span`. Hosts convert with `LineIndex` at LSP/browser edges. Never mix encodings inside lex/parse.

### 3.3 Diagnostics

```text
#[non_exhaustive]
pub enum Severity { Error, Warning, Info, Hint }

pub struct DiagnosticCode(pub &'static str);  // machine key; never localized

pub struct RelatedSpan {
    pub span: Span,
    pub message: Option<Cow<'static, str>>,
}

pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: DiagnosticCode,           // or &'static str on wire
    pub message: Cow<'static, str>,     // display-only; prefer &'static
    pub related: Box<[RelatedSpan]>,    // empty common case
}

// Language-local Copy enums implement:
pub trait DiagnosticKind: Copy {
    fn code(self) -> &'static str;
    fn severity(self) -> Severity;
    fn message(self) -> &'static str;
}

// Validity: no Severity::Error (warnings allowed).
// Wire form may use namespaced codes: "{language}/{local}" e.g. json/expected-value
// Legacy unprefixed codes (url-empty-host, expected-value) remain aliases until major.
```

### 3.4 Language identity

```text
pub struct LanguageId(pub &'static str);
// Lowercase ASCII [a-z][a-z0-9-]*; full words: javascript, csharp, cpp, powershell

pub struct DialectId(pub &'static str);
// Within one language: strict, jsonc, tsx, python3, sql.postgres, …

pub struct LanguageKey {
    pub language: LanguageId,
    pub dialect: DialectId,
}
// Wire: "json" (default dialect) or "json/jsonc"

pub struct DialectDescriptor {
    pub id: DialectId,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
}

pub struct LanguageDescriptor {
    pub language: LanguageId,
    pub display_name: &'static str,
    pub dialects: &'static [DialectDescriptor],
    pub default_dialect: DialectId,
    pub aliases: &'static [&'static str],       // editor language ids, etc.
    pub extensions: &'static [&'static str],    // ".json", ".py"
    pub mime_types: &'static [&'static str],
    pub capabilities: Capabilities,
    pub engine_version: &'static str,           // Cargo package version string
}

// Dialects are NOT separate LanguageIds when they share one engine:
// jsonc → json + dialect jsonc; tsx → typescript + dialect tsx
```

### 3.5 Input limits and options

```text
pub struct InputLimits {
    pub max_input_bytes: usize,
    pub max_tokens: usize,
    pub max_diagnostics: usize,
    pub max_depth: usize,
}

impl InputLimits {
    pub const fn conservative() -> Self; // e.g. 16MiB / 1_000_000 / 256 / 128
    pub const fn max_input_bytes(self, n: usize) -> Self;
    // … fluent const setters
}

// Zero never means unlimited.
// Language options embed InputLimits; dialect flags stay language-local.

// JSON example (language crate):
pub struct JsonOptions {
    pub limits: InputLimits,
    pub allow_comments: bool,
    pub allow_bom: bool,
    pub allow_trailing_commas: bool,
}
impl JsonOptions {
    pub const fn strict() -> Self;
    pub const fn jsonc() -> Self;
    pub const fn with_limits(self, limits: InputLimits) -> Self;
}
```

### 3.6 Lex layer

```text
pub struct RawSyntaxKind(pub u16);  // language-local; never merge across languages without LanguageId

pub struct TokenFlags(u16);
impl TokenFlags {
    pub const HAS_ERROR: Self;  // classified-but-invalid token
}

pub struct LexToken {
    pub kind: RawSyntaxKind,
    pub span: Span,
    pub flags: TokenFlags,
}

impl LexToken {
    pub fn text(self, source: &str) -> Option<&str>;
    pub fn has_error(self) -> bool;
}

pub struct Lexed<'source> { /* private */ }
impl<'source> Lexed<'source> {
    pub fn source(&self) -> &'source str;
    pub fn tokens(&self) -> &[LexToken];
    pub fn diagnostics(&self) -> &[Diagnostic];
    pub fn has_errors(&self) -> bool;
    pub fn is_truncated(&self) -> bool;
    pub fn significant_tokens(&self) -> impl Iterator<Item = LexToken> + '_;
    pub fn is_lossless(&self) -> bool;
}

// Hybrid invalid policy:
// - Best-effort concrete kind + HAS_ERROR for malformed String/Number/…
// - One residual Error/Unknown kind for unclassifiable / limit residual
// - Policy rejections (comments in strict JSON): keep real kind, diagnostic, NO HAS_ERROR
```

### 3.7 Lossless contract

```text
// Full stream (trivia included):
// I1 concat_i text(tokens[i]) == source (byte-for-byte)
// I2 tokens[0].start == 0; tokens[last].end == source.len() (if n > 0)
// I3 tokens[i].end == tokens[i+1].start
// I4 every token non-empty when n > 0; empty source => zero tokens
// I5 every span char-boundary-valid for source
// I6 token text is pure borrow; never owned rewrite as coverage truth
// I7 diagnostics / recovery must not drop coverage
// I8 truncation still covers all of source (one residual Error token for unread suffix)
// I9 significant_tokens() may skip trivia; full stream must not
// I10 semantic 1:1 projection and CST token walks obey the same cover rules

pub fn verify_spans(source: &str, spans: impl Iterator<Item = Span>)
    -> Result<(), LosslessViolation>;
```

### 3.8 Parse knobs (shared) and language AST

```text
pub enum RecoveryMode { Recover, FailFast }
pub enum InputCompleteness { Complete, Incomplete }

pub struct ParseConfig {
    pub recovery: RecoveryMode,
    pub completeness: InputCompleteness,
    pub limits: InputLimits,
}

impl ParseConfig {
    pub const fn editor() -> Self;    // Recover + Incomplete
    pub const fn validate() -> Self;  // Recover or FailFast + Complete
}

pub struct ParseStatus {
    pub has_root: bool,
    pub is_valid: bool,           // has_root && no Error diagnostics
    pub is_incomplete: bool,
    pub nesting_limited: bool,
    pub diagnostics_truncated: bool,
    pub depth_reached: u32,
}

// parse_* never returns Result for syntax errors; always a recovering bundle.
// Typed AST lives in the language crate (JSON Value<'source> is the reference model):
// - borrow source; no HashMap collapse; preserve duplicate keys order
// - numbers keep exact spelling; convert only via as_i64/as_u64/as_f64
// - string raw() always present; decoded() optional on valid escapes
```

### 3.9 Semantic vs syntax tokens

```text
pub enum TokenLayer { Syntax, Semantic }

// Syntax: pure lex atoms (language SyntaxKind).
// Semantic: 1:1 span-aligned projection from parse context (Property vs String, …).
// Never invent semantic spans that do not match a syntax token.

pub struct ThemeLegend {
    pub labels: &'static [&'static str],  // kebab-case wire names
}

pub enum StandardSemanticClass {
    Property, String, Number, Boolean, Null, Keyword, Operator,
    Punctuation, Comment, Whitespace, Invalid, Other,
}

// Kind naming: language-local #[repr(u16)] enums with const fn as_str() -> &'static str.
// Wire uses kebab-case names (not Debug). KindId is dense and language-local.
// No cross-language mega-enum in core.
```

### 3.10 CST, edits, navigation, visitor

```text
pub struct NodeId(u32);
pub struct TokenId(u32);
pub enum SyntaxElement { Node(NodeId), Token(TokenId) }

pub struct TextEdit {
    pub span: Span,              // pre-edit coordinates
    pub replacement: String,
}

pub enum EditError {
    ReversedSpan { .. },
    OutOfBounds { .. },
    NotCharBoundary { .. },
    Unsorted { .. },
    Overlapping { .. },
    OutputTooLarge,
}

pub fn apply_edits(source: &str, edits: &[TextEdit]) -> Result<String, EditError>;
// Batch: sorted, non-overlapping, char-boundary, UTF-8 half-open.
// apply_edits is text-only; host always reparses for a new tree (IDs die).

pub trait SyntaxSnapshot {
    fn source(&self) -> &str;
    fn root_id(&self) -> NodeId;
    fn node(&self, id: NodeId) -> Option<SyntaxNodeView<'_>>;
    fn token(&self, id: TokenId) -> Option<TokenView>;
    fn node_kind_name(&self, kind: u16) -> &'static str;
    fn token_kind_name(&self, kind: u16) -> &'static str;
    fn is_lossless(&self) -> bool;
}

// Navigation (object-safe; no full visitor required):
pub trait Navigate {
    fn source(&self) -> &str;
    fn root(&self) -> Option<NodeId>;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> &[NodeId];
    fn node_at_offset(&self, offset: usize) -> Result<Option<NodeId>, NavigationError>;
    fn path_at_offset(&self, offset: usize) -> Result<Option<NodePath>, NavigationError>;
    fn path_of(&self, id: NodeId) -> Option<NodePath>;
    fn resolve_path(&self, path: &NodePath) -> Option<NodeId>;
}

// Visitor control (shared vocabulary):
pub enum VisitControl { Continue, SkipChildren, Break }
pub enum VisitOutcome { Completed, Broken }

// Typed AstVisitor stays in language crates (JSON enter/leave Value).
// Host uses SyntaxVisitor over CST NodeId on the dyn facade.
```

### 3.11 BOM / encoding policy

```text
// Engines: valid UTF-8 only.
// Leading U+FEFF may be Bom trivia (JSONC allow) or ForbiddenTrivia + diagnostic.
// Non-UTF-8 / UTF-16 decode: host SourcePreparer only; spans index prepared text.
// sniff_bom is pure prefix match; no encoding crates in core.
```

---

## 4. LanguageEngine (typed) + HostLanguage (dyn)

### 4.1 Typed language engine (per language crate)

Primary surface for monomorphized callers and free functions (`json::lex`, `json::parse`, …). Associated types keep zero-cost paths.

```text
pub trait LanguageEngine: Send + Sync + 'static {
    type SyntaxKind: Copy + Eq;
    type LexOptions: Clone + Default;
    type Lexed<'s>;
    type ParseOptions: Clone + Default;
    type Parse<'s>;
    type Value<'s>;
    type SemanticKind: Copy + Eq;
    type SyntaxTree<'s>;

    fn descriptor(&self) -> &'static LanguageDescriptor;
    fn language_id(&self) -> LanguageId;
    fn capabilities(&self) -> Capabilities;

    fn lex<'s>(&self, source: &'s str, opts: &Self::LexOptions) -> Self::Lexed<'s>;
    fn parse<'s>(&self, source: &'s str, opts: &Self::ParseOptions) -> Self::Parse<'s>;
    fn semantic_tokens<'s>(
        &self,
        parse: &Self::Parse<'s>,
    ) -> SemanticTokenization<Self::SemanticKind>;

    // Capability-gated helpers (default / feature-gated in impls):
    // fn syntax_tree<'s>(&self, parse: &Self::Parse<'s>) -> Self::SyntaxTree<'s>;
    // fn navigate<'s>(&self, tree: &Self::SyntaxTree<'s>) -> &dyn Navigate;
    // fn validate(&self, source: &str, opts: &Self::ParseOptions) -> Vec<Diagnostic>;
}

// ZST singletons preferred:
// pub struct JsonEngine;
// pub static ENGINE: JsonEngine = JsonEngine;
// pub const DESCRIPTOR: LanguageDescriptor = …;
```

**Thread-safety:** analysis methods take `&self` only, pure and reentrant. Engines never retain source past the call. Share as `SharedEngine = Arc<dyn HostLanguage>` (or typed ZSTs). Optional `AnalysisSession: Send` holds per-document caches with `&mut self` (not required `Sync`).

### 4.2 Dyn host facade

Object-safe, string kinds, owned DTOs. Used by playground, WASM, LSP glue. Never exposes borrowed language AST types.

```text
pub trait HostLanguage: Send + Sync + 'static {
    fn descriptor(&self) -> &'static LanguageDescriptor;
    fn id(&self) -> LanguageId;
    fn capabilities(&self) -> Capabilities;
    fn dialects(&self) -> &'static [DialectId];
    fn token_legend(&self, layer: TokenLayer) -> ThemeLegend;
    fn default_options(&self) -> HostAnalysisOptions;

    fn lex(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError>;

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostSemanticTokenization, HostError>;

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError>;

    fn node_at(
        &self,
        source: &str,
        offset: usize,
        opts: &HostAnalysisOptions,
    ) -> Result<Option<HostHit>, HostError>;

    fn outline(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Option<HostOutlineNode>, HostError>;

    fn analyze(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
        req: HostAnalysisRequest,
    ) -> Result<HostDocumentAnalysis, HostError>;
}

pub struct HostAnalysisOptions {
    pub dialect: Cow<'static, str>,
    pub limits: InputLimits,
}

pub struct HostSpan { pub start: usize, pub end: usize }

pub struct HostToken {
    pub kind: Cow<'static, str>,  // kebab-case legend name
    pub span: HostSpan,
    pub error: bool,
}

pub struct HostDiagnostic {
    pub code: Cow<'static, str>,
    pub message: Cow<'static, str>,
    pub span: HostSpan,
    pub severity: Severity,
}

pub struct HostTokenization {
    pub tokens: Vec<HostToken>,
    pub diagnostics: Vec<HostDiagnostic>,
    pub valid: bool,
}

pub struct HostAnalysisRequest {
    pub syntax_tokens: bool,
    pub semantic_tokens: bool,
    pub diagnostics: bool,
    pub outline: bool,
}

pub struct HostDocumentAnalysis {
    pub language: LanguageId,
    pub dialect: String,
    pub source_bytes: usize,
    pub valid: bool,
    pub syntax: Option<HostTokenization>,
    pub semantic: Option<HostSemanticTokenization>,
    pub diagnostics: Vec<HostDiagnostic>,
    pub outline: Option<HostOutlineNode>,
}

pub enum HostError {
    UnsupportedCapability { capability: &'static str },
    UnknownDialect { dialect: String },
    InputTooLarge { max: usize, actual: usize },
    InvalidOffset { offset: usize, source_len: usize },
    InvalidOptions { message: String },
}
```

Adapters: `JsonHostLanguage`, `UrlHostLanguage`, … wrap typed engines and map kinds via `as_str()`.

### 4.3 Relationship

```text
Typed LanguageEngine ──free functions──► monomorphized Rust callers
        │
        └── adapter ──► HostLanguage (dyn) ──► playground / WASM / LSP
```

`trait Tokenizer` / `tokenize_json` are **legacy JSON-only**, not the multi-language plugin surface.

---

## 5. Capabilities

```text
pub struct Capabilities(u32);

impl Capabilities {
    pub const EMPTY: Self;
    pub const LEX: Self;        // 1 << 0  lossless syntax tokens
    pub const PARSE: Self;      // 1 << 1  recovering parse + AST
    pub const SEMANTIC: Self;   // 1 << 2  context-sensitive highlight
    pub const CST: Self;        // 1 << 3  arena syntax tree
    pub const NAVIGATE: Self;   // 1 << 4  offset → node / path
    pub const VISITOR: Self;    // 1 << 5  walk helpers
    pub const VALIDATE: Self;   // 1 << 6  may exist without PARSE (URL today)

    pub const JSON_FULL: Self = LEX|PARSE|SEMANTIC|CST|NAVIGATE|VISITOR|VALIDATE;
    // URL today: SEMANTIC|VALIDATE (or LEX|VALIDATE once true lossless lexer lands)

    pub const fn contains(self, required: Self) -> bool;
    pub const fn union(self, other: Self) -> Self;
    pub const fn is_empty(self) -> bool;
}

#[non_exhaustive]
pub enum Capability { Lex, Parse, Semantic, Cst, Navigate, Visitor, Validate }

pub struct CapabilityError {
    pub language_id: &'static str,
    pub required: Capabilities,
    pub available: Capabilities,
    pub missing: Capabilities,
}

// Prerequisites (close_prerequisites):
//   Parse ⇒ Lex
//   Semantic ⇒ Parse   (URL may advertise SEMANTIC without PARSE only while elevating;
//                        document as highlight tokens; prefer LEX|VALIDATE until parse ships)
//   Cst ⇒ Parse
//   Navigate ⇒ Parse
//   Visitor ⇒ Parse
//   Validate independent
```

**Rules:**

- `capabilities()` is pure and stable for a given engine value.
- Advertising a bit means the corresponding host/typed method is implemented (may still emit soft diagnostics).
- Calling a missing layer returns `CapabilityError` / `HostError::UnsupportedCapability`, never panics.
- Compile-time Cargo capability features should match the advertised bitset (unit-tested).

---

## 6. Registry and Cargo features

### 6.1 Registry

```text
pub struct RegistryBuilder { /* private */ }

impl RegistryBuilder {
    pub fn new() -> Self;
    pub fn register(&mut self, engine: &'static dyn HostLanguage)
        -> Result<&mut Self, RegisterError>;
    pub fn build(self) -> LanguageRegistry;
}

pub struct LanguageRegistry { /* private; freeze-after-init preferred */ }

impl LanguageRegistry {
    pub fn get(&self, id: LanguageId) -> Option<&'static dyn HostLanguage>;
    pub fn get_str(&self, id: &str) -> Option<&'static dyn HostLanguage>;
    pub fn resolve(&self, input: &str) -> Result<LanguageKey, LanguageKeyParseError>;
    pub fn resolve_extension(&self, ext: &str) -> Option<&'static dyn HostLanguage>;
    pub fn resolve_mime(&self, mime: &str) -> Option<&'static dyn HostLanguage>;
    pub fn iter(&self) -> impl Iterator<Item = &'static dyn HostLanguage>;
    pub fn supports(&self, id: LanguageId, caps: Capabilities) -> bool;
}

// Facade:
pub fn register_builtins(builder: &mut RegistryBuilder);
pub fn builtin_registry() -> LanguageRegistry;
// register_builtins is the single source of truth for feature-gated membership.
```

**Invariants:** no dynamic loading; disabled features omit deps; duplicate primary ids rejected; aliases must not collide with another primary id; first registration wins for extension/mime conflicts.

### 6.2 Facade features

```toml
[features]
default = ["json"]
json = ["dep:themoretheless-tokenizer-json"]
url = ["dep:themoretheless-tokenizer-url"]
# one feature per language_id as crates land
all-languages = ["json", "url" /* + … */]
web-bridge = ["json"]   # grows as host needs more languages; never in default
```

Language crate features (JSON reference DAG):

```toml
[features]
default = ["full"]
lex = []
parse = ["lex"]          # AST types always with parse (no separate ast flag)
semantic = ["parse"]
cst = ["parse"]
navigation = ["parse"]
visitor = ["parse"]
validate = []
full = ["lex", "parse", "semantic", "cst", "navigation", "visitor", "validate"]
```

URL today: `tokenize`, `validate`, aggregate `full`.

**Invariants:** features additive only; names are stable public surface; `default` never enables host bridges or third-party runtime deps; library graph stays zero runtime deps.

### 6.3 Public re-export graph

Umbrella `themoretheless_tokenizer` re-exports:

```text
// Always (from core / legacy):
crate::{Span, Diagnostic, Token, TokenKind, Tokenization, Tokenizer,
        JsonTokenizer, tokenize_json}
crate::{ColumnEncoding, LineColumn, LineIndex, PositionError}

// feature = "json"
crate::json::*   // identical to themoretheless_tokenizer_json::*

// feature = "url"
crate::url::*
crate::{UrlKind, UrlToken, UrlTokenization, tokenize_url, tokenize_url_validated, validate_url}

// Host / registry (core, always available with facade):
crate::registry::{LanguageRegistry, register_builtins, builtin_registry}
crate::host::{HostLanguage, …}
```

- Dual paths are **type-identical** (`pub use`), not wrappers.
- Removing or renaming a re-exported path is a **major** on the umbrella.
- Internal modules (`lexer`, `parser`, …) stay crate-private unless curated.
- `web_bridge` remains `doc(hidden)` + feature-gated.

---

## 7. Host / WASM payload schema

Evolve today’s playground contract into a **versioned, language-scoped** envelope. Core stays serde-free; bridge hand-builds JSON (as now).

### 7.1 Request

```json
{
  "schemaVersion": 1,
  "language": "json",
  "mode": "strict",
  "layer": "semantic",
  "source": "…",
  "options": {
    "includeText": true,
    "maxTokens": 1000000,
    "maxDiagnostics": 256
  }
}
```

| Field | Meaning |
|-------|---------|
| `language` | `LanguageId` kebab string |
| `mode` | dialect / profile scoped to language (`strict` \| `jsonc` for json; `default` for url) |
| `layer` | `syntax` (lex kinds) \| `semantic` (highlight categories) |
| `source` | UTF-8 text (JSON string) |

### 7.2 Success response

```json
{
  "schemaVersion": 1,
  "language": "json",
  "mode": "strict",
  "layer": "semantic",
  "valid": true,
  "sourceBytes": 42,
  "tokens": [
    {
      "index": 0,
      "kind": "property",
      "start": 0,
      "end": 5,
      "text": "\"a\"",
      "error": false
    }
  ],
  "diagnostics": [
    {
      "code": "expected-value",
      "message": "…",
      "start": 10,
      "end": 10,
      "severity": "error"
    }
  ]
}
```

**Invariants:**

- Spans are half-open UTF-8 byte offsets; `start <= end <= sourceBytes`; char boundaries.
- Syntax/semantic token streams are lossless when tokens present.
- `kind` is stable kebab-case vocabulary for `(language, layer)`, not Rust `Debug`.
- `valid` is false if any diagnostic has severity error (or any diagnostic when severity omitted).
- Echo resolved `language` / `mode` / `layer`.

### 7.3 Protocol error (not recovery)

```json
{
  "schemaVersion": 1,
  "ok": false,
  "error": {
    "code": "unknown-language",
    "message": "…"
  }
}
```

Codes: `unknown-language` \| `unknown-mode` \| `unknown-layer` \| `invalid-request` \| `internal`.

### 7.4 APIs

```text
// Bridge / WASM
fn analyze(request_json: &str) -> String;
fn list_engines() -> String;  // EngineInfo[] { language, modes[], layers[] }

// Legacy (keep during migration):
fn tokenize_json(source, mode, layer) -> String
  ≡ analyze({ language: "json", mode, layer, source })

// Discover engines only among compile-time linked features.
```

CST / AST / navigation stay **Rust-only** behind dyn/typed APIs; the JSON bridge remains highlight + validate + diagnostics.

---

## 8. Language catalog

### 8.1 Id conventions

- Lowercase ASCII, words fully spelled: `javascript`, `typescript`, `csharp`, `cpp`, `powershell`
- Prefer full names over abbreviations (`javascript` not `js`; registry may resolve alias `js`)
- Dialects are not separate crates

### 8.2 Include list

| language_id | Crate (dir) | Origin | Wave | Rationale |
|-------------|-------------|--------|------|-----------|
| `json` | tokenizer-json | existing | 0 | Full reference engine; dialects `strict`, `jsonc` |
| `url` | tokenizer-url | existing | 0 | Elevate to structured parse; keep tokenize/validate adapters |
| `xml` | tokenizer-xml | user formats | 1 | Markup family; well-formed + recovery |
| `html` | tokenizer-html | user + web | 1 | HTML5 recovery subset; separate from xml |
| `css` | tokenizer-css | user | 1 | Stylesheet engine |
| `yaml` | tokenizer-yaml | editor formats | 1 | Recommended config format |
| `toml` | tokenizer-toml | editor formats | 1 | Recommended config format |
| `sql` | tokenizer-sql | user + TIOBE | 2 | Dialect matrix (ansi, postgres, …) via options |
| `mongo` | tokenizer-mongo | user | 2 | MQL / aggregation / shell profiles |
| `bash` | tokenizer-bash | user | 2 | POSIX / bash line-continuation model |
| `powershell` | tokenizer-powershell | user | 2 | Separate shell dialect |
| `javascript` | tokenizer-javascript | user + TIOBE | 3 | Web scripting |
| `typescript` | tokenizer-typescript | user + top | 3 | Separate crate; may share JS lex utils later |
| `markdown` | tokenizer-markdown | editor formats | 3 | Doc / playground content |
| `python` | tokenizer-python | user + TIOBE | 4 | App languages wave |
| `java` | tokenizer-java | user + TIOBE | 4 | |
| `csharp` | tokenizer-csharp | user + TIOBE | 4 | |
| `go` | tokenizer-go | industry / top | 4 | |
| `php` | tokenizer-php | top | 4 | |
| `ruby` | tokenizer-ruby | top | 4 | |
| `c` | tokenizer-c | TIOBE top | 5 | Systems; no mega C-family crate |
| `cpp` | tokenizer-cpp | TIOBE top | 5 | Sibling of c, not the same crate |
| `rust` | tokenizer-rust | user + TIOBE | 5 | |
| `kotlin` | tokenizer-kotlin | top | 5 | |
| `swift` | tokenizer-swift | top | 5 | |
| `dart` | tokenizer-dart | top | 5 | |
| `r` | tokenizer-r | TIOBE top | 5 | |

TIOBE ranks shift; this catalog is product priority, not a live index.

### 8.3 Exclude / defer

| Id | Why |
|----|-----|
| `scratch` | Block language; not a text syntax-engine target |
| `visualbasic` / VB | Huge surface, low request priority |
| `delphi` | Same |
| `objective-c` | Defer; c/swift story first |
| `cobol`, `fortran`, `matlab` | Not requested; can re-evaluate later |
| tree-sitter / syntect as core | Conflicts with zero-dep ownership |

### 8.4 Delivery waves

| Wave | Goal | Languages |
|------|------|-----------|
| **0** | Contract + packaging | core, facade, migrate `json`, migrate `url`, multi-lang bridge |
| **1** | Markup / data | `xml`, `html`, `css`, `yaml`, `toml` |
| **2** | Queries / shells | `sql`, `mongo`, `bash`, `powershell` |
| **3** | Web scripting | `javascript`, `typescript`, `markdown` |
| **4** | Popular app langs | `python`, `java`, `csharp`, `go`, `php`, `ruby` |
| **5** | Systems / mobile | `c`, `cpp`, `rust`, `kotlin`, `swift`, `dart`, `r` |

**Definition of done per language:** lossless lex, recovering parse + borrowing AST (subset OK if documented in `DIALECT_SCOPE`), semantic tokens, stable diagnostic codes, fixtures, host adapter, dialect-scope README. Do not claim “full Python” until fixtures say so.

### 8.5 Family helpers (later, non-blocking)

| Family | Crate | Role |
|--------|-------|------|
| Markup | `tokenizer-core::markup` or thin helper | Shared markup kinds / tag match after second of html/xml |
| C-family | `tokenizer-c-family` | Pure scanners only; c/cpp/csharp/java never depend on each other |
| Shell | shared line model | Physical→logical lines; bash vs powershell stay separate engines |
| SQL/Mongo | dialect profiles in-language | `DialectId` + feature bitsets; not one crate per vendor |

---

## 9. Hard rules

1. **No dynamic plugins.** No `dlopen`, `libloading`, process plugin ABI, or runtime download of engines.
2. **Zero runtime deps** on core and language engines (std/alloc only; optional host adapters separate).
3. **UTF-8 byte spans only** inside engines. UTF-16 / line-column conversion is host-side via `LineIndex`.
4. **Lossless full token streams** for every accepted input (empty, malformed, incomplete, truncated).
5. **Token text is never owned** as the source of truth; always `source[span]`.
6. **No cross-language mega-enum** of syntax kinds in core.
7. **Recovering parse bundles**, not `Result` that drops partial trees, for editor engines.
8. **Capability honesty:** never advertise a bit you cannot serve; refuse missing layers explicitly.
9. **Dialect ≠ LanguageId** when one engine implements both (json/jsonc, ts/tsx).
10. **Single document purity:** engines do no filesystem, network, or multi-buffer resolution.
11. **Legacy JSON highlighter** (`Tokenizer`, `TokenKind`, `tokenize_json`) stays frozen; do not force new languages into it.
12. **Wire kind/code names** are explicit kebab-case `as_str()` / `code()`, never `Debug`.
13. **Send + Sync + 'static** on engines; analysis is reentrant on `&self`.
14. **Features are additive**; renaming a feature or language id is a breaking change.
15. **Forbidden API shapes:** `PluginLoader::load_so`, owned token text, UTF-16 primary spans, language cases in core enums, mutable shared AST, global/linkme registration.

---

## 10. Deferred items

| Item | Notes |
|------|-------|
| Streaming / chunked lex | Optional traits later; full-buffer oracle remains |
| Incremental relex / reparse | `PreferIncremental` may exist as request; v1 guarantees only Full |
| Multi-file / imports / VFS | Host-only; optional pure `ImportSurface` extract later |
| Green/red tree reuse | CST is AST-backed snapshot; IDs die on reparse |
| Comment attachment map for formatters | Pure post-pass over CST; not required for wave 0 |
| `no_std` without alloc | Engines need `alloc`; pure `no_std`+alloc is optional later CI goal |
| Grapheme-cluster columns | Out of scope; scalars / UTF-16 / UTF-8 only |
| Percent-decode / IDNA in URL AST | Raw slices first; host-side decode |
| Semantic token modifiers (LSP) | Kind-only until a language needs bits |
| Shared markup / c-family crates | After rule-of-three consumers |
| Schema / type-checking / eval | Explicit non-goals of language engines |
| Empty stub crates for all catalog ids | Forbidden until each language PR |

---

## 11. Migration PR order (milestone 0)

### PR1 — `tokenizer-core`

- Extract `Span`, `LineIndex` / positions, host-facing `Diagnostic`, `LanguageId` / `DialectId` / `LanguageDescriptor`
- `Capabilities`, `InputLimits`, lossless helpers, `TextEdit` / `apply_edits`
- `HostLanguage` skeleton, `RegistryBuilder` / `LanguageRegistry`
- Zero language deps; tests for span/lossless/registry only

### PR2 — move JSON

- `crates/tokenizer-json` with today’s full engine
- Facade re-exports `json::*` and root legacy (`tokenize_json`, `JsonTokenizer`, `TokenKind`, …)
- `JsonEngine` + `JsonHostLanguage` + `DESCRIPTOR` with `JSON_FULL` capabilities
- Tests / benches / fixtures green; public paths type-identical

### PR3 — move URL

- `crates/tokenizer-url`; feature `url`
- Preserve `tokenize` / `validate` / root re-exports
- Start structured `url::Parse` / `Url` AST elevation (scheme, authority, path, query)
- Advertise honest capabilities (highlight + validate; grow as layers land)

### PR4 — multi-language host bridge

- Bridge request with `language` + `mode` + `layer`; legacy `tokenize_json` wrapper
- WASM `analyze` / `list_engines`; playground language picker for `json` + `url`
- `register_builtins` cfg-gated on features
- Docs: this design file is the contract

### PR5+ — catalog waves

- One language per PR from wave 1 onward
- No empty stubs; each PR ships real lex (at least) + dialect scope statement + fixtures + host adapter

### Wave 0 success criteria

- Workspace builds: core, json, url, facade, wasm
- Existing JSON public API works via facade
- Playground can select `json` and `url`
- `docs/plugin-api-design.md` is the API source of truth
- No wave 1+ crates until their PRs

---

## 12. Testing contract (summary)

```text
tests/fixtures/<language_id>/{valid,invalid,recover}/…
```

Shared harness (core or path-dev `tokenizer-test`):

- **LosslessOracle** on every lex/semantic full stream
- **ConformanceSuite** accept/reject + span safety + optional diagnostic codes
- **PropertyRunner** deterministic seed, panic-free, optional language-local differential oracle (e.g. `serde_json` for strict JSON only)
- Dyn smoke: `EngineSmoke::against_dyn` on host facade tokens

Language crates supply adapter + fixtures; core harness does not depend on language crates.

---

## 13. Documentation contract per language crate

Every language crate exports:

```text
pub const LANGUAGE_ID: LanguageId;
pub const DIALECT_SCOPE: DialectScope;  // base standard, profiles, in/out of scope, capability tier
```

Human docs mirror: base standard, profiles, in scope, out of scope, non-goals, capability tier, spans and recovery. Changing in/out-of-scope grammar is semver-major.

---

## 14. Performance budget (API-level)

- Tokens are span + kind (+ flags); **no lexeme `String`**
- `WorkHints` may skip optional work (string decode, AST, CST) without breaking lossless cover
- Hard ceilings stay in `InputLimits` (truncate/recover with diagnostics)
- Highlight path: `WorkHints::highlight_only()` / layer=`syntax` cheap; semantic may parse
- Diagnostic codes/messages prefer `&'static str` on hot paths

---

## 15. Anti-patterns (deny list)

| Forbidden | Prefer |
|-----------|--------|
| `load_so` / dlopen plugins | Cargo features + `register_builtins` |
| Core `SyntaxKind::JsonString` mega-enum | Language-local kinds + `RawSyntaxKind` |
| UTF-16 spans in engines | `Span` UTF-8 + host `LineIndex` |
| Owned token text | `source.slice(span)` |
| `parse → Result` dropping tree | Recovering `Parse` bundle + diagnostics |
| New languages on legacy `Tokenizer` | `HostLanguage` / typed `LanguageEngine` |
| Global/linkme engine registration | Explicit `RegistryBuilder` |
| Empty stub crates for all ids | Ship engines only when ready |
| Claiming TIOBE “supported” before fixtures | Capability + dialect scope honesty |

---

## 16. Open points (resolve during PR1–PR4)

1. Publish core + each language on crates.io vs facade-only initially.
2. Umbrella default features: `["json"]` only vs `["json","url"]` (today both are public; lean default is `json`, url opt-in).
3. URL capability bits while elevating (LEX vs SEMANTIC naming).
4. Error code wire form: `json/expected-value` vs legacy unprefixed aliases duration.
5. Whether `DocumentId` lives in core now or stays host-private until multi-buffer needs arise.
6. Owned vs borrowed dyn parse views for long-lived playground snapshots.

---

## 17. Synthesis notes

This contract consolidates recommendations from completed API slices (spans, diagnostics, language id, capabilities, lex, parse, AST, semantic, CST, navigation, visitor, edits, limits, lossless, host facade, registry, WASM bridge, kind naming, error codes, legacy surface, shared scan utils, markup/C-family/shell/SQL-mongo boundaries, URL elevation, thread-safety, no_std policy, streaming deferral, multi-file out-of-scope, testing, re-exports, features, dialect docs, performance, recovery flags, BOM policy, comment attachment, anti-patterns) against the locked product decisions in the session plan.

Conflicts were resolved toward: zero-dep core, UTF-8 spans, lossless tokens, Cargo plugins, typed engines + dyn host, JSON-depth with capability flags, and legacy path re-exports.
```
