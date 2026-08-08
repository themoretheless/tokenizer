//! Shared input limits for language engines.

/// Hard caps applied before or during analysis.
///
/// Zero is never "unlimited"; use large explicit values when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputLimits {
    pub max_input_bytes: usize,
    pub max_tokens: usize,
    pub max_diagnostics: usize,
    pub max_depth: usize,
}

impl InputLimits {
    /// Conservative defaults suitable for editor documents.
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_tokens: 1_000_000,
            max_diagnostics: 256,
            max_depth: 128,
        }
    }

    #[must_use]
    pub const fn max_input_bytes(mut self, n: usize) -> Self {
        self.max_input_bytes = n;
        self
    }

    #[must_use]
    pub const fn max_tokens(mut self, n: usize) -> Self {
        self.max_tokens = n;
        self
    }

    #[must_use]
    pub const fn max_diagnostics(mut self, n: usize) -> Self {
        self.max_diagnostics = n;
        self
    }

    #[must_use]
    pub const fn max_depth(mut self, n: usize) -> Self {
        self.max_depth = n;
        self
    }

    /// Whether `source` exceeds the byte budget.
    #[must_use]
    pub const fn exceeds_input_bytes(self, source_len: usize) -> bool {
        source_len > self.max_input_bytes
    }
}

impl Default for InputLimits {
    fn default() -> Self {
        Self::conservative()
    }
}

/// A limit was hit while analyzing input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LimitExceeded {
    InputBytes { max: usize, actual: usize },
    Tokens { max: usize },
    Diagnostics { max: usize },
    Depth { max: usize },
}

impl core::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InputBytes { max, actual } => {
                write!(f, "input is {actual} bytes; max is {max}")
            }
            Self::Tokens { max } => write!(f, "token limit {max} exceeded"),
            Self::Diagnostics { max } => write!(f, "diagnostic limit {max} exceeded"),
            Self::Depth { max } => write!(f, "nesting depth limit {max} exceeded"),
        }
    }
}

impl std::error::Error for LimitExceeded {}
