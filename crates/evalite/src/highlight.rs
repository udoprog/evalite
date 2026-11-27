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
    /// A keyword: `true` or `false`, or the option literals `Some` and
    /// `None`.
    Keyword,
    /// A name: a value or a function.
    Ident,
    /// A type's name before `::`, such as `Int` in `Int::min`, or in a type
    /// annotation, such as `Int` in `None::<Int>`.
    Type,
    /// A name after a `.`, a method or a field, or before a `:`, a
    /// record literal's field.
    Member,
    /// An integer literal.
    Number,
    /// A string literal.
    String,
    /// Delimiters and separators: `( ) [ ] , . ::`.
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
            Class::Type => "type",
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
    let mut after_path = false;
    // How many `<` of a type annotation such as `None::<List<Int>>` are
    // open.
    let mut angles = 0usize;

    core::iter::from_fn(move || {
        let start = lexer.position();

        let (class, end) = match lexer.next() {
            Ok(Some(token)) => {
                let typed = angles > 0 || (after_path && token.kind == Syntax::Lt);

                if typed && token.kind != Syntax::Whitespace {
                    angles = match token.kind {
                        Syntax::Lt => angles.saturating_add(1),
                        Syntax::Gt => angles.saturating_sub(1),
                        Syntax::NameToken
                        | Syntax::StringToken
                        | Syntax::Comma
                        | Syntax::Colon
                        | Syntax::OpenBrace
                        | Syntax::CloseBrace => angles,
                        _ => 0,
                    };
                }

                let class = match token.kind {
                    Syntax::Whitespace => Class::Whitespace,
                    Syntax::Lt | Syntax::Gt if typed => Class::Punctuation,
                    Syntax::NameToken if typed && before(&lexer, Syntax::Colon) => Class::Member,
                    Syntax::NameToken if typed => Class::Type,
                    Syntax::IntegerToken => Class::Number,
                    Syntax::StringToken => Class::String,
                    Syntax::True | Syntax::False => Class::Keyword,
                    Syntax::NameToken if after_dot => Class::Member,
                    Syntax::NameToken if before(&lexer, Syntax::Colon) => Class::Member,
                    Syntax::NameToken if before_path(&lexer) => Class::Type,
                    Syntax::NameToken if is_option_literal(source.get(start..token.end)) => {
                        Class::Keyword
                    }
                    Syntax::NameToken => Class::Ident,
                    Syntax::OpenParen
                    | Syntax::CloseParen
                    | Syntax::OpenBracket
                    | Syntax::CloseBracket
                    | Syntax::OpenBrace
                    | Syntax::CloseBrace
                    | Syntax::Comma
                    | Syntax::Dot
                    | Syntax::Colon
                    | Syntax::ColonColon => Class::Punctuation,
                    _ => Class::Operator,
                };

                if token.kind != Syntax::Whitespace {
                    after_dot = token.kind == Syntax::Dot;
                    after_path = token.kind == Syntax::ColonColon;
                }

                (class, token.end)
            }
            Ok(None) => return None,
            Err(rejected) => {
                after_dot = false;
                after_path = false;
                angles = 0;
                (Class::Error, rejected.token.end)
            }
        };

        Some(Token {
            class,
            span: start..end,
        })
    })
}

/// Whether `name` is `Some` or `None`, which build options rather than
/// name a value or a function.
fn is_option_literal(name: Option<&str>) -> bool {
    matches!(name, Some("Some" | "None"))
}

/// Whether `::` is the next token past whitespace, and no `<` of a type
/// annotation after it.
fn before_path(lexer: &Lexer<'_>) -> bool {
    let mut lexer = lexer.clone();
    next_kind(&mut lexer) == Some(Syntax::ColonColon) && next_kind(&mut lexer) != Some(Syntax::Lt)
}

/// Whether `kind` is the next token past whitespace.
fn before(lexer: &Lexer<'_>, kind: Syntax) -> bool {
    next_kind(&mut lexer.clone()) == Some(kind)
}

/// The kind of the next token past whitespace.
fn next_kind(lexer: &mut Lexer<'_>) -> Option<Syntax> {
    loop {
        match lexer.next() {
            Ok(Some(token)) if token.kind == Syntax::Whitespace => {}
            Ok(Some(token)) => return Some(token.kind),
            _ => return None,
        }
    }
}
