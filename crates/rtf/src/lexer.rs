//! Byte-level RTF tokenizer.
//!
//! The lexer never fails: every byte of input is either consumed into a
//! token or skipped. Raw CR and LF bytes are not text in RTF and are dropped
//! here.

/// One lexical RTF token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token<'a> {
    /// `{`
    GroupStart,
    /// `}`
    GroupEnd,
    /// `\word` or `\wordN`. A single space delimiter after the word has
    /// already been consumed.
    Word { name: &'a str, param: Option<i32> },
    /// A control symbol such as `\*`, `\{`, `\~`, or `\` followed by a newline.
    Symbol(u8),
    /// A `\'hh` escape.
    Hex(u8),
    /// A `\'` that is not followed by two hex digits.
    BadHex,
    /// A run of plain bytes, free of `\`, `{`, `}`, CR and LF.
    Text(&'a [u8]),
    /// The payload of `\binN`. Ignored by the parser.
    Bin,
}

pub(crate) struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn control(&mut self) -> Option<Token<'a>> {
        // Skip the backslash. A lone backslash at end of input yields nothing.
        self.pos += 1;
        let c = self.peek()?;
        if c.is_ascii_alphabetic() {
            let start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            // ASCII letters are always valid UTF-8.
            let name = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or_default();

            let negative = self.peek() == Some(b'-')
                && self.src.get(self.pos + 1).is_some_and(u8::is_ascii_digit);
            let mut param = None;
            if negative || self.peek().is_some_and(|b| b.is_ascii_digit()) {
                if negative {
                    self.pos += 1;
                }
                let mut v: i64 = 0;
                while let Some(d) = self.peek().filter(u8::is_ascii_digit) {
                    // Saturate instead of overflowing on absurdly long numbers.
                    v = (v * 10 + i64::from(d - b'0')).min(i64::from(i32::MAX) + 1);
                    self.pos += 1;
                }
                let v = if negative { -v } else { v };
                param = Some(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
            }
            // Exactly one space after a control word is its delimiter.
            if self.peek() == Some(b' ') {
                self.pos += 1;
            }
            if name == "bin" {
                let n = usize::try_from(param.unwrap_or(0)).unwrap_or(0);
                self.pos = self.pos.saturating_add(n).min(self.src.len());
                return Some(Token::Bin);
            }
            return Some(Token::Word { name, param });
        }
        if c == b'\'' {
            self.pos += 1;
            let hex = |b: Option<&u8>| b.and_then(|b| (*b as char).to_digit(16));
            return Some(
                match (hex(self.src.get(self.pos)), hex(self.src.get(self.pos + 1))) {
                    (Some(h), Some(l)) => {
                        self.pos += 2;
                        Token::Hex((h * 16 + l) as u8)
                    }
                    _ => Token::BadHex,
                },
            );
        }
        self.pos += 1;
        Some(Token::Symbol(c))
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let b = self.peek()?;
            match b {
                b'{' => {
                    self.pos += 1;
                    return Some(Token::GroupStart);
                }
                b'}' => {
                    self.pos += 1;
                    return Some(Token::GroupEnd);
                }
                b'\r' | b'\n' => self.pos += 1,
                b'\\' => match self.control() {
                    Some(t) => return Some(t),
                    None => continue,
                },
                _ => {
                    let start = self.pos;
                    while self
                        .peek()
                        .is_some_and(|b| !matches!(b, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                    {
                        self.pos += 1;
                    }
                    return Some(Token::Text(&self.src[start..self.pos]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Token<'_>> {
        Lexer::new(s.as_bytes()).collect()
    }

    #[test]
    fn words_params_and_delimiters() {
        assert_eq!(
            lex("\\b0 x\\fs-12y\\par\r\nz"),
            vec![
                Token::Word {
                    name: "b",
                    param: Some(0)
                },
                Token::Text(b"x"),
                Token::Word {
                    name: "fs",
                    param: Some(-12)
                },
                Token::Text(b"y"),
                Token::Word {
                    name: "par",
                    param: None
                },
                Token::Text(b"z"),
            ]
        );
    }

    #[test]
    fn only_one_space_is_a_delimiter() {
        assert_eq!(
            lex("\\i  two"),
            vec![
                Token::Word {
                    name: "i",
                    param: None
                },
                Token::Text(b" two")
            ]
        );
    }

    #[test]
    fn dash_without_digit_is_text() {
        assert_eq!(
            lex("\\ul-x"),
            vec![
                Token::Word {
                    name: "ul",
                    param: None
                },
                Token::Text(b"-x")
            ]
        );
    }

    #[test]
    fn hex_and_symbols() {
        assert_eq!(
            lex("\\'e9\\'zz\\*\\{"),
            vec![
                Token::Hex(0xe9),
                Token::BadHex,
                Token::Text(b"zz"),
                Token::Symbol(b'*'),
                Token::Symbol(b'{'),
            ]
        );
    }

    #[test]
    fn huge_parameter_saturates() {
        assert_eq!(
            lex("\\fs99999999999999999999 \\li-99999999999999999999"),
            vec![
                Token::Word {
                    name: "fs",
                    param: Some(i32::MAX)
                },
                Token::Word {
                    name: "li",
                    param: Some(i32::MIN)
                },
            ]
        );
    }

    #[test]
    fn bin_skips_payload() {
        assert_eq!(
            lex("\\bin2 {}\\x"),
            vec![
                Token::Bin,
                Token::Word {
                    name: "x",
                    param: None
                }
            ]
        );
        assert_eq!(lex("\\bin999 ab"), vec![Token::Bin]);
    }

    #[test]
    fn trailing_backslash_is_dropped() {
        assert_eq!(lex("a\\"), vec![Token::Text(b"a")]);
    }
}
