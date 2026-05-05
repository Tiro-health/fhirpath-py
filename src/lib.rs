pub mod analyze;
pub mod bindings;
pub mod lexer;
pub mod parser;
pub mod resolve;
pub mod utf16;

pub use lexer::{Token, TokenKind};
pub use parser::AstNode;

use lexer::tokenize;
use parser::Parser;

/// Byte offset range into the source string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Typed parse error for the FHIRPath lexer + parser.
///
/// `Display` produces a human-readable string suitable for surfacing to
/// callers; bindings call `.to_string()` on this to forward the message.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// Lexer hit a character it doesn't recognize at all.
    UnexpectedChar { ch: char, byte_pos: usize },
    /// Parser expected a specific kind but found something else.
    UnexpectedToken {
        expected: TokenKind,
        found: TokenKind,
        found_text: String,
        span: Span,
    },
    /// Parser is in term position and saw a token that can't start a term.
    UnexpectedTokenInTerm {
        found: TokenKind,
        found_text: String,
        span: Span,
    },
    /// Parser finished an expression but more tokens remain.
    UnexpectedTokenAfterExpr {
        found: TokenKind,
        found_text: String,
        span: Span,
    },
    /// Parser expected an identifier but found something else.
    ExpectedIdentifier {
        found: TokenKind,
        found_text: String,
        span: Span,
    },
    /// Parser expected an invocation but found something else.
    ExpectedInvocation {
        found: TokenKind,
        found_text: String,
        span: Span,
    },
    /// Lexer hit EOF inside a `/* … */` comment.
    UnterminatedBlockComment { byte_pos: usize },
    /// Lexer hit EOF inside a `'…'` or `` `…` `` literal.
    UnterminatedQuoted { quote: char, byte_pos: usize },
    /// Lexer saw `$` followed by something other than `this`/`index`/`total`.
    UnknownDollarVariable { byte_pos: usize },
    /// Parser was looking for a quantity unit but found something else.
    /// `found_text` is `None` when no token at all was usable (the original
    /// "Expected unit" message).
    ExpectedUnit {
        found_text: Option<String>,
        span: Span,
    },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FHIRPath parse error: ")?;
        match self {
            ParseError::UnexpectedChar { ch, byte_pos } => {
                write!(f, "Unexpected character {ch:?} at position {byte_pos}")
            }
            ParseError::UnexpectedToken {
                expected,
                found,
                found_text,
                ..
            } => {
                write!(
                    f,
                    "Expected {expected:?} but found {found:?} ({found_text:?})"
                )
            }
            ParseError::UnexpectedTokenInTerm {
                found, found_text, ..
            } => {
                write!(f, "Unexpected token in term: {found:?} ({found_text:?})")
            }
            ParseError::UnexpectedTokenAfterExpr {
                found, found_text, ..
            } => {
                write!(
                    f,
                    "Unexpected token after expression: {found:?} ({found_text:?})"
                )
            }
            ParseError::ExpectedIdentifier {
                found, found_text, ..
            } => {
                write!(
                    f,
                    "Expected identifier but found {found:?} ({found_text:?})"
                )
            }
            ParseError::ExpectedInvocation {
                found, found_text, ..
            } => {
                write!(
                    f,
                    "Expected invocation but found {found:?} ({found_text:?})"
                )
            }
            ParseError::UnterminatedBlockComment { .. } => {
                write!(f, "Unterminated block comment")
            }
            ParseError::UnterminatedQuoted { quote, byte_pos } => {
                write!(f, "Unterminated {quote} literal starting at position {byte_pos}")
            }
            ParseError::UnknownDollarVariable { byte_pos } => {
                write!(f, "Unknown $ variable at position {byte_pos}")
            }
            ParseError::ExpectedUnit { found_text, .. } => match found_text {
                Some(t) => write!(f, "Expected unit but found identifier {t:?}"),
                None => write!(f, "Expected unit"),
            },
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a FHIRPath expression string into an AST.
pub fn parse(expr: &str) -> Result<AstNode, ParseError> {
    let tokens = tokenize(expr)?;
    let mut p = Parser::new(&tokens);
    p.parse_entire_expression()
}

// Re-export the PyO3 module entry point when the python feature is enabled.
#[cfg(feature = "python")]
pub use bindings::python::_rust;
