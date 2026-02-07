/// FHIRPath recursive-descent parser: &[Token] → AstNode tree

use crate::lexer::{Token, TokenKind};

#[derive(Debug, Clone)]
pub struct AstNode {
    pub node_type: &'static str,
    pub terminal_node_text: Vec<String>,
    pub children: Vec<AstNode>,
    /// Index range [token_start..token_end] into the token vec (for text computation).
    pub token_start: usize,
    pub token_end: usize,
}

impl AstNode {
    fn new(node_type: &'static str, token_start: usize) -> Self {
        AstNode {
            node_type,
            terminal_node_text: Vec::new(),
            children: Vec::new(),
            token_start,
            token_end: token_start,
        }
    }
}

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        self.pos += 1;
        tok
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<&Token, String> {
        if self.peek() == kind {
            Ok(self.advance())
        } else {
            Err(format!(
                "Expected {:?} but found {:?} ({:?})",
                kind,
                self.peek(),
                self.current().text
            ))
        }
    }

    // ── Entry point ─────────────────────────────────────────────────────

    pub fn parse_entire_expression(&mut self) -> Result<AstNode, String> {
        let expr = self.parse_expression()?;
        if *self.peek() != TokenKind::Eof {
            return Err(format!(
                "Unexpected token after expression: {:?} ({:?})",
                self.peek(),
                self.current().text
            ));
        }
        Ok(expr)
    }

    // ── Expression precedence chain (lowest → highest) ──────────────────

    fn parse_expression(&mut self) -> Result<AstNode, String> {
        self.parse_implies()
    }

    fn parse_implies(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left("ImpliesExpression", &[TokenKind::Implies], Self::parse_or)
    }

    fn parse_or(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left("OrExpression", &[TokenKind::Or, TokenKind::Xor], Self::parse_and)
    }

    fn parse_and(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left("AndExpression", &[TokenKind::And], Self::parse_membership)
    }

    fn parse_membership(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left(
            "MembershipExpression",
            &[TokenKind::In, TokenKind::Contains],
            Self::parse_equality,
        )
    }

    fn parse_equality(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left(
            "EqualityExpression",
            &[TokenKind::Eq, TokenKind::NotEq, TokenKind::Tilde, TokenKind::NotTilde],
            Self::parse_type,
        )
    }

    fn parse_type(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let mut left = self.parse_inequality()?;
        while *self.peek() == TokenKind::Is || *self.peek() == TokenKind::As {
            let op_tok = self.advance().clone();
            let type_spec = self.parse_type_specifier()?;
            let mut node = AstNode::new("TypeExpression", start);
            node.terminal_node_text.push(op_tok.text.clone());
            node.children.push(left);
            node.children.push(type_spec);
            node.token_end = self.pos;
            left = node;
        }
        Ok(left)
    }

    fn parse_inequality(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left(
            "InequalityExpression",
            &[TokenKind::Lt, TokenKind::Gt, TokenKind::LtEq, TokenKind::GtEq],
            Self::parse_union,
        )
    }

    fn parse_union(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left("UnionExpression", &[TokenKind::Pipe], Self::parse_additive)
    }

    fn parse_additive(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left(
            "AdditiveExpression",
            &[TokenKind::Plus, TokenKind::Minus, TokenKind::Ampersand],
            Self::parse_multiplicative,
        )
    }

    fn parse_multiplicative(&mut self) -> Result<AstNode, String> {
        self.parse_binary_left(
            "MultiplicativeExpression",
            &[TokenKind::Star, TokenKind::Slash, TokenKind::Div, TokenKind::Mod],
            Self::parse_unary,
        )
    }

    fn parse_unary(&mut self) -> Result<AstNode, String> {
        if *self.peek() == TokenKind::Plus || *self.peek() == TokenKind::Minus {
            let start = self.pos;
            let op_tok = self.advance().clone();
            let operand = self.parse_unary()?;
            let mut node = AstNode::new("PolarityExpression", start);
            node.terminal_node_text.push(op_tok.text.clone());
            node.children.push(operand);
            node.token_end = self.pos;
            Ok(node)
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let mut left = self.parse_term()?;
        loop {
            if *self.peek() == TokenKind::Dot {
                let dot = self.advance().clone();
                let inv = self.parse_invocation()?;
                let mut node = AstNode::new("InvocationExpression", start);
                node.terminal_node_text.push(dot.text.clone());
                node.children.push(left);
                node.children.push(inv);
                node.token_end = self.pos;
                left = node;
            } else if *self.peek() == TokenKind::LBracket {
                let lb = self.advance().clone();
                let index_expr = self.parse_expression()?;
                let rb = self.expect(&TokenKind::RBracket)?.clone();
                let mut node = AstNode::new("IndexerExpression", start);
                node.terminal_node_text.push(lb.text.clone());
                node.terminal_node_text.push(rb.text.clone());
                node.children.push(left);
                node.children.push(index_expr);
                node.token_end = self.pos;
                left = node;
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let inner = self.parse_term_inner()?;
        let mut term_expr = AstNode::new("TermExpression", start);
        term_expr.children.push(inner);
        term_expr.token_end = self.pos;
        Ok(term_expr)
    }

    fn parse_term_inner(&mut self) -> Result<AstNode, String> {
        match self.peek().clone() {
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
            _ => Err(format!(
                "Unexpected token in term: {:?} ({:?})",
                self.peek(),
                self.current().text
            )),
        }
    }

    // ── Term variants ───────────────────────────────────────────────────

    fn parse_parenthesized_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let lp = self.advance().clone();
        let expr = self.parse_expression()?;
        let rp = self.expect(&TokenKind::RParen)?.clone();
        let mut node = AstNode::new("ParenthesizedTerm", start);
        node.terminal_node_text.push(lp.text.clone());
        node.terminal_node_text.push(rp.text.clone());
        node.children.push(expr);
        node.token_end = self.pos;
        Ok(node)
    }

    /// Handle `[expr]` in term position — ANTLR error recovery compatibility.
    /// The brackets are dropped (they end up in the parent's terminalNodeText in ANTLR
    /// but that's benign). The inner expression is returned directly.
    fn parse_bracket_term(&mut self) -> Result<AstNode, String> {
        self.advance(); // skip '['
        let inner = self.parse_term_inner()?;
        if *self.peek() == TokenKind::RBracket {
            self.advance(); // skip ']'
        }
        Ok(inner)
    }

    fn parse_null_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let lb = self.advance().clone();
        let rb = self.expect(&TokenKind::RBrace)?.clone();
        let mut literal = AstNode::new("NullLiteral", start);
        literal.terminal_node_text.push(lb.text.clone());
        literal.terminal_node_text.push(rb.text.clone());
        literal.token_end = self.pos;
        let mut term = AstNode::new("LiteralTerm", start);
        term.children.push(literal);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_boolean_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let tok = self.advance().clone();
        let mut literal = AstNode::new("BooleanLiteral", start);
        literal.terminal_node_text.push(tok.text.clone());
        literal.token_end = self.pos;
        let mut term = AstNode::new("LiteralTerm", start);
        term.children.push(literal);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_string_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let tok = self.advance().clone();
        let mut literal = AstNode::new("StringLiteral", start);
        literal.terminal_node_text.push(tok.text.clone());
        literal.token_end = self.pos;
        let mut term = AstNode::new("LiteralTerm", start);
        term.children.push(literal);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_number_or_quantity_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let num_tok = self.advance().clone();

        // Check if next token is a unit (string literal or datetime precision word)
        if self.is_unit_start() {
            let unit_node = self.parse_unit()?;
            let mut quantity = AstNode::new("Quantity", start);
            quantity.terminal_node_text.push(num_tok.text.clone());
            quantity.children.push(unit_node);
            quantity.token_end = self.pos;
            let mut ql = AstNode::new("QuantityLiteral", start);
            ql.children.push(quantity);
            ql.token_end = self.pos;
            let mut term = AstNode::new("LiteralTerm", start);
            term.children.push(ql);
            term.token_end = self.pos;
            Ok(term)
        } else {
            let mut literal = AstNode::new("NumberLiteral", start);
            literal.terminal_node_text.push(num_tok.text.clone());
            literal.token_end = self.pos;
            let mut term = AstNode::new("LiteralTerm", start);
            term.children.push(literal);
            term.token_end = self.pos;
            Ok(term)
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

    fn parse_unit(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let mut unit = AstNode::new("Unit", start);
        match self.peek() {
            TokenKind::String => {
                let tok = self.advance().clone();
                unit.terminal_node_text.push(tok.text.clone());
            }
            TokenKind::Identifier => {
                let text = self.current().text.clone();
                if is_datetime_precision(&text) {
                    let tok = self.advance().clone();
                    let mut dtp = AstNode::new("DateTimePrecision", start);
                    dtp.terminal_node_text.push(tok.text.clone());
                    dtp.token_end = self.pos;
                    unit.children.push(dtp);
                } else if is_plural_datetime_precision(&text) {
                    let tok = self.advance().clone();
                    let mut pdtp = AstNode::new("PluralDateTimePrecision", start);
                    pdtp.terminal_node_text.push(tok.text.clone());
                    pdtp.token_end = self.pos;
                    unit.children.push(pdtp);
                } else {
                    return Err(format!("Expected unit but found identifier {:?}", text));
                }
            }
            _ => return Err("Expected unit".into()),
        }
        unit.token_end = self.pos;
        Ok(unit)
    }

    fn parse_datetime_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let tok = self.advance().clone();
        let mut literal = AstNode::new("DateTimeLiteral", start);
        literal.terminal_node_text.push(tok.text.clone());
        literal.token_end = self.pos;
        let mut term = AstNode::new("LiteralTerm", start);
        term.children.push(literal);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_time_literal_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let tok = self.advance().clone();
        let mut literal = AstNode::new("TimeLiteral", start);
        literal.terminal_node_text.push(tok.text.clone());
        literal.token_end = self.pos;
        let mut term = AstNode::new("LiteralTerm", start);
        term.children.push(literal);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_external_constant_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let pct = self.advance().clone();
        // After %, expect identifier or string
        let child = match self.peek() {
            TokenKind::String => {
                let tok = self.advance().clone();
                let mut id = AstNode::new("Identifier", start + 1);
                id.terminal_node_text.push(tok.text.clone());
                id.token_end = self.pos;
                id
            }
            _ => self.parse_identifier()?,
        };
        let mut ext = AstNode::new("ExternalConstant", start);
        ext.terminal_node_text.push(pct.text.clone());
        ext.children.push(child);
        ext.token_end = self.pos;
        let mut term = AstNode::new("ExternalConstantTerm", start);
        term.children.push(ext);
        term.token_end = self.pos;
        Ok(term)
    }

    fn parse_invocation_term(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let inv = self.parse_invocation()?;
        let mut term = AstNode::new("InvocationTerm", start);
        term.children.push(inv);
        term.token_end = self.pos;
        Ok(term)
    }

    // ── Invocation ──────────────────────────────────────────────────────

    fn parse_invocation(&mut self) -> Result<AstNode, String> {
        match self.peek().clone() {
            TokenKind::DollarThis => {
                let start = self.pos;
                let tok = self.advance().clone();
                let mut node = AstNode::new("ThisInvocation", start);
                node.terminal_node_text.push(tok.text.clone());
                node.token_end = self.pos;
                Ok(node)
            }
            TokenKind::DollarIndex => {
                let start = self.pos;
                let tok = self.advance().clone();
                let mut node = AstNode::new("IndexInvocation", start);
                node.terminal_node_text.push(tok.text.clone());
                node.token_end = self.pos;
                Ok(node)
            }
            TokenKind::DollarTotal => {
                let start = self.pos;
                let tok = self.advance().clone();
                let mut node = AstNode::new("TotalInvocation", start);
                node.terminal_node_text.push(tok.text.clone());
                node.token_end = self.pos;
                Ok(node)
            }
            TokenKind::Identifier
            | TokenKind::DelimitedIdentifier
            | TokenKind::As
            | TokenKind::Is
            | TokenKind::Contains
            | TokenKind::In => {
                // Lookahead: identifier followed by '(' → function invocation
                let start = self.pos;
                let ident = self.parse_identifier()?;
                if *self.peek() == TokenKind::LParen {
                    self.parse_function_invocation(start, ident)
                } else {
                    let mut node = AstNode::new("MemberInvocation", start);
                    node.children.push(ident);
                    node.token_end = self.pos;
                    Ok(node)
                }
            }
            _ => Err(format!(
                "Expected invocation but found {:?} ({:?})",
                self.peek(),
                self.current().text,
            )),
        }
    }

    fn parse_function_invocation(
        &mut self,
        start: usize,
        ident: AstNode,
    ) -> Result<AstNode, String> {
        let lp = self.advance().clone();
        let mut functn = AstNode::new("Functn", start);
        functn.terminal_node_text.push(lp.text.clone());
        functn.children.push(ident);
        if *self.peek() != TokenKind::RParen {
            let params = self.parse_param_list()?;
            functn.children.push(params);
        }
        let rp = self.expect(&TokenKind::RParen)?.clone();
        functn.terminal_node_text.push(rp.text.clone());
        functn.token_end = self.pos;
        let mut fi = AstNode::new("FunctionInvocation", start);
        fi.children.push(functn);
        fi.token_end = self.pos;
        Ok(fi)
    }

    fn parse_param_list(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let mut pl = AstNode::new("ParamList", start);
        let first = self.parse_expression()?;
        pl.children.push(first);
        while *self.peek() == TokenKind::Comma {
            let comma = self.advance().clone();
            pl.terminal_node_text.push(comma.text.clone());
            let next = self.parse_expression()?;
            pl.children.push(next);
        }
        pl.token_end = self.pos;
        Ok(pl)
    }

    // ── Identifier ──────────────────────────────────────────────────────

    fn parse_identifier(&mut self) -> Result<AstNode, String> {
        match self.peek() {
            TokenKind::Identifier
            | TokenKind::DelimitedIdentifier
            | TokenKind::As
            | TokenKind::Is
            | TokenKind::Contains
            | TokenKind::In => {
                let start = self.pos;
                let tok = self.advance().clone();
                let mut node = AstNode::new("Identifier", start);
                node.terminal_node_text.push(tok.text.clone());
                node.token_end = self.pos;
                Ok(node)
            }
            _ => Err(format!(
                "Expected identifier but found {:?} ({:?})",
                self.peek(),
                self.current().text,
            )),
        }
    }

    // ── TypeSpecifier ───────────────────────────────────────────────────

    fn parse_type_specifier(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let qi = self.parse_qualified_identifier()?;
        let mut ts = AstNode::new("TypeSpecifier", start);
        ts.children.push(qi);
        ts.token_end = self.pos;
        Ok(ts)
    }

    fn parse_qualified_identifier(&mut self) -> Result<AstNode, String> {
        let start = self.pos;
        let mut qi = AstNode::new("QualifiedIdentifier", start);
        let first = self.parse_identifier()?;
        qi.children.push(first);
        while *self.peek() == TokenKind::Dot {
            let dot = self.advance().clone();
            qi.terminal_node_text.push(dot.text.clone());
            let next = self.parse_identifier()?;
            qi.children.push(next);
        }
        qi.token_end = self.pos;
        Ok(qi)
    }

    // ── DRY binary left-associative helper ──────────────────────────────

    fn parse_binary_left(
        &mut self,
        node_type: &'static str,
        ops: &[TokenKind],
        next: fn(&mut Self) -> Result<AstNode, String>,
    ) -> Result<AstNode, String> {
        let start = self.pos;
        let mut left = next(self)?;
        while ops.contains(self.peek()) {
            let op_tok = self.advance().clone();
            let right = next(self)?;
            let mut node = AstNode::new(node_type, start);
            node.terminal_node_text.push(op_tok.text.clone());
            node.children.push(left);
            node.children.push(right);
            node.token_end = self.pos;
            left = node;
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
