use syntree::Span;

use crate::error::{Diagnostic, ErrorKind};
use crate::syntax::Syntax;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind: Syntax,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Token {
    pub(crate) fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub(crate) fn span(&self) -> Span<u32> {
        span(self.start, self.end)
    }
}

/// A span of byte offsets, which the parser has checked fit a `u32`.
pub(crate) fn span(start: usize, end: usize) -> Span<u32> {
    let index = |at: usize| u32::try_from(at).unwrap_or(u32::MAX);
    Span::new(index(start), index(end))
}

/// Text the lexer rejected, which it has skipped past.
pub(crate) struct Rejected {
    /// Why it is no token.
    pub(crate) diagnostic: Diagnostic,
    /// The text skipped, as a token of the kind it most likely stands for:
    /// for `=`, `&` or `|` alone the operator doubling it, otherwise
    /// `Invalid`.
    pub(crate) token: Token,
}

#[derive(Clone)]
pub(crate) struct Lexer<'a> {
    source: &'a str,
    at: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self { source, at: 0 }
    }

    pub(crate) fn len(&self) -> usize {
        self.source.len()
    }

    /// The byte offset the next token starts at.
    pub(crate) fn position(&self) -> usize {
        self.at
    }

    /// Go on lexing at `at`, a character boundary past text another parser
    /// has read.
    pub(crate) fn seek(&mut self, at: usize) {
        self.at = at.min(self.source.len());
    }

    /// Skip past text the lexer rejected: the whole string literal if one
    /// starts here, leniently, otherwise one character. Returns the new
    /// position.
    fn recover(&mut self) -> usize {
        let rest = self.source.get(self.at..).unwrap_or_default();
        let mut chars = rest.char_indices();

        let len = match chars.next() {
            Some((_, '"')) => {
                let mut end = rest.len();

                while let Some((at, c)) = chars.next() {
                    match c {
                        '"' => {
                            end = at.saturating_add(1);
                            break;
                        }
                        '\\' => {
                            chars.next();
                        }
                        _ => {}
                    }
                }

                end
            }
            // A `0x` without digits, as one token.
            Some((_, '0')) if matches!(chars.next(), Some((_, 'x' | 'X'))) => 2,
            Some((_, c)) => c.len_utf8(),
            None => 0,
        };

        self.at = self.at.saturating_add(len);
        self.at
    }

    pub(crate) fn text(&self, start: usize, end: usize) -> &'a str {
        self.source.get(start..end).unwrap_or_default()
    }

    /// The next token, whitespace included; `None` at the end.
    ///
    /// # Errors
    ///
    /// Text that is no token is [`Rejected`], and skipped past.
    pub(crate) fn next(&mut self) -> Result<Option<Token>, Rejected> {
        let start = self.at;

        let (kind, len) = match self.lex() {
            Ok(Some(token)) => token,
            Ok(None) => return Ok(None),
            Err((diagnostic, kind)) => {
                let end = self.recover();
                let token = Token { kind, start, end };
                return Err(Rejected { diagnostic, token });
            }
        };

        self.at = start.saturating_add(len);

        Ok(Some(Token {
            kind,
            start,
            end: self.at,
        }))
    }

    /// The kind and length of the token at the position, or why there is
    /// none and the kind it stands for.
    fn lex(&self) -> Result<Option<(Syntax, usize)>, (Diagnostic, Syntax)> {
        let rest = self.source.get(self.at..).unwrap_or_default();
        let mut chars = rest.chars();

        let Some(c) = chars.next() else {
            return Ok(None);
        };

        let token = match c {
            c if c.is_whitespace() => (Syntax::Whitespace, prefix(rest, char::is_whitespace)),
            '0' if matches!(chars.clone().next(), Some('x' | 'X')) => {
                // `0x` and the hexadecimal digits after it.
                let digits = rest.get(2..).unwrap_or_default();
                let len = prefix(digits, |c| c.is_ascii_hexdigit());

                if len == 0 {
                    return Err((self.error(2, ErrorKind::HexWithoutDigits), Syntax::Invalid));
                }

                (Syntax::IntegerToken, len.saturating_add(2))
            }
            '0'..='9' => (Syntax::IntegerToken, prefix(rest, |c| c.is_ascii_digit())),
            'a'..='z' | 'A'..='Z' | '_' => {
                let len = prefix(rest, |c| c.is_ascii_alphanumeric() || c == '_');

                let kind = match rest.get(..len) {
                    Some("true") => Syntax::True,
                    Some("false") => Syntax::False,
                    _ => Syntax::NameToken,
                };

                (kind, len)
            }
            '"' => match self.string(rest) {
                Ok(len) => (Syntax::StringToken, len),
                Err(error) => return Err((error, Syntax::Invalid)),
            },
            c => match (c, chars.next()) {
                ('=', Some('=')) => (Syntax::EqEq, 2),
                ('!', Some('=')) => (Syntax::NotEq, 2),
                ('<', Some('=')) => (Syntax::Le, 2),
                ('>', Some('=')) => (Syntax::Ge, 2),
                ('&', Some('&')) => (Syntax::AndAnd, 2),
                ('|', Some('|')) => (Syntax::OrOr, 2),
                (':', Some(':')) => (Syntax::ColonColon, 2),
                ('=' | '&' | '|', _) => {
                    let kind = match c {
                        '=' => Syntax::EqEq,
                        '&' => Syntax::AndAnd,
                        _ => Syntax::OrOr,
                    };

                    return Err((self.error(1, ErrorKind::SingleOperator(c)), kind));
                }
                ('+', _) => (Syntax::Plus, 1),
                ('-', _) => (Syntax::Minus, 1),
                ('*', _) => (Syntax::Star, 1),
                ('/', _) => (Syntax::Slash, 1),
                ('%', _) => (Syntax::Percent, 1),
                ('<', _) => (Syntax::Lt, 1),
                ('>', _) => (Syntax::Gt, 1),
                ('!', _) => (Syntax::Bang, 1),
                ('(', _) => (Syntax::OpenParen, 1),
                (')', _) => (Syntax::CloseParen, 1),
                ('[', _) => (Syntax::OpenBracket, 1),
                (']', _) => (Syntax::CloseBracket, 1),
                ('{', _) => (Syntax::OpenBrace, 1),
                ('}', _) => (Syntax::CloseBrace, 1),
                (':', _) => (Syntax::Colon, 1),
                (',', _) => (Syntax::Comma, 1),
                ('.', _) => (Syntax::Dot, 1),
                _ => {
                    let error = self.error(c.len_utf8(), ErrorKind::UnexpectedCharacter(c));
                    return Err((error, Syntax::Invalid));
                }
            },
        };

        Ok(Some(token))
    }

    fn error(&self, len: usize, kind: ErrorKind) -> Diagnostic {
        Diagnostic::new(span(self.at, self.at.saturating_add(len)), kind)
    }

    /// The length of the string literal `rest` starts with.
    fn string(&self, rest: &str) -> Result<usize, Diagnostic> {
        let mut chars = rest.char_indices().skip(1);

        while let Some((at, c)) = chars.next() {
            match c {
                '"' => return Ok(at.saturating_add(1)),
                '\\' => match chars.next() {
                    Some((_, '"' | '\\')) => {}
                    Some((e_at, e)) => {
                        let start = self.at.saturating_add(at);
                        let end = self.at.saturating_add(e_at).saturating_add(e.len_utf8());
                        return Err(Diagnostic::new(
                            span(start, end),
                            ErrorKind::InvalidEscape(e),
                        ));
                    }
                    None => break,
                },
                _ => {}
            }
        }

        Err(Diagnostic::new(
            span(self.at, self.source.len()),
            ErrorKind::UnterminatedString,
        ))
    }
}

/// The byte length of the longest prefix of `s` whose characters match.
fn prefix(s: &str, f: impl Fn(char) -> bool) -> usize {
    s.char_indices()
        .find(|(_, c)| !f(*c))
        .map_or(s.len(), |(at, _)| at)
}

/// The value of an integer token's text, `negative` when a `-` is written
/// before it, and whether it is written in hexadecimal (`0x1F`). `None` when
/// it is past the range of `Int`. Takes time linear in the text.
pub(crate) fn parse_int(text: &str, negative: bool) -> Option<(i64, bool)> {
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));

    let (magnitude, hex) = match hex {
        Some(digits) if digits.bytes().all(|b| b.is_ascii_hexdigit()) => {
            (u64::from_str_radix(digits, 16).ok()?, true)
        }
        Some(_) => return None,
        None if text.bytes().all(|b| b.is_ascii_digit()) => (text.parse::<u64>().ok()?, false),
        None => return None,
    };

    let value = if negative {
        i64::try_from(i128::from(magnitude).checked_neg()?).ok()?
    } else {
        i64::try_from(magnitude).ok()?
    };

    Some((value, hex))
}
