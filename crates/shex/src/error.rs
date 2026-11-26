use std::fmt;

use syntree::Span;

use crate::{Syntax, Type};

/// Why text is not an expression, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    span: Span<u32>,
    kind: Box<ErrorKind>,
}

impl Error {
    pub(crate) fn new(span: Span<u32>, kind: ErrorKind) -> Self {
        Self {
            span,
            kind: Box::new(kind),
        }
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
    /// A fault in shex itself, not in the input.
    Internal(String),

    /// A name the environment does not declare as a value. `suggestion` is
    /// a declared name it may be a misspelling of.
    UnknownName {
        name: String,
        suggestion: Option<String>,
    },
    /// A function used as a value, such as `min + 1`.
    FunctionAsValue(String),
    /// A call of a function the environment does not declare.
    UnknownFunction {
        name: String,
        suggestion: Option<String>,
    },
    /// A call of a name that is a value, not a function.
    NotAFunction(String),
    /// An operator applied to operands of types it does not take.
    Operator { op: Syntax, operands: Vec<Type> },
    /// A call with too few or too many arguments.
    ArgumentCount {
        callee: Callee,
        expected: Arity,
        found: usize,
    },
    /// A call's argument of the wrong type, counting from 1.
    ArgumentType {
        callee: Callee,
        index: usize,
        expected: String,
        found: Type,
    },
    /// A call of a function declared several ways, none of which takes
    /// these arguments.
    NoOverload {
        function: String,
        found: Vec<Type>,
        candidates: Vec<String>,
    },
    /// A method the receiver's type does not have.
    UnknownMethod {
        receiver: Type,
        method: String,
        available: Vec<&'static str>,
    },
    /// `.name` on a value without fields.
    NoFields { receiver: Type, field: String },
    /// `.name` or `["name"]` on a record without that field.
    UnknownField { field: String, fields: Vec<String> },
    /// `[...]` after a value that is neither a list nor a record.
    NotIndexable(Type),
    /// A list indexed by other than an `Int`.
    IndexType { list: Type, found: Type },
    /// A record indexed by other than a string literal.
    RecordIndex,
    /// A list entry of another type than the first entry, counting from 1.
    ListEntry {
        index: usize,
        expected: Type,
        found: Type,
    },
    /// `[]`, whose entry type is unknown.
    EmptyList,
    /// A value whose type nests more than
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH) levels.
    TypeTooDeep,
    /// A program too large to count its parts.
    TooLarge,
}

/// What a call calls, as an error names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callee {
    Function(String),
    Method { receiver: Type, name: String },
}

impl fmt::Display for Callee {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function(name) => write!(f, "`{name}`"),
            Self::Method { receiver, name } => write!(f, "`{name}` on {receiver}"),
        }
    }
}

/// How many arguments a function takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Exactly(usize),
    AtLeast(usize),
}

impl fmt::Display for Arity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, n) = match *self {
            Self::Exactly(0) => return f.write_str("no arguments"),
            Self::Exactly(n) => ("", n),
            Self::AtLeast(n) => ("at least ", n),
        };

        let s = if n == 1 { "" } else { "s" };
        write!(f, "{prefix}{n} argument{s}")
    }
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
        match &*self.kind {
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
            ErrorKind::Internal(message) => write!(f, "internal error in shex: {message}"),
            ErrorKind::UnknownName { name, suggestion } => {
                write!(f, "unknown name `{name}`")?;
                did_you_mean(f, suggestion)
            }
            ErrorKind::FunctionAsValue(name) => {
                write!(f, "`{name}` is a function, call it with `{name}(...)`")
            }
            ErrorKind::UnknownFunction { name, suggestion } => {
                write!(f, "unknown function `{name}`")?;
                did_you_mean(f, suggestion)
            }
            ErrorKind::NotAFunction(name) => write!(f, "`{name}` is a value, not a function"),
            ErrorKind::Operator { op, operands } => {
                let expected = match (op, operands.len()) {
                    (Syntax::Minus, 1) => "an Int",
                    (Syntax::Bang, 1) => "a Bool",
                    (Syntax::AndAnd | Syntax::OrOr, _) => "Bool and Bool",
                    (Syntax::EqEq | Syntax::NotEq, _) => "two Ints or two Bools",
                    _ => "Int and Int",
                };

                let found = operands
                    .iter()
                    .map(Type::to_string)
                    .collect::<Vec<_>>()
                    .join(" and ");

                write!(f, "{} takes {expected}, found {found}", op.describe())?;
                unwrap_hint(f, operands)
            }
            ErrorKind::ArgumentCount {
                callee,
                expected,
                found,
            } => write!(f, "{callee} takes {expected}, found {found}"),
            ErrorKind::ArgumentType {
                callee,
                index,
                expected,
                found,
            } => {
                write!(
                    f,
                    "argument {index} of {callee} must be {expected}, found {found}"
                )?;
                unwrap_hint(f, std::slice::from_ref(found))
            }
            ErrorKind::NoOverload {
                function,
                found,
                candidates,
            } => {
                let found = found
                    .iter()
                    .map(Type::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "no `{function}` takes ({found}), it takes {}",
                    list(candidates.iter().map(String::as_str), "or")
                )
            }
            ErrorKind::UnknownMethod {
                receiver,
                method,
                available,
            } => {
                write!(f, "{receiver} has no method `{method}`")?;

                match receiver {
                    Type::Option(..) if !available.contains(&method.as_str()) => {
                        f.write_str(", unwrap the optional value first with `.unwrap_or(...)`")
                    }
                    _ if available.is_empty() => f.write_str(", it has no methods"),
                    _ => write!(
                        f,
                        ", its methods are {}",
                        list(available.iter().map(|m| format!("`{m}`")), "and")
                    ),
                }
            }
            ErrorKind::NoFields { receiver, field } => {
                write!(f, "{receiver} has no fields, so no `.{field}`")?;

                if crate::check::method_names(receiver).contains(&field.as_str()) {
                    write!(f, ", `{field}` is a method, call it with `.{field}(...)`")?;
                }

                unwrap_hint(f, std::slice::from_ref(receiver))
            }
            ErrorKind::UnknownField { field, fields } => {
                const SHOWN: usize = 8;

                write!(f, "this record has no field {field:?}")?;

                let mut shown = fields
                    .iter()
                    .take(SHOWN)
                    .map(|f| format!("{f:?}"))
                    .collect::<Vec<_>>();

                if let Some(more) = fields.len().checked_sub(SHOWN).filter(|n| *n > 0) {
                    shown.push(format!("{more} more"));
                }

                write!(f, ", its fields are {}", list(shown, "and"))
            }
            ErrorKind::NotIndexable(ty) => {
                write!(f, "{ty} cannot be indexed, only a List or a record can")?;
                unwrap_hint(f, std::slice::from_ref(ty))
            }
            ErrorKind::IndexType { list, found } => {
                write!(f, "a {list} is indexed by an Int, found {found}")
            }
            ErrorKind::RecordIndex => f.write_str(
                "a record is indexed by a string literal naming its field, such as `m[\"name\"]`",
            ),
            ErrorKind::ListEntry {
                index,
                expected,
                found,
            } => write!(
                f,
                "entry {index} of this list is {found}, but the first is {expected}, a list's entries have one type"
            ),
            ErrorKind::EmptyList => {
                f.write_str("an empty list `[]` has no entry type, give it at least one entry")
            }
            ErrorKind::TypeTooDeep => write!(
                f,
                "this value's type nests more than {} levels",
                crate::MAX_TYPE_DEPTH
            ),
            ErrorKind::TooLarge => f.write_str("this expression is too large to compile"),
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

fn did_you_mean(f: &mut fmt::Formatter<'_>, suggestion: &Option<String>) -> fmt::Result {
    match suggestion {
        Some(s) => write!(f, ", did you mean `{s}`?"),
        None => Ok(()),
    }
}

fn unwrap_hint(f: &mut fmt::Formatter<'_>, types: &[Type]) -> fmt::Result {
    if types.iter().any(Type::is_option) {
        f.write_str(", unwrap the optional value first with `.unwrap_or(...)`")?;
    }

    Ok(())
}

/// `a`, `a and b`, `a, b and c`.
fn list<S: AsRef<str>>(items: impl IntoIterator<Item = S>, last: &str) -> String {
    let items = items.into_iter().collect::<Vec<_>>();
    let mut out = String::new();

    for (n, item) in items.iter().enumerate() {
        if n > 0 {
            out.push_str(if n.saturating_add(1) == items.len() {
                if last == "and" { " and " } else { " or " }
            } else {
                ", "
            });
        }

        out.push_str(item.as_ref());
    }

    out
}
