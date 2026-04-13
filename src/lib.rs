pub mod bindings;
pub mod lexer;
pub mod parser;

pub use lexer::{Token, TokenKind};
pub use parser::AstNode;

use lexer::tokenize;
use parser::Parser;

/// Parse error type.
#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FHIRPath parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a FHIRPath expression string into an AST.
pub fn parse(expr: &str) -> Result<AstNode, ParseError> {
    let tokens = tokenize(expr).map_err(ParseError)?;
    let mut p = Parser::new(&tokens);
    p.parse_entire_expression().map_err(ParseError)
}

// Re-export the PyO3 module entry point when the python feature is enabled.
#[cfg(feature = "python")]
pub use bindings::python::_rust;
