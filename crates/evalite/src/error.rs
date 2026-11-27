use std::fmt;

use syntree::Span;

use crate::literal::{Expected, LiteralFound};
use crate::message::{Labels, plural};
use crate::types::Kind;
use crate::{Syntax, Type};

/// One reason text is not an expression of its environment, and where: a
/// syntax error, or a type error found when it is compiled. Compiling
/// reports them as [`Diagnostics`].
///
/// It is opaque. Its [`Display`](fmt::Display) is the primary message, and
/// it is inspected through its [`span`](Self::span) and its
/// [`labels`](Self::labels).
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
///
/// let errors = evalite::compile("strength + true", &env).unwrap_err();
/// let error = errors.iter().next().unwrap();
/// assert_eq!(error.to_string(), "`+` takes Int and Int, found Int and Bool");
/// assert_eq!(error.span().range(), 0..15);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Diagnostic {
    span: Span<u32>,
    kind: Box<ErrorKind>,
}

impl Diagnostic {
    pub(crate) fn new(span: Span<u32>, kind: ErrorKind) -> Self {
        Self {
            span,
            kind: Box::new(kind),
        }
    }

    /// The diagnostic of text parsed on its own, moved to where the text
    /// starts at `by` in a larger source.
    pub(crate) fn shifted(mut self, by: u32) -> Self {
        let start = self.span.start.saturating_add(by);
        let end = self.span.end.saturating_add(by);
        self.span = Span::new(start, end);
        self
    }

    /// The bytes of the input the error is about: the span of its primary
    /// label.
    ///
    /// ```
    /// let env = evalite::Environment::new();
    ///
    /// let errors = evalite::compile("1 +", &env).unwrap_err();
    /// let error = errors.iter().next().unwrap();
    /// assert_eq!(error.to_string(), "expected an expression, found the end of the expression");
    /// assert_eq!(error.span().range(), 3..3);
    /// ```
    pub fn span(&self) -> Span<u32> {
        self.span
    }

    /// Each place in the input the error is about, primary first, then
    /// secondary ones, such as where an unclosed group had to close. Every
    /// label of a parse error has a span.
    ///
    /// A syntax error has one label:
    ///
    /// ```
    /// let env = evalite::Environment::new();
    ///
    /// let errors = evalite::compile("1 $ 2", &env).unwrap_err();
    /// let error = errors.iter().next().unwrap();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()), l.is_primary())).collect();
    /// assert_eq!(labels, [("unexpected character '$'".to_owned(), Some(2..3), true)]);
    /// ```
    ///
    /// An unclosed group has two, the first at its opener and the second
    /// where it had to close:
    ///
    /// ```
    /// let env = evalite::Environment::new();
    ///
    /// let errors = evalite::compile("(1 + 2", &env).unwrap_err();
    /// let error = errors.iter().next().unwrap();
    /// let labels: Vec<_> = error.labels().map(|l| (l.message().to_string(), l.span().map(|s| s.range()), l.is_primary())).collect();
    /// assert_eq!(labels, [
    ///     ("expected `)` to close the `(` at byte 0, found the end of the expression".to_owned(), Some(0..1), true),
    ///     ("expected `)` here".to_owned(), Some(6..6), false),
    /// ]);
    /// ```
    ///
    /// A type error:
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("skills", Type::list(Type::int()))?;
    ///
    /// let errors = evalite::compile("skills[true]", &env).unwrap_err();
    /// let error = errors.iter().next().unwrap();
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
    /// let env = Environment::new();
    /// let options = Options::new().max_depth(2);
    ///
    /// let errors = evalite::compile_with("(((1)))", &env, &options).unwrap_err();
    /// let error = errors.iter().next().unwrap();
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
                    open, close_span, ..
                },
            ) => Some((ParseText::Expected(*open), *close_span)),
            (1, ErrorKind::RepeatedKey { first, .. } | ErrorKind::RepeatedField { first, .. }) => {
                Some((ParseText::FirstKey, *first))
            }
            _ => None,
        }
    }

    /// Whether it is about evalite itself or the size of the whole
    /// program, not one expression, so nothing after it can be checked.
    pub(crate) fn is_fatal(&self) -> bool {
        matches!(&*self.kind, ErrorKind::Internal(_) | ErrorKind::TooLarge)
    }

    /// How many labels it has.
    pub(crate) fn label_count(&self) -> usize {
        match &*self.kind {
            ErrorKind::Unclosed { .. }
            | ErrorKind::RepeatedKey { .. }
            | ErrorKind::RepeatedField { .. } => 2,
            _ => 1,
        }
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> &ErrorKind {
        &self.kind
    }
}

impl fmt::Debug for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostic")
            .field("span", &self.span)
            .field("kind", &self.kind)
            .finish()
    }
}

/// Every reason text is not an expression of its environment, in source
/// order: what [`compile`](crate::compile) returns when it fails. It is
/// never empty.
///
/// Compiling keeps going after an error, so independent mistakes are all
/// reported, while one mistake is reported once, not again by everything
/// around it. At most [`MAX_DIAGNOSTICS`] are kept, followed by one saying
/// how many more there were.
///
/// Its [`Display`](fmt::Display) is each [`Diagnostic`] on its own line.
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("rank", Type::int())?;
///
/// let errors = evalite::compile("rank + true + bonus * nothing", &env).unwrap_err();
/// assert_eq!(errors.len(), 3);
///
/// let messages: Vec<_> = errors.iter().map(|e| (e.to_string(), e.span().range())).collect();
/// assert_eq!(messages, [
///     ("`+` takes Int and Int, found Int and Bool".to_owned(), 0..11),
///     ("unknown name `bonus`".to_owned(), 14..19),
///     ("unknown name `nothing`".to_owned(), 22..29),
/// ]);
///
/// // A syntax error is recovered from, and what is around it checked.
/// let errors = evalite::compile("Int::min(rank, , true) + bonus", &env).unwrap_err();
/// let messages: Vec<_> = errors.iter().map(|e| (e.to_string(), e.span().range())).collect();
/// assert_eq!(messages, [
///     ("expected an expression, found `,`".to_owned(), 15..16),
///     ("unknown name `bonus`".to_owned(), 25..30),
/// ]);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    list: Vec<Diagnostic>,
}

/// The most [`Diagnostics`] keeps; past it, one more diagnostic says how
/// many were left out.
///
/// ```
/// let env = evalite::Environment::new();
/// let source = vec!["x"; 150].join(" + ");
///
/// let errors = evalite::compile(&source, &env).unwrap_err();
/// assert_eq!(errors.len(), evalite::MAX_DIAGNOSTICS + 1);
/// assert_eq!(errors.iter().last().unwrap().to_string(), "and 50 more errors");
/// ```
pub const MAX_DIAGNOSTICS: usize = 100;

impl Diagnostics {
    /// Diagnostics of one diagnostic.
    pub(crate) fn one(diagnostic: Diagnostic) -> Self {
        Self {
            list: vec![diagnostic],
        }
    }

    /// How many diagnostics there are, at least one.
    ///
    /// ```
    /// let env = evalite::Environment::new();
    ///
    /// assert_eq!(evalite::compile("a + b", &env).unwrap_err().len(), 2);
    /// ```
    #[allow(clippy::len_without_is_empty, reason = "it is never empty")]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Each diagnostic, in source order.
    ///
    /// ```
    /// let env = evalite::Environment::new();
    ///
    /// let errors = evalite::compile("a + b", &env).unwrap_err();
    /// let spans: Vec<_> = errors.iter().map(|e| e.span().range()).collect();
    /// assert_eq!(spans, [0..1, 4..5]);
    /// ```
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.list.iter()
    }

    /// The first diagnostic.
    pub(crate) fn first(&self) -> Option<&Diagnostic> {
        self.list.first()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.into_iter()
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, diagnostic) in self.list.iter().enumerate() {
            if n > 0 {
                f.write_str("\n")?;
            }

            diagnostic.fmt(f)?;
        }

        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

/// Collects diagnostics as they are found, keeping the
/// [`MAX_DIAGNOSTICS`] earliest in the source, so its memory is bounded
/// whatever the input.
#[derive(Default)]
pub(crate) struct Reporter {
    /// Each kept diagnostic, with the order it was found in.
    kept: Vec<(Diagnostic, usize)>,
    found: usize,
    /// How many were left out, and the span of the earliest of them.
    dropped: Option<(usize, Span<u32>)>,
}

impl Reporter {
    pub(crate) fn report(&mut self, diagnostic: Diagnostic) {
        let key = (diagnostic.span.start, self.found);
        self.found = self.found.saturating_add(1);

        let out = if self.kept.len() < MAX_DIAGNOSTICS {
            self.kept.push((diagnostic, key.1));
            None
        } else {
            let last = self
                .kept
                .iter()
                .enumerate()
                .max_by_key(|(_, (d, seq))| (d.span.start, *seq))
                .map(|(n, (d, seq))| (n, (d.span.start, *seq)));

            match last {
                Some((n, last)) if key < last => self
                    .kept
                    .get_mut(n)
                    .map(|slot| std::mem::replace(slot, (diagnostic, key.1)).0),
                _ => Some(diagnostic),
            }
        };

        if let Some(out) = out {
            self.dropped = Some(match self.dropped {
                Some((n, span)) if span.start <= out.span.start => (n.saturating_add(1), span),
                Some((n, _)) => (n.saturating_add(1), out.span),
                None => (1, out.span),
            });
        }
    }

    /// The diagnostics found, in source order, if any were.
    pub(crate) fn finish(mut self) -> Option<Diagnostics> {
        if self.kept.is_empty() {
            return None;
        }

        self.kept.sort_by_key(|(d, seq)| (d.span.start, *seq));
        let mut list = self.kept.into_iter().map(|(d, _)| d).collect::<Vec<_>>();

        if let Some((n, span)) = self.dropped {
            list.push(Diagnostic::new(span, ErrorKind::MoreErrors(n)));
        }

        Some(Diagnostics { list })
    }
}

/// What a label of a [`Diagnostic`] says.
pub(crate) enum ParseText<'a> {
    /// The error itself.
    Primary(&'a Diagnostic),
    /// Where a group that was not closed had to be closed.
    Expected(Delimiter),
    /// Where a map key given twice is given first.
    FirstKey,
}

impl fmt::Display for ParseText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary(error) => error.fmt(f),
            Self::Expected(open) => write!(f, "expected `{}` here", closer(*open)),
            Self::FirstKey => f.write_str("given first here"),
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
    /// A `0x` not followed by a hexadecimal digit.
    HexWithoutDigits,
    /// An operand was expected.
    ExpectedExpression(Found),
    /// An operand was complete, but what follows cannot continue it. `open`
    /// is the innermost unclosed group.
    ExpectedOperator {
        found: Found,
        open: Option<Delimiter>,
    },
    /// A map's key followed by other than an operator or `:`.
    ExpectedKeyEnd(Found),
    /// A `.` not followed by a name.
    ExpectedName(Found),
    /// No function name after a type's `::`.
    ExpectedPathName(Found),
    /// A type's function used as a value, such as `Int::min + 1`.
    PathAsValue(String),
    /// A type annotation `::<T>` after a name other than `None`.
    AnnotatedName(String),
    /// An annotation's type not followed by `>`.
    ExpectedAnnotationEnd(Found),
    /// A group that was not closed at `close_span`, where it had to be.
    /// The diagnostic's span is its opener.
    Unclosed {
        open: Delimiter,
        close_span: Span<u32>,
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
    /// An input longer than the configured `max_len`.
    OverLimit { len: usize, max: usize },
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
    /// A path whose type has no functions, such as `Foo::min`.
    UnknownPathType(String),
    /// A function the type of a path does not have, such as `Int::foo`.
    UnknownAssociated {
        ty: &'static str,
        name: String,
        available: Vec<&'static str>,
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
    /// `[...]` after a value that is not a list, a map or a record.
    NotIndexable(Type),
    /// `!` after a value that is not an option.
    UnwrapNonOption(Type),
    /// A list indexed by other than an `Int`.
    IndexType { list: Type, found: Type },
    /// A map indexed by other than its key type.
    KeyType { map: Type, key: Type, found: Type },
    /// A record indexed by other than a string literal.
    RecordIndex,
    /// A list entry of another type than the first entry, counting from 1.
    ListEntry {
        index: usize,
        expected: Type,
        found: Type,
    },
    /// `[]` where no list type is expected, or where the type given is
    /// expected, which is not a list.
    EmptyList(Option<Type>),
    /// A map key other than an integer or string literal.
    MapKeyLiteral,
    /// A map key of another type than the first key, counting entries
    /// from 1.
    MapKeyMixed {
        index: usize,
        expected: Type,
        found: Type,
    },
    /// A map key given twice, `key` as its value is written in messages;
    /// `first` is where it is first given.
    RepeatedKey { key: String, first: Span<u32> },
    /// A map value of another type than the first value, counting entries
    /// from 1.
    MapValue {
        index: usize,
        expected: Type,
        found: Type,
    },
    /// `{}` where no map type is expected, or where the type given is
    /// expected, which is not a map.
    EmptyMap(Option<Type>),
    /// A key of a `{...}` that is a name where the first is a literal, or
    /// the other way around: names make a record, literals a map.
    RecordKeyMixed {
        /// Whether the first key is a name, so it is a record.
        record: bool,
    },
    /// A record literal's field given twice; `first` is where it is first
    /// given.
    RepeatedField { field: String, first: Span<u32> },
    /// A record literal's field that the record type expected of it lacks.
    ExtraField { field: String, expected: Type },
    /// The fields of the record type expected of a record literal that it
    /// does not give, in name order.
    MissingFields { fields: Vec<String>, expected: Type },
    /// `None` where no option type is expected, or where the type given
    /// is expected, which is not an option.
    NoneType(Option<Type>),
    /// `None::<T>`, an `annotated` option, where another type is
    /// `expected`.
    AnnotatedNone { annotated: Type, expected: Type },
    /// A value whose type nests more than
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH) levels.
    TypeTooDeep,
    /// A value whose type has more than
    /// [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES) nodes once written out.
    TypeTooLarge,
    /// A program whose globals' and host functions' types, written out
    /// together, have more than `max` nodes.
    TypesTooLarge { max: usize },
    /// A program too large to count its parts.
    TooLarge,
    /// The diagnostics past [`MAX_DIAGNOSTICS`], which are left out.
    MoreErrors(usize),

    /// A type or value literal longer than
    /// [`DEFAULT_MAX_LEN`](crate::DEFAULT_MAX_LEN).
    LiteralTooLong { len: usize, max: usize },
    /// A type was expected.
    ExpectedType(LiteralFound),
    /// A word that names no type.
    UnknownType(String),
    /// A type literal nesting more than
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH) levels.
    LiteralTooDeep,
    /// A punctuation character was expected.
    ExpectedChar { expected: char, found: LiteralFound },
    /// A record field's name was expected.
    ExpectedFieldName(LiteralFound),
    /// A value of the type was expected.
    ExpectedValue { ty: Type, found: LiteralFound },
    /// A literal is complete, but more follows it.
    ExpectedEnd(LiteralFound),
    /// An integer below the range of `Int`.
    IntegerTooSmall,
    /// A record field given twice.
    DuplicateField(String),
    /// A map key given twice, as written.
    DuplicateKey(String),
    /// A map type keyed by other than `Int` or `String`.
    MapKeyType(Type),
    /// A field the record type does not have.
    NoSuchField { ty: Type, field: String },
    /// A field of the record type that is given no value.
    MissingField(String),
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
    /// A map `{ key: value, ... }`.
    Map,
}

impl Delimiter {
    pub(crate) fn close(self) -> Syntax {
        match self {
            Self::Paren | Self::Args => Syntax::CloseParen,
            Self::List | Self::Index => Syntax::CloseBracket,
            Self::Map => Syntax::CloseBrace,
        }
    }

    fn open_text(self) -> &'static str {
        match self {
            Self::Paren | Self::Args => "`(`",
            Self::List | Self::Index => "`[`",
            Self::Map => "`{`",
        }
    }

    /// Whether `,` separates its entries.
    pub(crate) fn separated(self) -> bool {
        matches!(self, Self::Args | Self::List | Self::Map)
    }
}

/// A construct that nests one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nesting {
    Paren,
    Args,
    List,
    Index,
    Map,
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
            Self::Map => f.write_str("this map"),
            Self::Unary(op) => write!(f, "this {}", op.describe()),
        }
    }
}

impl fmt::Display for Diagnostic {
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
            ErrorKind::HexWithoutDigits => {
                f.write_str("`0x` starts a hexadecimal integer, expected digits `0` to `9` or `a` to `f` after it")
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
            ErrorKind::ExpectedKeyEnd(found) => {
                write!(f, "expected an operator or `:` after this map key, found {found}")
            }
            ErrorKind::ExpectedName(found) => {
                write!(
                    f,
                    "expected a field or method name after `.`, found {found}"
                )
            }
            ErrorKind::Unclosed { open, found, .. } => write!(
                f,
                "expected `{}` to close the {} at byte {}, found {found}",
                closer(*open),
                open.open_text(),
                self.span.start
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
            ErrorKind::OverLimit { len, max } => write!(
                f,
                "the expression is {} long, the limit is {max}",
                plural(*len, "byte", "bytes"),
            ),
            ErrorKind::Internal(message) => write!(f, "internal error in evalite: {message}"),
            ErrorKind::UnknownName { name, suggestion } => {
                write!(f, "unknown name `{name}`")?;
                did_you_mean(f, suggestion)
            }
            ErrorKind::ExpectedPathName(found) => {
                write!(f, "expected a function name after `::`, found {found}")
            }
            ErrorKind::AnnotatedName(name) => write!(
                f,
                "only `None` takes a type in `::<..>`, such as `None::<Int>`, not `{name}`"
            ),
            ErrorKind::ExpectedAnnotationEnd(found) => {
                write!(f, "expected `>` after the type, found {found}")
            }
            ErrorKind::PathAsValue(name) | ErrorKind::FunctionAsValue(name) => {
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
                    (Syntax::EqEq | Syntax::NotEq, _) => "two Ints, two Bools or two options of them",
                    _ => "Int and Int",
                };

                let found = operands
                    .iter()
                    .map(Type::to_string)
                    .collect::<Vec<_>>()
                    .join(" and ");

                write!(f, "{} takes {expected}, found {found}", op.describe())?;

                // Two options of one type compare when anything they hold
                // does, so unwrapping them would not help.
                if matches!(op, Syntax::EqEq | Syntax::NotEq)
                    && matches!(operands.as_slice(), [a, b] if a == b)
                {
                    return Ok(());
                }

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
            ErrorKind::UnknownPathType(ty) => {
                write!(f, "`{ty}` is not a type with functions, such as `Int`")
            }
            ErrorKind::UnknownAssociated {
                ty,
                name,
                available,
            } => write!(
                f,
                "{ty} has no function `{name}`, its functions are {}",
                list(available.iter().map(|m| format!("`{m}`")), "and")
            ),
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
                write!(
                    f,
                    "{ty} cannot be indexed, only a List, a Map or a record can"
                )?;
                unwrap_hint(f, std::slice::from_ref(ty))
            }
            ErrorKind::UnwrapNonOption(ty) => write!(
                f,
                "`!` unwraps an optional value, found {ty}, which is never absent"
            ),
            ErrorKind::IndexType { list, found } => {
                write!(f, "a {list} is indexed by an Int, found {found}")
            }
            ErrorKind::KeyType { map, key, found } => {
                let key = key.to_string();
                let a = article(&key);
                write!(f, "a {map} is indexed by its key, {a} {key}, found {found}")
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
            ErrorKind::EmptyList(None) => f.write_str(
                "an empty list `[]` needs a type from where it is used, such as beside a typed entry in a list or as an argument that takes a list, or give it at least one entry",
            ),
            ErrorKind::EmptyList(Some(expected)) => {
                let expected = expected.to_string();
                let a = article(&expected);
                write!(f, "`[]` is a list, but {a} {expected} is expected here")
            }
            ErrorKind::MapKeyLiteral => f.write_str(
                "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            ),
            ErrorKind::MapKeyMixed {
                index,
                expected,
                found,
            } => write!(
                f,
                "key {index} of this map is {found}, but the first is {expected}, a map's keys have one type"
            ),
            ErrorKind::RepeatedKey { key, .. } => {
                write!(f, "the key {key} is given twice in this map")
            }
            ErrorKind::MapValue {
                index,
                expected,
                found,
            } => write!(
                f,
                "the value of entry {index} of this map is {found}, but the first is {expected}, a map's values have one type"
            ),
            ErrorKind::RecordKeyMixed { record: true } => f.write_str(
                "this key is no name, but the first is, which makes this a record, whose fields are all names, such as `{a: 1, b: 2}`",
            ),
            ErrorKind::RecordKeyMixed { record: false } => f.write_str(
                "this key is a name, but the first is a literal, which makes this a map, whose keys are all integer or string literals, such as `{\"a\": 1}`",
            ),
            ErrorKind::RepeatedField { field, .. } => {
                write!(f, "the field `{field}` is given twice in this record")
            }
            ErrorKind::ExtraField { field, expected } => {
                write!(f, "a {expected} is expected here, which has no field `{field}`")
            }
            ErrorKind::MissingFields { fields, expected } => {
                let names = fields
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let noun = if fields.len() == 1 { "field" } else { "fields" };
                write!(
                    f,
                    "a {expected} is expected here, but this record lacks its {noun} {names}"
                )
            }
            ErrorKind::EmptyMap(None) => f.write_str(
                "an empty map `{}` needs a type from where it is used, such as beside a typed entry in a list or as an argument that takes a map, or give it at least one entry",
            ),
            ErrorKind::EmptyMap(Some(expected)) => {
                let expected = expected.to_string();
                let a = article(&expected);
                write!(f, "`{{}}` is a map, but {a} {expected} is expected here")
            }
            ErrorKind::NoneType(None) => f.write_str(
                "`None` needs a type from where it is used, such as beside `Some(...)` in a list or as an argument of an option, or written after it, such as `None::<Int>`",
            ),
            ErrorKind::AnnotatedNone {
                annotated,
                expected,
            } => {
                let expected = expected.to_string();
                let a = article(&expected);
                write!(f, "this `None` is {annotated}, but {a} {expected} is expected here")
            }
            ErrorKind::NoneType(Some(expected)) => {
                let expected = expected.to_string();
                let a = article(&expected);
                write!(f, "`None` is an option, but {a} {expected} is expected here")
            }
            ErrorKind::TypeTooDeep => write!(
                f,
                "this value's type nests more than {}",
                plural(crate::MAX_TYPE_DEPTH, "level", "levels")
            ),
            ErrorKind::TypeTooLarge => write!(
                f,
                "this value's type has more than {} parts once written out, which is the most a type may have",
                crate::MAX_TYPE_NODES
            ),
            ErrorKind::TypesTooLarge { max } => write!(
                f,
                "the types of the values and functions used so far have more than {max} parts together once written out, which is the most an expression's may have"
            ),
            ErrorKind::TooLarge => f.write_str("this expression is too large to compile"),
            ErrorKind::MoreErrors(n) => {
                write!(f, "and {}", plural(*n, "more error", "more errors"))
            }
            ErrorKind::LiteralTooLong { len, max } => write!(
                f,
                "the text is {} long, the limit is {max}",
                plural(*len, "byte", "bytes"),
            ),
            ErrorKind::ExpectedType(found) => write!(f, "expected a type, found {found}"),
            ErrorKind::UnknownType(name) => write!(
                f,
                "unknown type `{name}`, expected Int, Bool, String, Option<T>, List<T>, Map<K, V> or {{name: T}}"
            ),
            ErrorKind::LiteralTooDeep => write!(
                f,
                "this type nests more than {}",
                plural(crate::MAX_TYPE_DEPTH, "level", "levels")
            ),
            ErrorKind::ExpectedChar { expected, found } => {
                write!(f, "expected `{expected}`, found {found}")
            }
            ErrorKind::ExpectedFieldName(found) => {
                write!(f, "expected a field name, found {found}")
            }
            ErrorKind::ExpectedValue { ty, found } => {
                write!(f, "expected {}, found {found}", Expected(ty))
            }
            ErrorKind::ExpectedEnd(found) => write!(f, "expected the end, found {found}"),
            ErrorKind::IntegerTooSmall => {
                write!(f, "this integer is too small, the smallest is {}", i64::MIN)
            }
            ErrorKind::DuplicateField(name) => write!(f, "the field `{name}` is given twice"),
            ErrorKind::DuplicateKey(key) => write!(f, "the key {key} is given twice"),
            ErrorKind::MapKeyType(ty) => {
                write!(f, "a map's keys are Int or String, found {ty}")
            }
            ErrorKind::NoSuchField { ty, field } => write!(f, "{ty} has no field `{field}`"),
            ErrorKind::MissingField(name) => write!(f, "the field `{name}` has no value"),
        }
    }
}

fn closer(open: Delimiter) -> char {
    match open.close() {
        Syntax::CloseBracket => ']',
        Syntax::CloseBrace => '}',
        _ => ')',
    }
}

impl std::error::Error for Diagnostic {}

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

/// The indefinite article for a type's name as written: `an Int`, `a Bool`.
pub(crate) fn article(name: &str) -> &'static str {
    match name.chars().next() {
        Some('A' | 'E' | 'I' | 'O' | 'U' | 'a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}
