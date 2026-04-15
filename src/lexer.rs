/// FHIRPath lexer: &str → Vec<Token>

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number,
    String,
    DateTime,
    Time,
    Identifier,
    DelimitedIdentifier,

    // Boolean keywords
    True,
    False,

    // Operator keywords
    And,
    Or,
    Xor,
    Implies,
    Is,
    As,
    In,
    Contains,
    Div,
    Mod,

    // Special variables
    DollarThis,
    DollarIndex,
    DollarTotal,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Pipe,
    Ampersand,
    Eq,
    NotEq,
    Tilde,
    NotTilde,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // Delimiters
    Dot,
    Comma,
    Percent,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    // Build a mapping from char index → byte offset in the original string.
    let char_to_byte: Vec<usize> = input.char_indices().map(|(b, _)| b).collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];

        // 1. Skip whitespace
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // 2. Comments and slash
        if c == '/' {
            if i + 1 < len && chars[i + 1] == '/' {
                // Line comment
                i += 2;
                while i < len && chars[i] != '\n' && chars[i] != '\r' {
                    i += 1;
                }
                continue;
            } else if i + 1 < len && chars[i + 1] == '*' {
                // Block comment
                i += 2;
                loop {
                    if i + 1 >= len {
                        return Err("Unterminated block comment".into());
                    }
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            } else {
                tokens.push(Token { kind: TokenKind::Slash, text: "/".into(), byte_start: char_to_byte[i], byte_end: char_to_byte[i] + 1 });
                i += 1;
                continue;
            }
        }

        // 3. Multi-char operators
        if c == '!' && i + 1 < len {
            if chars[i + 1] == '=' {
                tokens.push(Token { kind: TokenKind::NotEq, text: "!=".into(), byte_start: char_to_byte[i], byte_end: char_to_byte[i + 1] + 1 });
                i += 2;
                continue;
            } else if chars[i + 1] == '~' {
                tokens.push(Token { kind: TokenKind::NotTilde, text: "!~".into(), byte_start: char_to_byte[i], byte_end: char_to_byte[i + 1] + 1 });
                i += 2;
                continue;
            }
            return Err(format!("Unexpected character '!' at position {i}"));
        }
        if c == '<' && i + 1 < len && chars[i + 1] == '=' {
            tokens.push(Token { kind: TokenKind::LtEq, text: "<=".into(), byte_start: char_to_byte[i], byte_end: char_to_byte[i + 1] + 1 });
            i += 2;
            continue;
        }
        if c == '>' && i + 1 < len && chars[i + 1] == '=' {
            tokens.push(Token { kind: TokenKind::GtEq, text: ">=".into(), byte_start: char_to_byte[i], byte_end: char_to_byte[i + 1] + 1 });
            i += 2;
            continue;
        }

        // 4. Single-char operators & delimiters
        let single = match c {
            '+' => Some(TokenKind::Plus),
            '-' => Some(TokenKind::Minus),
            '*' => Some(TokenKind::Star),
            '|' => Some(TokenKind::Pipe),
            '&' => Some(TokenKind::Ampersand),
            '=' => Some(TokenKind::Eq),
            '~' => Some(TokenKind::Tilde),
            '<' => Some(TokenKind::Lt),
            '>' => Some(TokenKind::Gt),
            '.' => Some(TokenKind::Dot),
            ',' => Some(TokenKind::Comma),
            '%' => Some(TokenKind::Percent),
            '(' => Some(TokenKind::LParen),
            ')' => Some(TokenKind::RParen),
            '[' => Some(TokenKind::LBracket),
            ']' => Some(TokenKind::RBracket),
            '{' => Some(TokenKind::LBrace),
            '}' => Some(TokenKind::RBrace),
            _ => None,
        };
        if let Some(kind) = single {
            tokens.push(Token { kind, text: c.to_string(), byte_start: char_to_byte[i], byte_end: char_to_byte[i] + c.len_utf8() });
            i += 1;
            continue;
        }

        // 5. $ variables
        if c == '$' {
            if input[char_to_byte[i]..].starts_with("$this") && !is_ident_continue(chars.get(i + 5).copied()) {
                let bs = char_to_byte[i];
                let be = bs + "$this".len();
                tokens.push(Token { kind: TokenKind::DollarThis, text: "$this".into(), byte_start: bs, byte_end: be });
                i += 5;
                continue;
            }
            if input[char_to_byte[i]..].starts_with("$index") && !is_ident_continue(chars.get(i + 6).copied()) {
                let bs = char_to_byte[i];
                let be = bs + "$index".len();
                tokens.push(Token { kind: TokenKind::DollarIndex, text: "$index".into(), byte_start: bs, byte_end: be });
                i += 6;
                continue;
            }
            if input[char_to_byte[i]..].starts_with("$total") && !is_ident_continue(chars.get(i + 6).copied()) {
                let bs = char_to_byte[i];
                let be = bs + "$total".len();
                tokens.push(Token { kind: TokenKind::DollarTotal, text: "$total".into(), byte_start: bs, byte_end: be });
                i += 6;
                continue;
            }
            return Err(format!("Unknown $ variable at position {i}"));
        }

        // 6. DateTime/Time literals (starts with @)
        if c == '@' {
            let start = i;
            let byte_start = char_to_byte[i];
            i += 1;
            if i < len && chars[i] == 'T' {
                // Time literal: @T followed by TIMEFORMAT
                i += 1;
                i = scan_timeformat(&chars, i);
                let text: String = chars[start..i].iter().collect();
                let byte_end = if i < len { char_to_byte[i] } else { input.len() };
                tokens.push(Token { kind: TokenKind::Time, text, byte_start, byte_end });
                continue;
            }
            // DateTime literal: @YYYY(-MM(-DD(T TIMEFORMAT)?)?)?Z?
            // Date portion: only digits and '-'
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '-') {
                i += 1;
            }
            // Optional T + time portion
            if i < len && chars[i] == 'T' {
                i += 1;
                i = scan_timeformat(&chars, i);
            }
            // Optional trailing Z (for date-only with Z, though unusual)
            if i < len && chars[i] == 'Z' {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let byte_end = if i < len { char_to_byte[i] } else { input.len() };
            tokens.push(Token { kind: TokenKind::DateTime, text, byte_start, byte_end });
            continue;
        }

        // 7. String literals (starts with ')
        if c == '\'' {
            let byte_start = char_to_byte[i];
            let text = scan_quoted(&chars, &mut i, '\'')?;
            let byte_end = if i < len { char_to_byte[i] } else { input.len() };
            tokens.push(Token { kind: TokenKind::String, text, byte_start, byte_end });
            continue;
        }

        // 8. Delimited identifiers (starts with `)
        if c == '`' {
            let byte_start = char_to_byte[i];
            let text = scan_quoted(&chars, &mut i, '`')?;
            let byte_end = if i < len { char_to_byte[i] } else { input.len() };
            tokens.push(Token { kind: TokenKind::DelimitedIdentifier, text, byte_start, byte_end });
            continue;
        }

        // 9. Numbers
        if c.is_ascii_digit() {
            let start = i;
            let byte_start = char_to_byte[i];
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            // Only consume '.' if followed by a digit
            if i < len && chars[i] == '.' && i + 1 < len && chars[i + 1].is_ascii_digit() {
                i += 1; // consume '.'
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let byte_end = if i < len { char_to_byte[i] } else { input.len() };
            tokens.push(Token { kind: TokenKind::Number, text, byte_start, byte_end });
            continue;
        }

        // 10. Identifiers and keywords
        if is_ident_start(c) {
            let start = i;
            let byte_start = char_to_byte[i];
            while i < len && is_ident_continue(Some(chars[i])) {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let byte_end = if i < len { char_to_byte[i] } else { input.len() };
            let kind = match text.as_str() {
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                "and" => TokenKind::And,
                "or" => TokenKind::Or,
                "xor" => TokenKind::Xor,
                "implies" => TokenKind::Implies,
                "is" => TokenKind::Is,
                "as" => TokenKind::As,
                "in" => TokenKind::In,
                "contains" => TokenKind::Contains,
                "div" => TokenKind::Div,
                "mod" => TokenKind::Mod,
                _ => TokenKind::Identifier,
            };
            tokens.push(Token { kind, text, byte_start, byte_end });
            continue;
        }

        return Err(format!("Unexpected character {c:?} at position {i}"));
    }

    tokens.push(Token { kind: TokenKind::Eof, text: String::new(), byte_start: input.len(), byte_end: input.len() });
    Ok(tokens)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: Option<char>) -> bool {
    match c {
        Some(c) => c.is_ascii_alphanumeric() || c == '_',
        None => false,
    }
}

fn scan_timeformat(chars: &[char], mut i: usize) -> usize {
    let len = chars.len();
    // HH
    while i < len && chars[i].is_ascii_digit() {
        i += 1;
    }
    // :MM
    if i < len && chars[i] == ':' {
        i += 1;
        while i < len && chars[i].is_ascii_digit() {
            i += 1;
        }
        // :SS
        if i < len && chars[i] == ':' {
            i += 1;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            // .fractional — only if '.' is followed by a digit
            if i < len && chars[i] == '.' && i + 1 < len && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
    }
    // timezone: Z or +/-HH:MM
    if i < len && chars[i] == 'Z' {
        i += 1;
    } else if i < len && (chars[i] == '+' || chars[i] == '-') {
        i += 1;
        while i < len && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i < len && chars[i] == ':' {
            i += 1;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// Scan a quoted string (single-quote or backtick). Handles escapes.
/// Returns the raw token text including the surrounding quote characters.
/// Advances `i` past the closing quote.
fn scan_quoted(chars: &[char], i: &mut usize, quote: char) -> Result<String, String> {
    let start = *i;
    *i += 1; // skip opening quote
    let len = chars.len();
    while *i < len {
        if chars[*i] == '\\' {
            *i += 2; // skip escape sequence
            continue;
        }
        if chars[*i] == quote {
            *i += 1; // skip closing quote
            let text: String = chars[start..*i].iter().collect();
            return Ok(text);
        }
        *i += 1;
    }
    Err(format!("Unterminated {quote} literal starting at position {start}"))
}
