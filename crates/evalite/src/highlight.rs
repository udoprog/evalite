//! Split source text into classified spans for syntax highlighting.
//!
//! The spans cover the whole source and the highlighter copes with text that
//! does not compile: what the lexer rejects becomes [`Class::Error`], the
//! same text that compiling reports and recovers from as a token that is
//! not one.
//!
//! ```
//! use evalite::highlight::{Class, highlight};
//!
//! let classes = highlight("a.len() + 1")
//!     .filter(|t| t.class != Class::Whitespace)
//!     .map(|t| t.class.as_str())
//!     .collect::<Vec<_>>();
//!
//! assert_eq!(
//!     classes,
//!     ["ident", "punctuation", "member", "punctuation", "punctuation", "operator", "number"],
//! );
//! ```

use core::ops::Range;

use crate::lexer::Lexer;
use crate::syntax::Syntax;

/// The class of a [`Token`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Class {
    /// A keyword: `true` or `false`.
    Keyword,
    /// A name: a value or a function.
    Ident,
    /// A name after a `.`: a method or a field.
    Member,
    /// An integer literal.
    Number,
    /// A string literal.
    String,
    /// Delimiters and separators: `( ) [ ] , .`.
    Punctuation,
    /// Operators.
    Operator,
    /// Whitespace.
    Whitespace,
    /// Text which could not be lexed, such as a stray character or a string
    /// that is unterminated or has an invalid escape.
    Error,
}

impl Class {
    /// A stable lowercase name for the class, usable as a CSS class.
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Keyword => "keyword",
            Class::Ident => "ident",
            Class::Member => "member",
            Class::Number => "number",
            Class::String => "string",
            Class::Punctuation => "punctuation",
            Class::Operator => "operator",
            Class::Whitespace => "whitespace",
            Class::Error => "error",
        }
    }
}

/// A classified span of source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The class of the span.
    pub class: Class,
    /// The byte range in the source.
    pub span: Range<usize>,
}

/// Classify all of `source`, in order and without gaps.
pub fn highlight(source: &str) -> impl Iterator<Item = Token> + '_ {
    let mut lexer = Lexer::new(source);
    let mut after_dot = false;

    core::iter::from_fn(move || {
        let start = lexer.position();

        let (class, end) = match lexer.next() {
            Ok(Some(token)) => {
                let class = match token.kind {
                    Syntax::Whitespace => Class::Whitespace,
                    Syntax::IntegerToken => Class::Number,
                    Syntax::StringToken => Class::String,
                    Syntax::True | Syntax::False => Class::Keyword,
                    Syntax::NameToken if after_dot => Class::Member,
                    Syntax::NameToken => Class::Ident,
                    Syntax::OpenParen
                    | Syntax::CloseParen
                    | Syntax::OpenBracket
                    | Syntax::CloseBracket
                    | Syntax::Comma
                    | Syntax::Dot => Class::Punctuation,
                    _ => Class::Operator,
                };

                if token.kind != Syntax::Whitespace {
                    after_dot = token.kind == Syntax::Dot;
                }

                (class, token.end)
            }
            Ok(None) => return None,
            Err(rejected) => {
                after_dot = false;
                (Class::Error, rejected.token.end)
            }
        };

        Some(Token {
            class,
            span: start..end,
        })
    })
}
