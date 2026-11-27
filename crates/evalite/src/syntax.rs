/// The kinds of a parse tree's nodes and tokens.
///
/// Nodes hold their parts in source order, whitespace tokens included:
/// a `Binary` holds its left operand, its operator token and its right
/// operand; a `Method` its receiver, `.`, a `Name` and its `Args`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub(crate) enum Syntax {
    /// The whole input: one expression and any whitespace around it.
    Root,
    /// An integer literal.
    Integer,
    /// A string literal.
    String,
    /// `true` or `false`.
    Bool,
    /// A name: a value, or with `Args` after it in a `Call`, a function.
    Name,
    /// `( expr )`.
    Paren,
    /// A prefix operator and its operand.
    Unary,
    /// Two operands and the operator between them.
    Binary,
    /// A `Name` or `Path` and its `Args`.
    Call,
    /// A type's name, `::` and a function's name, such as `Int::min`, which
    /// only a `Call` holds.
    Path,
    /// `None`, `::`, `<`, a `TypeToken` and `>`: `None` of the type given.
    Annotated,
    /// A receiver, `.`, a `Name` and its `Args`.
    Method,
    /// A receiver, `.` and a `Name`.
    Field,
    /// A receiver, `[`, an expression and `]`.
    Index,
    /// An optional value and `!`, which unwraps it.
    Unwrap,
    /// `[` entries separated by `,` `]`.
    List,
    /// `{` `Entry` nodes separated by `,` `}`. One whose key lacks its
    /// `:` is an `Error` node in an entry's place, already reported.
    Map,
    /// A map's key, `:` and value.
    Entry,
    /// `(` arguments separated by `,` `)`.
    Args,
    /// What the parser recovered from a syntax error in: the expressions
    /// it could make sense of and the tokens it skipped. It may be empty,
    /// where an expression is missing. Its diagnostic is already reported,
    /// so it type checks as anything and nothing about it is reported
    /// again.
    Error,

    Whitespace,
    IntegerToken,
    StringToken,
    NameToken,
    True,
    False,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Comma,
    Dot,
    Colon,
    ColonColon,
    /// The type of an `Annotated`, as [`Type::parse`](crate::Type::parse)
    /// reads it.
    TypeToken,
    /// Text the lexer rejected, already reported.
    Invalid,
}

impl Syntax {
    /// How a token is named in an error message.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::IntegerToken | Self::Integer => "an integer",
            Self::StringToken | Self::String => "a string",
            Self::NameToken | Self::Name => "a name",
            Self::True => "`true`",
            Self::False => "`false`",
            Self::Plus => "`+`",
            Self::Minus => "`-`",
            Self::Star => "`*`",
            Self::Slash => "`/`",
            Self::Percent => "`%`",
            Self::EqEq => "`==`",
            Self::NotEq => "`!=`",
            Self::Lt => "`<`",
            Self::Le => "`<=`",
            Self::Gt => "`>`",
            Self::Ge => "`>=`",
            Self::AndAnd => "`&&`",
            Self::OrOr => "`||`",
            Self::Bang => "`!`",
            Self::OpenParen => "`(`",
            Self::CloseParen => "`)`",
            Self::OpenBracket => "`[`",
            Self::CloseBracket => "`]`",
            Self::OpenBrace => "`{`",
            Self::CloseBrace => "`}`",
            Self::Colon => "`:`",
            Self::Comma => "`,`",
            Self::Dot => "`.`",
            Self::ColonColon => "`::`",
            Self::Whitespace => "whitespace",
            Self::Invalid => "text that is no token",
            Self::TypeToken => "a type",
            Self::Root
            | Self::Bool
            | Self::Paren
            | Self::Unary
            | Self::Binary
            | Self::Call
            | Self::Path
            | Self::Annotated
            | Self::Method
            | Self::Field
            | Self::Index
            | Self::Unwrap
            | Self::List
            | Self::Map
            | Self::Entry
            | Self::Args
            | Self::Error => "an expression",
        }
    }
}

/// The text of a string literal's source, quotes included: what is between
/// the quotes with `\"` and `\\` unescaped. `None` when `literal` is not a
/// well-formed string literal.
pub(crate) fn unescape(literal: &str) -> Option<String> {
    let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();

    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                e @ ('"' | '\\') => out.push(e),
                _ => return None,
            },
            '"' => return None,
            c => out.push(c),
        }
    }

    Some(out)
}

/// A `Call`'s callee and the name it calls, such as `min` or `Int::min`,
/// whitespace in a path left out.
pub(crate) fn callee<'a>(
    call: syntree::Element<'a, Syntax, crate::parser::Flavor>,
    source: &str,
) -> Option<(syntree::Element<'a, Syntax, crate::parser::Flavor>, String)> {
    let node = call
        .children()
        .find(|n| matches!(n.value(), Syntax::Name | Syntax::Path))?;

    let mut names = node
        .children()
        .filter(|n| n.value() == Syntax::NameToken)
        .filter_map(|n| source.get(n.range()));

    let first = names.next()?;

    let name = match names.next() {
        Some(second) => format!("{first}::{second}"),
        None => first.to_owned(),
    };

    Some((node, name))
}

/// Whether `kind` is a unary or binary operator's token.
pub(crate) fn is_operator(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::Plus
            | Syntax::Minus
            | Syntax::Star
            | Syntax::Slash
            | Syntax::Percent
            | Syntax::EqEq
            | Syntax::NotEq
            | Syntax::Lt
            | Syntax::Le
            | Syntax::Gt
            | Syntax::Ge
            | Syntax::AndAnd
            | Syntax::OrOr
            | Syntax::Bang
    )
}
