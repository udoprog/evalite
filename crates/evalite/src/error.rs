use std::fmt;

use syntree::Span;

use crate::message::{Labels, plural};
use crate::types::Kind;
use crate::{Syntax, Type};

/// Why text is not an expression of its environment, and where: a syntax
/// error, or a type error found when it is compiled.
///
/// It is opaque. Its [`Display`](fmt::Display) is the primary message, and
/// it is inspected through its [`span`](Self::span) and its
/// [`labels`](Self::labels).
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::with_builtins();
/// env.value("strength", Type::int())?;
///
/// let error = evalite::compile("strength + true", &env).unwrap_err();
/// assert_eq!(error.to_string(), "`+` takes Int and Int, found Int and Bool");
/// assert_eq!(error.span().range(), 0..15);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ParseError {
    span: Span<u32>,
    kind: Box<ErrorKind>,
}

impl ParseError {
    pub(crate) fn new(span: Span<u32>, kind: ErrorKind) -> Self {
        Self {
            span,
            kind: Box::new(kind),
        }
    }

    /// The bytes of the input the error is about: the span of its primary
    /// label.
    ///
    /// ```
    /// let env = evalite::Environment::with_builtins();
    ///
    /// let error = evalite::compile("1 +", &env).unwrap_err();
    /// assert_eq!(error.to_string(), "expected an expression, found the end of the expression");
    /// assert_eq!(error.span().range(), 3..3);
    /// ```
    pub fn span(&self) -> Span<u32> {
        self.span
    }

    /// Each place in the input the error is about, primary first, then
    /// secondary ones, such as where an unclosed group was opened. Every
    /// label of a parse error has a span.
    ///
    /// A syntax error has one label:
    ///
    /// ```
    /// let env = evalite::Environment::with_builtins();
    ///
    /// let error = evalite::compile("1 $ 2", &env).unwrap_err();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()), l.is_primary())).collect();
    /// assert_eq!(labels, [("unexpected character '$'".to_owned(), Some(2..3), true)]);
    /// ```
    ///
    /// An unclosed group has two, the second where it was opened:
    ///
    /// ```
    /// let env = evalite::Environment::with_builtins();
    ///
    /// let error = evalite::compile("(1 + 2", &env).unwrap_err();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()), l.is_primary())).collect();
    /// assert_eq!(labels, [
    ///     ("expected `)` to close the `(` at byte 0, found the end of the expression".to_owned(), Some(6..6), true),
    ///     ("the `(` opened here".to_owned(), Some(0..1), false),
    /// ]);
    /// ```
    ///
    /// A type error:
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("skills", Type::list(Type::int()))?;
    ///
    /// let error = evalite::compile("skills[true]", &env).unwrap_err();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()))).collect();
    /// assert_eq!(labels, [("a List<Int> is indexed by an Int, found Bool".to_owned(), Some(7..11))]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// An expression nested deeper than its options allow:
    ///
    /// ```
    /// use evalite::{Environment, Options};
    ///
    /// let env = Environment::with_builtins();
    /// let options = Options::new().max_depth(2);
    ///
    /// let error = evalite::compile_with("(((1)))", &env, &options).unwrap_err();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()))).collect();
    /// assert_eq!(labels, [("this parenthesis nests too deeply, the limit is 2 levels of nesting".to_owned(), Some(2..3))]);
    /// ```
    pub fn labels(&self) -> Labels<'_> {
        Labels::parse(self)
    }

    /// The message and span of label `n`.
    pub(crate) fn label(&self, n: usize) -> Option<(ParseText<'_>, Span<u32>)> {
        match (n, &*self.kind) {
            (0, _) => Some((ParseText::Primary(self), self.span)),
            (
                1,
                ErrorKind::Unclosed {
                    open, open_span, ..
                },
            ) => Some((ParseText::Opened(*open), *open_span)),
            _ => None,
        }
    }

    /// How many labels it has.
    pub(crate) fn label_count(&self) -> usize {
        match &*self.kind {
            ErrorKind::Unclosed { .. } => 2,
            _ => 1,
        }
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

impl fmt::Debug for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParseError")
            .field("span", &self.span)
            .field("kind", &self.kind)
            .finish()
    }
}

/// What a label of a [`ParseError`] says.
pub(crate) enum ParseText<'a> {
    /// The error itself.
    Primary(&'a ParseError),
    /// Where a group that was not closed was opened.
    Opened(Delimiter),
}

impl fmt::Display for ParseText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary(error) => error.fmt(f),
            Self::Opened(open) => write!(f, "the {} opened here", open.open_text()),
        }
    }
}

/// What is wrong with an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ErrorKind {
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
    /// A fault in evalite itself, not in the input.
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
    /// `!` after a value that is not an option.
    UnwrapNonOption(Type),
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
pub(crate) enum Callee {
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
pub(crate) enum Arity {
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

        write!(f, "{prefix}{}", plural(n, "argument", "arguments"))
    }
}

/// What was found where something else was expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Found {
    Token(Syntax),
    End,
}

/// A group that a closing `)` or `]` ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delimiter {
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
pub(crate) enum Nesting {
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

impl fmt::Display for ParseError {
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
                "{nesting} nests too deeply, the limit is {} of nesting",
                plural(*max, "level", "levels")
            ),
            ErrorKind::TooLong(len) => write!(
                f,
                "the expression is {} long, more than the {} that can be parsed",
                plural(*len, "byte", "bytes"),
                u32::MAX
            ),
            ErrorKind::Internal(message) => write!(f, "internal error in evalite: {message}"),
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
                    Type(Kind::Option(..)) if !available.contains(&method.as_str()) => {
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

                if crate::methods::method_names(receiver).contains(&field.as_str()) {
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
            ErrorKind::UnwrapNonOption(ty) => write!(
                f,
                "`!` unwraps an optional value, found {ty}, which is never absent"
            ),
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
                "this value's type nests more than {}",
                plural(crate::MAX_TYPE_DEPTH, "level", "levels")
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

impl std::error::Error for ParseError {}

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
