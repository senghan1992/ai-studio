//! Lexer and Pratt parser for spreadsheet formulas.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::refs::{parse_range, parse_ref};
use crate::values::{ERROR_CODES, VALUE_ERR};

#[derive(Clone, Debug, PartialEq)]
pub struct FormulaError {
    pub code: String,
    pub message: String,
}

impl FormulaError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FormulaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FormulaError {}

pub type Result<T> = std::result::Result<T, FormulaError>;

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Ref(String),
    Range(String),
    Name(String),
    Op(Op),
    LParen,
    RParen,
    Comma,
    LBrace,
    RBrace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Percent,
}

impl Op {
    fn from_str(s: &str) -> Option<Op> {
        Some(match s {
            "+" => Op::Add,
            "-" => Op::Sub,
            "*" => Op::Mul,
            "/" => Op::Div,
            "^" => Op::Pow,
            "&" => Op::Concat,
            "=" => Op::Eq,
            "<>" => Op::Ne,
            "<" => Op::Lt,
            ">" => Op::Gt,
            "<=" => Op::Le,
            ">=" => Op::Ge,
            "%" => Op::Percent,
            _ => return None,
        })
    }

    /// `None` for `%`, which is postfix only.
    fn precedence(self) -> Option<u8> {
        Some(match self {
            Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge => 1,
            Op::Concat => 2,
            Op::Add | Op::Sub => 3,
            Op::Mul | Op::Div => 4,
            Op::Pow => 5,
            Op::Percent => return None,
        })
    }
}

/// Non-ASCII characters are allowed in names so Korean named ranges work.
fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$' || (c as u32) >= 0xC0
}

fn is_word_char(c: char) -> bool {
    is_word_start(c) || c.is_ascii_digit() || c == '.'
}

static NUMBER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d*\.?\d+(?:[eE][+-]?\d+)?").unwrap());
static RANGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\$?[A-Za-z]{1,3}\$?\d{1,7}[ \t]*:[ \t]*\$?[A-Za-z]{1,3}\$?\d{1,7}").unwrap()
});

pub fn tokenize(input: &str) -> Result<Vec<Token>> {
    let src = input;
    let mut tokens = Vec::new();
    let mut i = 0usize;

    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().expect("non-empty slice");

        if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
            i += c.len_utf8();
            continue;
        }

        if let Some(code) = ERROR_CODES.iter().find(|e| rest.starts_with(**e)) {
            tokens.push(Token::Error((*code).to_string()));
            i += code.len();
            continue;
        }

        if c == '"' {
            let mut text = String::new();
            let mut j = i + 1;
            let mut terminated = false;
            while j < src.len() {
                let ch = src[j..].chars().next().unwrap();
                if ch == '"' {
                    if src[j + 1..].starts_with('"') {
                        text.push('"');
                        j += 2;
                        continue;
                    }
                    terminated = true;
                    break;
                }
                text.push(ch);
                j += ch.len_utf8();
            }
            if !terminated {
                return Err(FormulaError::new(VALUE_ERR, "unterminated string"));
            }
            tokens.push(Token::Text(text));
            i = j + 1;
            continue;
        }

        let next_is_digit = rest[c.len_utf8()..].starts_with(|d: char| d.is_ascii_digit());
        if c.is_ascii_digit() || (c == '.' && next_is_digit) {
            let m = NUMBER_RE.find(rest).expect("digit start always matches");
            let literal = m.as_str();
            let value: f64 = literal
                .parse()
                .map_err(|_| FormulaError::new(VALUE_ERR, format!("bad number {literal}")))?;
            tokens.push(Token::Number(value));
            i += literal.len();
            continue;
        }

        // A run of letters/digits/$ may be a range, a ref, a boolean, or a name.
        if is_word_start(c) {
            // Range first: `A1:B3` would otherwise lex as ref, op, ref.
            if let Some(m) = RANGE_RE.find(rest) {
                let compact: String = m
                    .as_str()
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect();
                if parse_range(&compact).is_some() {
                    tokens.push(Token::Range(compact.to_uppercase()));
                    i += m.as_str().len();
                    continue;
                }
            }

            let word: String = rest.chars().take_while(|ch| is_word_char(*ch)).collect();
            let len = word.len();
            let upper = word.to_uppercase();

            if upper == "TRUE" || upper == "FALSE" {
                tokens.push(Token::Bool(upper == "TRUE"));
            } else if parse_ref(&word).is_some() {
                tokens.push(Token::Ref(upper));
            } else {
                tokens.push(Token::Name(word));
            }
            i += len;
            continue;
        }

        // `get` rather than a slice: the next byte may be mid-character.
        if let Some(two) = rest.get(..2) {
            if two == "<=" || two == ">=" || two == "<>" {
                tokens.push(Token::Op(Op::from_str(two).unwrap()));
                i += 2;
                continue;
            }
        }

        if let Some(op) = Op::from_str(&c.to_string()) {
            tokens.push(Token::Op(op));
            i += 1;
            continue;
        }
        match c {
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            ',' | ';' => tokens.push(Token::Comma),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            _ => {
                return Err(FormulaError::new(
                    VALUE_ERR,
                    format!("unexpected character \"{c}\""),
                ))
            }
        }
        i += c.len_utf8();
    }
    Ok(tokens)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    /// An omitted argument, as in `IF(A1,,2)`.
    Blank,
    Ref(String),
    Range(String),
    Name(String),
    Array(Vec<Node>),
    Call {
        name: String,
        args: Vec<Node>,
    },
    Binary {
        op: Op,
        left: Box<Node>,
        right: Box<Node>,
    },
    Unary {
        negate: bool,
        arg: Box<Node>,
    },
    Percent(Box<Node>),
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, want: &Token) -> Result<()> {
        match self.next() {
            Some(ref got) if std::mem::discriminant(got) == std::mem::discriminant(want) => Ok(()),
            _ => Err(FormulaError::new(VALUE_ERR, format!("expected {want:?}"))),
        }
    }

    fn parse_expr(&mut self, min_prec: u8) -> Result<Node> {
        let mut left = self.parse_unary()?;
        // Climb while the next token is an infix operator that binds at least as
        // tightly as the caller's floor.
        while let Some(Token::Op(op)) = self.peek().cloned() {
            let Some(prec) = op.precedence().filter(|p| *p >= min_prec) else {
                break;
            };
            self.next();
            // '^' is right-associative in spreadsheets.
            let next_min = if op == Op::Pow { prec } else { prec + 1 };
            let right = self.parse_expr(next_min)?;
            left = Node::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Node> {
        if let Some(Token::Op(op)) = self.peek().cloned() {
            if op == Op::Sub || op == Op::Add {
                self.next();
                let arg = self.parse_unary()?;
                return Ok(Node::Unary {
                    negate: op == Op::Sub,
                    arg: Box::new(arg),
                });
            }
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Node> {
        let mut node = self.parse_primary()?;
        while matches!(self.peek(), Some(Token::Op(Op::Percent))) {
            self.next();
            node = Node::Percent(Box::new(node));
        }
        Ok(node)
    }

    fn parse_primary(&mut self) -> Result<Node> {
        let Some(token) = self.next() else {
            return Err(FormulaError::new(VALUE_ERR, "unexpected end of formula"));
        };
        match token {
            Token::Number(v) => Ok(Node::Number(v)),
            Token::Text(v) => Ok(Node::Text(v)),
            Token::Bool(v) => Ok(Node::Bool(v)),
            Token::Error(v) => Ok(Node::Error(v)),
            Token::Ref(v) => Ok(Node::Ref(v)),
            Token::Range(v) => Ok(Node::Range(v)),
            Token::LParen => {
                let inner = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Token::LBrace => {
                // Inline array {1,2;3} — flattened, enough for SUM({1,2,3}).
                let mut items = Vec::new();
                while self.peek().is_some() && !matches!(self.peek(), Some(Token::RBrace)) {
                    items.push(self.parse_expr(0)?);
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.next();
                    }
                }
                self.expect(&Token::RBrace)?;
                Ok(Node::Array(items))
            }
            Token::Name(name) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.next();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            if matches!(self.peek(), Some(Token::Comma)) {
                                args.push(Node::Blank);
                            } else {
                                args.push(self.parse_expr(0)?);
                            }
                            if matches!(self.peek(), Some(Token::Comma)) {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(&Token::RParen)?;
                    return Ok(Node::Call {
                        name: name.to_uppercase(),
                        args,
                    });
                }
                Ok(Node::Name(name))
            }
            other => Err(FormulaError::new(
                VALUE_ERR,
                format!("unexpected token {other:?}"),
            )),
        }
    }
}

/// Parse a formula, with or without the leading `=`.
pub fn parse(formula: &str) -> Result<Node> {
    let text = formula.trim_start();
    let text = text.strip_prefix('=').unwrap_or(text);
    let tokens = tokenize(text)?;
    let mut parser = Parser { tokens, pos: 0 };
    let ast = parser.parse_expr(0)?;
    if parser.pos < parser.tokens.len() {
        return Err(FormulaError::new(VALUE_ERR, "trailing input"));
    }
    Ok(ast)
}

#[derive(Default, Debug, PartialEq)]
pub struct Refs {
    pub refs: Vec<String>,
    pub ranges: Vec<String>,
    pub names: Vec<String>,
}

/// Every cell address a formula reads, ranges left unexpanded.
pub fn collect_refs(ast: &Node) -> Refs {
    let mut out = Refs::default();
    walk_refs(ast, &mut out);
    out
}

fn walk_refs(ast: &Node, out: &mut Refs) {
    match ast {
        Node::Ref(r) => out.refs.push(r.clone()),
        Node::Range(r) => out.ranges.push(r.clone()),
        Node::Name(n) => out.names.push(n.clone()),
        Node::Binary { left, right, .. } => {
            walk_refs(left, out);
            walk_refs(right, out);
        }
        Node::Unary { arg, .. } | Node::Percent(arg) => walk_refs(arg, out),
        Node::Call { args, .. } | Node::Array(args) => args.iter().for_each(|a| walk_refs(a, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_the_awkward_cases() {
        assert_eq!(
            tokenize("A1:B3").unwrap(),
            vec![Token::Range("A1:B3".into())]
        );
        // Whitespace inside a range is tolerated and normalized away.
        assert_eq!(
            tokenize("A1 : B3").unwrap(),
            vec![Token::Range("A1:B3".into())]
        );
        assert_eq!(
            tokenize("\"a\"\"b\"").unwrap(),
            vec![Token::Text("a\"b".into())]
        );
        assert_eq!(
            tokenize("#DIV/0!").unwrap(),
            vec![Token::Error("#DIV/0!".into())]
        );
        assert_eq!(tokenize("매출").unwrap(), vec![Token::Name("매출".into())]);
        assert!(tokenize("\"open").is_err());
    }

    #[test]
    fn respects_precedence_and_associativity() {
        // 2^3^2 is 2^(3^2) = 512, not (2^3)^2 = 64.
        let ast = parse("=2^3^2").unwrap();
        let Node::Binary { op, right, .. } = &ast else {
            panic!("{ast:?}")
        };
        assert_eq!(*op, Op::Pow);
        assert!(matches!(**right, Node::Binary { op: Op::Pow, .. }));

        // '&' binds looser than '+'.
        let ast = parse("=1+2&\"x\"").unwrap();
        let Node::Binary { op, left, .. } = &ast else {
            panic!()
        };
        assert_eq!(*op, Op::Concat);
        assert!(matches!(**left, Node::Binary { op: Op::Add, .. }));
    }

    #[test]
    fn omitted_arguments_become_blank() {
        let Node::Call { args, .. } = parse("=IF(A1,,2)").unwrap() else {
            panic!()
        };
        assert_eq!(args.len(), 3);
        assert_eq!(args[1], Node::Blank);
    }

    #[test]
    fn collects_what_a_formula_reads() {
        let refs = collect_refs(&parse("=SUM(A1:B2)+C3*예산").unwrap());
        assert_eq!(refs.ranges, ["A1:B2"]);
        assert_eq!(refs.refs, ["C3"]);
        assert_eq!(refs.names, ["예산"]);
    }

    #[test]
    fn rejects_trailing_input() {
        assert!(parse("=1 2").is_err());
        assert!(parse("=SUM(").is_err());
    }
}
