//! FHIRPath recursive-descent parser: `&[Token]` → typed `Expr` tree.
//!
//! The parser produces the typed `Expr` enum from `crate::ast`. The lowering
//! layer in `crate::compat::lower` walks `Expr` and produces the legacy
//! `AstNode` ANTLR-shaped tree consumed by the bindings, the analyze passes,
//! and `resolve.rs`.
//!
//! Each `Expr`/`Invocation`/`Identifier`/etc. carries a `Span` covering both
//! the byte range and the half-open token-index range `[token_start..token_end)`.
//! The lowering layer relies on the token range to populate
//! `AstNode::token_start/token_end`, which `compat::compute_text` uses to
//! reconstruct the legacy `text` field.

use crate::ast::{
    BinOp, Expr, ExternalConstantId, Identifier, Invocation, Literal, QualifiedIdentifier,
    TypeOp, TypeSpecifier, Unit, UnaryOp,
};
use crate::lexer::{Token, TokenKind};
use crate::{ParseError, Span};

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> TokenKind {
        self.tokens[self.pos].kind
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        self.pos += 1;
        tok
    }

    fn expect(&mut self, kind: TokenKind) -> Result<&Token, ParseError> {
        if self.peek() == kind {
            Ok(self.advance())
        } else {
            let tok = self.current();
            Err(ParseError::UnexpectedToken {
                expected: kind,
                found: tok.kind,
                found_text: tok.text.clone(),
                span: self.token_span(tok),
            })
        }
    }

    /// Build a `Span` covering a single token (used for error diagnostics).
    fn token_span(&self, tok: &Token) -> Span {
        Span {
            byte_start: tok.byte_start,
            byte_end: tok.byte_end,
            token_start: 0,
            token_end: 0,
        }
    }

    /// Build a `Span` covering tokens `[token_start..self.pos)`. Assumes at
    /// least one token has been consumed since `token_start`.
    fn span_from(&self, token_start: usize) -> Span {
        let token_end = self.pos;
        let byte_start = self.tokens[token_start].byte_start;
        let byte_end = self.tokens[token_end.saturating_sub(1)].byte_end;
        Span {
            byte_start,
            byte_end,
            token_start,
            token_end,
        }
    }

    // ── Entry point ─────────────────────────────────────────────────────

    pub fn parse_entire_expression(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_expression()?;
        if self.peek() != TokenKind::Eof {
            let tok = self.current();
            return Err(ParseError::UnexpectedTokenAfterExpr {
                found: tok.kind,
                found_text: tok.text.clone(),
                span: self.token_span(tok),
            });
        }
        Ok(expr)
    }

    // ── Expression precedence chain (lowest → highest) ──────────────────

    fn parse_expression(&mut self) -> Result<Expr, ParseError> {
        self.parse_implies()
    }

    fn parse_implies(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(&[(TokenKind::Implies, BinOp::Implies)], Self::parse_or)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[(TokenKind::Or, BinOp::Or), (TokenKind::Xor, BinOp::Xor)],
            Self::parse_and,
        )
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(&[(TokenKind::And, BinOp::And)], Self::parse_membership)
    }

    fn parse_membership(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[
                (TokenKind::In, BinOp::In),
                (TokenKind::Contains, BinOp::Contains),
            ],
            Self::parse_equality,
        )
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[
                (TokenKind::Eq, BinOp::Eq),
                (TokenKind::NotEq, BinOp::NotEq),
                (TokenKind::Tilde, BinOp::EquivTilde),
                (TokenKind::NotTilde, BinOp::NotEquivTilde),
            ],
            Self::parse_type,
        )
    }

    fn parse_type(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let mut left = self.parse_inequality()?;
        while self.peek() == TokenKind::Is || self.peek() == TokenKind::As {
            let op = match self.peek() {
                TokenKind::Is => TypeOp::Is,
                TokenKind::As => TypeOp::As,
                _ => unreachable!(),
            };
            self.advance();
            let type_spec = self.parse_type_specifier()?;
            let span = self.span_from(token_start);
            left = Expr::Type {
                lhs: Box::new(left),
                op,
                type_spec,
                span,
            };
        }
        Ok(left)
    }

    fn parse_inequality(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[
                (TokenKind::Lt, BinOp::Lt),
                (TokenKind::Gt, BinOp::Gt),
                (TokenKind::LtEq, BinOp::LtEq),
                (TokenKind::GtEq, BinOp::GtEq),
            ],
            Self::parse_union,
        )
    }

    fn parse_union(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(&[(TokenKind::Pipe, BinOp::Union)], Self::parse_additive)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[
                (TokenKind::Plus, BinOp::Plus),
                (TokenKind::Minus, BinOp::Minus),
                (TokenKind::Ampersand, BinOp::Concat),
            ],
            Self::parse_multiplicative,
        )
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        self.parse_binary_left(
            &[
                (TokenKind::Star, BinOp::Mul),
                (TokenKind::Slash, BinOp::TrueDiv),
                (TokenKind::Div, BinOp::IntDiv),
                (TokenKind::Mod, BinOp::Mod),
            ],
            Self::parse_unary,
        )
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == TokenKind::Plus || self.peek() == TokenKind::Minus {
            let token_start = self.pos;
            let op = match self.peek() {
                TokenKind::Plus => UnaryOp::Plus,
                TokenKind::Minus => UnaryOp::Minus,
                _ => unreachable!(),
            };
            self.advance();
            let operand = self.parse_unary()?;
            let span = self.span_from(token_start);
            Ok(Expr::Polarity {
                op,
                operand: Box::new(operand),
                span,
            })
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let mut left = self.parse_term()?;
        loop {
            if self.peek() == TokenKind::Dot {
                self.advance();
                let call = self.parse_invocation()?;
                let span = self.span_from(token_start);
                left = Expr::Invocation {
                    receiver: Some(Box::new(left)),
                    call,
                    span,
                };
            } else if self.peek() == TokenKind::LBracket {
                self.advance();
                let index = self.parse_expression()?;
                self.expect(TokenKind::RBracket)?;
                let span = self.span_from(token_start);
                left = Expr::Indexer {
                    receiver: Box::new(left),
                    index: Box::new(index),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        // The legacy AST wrapped every term in `TermExpression`. Lowering does
        // that now; the parser just returns the inner Expr.
        self.parse_term_inner()
    }

    fn parse_term_inner(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            TokenKind::LParen => self.parse_parenthesized_term(),
            // ANTLR error recovery treats [expr] like (expr) in term position.
            // We replicate this so expressions like `intersect([list])` work.
            TokenKind::LBracket => self.parse_bracket_term(),
            TokenKind::LBrace => self.parse_null_literal_term(),
            TokenKind::True | TokenKind::False => self.parse_boolean_literal_term(),
            TokenKind::String => self.parse_string_literal_term(),
            TokenKind::Number => self.parse_number_or_quantity_literal_term(),
            TokenKind::DateTime => self.parse_datetime_literal_term(),
            TokenKind::Time => self.parse_time_literal_term(),
            TokenKind::Percent => self.parse_external_constant_term(),
            TokenKind::Identifier
            | TokenKind::DelimitedIdentifier
            | TokenKind::As
            | TokenKind::Is
            | TokenKind::Contains
            | TokenKind::In
            | TokenKind::DollarThis
            | TokenKind::DollarIndex
            | TokenKind::DollarTotal => self.parse_invocation_term(),
            _ => {
                let tok = self.current();
                Err(ParseError::UnexpectedTokenInTerm {
                    found: tok.kind,
                    found_text: tok.text.clone(),
                    span: self.token_span(tok),
                })
            }
        }
    }

    // ── Term variants ───────────────────────────────────────────────────

    fn parse_parenthesized_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        self.advance(); // (
        let inner = self.parse_expression()?;
        self.expect(TokenKind::RParen)?;
        let span = self.span_from(token_start);
        Ok(Expr::Parenthesized {
            inner: Box::new(inner),
            span,
        })
    }

    /// Handle `[expr]` in term position — ANTLR error recovery compatibility.
    /// The brackets are dropped (they end up in the parent's terminalNodeText
    /// in ANTLR but that's benign). The inner expression is returned directly,
    /// without any wrapping.
    fn parse_bracket_term(&mut self) -> Result<Expr, ParseError> {
        self.advance(); // skip '['
        let inner = self.parse_term_inner()?;
        if self.peek() == TokenKind::RBracket {
            self.advance(); // skip ']'
        }
        Ok(inner)
    }

    fn parse_null_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        self.advance(); // {
        self.expect(TokenKind::RBrace)?;
        let span = self.span_from(token_start);
        Ok(Expr::Literal(Literal::Null { span }))
    }

    fn parse_boolean_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let value = self.peek() == TokenKind::True;
        self.advance();
        let span = self.span_from(token_start);
        Ok(Expr::Literal(Literal::Boolean { value, span }))
    }

    fn parse_string_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let raw = self.advance().text.clone();
        let span = self.span_from(token_start);
        Ok(Expr::Literal(Literal::String { raw, span }))
    }

    fn parse_datetime_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let raw = self.advance().text.clone();
        let span = self.span_from(token_start);
        Ok(Expr::Literal(Literal::DateTime { raw, span }))
    }

    fn parse_time_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let raw = self.advance().text.clone();
        let span = self.span_from(token_start);
        Ok(Expr::Literal(Literal::Time { raw, span }))
    }

    fn parse_number_or_quantity_literal_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let number_text = self.advance().text.clone();

        // Check if next token is a unit (string literal or datetime precision word)
        if self.is_unit_start() {
            let unit = self.parse_unit()?;
            let span = self.span_from(token_start);
            Ok(Expr::Literal(Literal::Quantity {
                number: number_text,
                unit,
                span,
            }))
        } else {
            let span = self.span_from(token_start);
            Ok(Expr::Literal(Literal::Number {
                raw: number_text,
                span,
            }))
        }
    }

    fn is_unit_start(&self) -> bool {
        match self.peek() {
            TokenKind::String => true,
            TokenKind::Identifier => {
                let text = &self.current().text;
                is_datetime_precision(text) || is_plural_datetime_precision(text)
            }
            _ => false,
        }
    }

    fn parse_unit(&mut self) -> Result<Unit, ParseError> {
        let token_start = self.pos;
        match self.peek() {
            TokenKind::String => {
                let raw = self.advance().text.clone();
                let span = self.span_from(token_start);
                Ok(Unit::String { raw, span })
            }
            TokenKind::Identifier => {
                let text = self.current().text.clone();
                if is_datetime_precision(&text) {
                    self.advance();
                    let span = self.span_from(token_start);
                    Ok(Unit::DateTimePrecision { word: text, span })
                } else if is_plural_datetime_precision(&text) {
                    self.advance();
                    let span = self.span_from(token_start);
                    Ok(Unit::PluralDateTimePrecision { word: text, span })
                } else {
                    let tok = self.current();
                    Err(ParseError::ExpectedUnit {
                        found_text: Some(text),
                        span: self.token_span(tok),
                    })
                }
            }
            _ => {
                let tok = self.current();
                Err(ParseError::ExpectedUnit {
                    found_text: None,
                    span: self.token_span(tok),
                })
            }
        }
    }

    fn parse_external_constant_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        self.advance(); // %
        // After %, expect identifier or string. The string-form uses the
        // string token's span (NOT the leading %); this matches the ANTLR
        // off-by-one where the inner Identifier's token_start sits AFTER the
        // `%` token.
        let ident = match self.peek() {
            TokenKind::String => {
                let str_token_start = self.pos;
                let raw = self.advance().text.clone();
                let span = self.span_from(str_token_start);
                ExternalConstantId::String { raw, span }
            }
            _ => ExternalConstantId::Identifier(self.parse_identifier()?),
        };
        let span = self.span_from(token_start);
        Ok(Expr::ExternalConstant { ident, span })
    }

    fn parse_invocation_term(&mut self) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let call = self.parse_invocation()?;
        let span = self.span_from(token_start);
        Ok(Expr::Invocation {
            receiver: None,
            call,
            span,
        })
    }

    // ── Invocation ──────────────────────────────────────────────────────

    fn parse_invocation(&mut self) -> Result<Invocation, ParseError> {
        match self.peek() {
            TokenKind::DollarThis => {
                let token_start = self.pos;
                self.advance();
                let span = self.span_from(token_start);
                Ok(Invocation::This { span })
            }
            TokenKind::DollarIndex => {
                let token_start = self.pos;
                self.advance();
                let span = self.span_from(token_start);
                Ok(Invocation::Index { span })
            }
            TokenKind::DollarTotal => {
                let token_start = self.pos;
                self.advance();
                let span = self.span_from(token_start);
                Ok(Invocation::Total { span })
            }
            TokenKind::Identifier
            | TokenKind::DelimitedIdentifier
            | TokenKind::As
            | TokenKind::Is
            | TokenKind::Contains
            | TokenKind::In => {
                let token_start = self.pos;
                let ident = self.parse_identifier()?;
                if self.peek() == TokenKind::LParen {
                    self.parse_function_invocation(token_start, ident)
                } else {
                    Ok(Invocation::Member { ident })
                }
            }
            _ => {
                let tok = self.current();
                Err(ParseError::ExpectedInvocation {
                    found: tok.kind,
                    found_text: tok.text.clone(),
                    span: self.token_span(tok),
                })
            }
        }
    }

    fn parse_function_invocation(
        &mut self,
        token_start: usize,
        name: Identifier,
    ) -> Result<Invocation, ParseError> {
        self.advance(); // (
        let mut args: Vec<Expr> = Vec::new();
        if self.peek() != TokenKind::RParen {
            args.push(self.parse_expression()?);
            while self.peek() == TokenKind::Comma {
                self.advance();
                args.push(self.parse_expression()?);
            }
        }
        self.expect(TokenKind::RParen)?;
        let span = self.span_from(token_start);
        Ok(Invocation::Function { name, args, span })
    }

    // ── Identifier ──────────────────────────────────────────────────────

    fn parse_identifier(&mut self) -> Result<Identifier, ParseError> {
        match self.peek() {
            TokenKind::Identifier
            | TokenKind::DelimitedIdentifier
            | TokenKind::As
            | TokenKind::Is
            | TokenKind::Contains
            | TokenKind::In => {
                let token_start = self.pos;
                let raw = self.advance().text.clone();
                let span = self.span_from(token_start);
                Ok(Identifier { raw, span })
            }
            _ => {
                let tok = self.current();
                Err(ParseError::ExpectedIdentifier {
                    found: tok.kind,
                    found_text: tok.text.clone(),
                    span: self.token_span(tok),
                })
            }
        }
    }

    // ── TypeSpecifier ───────────────────────────────────────────────────

    fn parse_type_specifier(&mut self) -> Result<TypeSpecifier, ParseError> {
        let token_start = self.pos;
        let qualified = self.parse_qualified_identifier()?;
        let span = self.span_from(token_start);
        Ok(TypeSpecifier { qualified, span })
    }

    fn parse_qualified_identifier(&mut self) -> Result<QualifiedIdentifier, ParseError> {
        let token_start = self.pos;
        let mut parts = Vec::new();
        parts.push(self.parse_identifier()?);
        while self.peek() == TokenKind::Dot {
            self.advance();
            parts.push(self.parse_identifier()?);
        }
        let span = self.span_from(token_start);
        Ok(QualifiedIdentifier { parts, span })
    }

    // ── DRY binary left-associative helper ──────────────────────────────

    fn parse_binary_left(
        &mut self,
        ops: &[(TokenKind, BinOp)],
        next: fn(&mut Self) -> Result<Expr, ParseError>,
    ) -> Result<Expr, ParseError> {
        let token_start = self.pos;
        let mut left = next(self)?;
        loop {
            let op = match ops.iter().find(|(k, _)| *k == self.peek()) {
                Some((_, op)) => *op,
                None => break,
            };
            self.advance();
            let right = next(self)?;
            let span = self.span_from(token_start);
            left = Expr::Binary {
                op,
                lhs: Box::new(left),
                rhs: Box::new(right),
                span,
            };
        }
        Ok(left)
    }
}

fn is_datetime_precision(s: &str) -> bool {
    matches!(
        s,
        "year" | "month" | "week" | "day" | "hour" | "minute" | "second" | "millisecond"
    )
}

fn is_plural_datetime_precision(s: &str) -> bool {
    matches!(
        s,
        "years" | "months" | "weeks" | "days" | "hours" | "minutes" | "seconds" | "milliseconds"
    )
}

