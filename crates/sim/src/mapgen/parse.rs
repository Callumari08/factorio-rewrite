//! Parser for Factorio's noise expression language (the `NoiseExpression` type).
//!
//! Grammar, highest precedence first: `^` (right associative), unary `+ - ~`,
//! `* / % %%`, `+ -`, `< <= > >=`, `== ~= !=`, `&`, `~` (xor), `|`. Calls take positional
//! `f(a, b)` or named `f{a = 1, b = 2}` arguments.

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Var(String),
    Unary(char, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Args),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Args {
    Positional(Vec<Expr>),
    Named(Vec<(String, Expr)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Pow,
    Mul,
    Div,
    Mod,
    Rem,
    Add,
    Sub,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
}

const OPS: &[&str] = &[
    "%%", "<=", ">=", "==", "~=", "!=", "^", "+", "-", "~", "*", "/", "%", "<", ">", "&", "|", "(", ")", "{", "}", ",",
    "=",
];

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b':') {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].to_owned()));
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            if c == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X')) {
                i += 2;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let v = u64::from_str_radix(&s[start + 2..i], 16).map_err(|e| e.to_string())?;
                out.push(Tok::Num(v as f64));
                continue;
            }
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                i += 1;
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            out.push(Tok::Num(s[start..i].parse().map_err(|_| format!("bad number '{}'", &s[start..i]))?));
        } else if c == b'"' || c == b'\'' {
            let end = s[i + 1..].find(c as char).ok_or("unterminated string")? + i + 1;
            out.push(Tok::Str(s[i + 1..end].to_owned()));
            i = end + 1;
        } else {
            let op =
                OPS.iter().find(|op| s[i..].starts_with(**op)).ok_or_else(|| format!("unexpected '{}'", c as char))?;
            out.push(Tok::Op(op));
            i += op.len();
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek_op(&self) -> Option<&'static str> {
        match self.toks.get(self.pos) {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.peek_op() == Some(op) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{op}', found {:?}", self.toks.get(self.pos)))
        }
    }

    /// Binary operator levels, lowest precedence first.
    const LEVELS: &'static [&'static [(&'static str, BinOp)]] = &[
        &[("|", BinOp::BitOr)],
        &[("~", BinOp::BitXor)],
        &[("&", BinOp::BitAnd)],
        &[("==", BinOp::Eq), ("~=", BinOp::Ne), ("!=", BinOp::Ne)],
        &[("<", BinOp::Lt), ("<=", BinOp::Le), (">", BinOp::Gt), (">=", BinOp::Ge)],
        &[("+", BinOp::Add), ("-", BinOp::Sub)],
        &[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Mod), ("%%", BinOp::Rem)],
    ];

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        if level == Self::LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = self.peek_op().and_then(|o| Self::LEVELS[level].iter().find(|(s, _)| *s == o)) {
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op.1, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek_op() {
            Some(op @ ("-" | "+" | "~")) => {
                self.pos += 1;
                let e = self.unary()?;
                Ok(Expr::Unary(op.chars().next().unwrap(), Box::new(e)))
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if self.peek_op() == Some("^") {
            self.pos += 1;
            // Right associative, and the exponent may carry a unary sign.
            let exp = self.unary()?;
            return Ok(Expr::Binary(BinOp::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let tok = self.toks.get(self.pos).cloned().ok_or("unexpected end of expression")?;
        self.pos += 1;
        match tok {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Op("(") => {
                let e = self.binary(0)?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(name) => match self.peek_op() {
                Some("(") => {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek_op() != Some(")") {
                        loop {
                            args.push(self.binary(0)?);
                            if self.peek_op() == Some(",") {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(")")?;
                    Ok(Expr::Call(name, Args::Positional(args)))
                }
                Some("{") => {
                    self.pos += 1;
                    let mut args = Vec::new();
                    while self.peek_op() != Some("}") {
                        let Some(Tok::Ident(key)) = self.toks.get(self.pos).cloned() else {
                            return Err(format!("expected argument name in call to {name}"));
                        };
                        self.pos += 1;
                        self.expect("=")?;
                        args.push((key, self.binary(0)?));
                        if self.peek_op() == Some(",") {
                            self.pos += 1;
                        }
                    }
                    self.expect("}")?;
                    Ok(Expr::Call(name, Args::Named(args)))
                }
                _ => Ok(Expr::Var(name)),
            },
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

pub fn parse(s: &str) -> Result<Expr, String> {
    let mut p = Parser { toks: lex(s)?, pos: 0 };
    let e = p.binary(0)?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected {:?} after expression", p.toks[p.pos]));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: f64) -> Box<Expr> {
        Box::new(Expr::Num(n))
    }

    #[test]
    fn precedence_and_calls() {
        // -2^2 is -(2^2); 1 + 2 * 3 multiplies first.
        assert_eq!(parse("-2^2").unwrap(), Expr::Unary('-', Box::new(Expr::Binary(BinOp::Pow, num(2.0), num(2.0)))));
        assert_eq!(
            parse("1 + 2 * 3").unwrap(),
            Expr::Binary(BinOp::Add, num(1.0), Box::new(Expr::Binary(BinOp::Mul, num(2.0), num(3.0))))
        );
        assert_eq!(
            parse("2^3^2").unwrap(),
            Expr::Binary(BinOp::Pow, num(2.0), Box::new(Expr::Binary(BinOp::Pow, num(3.0), num(2.0))))
        );
        assert!(
            matches!(parse("clamp(x, -1, 1)").unwrap(), Expr::Call(n, Args::Positional(a)) if n == "clamp" && a.len() == 3)
        );
        assert!(
            matches!(parse("basis_noise{x = x, y = y, seed1 = 'a'}").unwrap(), Expr::Call(_, Args::Named(a)) if a.len() == 3)
        );
        assert_eq!(
            parse("var('control:iron-ore:size')").unwrap(),
            Expr::Call("var".into(), Args::Positional(vec![Expr::Str("control:iron-ore:size".into())]))
        );
        assert_eq!(parse("control:water:size").unwrap(), Expr::Var("control:water:size".into()));
        assert_eq!(parse(".5e1").unwrap(), Expr::Num(5.0));
        assert_eq!(parse("0x10").unwrap(), Expr::Num(16.0));
        assert!(matches!(parse("a ~= b").unwrap(), Expr::Binary(BinOp::Ne, _, _)));
        assert!(matches!(parse("a ~ b").unwrap(), Expr::Binary(BinOp::BitXor, _, _)));
        assert!(matches!(parse("(a > 0) * 2").unwrap(), Expr::Binary(BinOp::Mul, _, _)));
    }
}
