use syntree::Span;

use crate::error::{Error, ErrorKind};
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

    pub(crate) fn text(&self, start: usize, end: usize) -> &'a str {
        self.source.get(start..end).unwrap_or_default()
    }

    /// The next token, whitespace included; `None` at the end.
    pub(crate) fn next(&mut self) -> Result<Option<Token>, Error> {
        let rest = self.source.get(self.at..).unwrap_or_default();
        let mut chars = rest.chars();

        let Some(c) = chars.next() else {
            return Ok(None);
        };

        let (kind, len) = match c {
            c if c.is_whitespace() => (Syntax::Whitespace, prefix(rest, char::is_whitespace)),
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
            '"' => (Syntax::StringToken, self.string(rest)?),
            c => match (c, chars.next()) {
                ('=', Some('=')) => (Syntax::EqEq, 2),
                ('!', Some('=')) => (Syntax::NotEq, 2),
                ('<', Some('=')) => (Syntax::Le, 2),
                ('>', Some('=')) => (Syntax::Ge, 2),
                ('&', Some('&')) => (Syntax::AndAnd, 2),
                ('|', Some('|')) => (Syntax::OrOr, 2),
                ('=' | '&' | '|', _) => {
                    return Err(self.error(1, ErrorKind::SingleOperator(c)));
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
                (',', _) => (Syntax::Comma, 1),
                ('.', _) => (Syntax::Dot, 1),
                _ => {
                    return Err(self.error(c.len_utf8(), ErrorKind::UnexpectedCharacter(c)));
                }
            },
        };

        let start = self.at;
        self.at = start.saturating_add(len);

        Ok(Some(Token {
            kind,
            start,
            end: self.at,
        }))
    }

    fn error(&self, len: usize, kind: ErrorKind) -> Error {
        Error::new(span(self.at, self.at.saturating_add(len)), kind)
    }

    /// The length of the string literal `rest` starts with.
    fn string(&self, rest: &str) -> Result<usize, Error> {
        let mut chars = rest.char_indices().skip(1);

        while let Some((at, c)) = chars.next() {
            match c {
                '"' => return Ok(at.saturating_add(1)),
                '\\' => match chars.next() {
                    Some((_, '"' | '\\')) => {}
                    Some((e_at, e)) => {
                        let start = self.at.saturating_add(at);
                        let end = self.at.saturating_add(e_at).saturating_add(e.len_utf8());
                        return Err(Error::new(span(start, end), ErrorKind::InvalidEscape(e)));
                    }
                    None => break,
                },
                _ => {}
            }
        }

        Err(Error::new(
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
