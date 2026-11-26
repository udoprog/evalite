/// The kinds of a parse tree's nodes and tokens.
///
/// Nodes hold their parts in source order, whitespace tokens included:
/// a `Binary` holds its left operand, its operator token and its right
/// operand; a `Method` its receiver, `.`, a `Name` and its `Args`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Syntax {
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
    /// A `Name` and its `Args`.
    Call,
    /// A receiver, `.`, a `Name` and its `Args`.
    Method,
    /// A receiver, `.` and a `Name`.
    Field,
    /// A receiver, `[`, an expression and `]`.
    Index,
    /// `[` entries separated by `,` `]`.
    List,
    /// `(` arguments separated by `,` `)`.
    Args,

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
    Comma,
    Dot,
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
            Self::Comma => "`,`",
            Self::Dot => "`.`",
            Self::Whitespace => "whitespace",
            Self::Root
            | Self::Bool
            | Self::Paren
            | Self::Unary
            | Self::Binary
            | Self::Call
            | Self::Method
            | Self::Field
            | Self::Index
            | Self::List
            | Self::Args => "an expression",
        }
    }
}

/// The text of a string literal's source, quotes included: what is between
/// the quotes with `\"` and `\\` unescaped. `None` when `literal` is not a
/// well-formed string literal.
pub fn unescape(literal: &str) -> Option<String> {
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
