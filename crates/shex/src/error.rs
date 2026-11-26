use std::fmt;

use syntree::Span;

use crate::Syntax;

/// Why text is not an expression, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    span: Span<u32>,
    kind: ErrorKind,
}

impl Error {
    pub(crate) fn new(span: Span<u32>, kind: ErrorKind) -> Self {
        Self { span, kind }
    }

    /// The bytes of the input the error is about.
    pub fn span(&self) -> Span<u32> {
        self.span
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

/// What is wrong with an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A character that is no part of any token.
    UnexpectedCharacter(char),
    /// `=`, `&` or `|` alone, which are not operators.
    SingleOperator(char),
    /// A string literal without its closing `"`.
    UnterminatedString,
    /// A `\` in a string literal followed by other than `"` or `\`.
    InvalidEscape(char),
    /// An integer past the range of `Int`.
    IntegerTooLarge,
    /// An operand was expected.
    ExpectedExpression(Found),
    /// An operand was complete, but what follows cannot continue it. `open`
    /// is the innermost unclosed group.
    ExpectedOperator {
        found: Found,
        open: Option<Delimiter>,
    },
    /// A `.` not followed by a name.
    ExpectedName(Found),
    /// A group opened at `open_span` that was not closed where it had to
    /// be.
    Unclosed {
        open: Delimiter,
        open_span: Span<u32>,
        found: Found,
    },
    /// A `)` or `]` with no group open.
    Unmatched(Found),
    /// A comparison of a comparison, such as `a < b < c`.
    ChainedComparison,
    /// A construct nested more than the parser's `max_depth` levels deep.
    TooDeep { nesting: Nesting, max: usize },
    /// An input longer than spans can address.
    TooLong(usize),
    /// A fault in the parser itself, not in the input.
    Internal(String),
}

/// What was found where something else was expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Token(Syntax),
    End,
}

/// A group that a closing `)` or `]` ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// `( expr )`.
    Paren,
    /// A call's or method's `( args )`.
    Args,
    /// A list `[ entries ]`.
    List,
    /// An index `value[ expr ]`.
    Index,
}

impl Delimiter {
    pub(crate) fn close(self) -> Syntax {
        match self {
            Self::Paren | Self::Args => Syntax::CloseParen,
            Self::List | Self::Index => Syntax::CloseBracket,
        }
    }

    fn open_text(self) -> &'static str {
        match self {
            Self::Paren | Self::Args => "`(`",
            Self::List | Self::Index => "`[`",
        }
    }

    fn separated(self) -> bool {
        matches!(self, Self::Args | Self::List)
    }
}

/// A construct that nests one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nesting {
    Paren,
    Args,
    List,
    Index,
    /// A prefix operator, `-` or `!`.
    Unary(Syntax),
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(syntax) => f.write_str(syntax.describe()),
            Self::End => f.write_str("the end of the expression"),
        }
    }
}

impl fmt::Display for Nesting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paren => f.write_str("this parenthesis"),
            Self::Args => f.write_str("this call"),
            Self::List => f.write_str("this list"),
            Self::Index => f.write_str("this index"),
            Self::Unary(op) => write!(f, "this {}", op.describe()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::UnexpectedCharacter(c) => {
                write!(f, "unexpected character {c:?}")
            }
            ErrorKind::SingleOperator(c) => {
                write!(f, "`{c}` is not an operator, did you mean `{c}{c}`?")
            }
            ErrorKind::UnterminatedString => {
                f.write_str("this string is never closed, expected a `\"` before the end")
            }
            ErrorKind::InvalidEscape(c) => write!(
                f,
                "unknown escape `\\{c}` in a string, only `\\\"` and `\\\\` are allowed"
            ),
            ErrorKind::IntegerTooLarge => {
                write!(f, "this integer is too large, the largest is {}", i64::MAX)
            }
            ErrorKind::ExpectedExpression(found) => {
                write!(f, "expected an expression, found {found}")
            }
            ErrorKind::ExpectedOperator { found, open } => match open {
                Some(open) if open.separated() => write!(
                    f,
                    "expected an operator, `,` or `{}`, found {found}",
                    closer(*open)
                ),
                Some(open) => write!(
                    f,
                    "expected an operator or `{}`, found {found}",
                    closer(*open)
                ),
                None => write!(
                    f,
                    "expected an operator or the end of the expression, found {found}"
                ),
            },
            ErrorKind::ExpectedName(found) => {
                write!(
                    f,
                    "expected a field or method name after `.`, found {found}"
                )
            }
            ErrorKind::Unclosed {
                open,
                open_span,
                found,
            } => write!(
                f,
                "expected `{}` to close the {} at byte {}, found {found}",
                closer(*open),
                open.open_text(),
                open_span.start
            ),
            ErrorKind::Unmatched(found) => {
                write!(f, "unmatched {found}, there is no group open to close")
            }
            ErrorKind::ChainedComparison => {
                f.write_str("comparisons do not chain, combine them with `&&` or add parentheses")
            }
            ErrorKind::TooDeep { nesting, max } => write!(
                f,
                "{nesting} nests too deeply, the limit is {max} levels of nesting"
            ),
            ErrorKind::TooLong(len) => write!(
                f,
                "the expression is {len} bytes long, more than the {} that can be parsed",
                u32::MAX
            ),
            ErrorKind::Internal(message) => write!(f, "internal parser error: {message}"),
        }
    }
}

fn closer(open: Delimiter) -> char {
    match open.close() {
        Syntax::CloseBracket => ']',
        _ => ')',
    }
}

impl std::error::Error for Error {}
