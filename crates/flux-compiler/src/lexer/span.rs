//! Span and SpannedToken definitions for source location tracking

use super::Token;

/// Represents a byte range in source code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start byte offset (inclusive)
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
}

impl Span {
    /// Create a new span from start to end byte offsets
    ///
    /// # Panics
    ///
    /// Debug-asserts that `start <= end`
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "Span start ({start}) must be <= end ({end})");
        Self { start, end }
    }

    /// Returns the length of the span in bytes
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns true if the span covers zero bytes
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A token paired with its source location span
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    /// The token value
    pub token: Token,
    /// The source location span
    pub span: Span,
}
