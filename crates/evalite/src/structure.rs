//! Expressions as data: the shape a form builds, which may have holes while
//! it is being edited, and which compiles through the same parser and
//! checker as text.

use std::fmt;

use musli::{Decode, Encode};
use syntree::{FlavorDefault, Node as TreeNode, Span};

use crate::check::compile_parsed;
use crate::debug::DebugInfo;
use crate::env::Environment;
use crate::error::{Diagnostic, Diagnostics, ErrorKind};
use crate::message::{Invalid, Labels, Message, StoreError};
use crate::parser::{Options, Tree, parse_with};
use crate::program::Program;
use crate::syntax::{Syntax, unescape};
use crate::types::{is_field_name, is_value_name};

type N<'a> = TreeNode<'a, Syntax, FlavorDefault>;

/// What a stored structure starts with, so later versions can tell.
const FORMAT: u32 = 1;

/// How a hole is written as text, in a value's place or a member's name.
pub const HOLE: &str = "_";

/// A node of a [`Structure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Encode, Decode)]
#[musli(transparent)]
pub struct NodeId(u32);

impl NodeId {
    fn index(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum UnaryOp {
    /// `-x`, of an `Int`.
    Neg,
    /// `!x`, of a `Bool`.
    Not,
}

impl UnaryOp {
    /// How it is written.
    ///
    /// ```
    /// use evalite::UnaryOp;
    ///
    /// assert_eq!(UnaryOp::Neg.symbol(), "-");
    /// assert_eq!(UnaryOp::Not.symbol(), "!");
    /// ```
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
        }
    }
}

/// An operator between two operands.
///
/// The operators are the language's own, so a form can offer them as a
/// menu; [`BinaryOp::ALL`] lists them. Their order is the stored form's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum BinaryOp {
    /// `a + b`, of `Int`s.
    Add,
    /// `a - b`, of `Int`s.
    Sub,
    /// `a * b`, of `Int`s.
    Mul,
    /// `a / b`, of `Int`s, rounding toward zero.
    Div,
    /// `a % b`, of `Int`s.
    Rem,
    /// `a == b`, of two `Int`s or two `Bool`s.
    Eq,
    /// `a != b`, of two `Int`s or two `Bool`s.
    Ne,
    /// `a < b`, of `Int`s.
    Lt,
    /// `a <= b`, of `Int`s.
    Le,
    /// `a > b`, of `Int`s.
    Gt,
    /// `a >= b`, of `Int`s.
    Ge,
    /// `a && b`, of `Bool`s, not reading `b` when `a` is `false`.
    And,
    /// `a || b`, of `Bool`s, not reading `b` when `a` is `true`.
    Or,
}

impl BinaryOp {
    /// Every operator, as a form would list them.
    ///
    /// ```
    /// use evalite::BinaryOp;
    ///
    /// let symbols = BinaryOp::ALL.map(BinaryOp::symbol);
    /// assert_eq!(symbols[..3], ["+", "-", "*"]);
    /// assert_eq!(symbols.len(), 13);
    /// ```
    pub const ALL: [BinaryOp; 13] = [
        Self::Add,
        Self::Sub,
        Self::Mul,
        Self::Div,
        Self::Rem,
        Self::Eq,
        Self::Ne,
        Self::Lt,
        Self::Le,
        Self::Gt,
        Self::Ge,
        Self::And,
        Self::Or,
    ];

    /// How it is written.
    ///
    /// ```
    /// use evalite::BinaryOp;
    ///
    /// assert_eq!(BinaryOp::Le.symbol(), "<=");
    /// assert_eq!(BinaryOp::And.symbol(), "&&");
    /// ```
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "&&",
            Self::Or => "||",
        }
    }

    /// How tightly it binds, as the parser has it: `||` loosest at 1.
    fn power(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge => COMPARE,
            Self::Add | Self::Sub => 4,
            Self::Mul | Self::Div | Self::Rem => 5,
        }
    }

    fn from_syntax(kind: Syntax) -> Option<Self> {
        Some(match kind {
            Syntax::Plus => Self::Add,
            Syntax::Minus => Self::Sub,
            Syntax::Star => Self::Mul,
            Syntax::Slash => Self::Div,
            Syntax::Percent => Self::Rem,
            Syntax::EqEq => Self::Eq,
            Syntax::NotEq => Self::Ne,
            Syntax::Lt => Self::Lt,
            Syntax::Le => Self::Le,
            Syntax::Gt => Self::Gt,
            Syntax::Ge => Self::Ge,
            Syntax::AndAnd => Self::And,
            Syntax::OrOr => Self::Or,
            _ => return None,
        })
    }
}

const COMPARE: u8 = 3;
/// The binding power of prefix operators, and of negative integers, which
/// are written with one.
const PREFIX: u8 = 6;
/// The binding power of literals, names, calls and postfixes.
const ATOM: u8 = 7;

/// One expression of a [`Structure`], its parts other nodes.
///
/// A node is built through its constructors, one per kind of expression,
/// and read through try-accessors, each `Some` only for its own kind. This
/// is the same shape as [`Value`](crate::Value)'s: a form reads a node by
/// asking what it is, never by matching on its insides.
///
/// ```
/// use evalite::{BinaryOp, Node, Structure};
///
/// let mut s = Structure::new();
/// let wits = s.add(Node::name("wits"))?;
/// let one = s.add(Node::int(1))?;
/// s.replace(s.root(), Node::binary(BinaryOp::Add, wits, one))?;
/// assert_eq!(s.to_text(), "wits + 1");
///
/// let root = s.get(s.root()).ok_or("no root")?;
/// assert_eq!(root.as_binary(), Some((BinaryOp::Add, wits, one)));
/// assert_eq!(root.as_int(), None);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode)]
#[musli(transparent)]
pub struct Node(pub(crate) Repr);

impl Node {
    /// An unfinished expression, written `_`.
    ///
    /// ```
    /// use evalite::Node;
    ///
    /// assert!(Node::hole().is_hole());
    /// assert!(!Node::int(0).is_hole());
    /// ```
    pub fn hole() -> Self {
        Self(Repr::Hole)
    }

    /// An integer, written `-12` when negative.
    ///
    /// ```
    /// use evalite::Node;
    ///
    /// assert_eq!(Node::int(i64::MIN).as_int(), Some(i64::MIN));
    /// assert_eq!(Node::int(i64::MAX).as_int(), Some(i64::MAX));
    /// ```
    pub fn int(value: i64) -> Self {
        Self(Repr::Int(value))
    }

    /// `true` or `false`.
    ///
    /// ```
    /// use evalite::Node;
    ///
    /// assert_eq!(Node::bool(true).as_bool(), Some(true));
    /// ```
    pub fn bool(value: bool) -> Self {
        Self(Repr::Bool(value))
    }

    /// A string literal, escaped as needed when written.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// s.replace(s.root(), Node::string("a \"b\""))?;
    /// assert_eq!(s.to_text(), r#""a \"b\"""#);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn string(value: impl Into<String>) -> Self {
        Self(Repr::String(value.into()))
    }

    /// A value of the environment.
    ///
    /// ```
    /// use evalite::Node;
    ///
    /// assert_eq!(Node::name("wits").as_name(), Some("wits"));
    /// assert_eq!(Node::string("wits").as_name(), None);
    /// ```
    pub fn name(name: impl Into<String>) -> Self {
        Self(Repr::Name(name.into()))
    }

    /// A prefix operator applied to `operand`.
    ///
    /// ```
    /// use evalite::{Node, Structure, UnaryOp};
    ///
    /// let mut s = Structure::new();
    /// let dead = s.add(Node::name("dead"))?;
    /// s.replace(s.root(), Node::unary(UnaryOp::Not, dead))?;
    /// assert_eq!(s.to_text(), "!dead");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn unary(op: UnaryOp, operand: NodeId) -> Self {
        Self(Repr::Unary { op, operand })
    }

    /// An operator between `left` and `right`.
    ///
    /// ```
    /// use evalite::{BinaryOp, Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let a = s.add(Node::int(1))?;
    /// let b = s.add(Node::int(2))?;
    /// s.replace(s.root(), Node::binary(BinaryOp::Mul, a, b))?;
    /// assert_eq!(s.to_text(), "1 * 2");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn binary(op: BinaryOp, left: NodeId, right: NodeId) -> Self {
        Self(Repr::Binary { op, left, right })
    }

    /// A call of the environment's function `function`.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let a = s.add(Node::int(1))?;
    /// let b = s.add(Node::hole())?;
    /// s.replace(s.root(), Node::call("max", [a, b]))?;
    /// assert_eq!(s.to_text(), "max(1, _)");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn call(function: impl Into<String>, args: impl IntoIterator<Item = NodeId>) -> Self {
        Self(Repr::Call {
            function: function.into(),
            args: args.into_iter().collect(),
        })
    }

    /// `receiver.name(args)`; a `name` of `None` is a hole, written `_`.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let bonus = s.add(Node::name("bonus"))?;
    /// let zero = s.add(Node::int(0))?;
    /// s.replace(s.root(), Node::method(bonus, Some("unwrap_or"), [zero]))?;
    /// assert_eq!(s.to_text(), "bonus.unwrap_or(0)");
    ///
    /// s.replace(s.root(), Node::method(bonus, None::<&str>, [zero]))?;
    /// assert_eq!(s.to_text(), "bonus._(0)");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn method(
        receiver: NodeId,
        name: Option<impl Into<String>>,
        args: impl IntoIterator<Item = NodeId>,
    ) -> Self {
        Self(Repr::Method {
            receiver,
            name: name.map(Into::into),
            args: args.into_iter().collect(),
        })
    }

    /// `receiver.name`; a `name` of `None` is a hole, written `_`. A name
    /// that is not written as one is `receiver["name"]`, an
    /// [`index`](Self::index) by a string.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let m = s.add(Node::name("m"))?;
    /// s.replace(s.root(), Node::field(m, Some("athletics")))?;
    /// assert_eq!(s.to_text(), "m.athletics");
    ///
    /// s.replace(s.root(), Node::field(m, None::<&str>))?;
    /// assert_eq!(s.to_text(), "m._");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn field(receiver: NodeId, name: Option<impl Into<String>>) -> Self {
        Self(Repr::Field {
            receiver,
            name: name.map(Into::into),
        })
    }

    /// `receiver[index]`.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let dots = s.add(Node::name("dots"))?;
    /// let zero = s.add(Node::int(0))?;
    /// s.replace(s.root(), Node::index(dots, zero))?;
    /// assert_eq!(s.to_text(), "dots[0]");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn index(receiver: NodeId, index: NodeId) -> Self {
        Self(Repr::Index { receiver, index })
    }

    /// `[entries]`.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let a = s.add(Node::int(1))?;
    /// let b = s.add(Node::int(2))?;
    /// s.replace(s.root(), Node::list([a, b]))?;
    /// assert_eq!(s.to_text(), "[1, 2]");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn list(entries: impl IntoIterator<Item = NodeId>) -> Self {
        Self(Repr::List {
            entries: entries.into_iter().collect(),
        })
    }

    /// `operand!`: the value of an option, which fails the run when it is
    /// absent.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let mut s = Structure::new();
    /// let bonus = s.add(Node::name("bonus"))?;
    /// s.replace(s.root(), Node::unwrap(bonus))?;
    /// assert_eq!(s.to_text(), "bonus!");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn unwrap(operand: NodeId) -> Self {
        Self(Repr::Unwrap { operand })
    }

    /// The integer, if it is one.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("-12")?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_int), Some(-12));
    /// assert_eq!(Node::bool(true).as_int(), None);
    /// assert_eq!(Node::string("1").as_int(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_int(&self) -> Option<i64> {
        match self.0 {
            Repr::Int(n) => Some(n),
            _ => None,
        }
    }

    /// The boolean, if it is one.
    ///
    /// ```
    /// use evalite::Node;
    ///
    /// assert_eq!(Node::bool(false).as_bool(), Some(false));
    /// assert_eq!(Node::int(0).as_bool(), None);
    /// assert_eq!(Node::name("true_ish").as_bool(), None);
    /// ```
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Repr::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// The string literal's text, unescaped, if it is one.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text(r#""a \"b\"""#)?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_string), Some("a \"b\""));
    /// assert_eq!(Node::name("a").as_string(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_string(&self) -> Option<&str> {
        match &self.0 {
            Repr::String(s) => Some(s),
            _ => None,
        }
    }

    /// The environment value's name, if it is one.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("wits")?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_name), Some("wits"));
    /// assert_eq!(Node::string("wits").as_name(), None);
    /// assert_eq!(Node::hole().as_name(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_name(&self) -> Option<&str> {
        match &self.0 {
            Repr::Name(s) => Some(s),
            _ => None,
        }
    }

    /// The operator and operand, if it is a prefix operator.
    ///
    /// ```
    /// use evalite::{Node, Structure, UnaryOp};
    ///
    /// let s = Structure::from_text("-wits")?;
    /// let root = s.get(s.root()).ok_or("no root")?;
    /// let (op, operand) = root.as_unary().ok_or("not unary")?;
    /// assert_eq!(op, UnaryOp::Neg);
    /// assert_eq!(s.get(operand).and_then(Node::as_name), Some("wits"));
    ///
    /// // A negative integer is a literal, not an operator.
    /// let s = Structure::from_text("-1")?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_unary), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_unary(&self) -> Option<(UnaryOp, NodeId)> {
        match self.0 {
            Repr::Unary { op, operand } => Some((op, operand)),
            _ => None,
        }
    }

    /// The operator and operands, if it is an operator between two.
    ///
    /// ```
    /// use evalite::{BinaryOp, Node, Structure};
    ///
    /// let s = Structure::from_text("1 < 2 && dead")?;
    /// let (op, left, _) = s.get(s.root()).and_then(Node::as_binary).ok_or("not binary")?;
    /// assert_eq!(op, BinaryOp::And);
    /// assert_eq!(s.get(left).and_then(Node::as_binary).map(|(op, ..)| op), Some(BinaryOp::Lt));
    /// assert_eq!(Node::int(1).as_binary(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_binary(&self) -> Option<(BinaryOp, NodeId, NodeId)> {
        match self.0 {
            Repr::Binary { op, left, right } => Some((op, left, right)),
            _ => None,
        }
    }

    /// The function and arguments, if it is a call.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("max(1, 2)")?;
    /// let (function, args) = s.get(s.root()).and_then(Node::as_call).ok_or("not a call")?;
    /// assert_eq!(function, "max");
    /// assert_eq!(args.len(), 2);
    ///
    /// // A method is not a call.
    /// let s = Structure::from_text("wits.max(1)")?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_call), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_call(&self) -> Option<(&str, &[NodeId])> {
        match &self.0 {
            Repr::Call { function, args } => Some((function, args)),
            _ => None,
        }
    }

    /// The receiver, the name (`None` for a hole) and the arguments, if
    /// it is a method.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("bonus.unwrap_or(0)")?;
    /// let (_, name, args) = s.get(s.root()).and_then(Node::as_method).ok_or("not a method")?;
    /// assert_eq!((name, args.len()), (Some("unwrap_or"), 1));
    ///
    /// let s = Structure::from_text("bonus._(0)")?;
    /// let (_, name, _) = s.get(s.root()).and_then(Node::as_method).ok_or("not a method")?;
    /// assert_eq!(name, None);
    ///
    /// let s = Structure::from_text("m.athletics")?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_method), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_method(&self) -> Option<(NodeId, Option<&str>, &[NodeId])> {
        match &self.0 {
            Repr::Method {
                receiver,
                name,
                args,
            } => Some((*receiver, name.as_deref(), args)),
            _ => None,
        }
    }

    /// The receiver and the name (`None` for a hole), if it is a field.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("m.athletics")?;
    /// let (_, name) = s.get(s.root()).and_then(Node::as_field).ok_or("not a field")?;
    /// assert_eq!(name, Some("athletics"));
    ///
    /// let s = Structure::from_text("m._")?;
    /// let (_, name) = s.get(s.root()).and_then(Node::as_field).ok_or("not a field")?;
    /// assert_eq!(name, None);
    ///
    /// // A name that is not written as one is an index.
    /// let s = Structure::from_text(r#"m["Brawl (bar)"]"#)?;
    /// assert_eq!(s.get(s.root()).and_then(Node::as_field), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_field(&self) -> Option<(NodeId, Option<&str>)> {
        match &self.0 {
            Repr::Field { receiver, name } => Some((*receiver, name.as_deref())),
            _ => None,
        }
    }

    /// The receiver and index, if it is an index.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("dots[0]")?;
    /// let (receiver, index) = s.get(s.root()).and_then(Node::as_index).ok_or("not an index")?;
    /// assert_eq!(s.get(receiver).and_then(Node::as_name), Some("dots"));
    /// assert_eq!(s.get(index).and_then(Node::as_int), Some(0));
    /// assert_eq!(Node::list([]).as_index(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_index(&self) -> Option<(NodeId, NodeId)> {
        match self.0 {
            Repr::Index { receiver, index } => Some((receiver, index)),
            _ => None,
        }
    }

    /// The operand, if it is an unwrap, `operand!`.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("lookup(1, dots)!")?;
    /// let operand = s.get(s.root()).and_then(Node::as_unwrap).ok_or("not an unwrap")?;
    /// assert!(s.get(operand).and_then(Node::as_call).is_some());
    /// assert_eq!(Node::int(1).as_unwrap(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_unwrap(&self) -> Option<NodeId> {
        match self.0 {
            Repr::Unwrap { operand } => Some(operand),
            _ => None,
        }
    }

    /// The entries, if it is a list.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("[1, [2]]")?;
    /// let entries = s.get(s.root()).and_then(Node::as_list).ok_or("not a list")?;
    /// assert_eq!(entries.len(), 2);
    /// assert_eq!(s.get(entries[1]).and_then(Node::as_list).map(<[_]>::len), Some(1));
    /// assert_eq!(Node::list([]).as_list(), Some(&[][..]));
    /// assert_eq!(Node::int(1).as_list(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn as_list(&self) -> Option<&[NodeId]> {
        match &self.0 {
            Repr::List { entries } => Some(entries),
            _ => None,
        }
    }

    /// Its parts, in the order they are written.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("bonus.unwrap_or(0)")?;
    /// let root = s.get(s.root()).ok_or("no root")?;
    /// let parts = root.children().into_iter().map(|id| s.text_of(id)).collect::<Vec<_>>();
    /// assert_eq!(parts, ["bonus", "0"]);
    /// assert!(Node::int(1).children().is_empty());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn children(&self) -> Vec<NodeId> {
        self.0.children()
    }

    /// Whether it is a hole: a value's, or a member name's.
    ///
    /// ```
    /// use evalite::{Node, Structure};
    ///
    /// let s = Structure::from_text("_ + m._ + bonus._(1)")?;
    /// assert_eq!(s.holes().len(), 3);
    /// assert!(s.holes().iter().all(|&h| s.get(h).is_some_and(Node::is_hole)));
    /// assert!(!Node::name("_x").is_hole());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn is_hole(&self) -> bool {
        self.0.is_hole()
    }
}

/// What a [`Node`] is. Its encoding is the stored form's, so variants
/// and their fields keep their order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode)]
pub(crate) enum Repr {
    /// An unfinished expression, written `_`.
    Hole,
    /// An integer, written `-12` when negative.
    Int(i64),
    Bool(bool),
    String(String),
    /// A value of the environment.
    Name(String),
    Unary {
        op: UnaryOp,
        operand: NodeId,
    },
    Binary {
        op: BinaryOp,
        left: NodeId,
        right: NodeId,
    },
    /// A call of the environment's function `function`.
    Call {
        function: String,
        args: Vec<NodeId>,
    },
    /// `receiver.name(args)`; a `name` of `None` is a hole.
    Method {
        receiver: NodeId,
        name: Option<String>,
        args: Vec<NodeId>,
    },
    /// `receiver.name`; a `name` of `None` is a hole.
    Field {
        receiver: NodeId,
        name: Option<String>,
    },
    /// `receiver[index]`.
    Index {
        receiver: NodeId,
        index: NodeId,
    },
    /// `[entries]`.
    List {
        entries: Vec<NodeId>,
    },
    /// `operand!`.
    Unwrap {
        operand: NodeId,
    },
}

impl Repr {
    /// Its parts, in the order they are written.
    fn children(&self) -> Vec<NodeId> {
        match self {
            Self::Hole | Self::Int(_) | Self::Bool(_) | Self::String(_) | Self::Name(_) => {
                Vec::new()
            }
            Self::Unary { operand, .. } | Self::Unwrap { operand } => vec![*operand],
            Self::Binary { left, right, .. } => vec![*left, *right],
            Self::Call { args, .. } | Self::List { entries: args } => args.clone(),
            Self::Method { receiver, args, .. } => {
                let mut out = Vec::with_capacity(args.len().saturating_add(1));
                out.push(*receiver);
                out.extend(args);
                out
            }
            Self::Field { receiver, .. } => vec![*receiver],
            Self::Index { receiver, index } => vec![*receiver, *index],
        }
    }

    /// Whether it is a hole: a value's, or a member name's.
    fn is_hole(&self) -> bool {
        matches!(
            self,
            Self::Hole | Self::Method { name: None, .. } | Self::Field { name: None, .. }
        )
    }

    /// How tightly it binds when written.
    fn power(&self) -> u8 {
        match self {
            Self::Binary { op, .. } => op.power(),
            Self::Unary { .. } => PREFIX,
            Self::Int(n) if *n < 0 => PREFIX,
            _ => ATOM,
        }
    }
}

/// Why a structure could not be edited as asked.
///
/// ```
/// use evalite::Structure;
///
/// let mut s = Structure::from_text("1 + 2")?;
/// let one = s.get(s.root()).unwrap().children()[0];
///
/// // `1` is already part of `1 + 2`.
/// let error = s.set_root(one).unwrap_err();
/// assert_eq!(error.to_string(), format!("node {one} is already part of another node"));
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError {
    kind: EditKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditKind {
    /// A node the structure does not have.
    UnknownNode(NodeId),
    /// A node already part of the expression, or of another node, which
    /// can only be used once.
    Attached(NodeId),
    /// Making a node part of itself.
    Cycle(NodeId),
    /// A completion of a field or method in place of a node that is
    /// neither.
    NotAMember(NodeId),
    /// Too many nodes to number.
    TooLarge,
}

impl From<EditKind> for EditError {
    #[inline]
    fn from(kind: EditKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            EditKind::UnknownNode(id) => write!(f, "there is no node {id}"),
            EditKind::Attached(id) => write!(f, "node {id} is already part of another node"),
            EditKind::Cycle(id) => write!(f, "node {id} would be part of itself"),
            EditKind::NotAMember(id) => write!(f, "node {id} is not a field or a method"),
            EditKind::TooLarge => write!(f, "the structure has too many nodes"),
        }
    }
}

impl std::error::Error for EditError {}

/// Why a structure does not compile at one node.
///
/// A [`Structure::compile`] reports every one of them, as
/// [`StructureErrors`].
///
/// ```
/// use evalite::{Environment, Structure};
///
/// let s = Structure::new();
/// let errors = s.compile(&Environment::new()).unwrap_err();
/// let error = errors.iter().next().unwrap();
/// assert_eq!(error.node(), s.root());
/// assert_eq!(error.to_string(), "the expression is unfinished, fill in the `_`");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureError {
    node: NodeId,
    kind: StructureKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StructureKind {
    /// A hole, which must be filled before the expression compiles.
    Hole,
    /// A name that cannot be written as one, such as `"Strength"` for a
    /// value or `"true"`. A record field of any name is read with
    /// [`Node::index`] by a [`Node::string`].
    InvalidName(Box<str>),
    /// An error compiling [`Structure::to_text`]'s text: its span is in
    /// that text.
    Compile(Diagnostics),
}

impl StructureError {
    /// The node the error is about.
    ///
    /// ```
    /// use evalite::{Environment, Structure};
    ///
    /// let s = Structure::from_text("1 + _")?;
    /// let errors = s.compile(&Environment::new()).unwrap_err();
    /// assert_eq!(errors.iter().next().unwrap().node(), s.holes()[0]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn node(&self) -> NodeId {
        self.node
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> &StructureKind {
        &self.kind
    }

    /// What is wrong, as [`Label`](crate::Label)s. An error from compiling
    /// has the labels of its [`Diagnostic`], with spans in
    /// [`Structure::to_text`]'s text; a hole or a name that cannot be
    /// written has one label and no span, since [`node`](Self::node) says
    /// where it is.
    ///
    /// ```
    /// use evalite::{Environment, Structure};
    ///
    /// let env = Environment::new();
    ///
    /// let errors = Structure::new().compile(&env).unwrap_err();
    /// let labels: Vec<_> = errors.iter().next().unwrap().labels().collect();
    /// assert_eq!(labels.len(), 1);
    /// assert!(labels[0].is_primary());
    /// assert_eq!(labels[0].span(), None);
    ///
    /// let errors = Structure::from_text("nothing")?.compile(&env).unwrap_err();
    /// let label = errors.iter().next().unwrap().labels().next().unwrap();
    /// assert_eq!(label.message().to_string(), "unknown name `nothing`");
    /// assert_eq!(label.span().map(|s| s.range()), Some(0..7));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn labels(&self) -> Labels<'_> {
        match &self.kind {
            StructureKind::Compile(errors) => match errors.first() {
                Some(error) => error.labels(),
                None => Labels::one(self),
            },
            _ => Labels::one(self),
        }
    }
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StructureKind::Hole => write!(f, "the expression is unfinished, fill in the `_`"),
            StructureKind::InvalidName(name) => write!(
                f,
                "`{name}` cannot be written as a name, names are letters, digits and `_`, starting with a letter or `_`"
            ),
            StructureKind::Compile(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StructureError {}

/// Every reason a structure does not compile, each a [`StructureError`]
/// naming its node.
///
/// There is at least one: every hole, every name that cannot be written
/// and every error of the text are reported, in the order of
/// [`Structure::to_text`]'s text.
///
/// ```
/// use evalite::{Environment, Structure};
///
/// let s = Structure::from_text("a + b")?;
/// let errors = s.compile(&Environment::new()).unwrap_err();
/// let texts: Vec<_> = errors.iter().map(|e| s.text_of(e.node())).collect();
/// assert_eq!(texts, ["a", "b"]);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureErrors {
    list: Vec<StructureError>,
}

impl StructureErrors {
    /// How many errors there are, at least one.
    #[allow(clippy::len_without_is_empty, reason = "it is never empty")]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Each error, in the order of [`Structure::to_text`]'s text.
    pub fn iter(&self) -> std::slice::Iter<'_, StructureError> {
        self.list.iter()
    }

    #[cfg(test)]
    pub(crate) fn first(&self) -> Option<&StructureError> {
        self.list.first()
    }
}

impl<'a> IntoIterator for &'a StructureErrors {
    type Item = &'a StructureError;
    type IntoIter = std::slice::Iter<'a, StructureError>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl IntoIterator for StructureErrors {
    type Item = StructureError;
    type IntoIter = std::vec::IntoIter<StructureError>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.into_iter()
    }
}

impl fmt::Display for StructureErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, error) in self.list.iter().enumerate() {
            if n > 0 {
                f.write_str("\n")?;
            }

            error.fmt(f)?;
        }

        Ok(())
    }
}

impl std::error::Error for StructureErrors {}

#[derive(Encode, Decode)]
struct Stored {
    format: u32,
    nodes: Vec<Node>,
    root: NodeId,
}

/// An expression as a tree of [`Node`]s, which a form edits and
/// [`Structure::compile`] compiles exactly as it would its
/// [`to_text`](Structure::to_text).
///
/// Nodes live in a table and refer to their parts by [`NodeId`], so no walk
/// over a structure, its encoding or its drop recurses. Every node is part
/// of at most one other; nodes left over by edits are kept, unused, until
/// it is stored.
///
/// ```
/// use evalite::{Environment, Node, Structure, Type};
///
/// let mut env = Environment::with_builtins();
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let mut s = Structure::new();
/// let bonus = s.add(Node::name("bonus"))?;
/// let zero = s.add(Node::int(0))?;
/// let unwrap = s.add(Node::method(bonus, Some("unwrap_or"), [zero]))?;
/// s.replace(s.root(), Node::call("max", [unwrap]))?;
///
/// assert_eq!(s.to_text(), "max(bonus.unwrap_or(0))");
/// assert_eq!(Structure::from_text("max(bonus.unwrap_or(0))")?, s);
/// assert_eq!(s.compile(&env)?.result_type(), Type::int());
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Structure {
    nodes: Vec<Node>,
    /// The node each node is part of.
    parents: Vec<Option<NodeId>>,
    root: NodeId,
}

impl Default for Structure {
    fn default() -> Self {
        Self::new()
    }
}

impl Structure {
    /// A structure that is one hole.
    pub fn new() -> Self {
        Self {
            nodes: vec![Node(Repr::Hole)],
            parents: vec![None],
            root: NodeId(0),
        }
    }

    /// The whole expression's node.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The node `id`; `None` when the structure has no such node.
    ///
    /// ```
    /// use evalite::Structure;
    ///
    /// let big = Structure::from_text("[1, 2, 3, 4]")?;
    /// let root = big.get(big.root()).ok_or("no root")?;
    /// assert_eq!(root.children().len(), 4);
    ///
    /// let last = root.children().into_iter().max().ok_or("no entries")?;
    /// assert_eq!(big.get(last).and_then(|n| n.as_int()), Some(4));
    /// assert!(Structure::new().get(last).is_none());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// The node `id` is part of; `None` for the root and unused nodes.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents.get(id.index()).copied().flatten()
    }

    /// Every node of the expression, parts before what holds them.
    pub fn post_order(&self) -> Vec<NodeId> {
        post_order(&self.nodes, self.root)
    }

    /// The holes of the expression, in the order they are written.
    pub fn holes(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];

        while let Some(id) = stack.pop() {
            let Some(node) = self.get(id) else { continue };

            if node.is_hole() {
                out.push(id);
            }

            stack.extend(node.children().into_iter().rev());
        }

        out
    }

    /// Add `node`, which is not yet part of anything. Its parts must be
    /// nodes that are not part of anything either.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when a part of `node` is not a node of the
    /// structure or is already part of something, or the structure has too many
    /// nodes to number.
    pub fn add(&mut self, node: Node) -> Result<NodeId, EditError> {
        let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| EditKind::TooLarge)?);
        self.check_free(&node, None)?;
        self.attach(&node, id);
        self.nodes.push(node);
        self.parents.push(None);
        Ok(id)
    }

    /// Replace node `at` with `node`. `at`'s old parts are no longer part
    /// of it, and so may be used again, such as by `node`.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when `at` or a part of `node` is not a node of
    /// the structure, a part is already part of something else, or `node` would
    /// hold `at` itself.
    pub fn replace(&mut self, at: NodeId, node: Node) -> Result<(), EditError> {
        let old = self.get(at).ok_or(EditKind::UnknownNode(at))?;
        let old_children = old.children();
        self.check_free(&node, Some((at, &old_children)))?;

        for child in old_children {
            self.set_parent(child, None);
        }

        self.attach(&node, at);

        if let Some(slot) = self.nodes.get_mut(at.index()) {
            *slot = node;
        }

        Ok(())
    }

    /// Make `id`, which is not part of anything, the whole expression.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when `id` is not a node of the structure or is
    /// part of something.
    pub fn set_root(&mut self, id: NodeId) -> Result<(), EditError> {
        if self.get(id).is_none() {
            return Err(EditKind::UnknownNode(id).into());
        }

        if id != self.root && self.parent(id).is_some() {
            return Err(EditKind::Attached(id).into());
        }

        self.root = id;
        Ok(())
    }

    /// Copy the expression of `other` in place of node `at`.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when `at` is not a node of the structure, or
    /// the structure would have too many nodes to number.
    pub fn graft(&mut self, at: NodeId, other: &Structure) -> Result<(), EditError> {
        if self.get(at).is_none() {
            return Err(EditKind::UnknownNode(at).into());
        }

        let mut ids = vec![None; other.nodes.len()];

        for id in other.post_order() {
            let node = other
                .get(id)
                .ok_or(EditKind::UnknownNode(id))?
                .0
                .clone()
                .renumber(|child| ids.get(child.index()).copied().flatten())
                .map(Node)
                .ok_or(EditKind::UnknownNode(id))?;

            let new = if id == other.root {
                self.replace(at, node)?;
                at
            } else {
                self.add(node)?
            };

            if let Some(slot) = ids.get_mut(id.index()) {
                *slot = Some(new);
            }
        }

        Ok(())
    }

    fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) {
        if let Some(slot) = self.parents.get_mut(id.index()) {
            *slot = parent;
        }
    }

    fn attach(&mut self, node: &Node, parent: NodeId) {
        for child in node.children() {
            self.set_parent(child, Some(parent));
        }
    }

    /// Check that `node`'s parts can become part of a node: each exists,
    /// is used once, is not the root nor part of anything but `at`, and
    /// does not hold `at`.
    fn check_free(&self, node: &Node, at: Option<(NodeId, &[NodeId])>) -> Result<(), EditError> {
        let children = node.children();

        for (n, child) in children.iter().enumerate() {
            if self.get(*child).is_none() {
                return Err(EditKind::UnknownNode(*child).into());
            }

            let reused = at.is_some_and(|(_, old)| old.contains(child));

            if *child == self.root
                || (self.parent(*child).is_some() && !reused)
                || children.iter().take(n).any(|c| c == child)
            {
                return Err(EditKind::Attached(*child).into());
            }
        }

        if let Some((at, _)) = at {
            // Walk up from `at`: a part of `node` above it would hold
            // itself.
            let mut up = Some(at);
            let mut steps = 0usize;

            while let Some(id) = up {
                if children.contains(&id) {
                    return Err(EditKind::Cycle(id).into());
                }

                steps = steps.saturating_add(1);

                if steps > self.nodes.len() {
                    return Err(EditKind::Cycle(id).into());
                }

                up = self.parent(id);
            }
        }

        Ok(())
    }

    /// The structure as text, holes written `_`. Parentheses are added
    /// where precedence needs them.
    pub fn to_text(&self) -> String {
        render(&self.nodes, self.root).0
    }

    /// The text of node `id`'s expression.
    pub fn text_of(&self, id: NodeId) -> String {
        render(&self.nodes, id).0
    }

    /// Parse `source` into a structure, with the default [`Options`]. `_`
    /// is a hole, as a value or as a field's or method's name.
    /// Parentheses are dropped, and `-12` is the integer `-12`.
    ///
    /// # Errors
    ///
    /// Returns [`Diagnostics`] when `source` does not parse or nests deeper
    /// than the default `max_depth`.
    pub fn from_text(source: &str) -> Result<Self, Diagnostics> {
        Self::from_text_with(source, &Options::new())
    }

    /// Parse `source` into a structure.
    ///
    /// # Errors
    ///
    /// Returns [`Diagnostics`] when `source` does not parse or nests deeper
    /// than `options` allows.
    pub fn from_text_with(source: &str, options: &Options) -> Result<Self, Diagnostics> {
        let tree = parse_with(source, options)?.complete()?;
        from_tree(source, &tree).map_err(Diagnostics::one)
    }

    /// Compile against `env`, with the default [`Options`].
    ///
    /// # Errors
    ///
    /// Returns a [`StructureErrors`] when the structure has a hole, a name that
    /// cannot be written, or does not compile against `env`, with every one of
    /// them in the [`StructureErrors`], each naming its node.
    pub fn compile(&self, env: &Environment) -> Result<Program, StructureErrors> {
        self.compile_with(env, &Options::new())
    }

    /// Compile against `env`: the same program, or the same error, as
    /// compiling [`to_text`](Self::to_text), once it has no holes.
    ///
    /// # Errors
    ///
    /// Returns a [`StructureErrors`] when the structure has a hole, a name that
    /// cannot be written, or its text does not compile against `env` with
    /// `options`, with every one of them in the [`StructureErrors`].
    pub fn compile_with(
        &self,
        env: &Environment,
        options: &Options,
    ) -> Result<Program, StructureErrors> {
        compile_node(&self.nodes, self.root, env, options)
    }

    /// Compile against `env` as [`compile_with`](Self::compile_with) does,
    /// also giving the [`DebugInfo`] whose spans are into
    /// [`to_text`](Self::to_text).
    ///
    /// # Examples
    ///
    /// ```
    /// use evalite::{Environment, Machine, Options, Structure};
    ///
    /// let env = Environment::with_builtins();
    /// let structure = Structure::from_text("(1 + 2) * [3][4]")?;
    /// let (program, debug) = structure.compile_debug(&env, &Options::new())?;
    ///
    /// let text = structure.to_text();
    /// let error = Machine::default().load(&program)?.run().unwrap_err();
    /// let span = error.labels(&program, Some(&debug)).next().and_then(|l| l.span());
    /// assert_eq!(span.map(|s| &text[s.range()]), Some("[3][4]"));
    ///
    /// let hole = Structure::from_text("1 + _")?;
    /// assert!(hole.compile_debug(&env, &Options::new()).is_err());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// The same as [`compile_with`](Self::compile_with).
    pub fn compile_debug(
        &self,
        env: &Environment,
        options: &Options,
    ) -> Result<(Program, DebugInfo), StructureErrors> {
        compile_node_debug(&self.nodes, self.root, env, options)
    }

    /// Compile the expression at node `id` alone.
    pub(crate) fn compile_at(
        &self,
        id: NodeId,
        env: &Environment,
    ) -> Result<Program, StructureErrors> {
        compile_node(&self.nodes, id, env, &Options::new())
    }

    /// The stored form, which [`Structure::from_bytes`] reads. Unused
    /// nodes are left out.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the structure cannot be encoded, which
    /// does not happen in practice.
    pub fn to_bytes(&self) -> Result<Vec<u8>, StoreError> {
        let (nodes, root) = self.compact().ok_or_else(|| {
            StoreError::from(Invalid::NotOneTree).context(Message::StoringStructure)
        })?;

        let stored = Stored {
            format: FORMAT,
            nodes,
            root,
        };

        musli::storage::to_vec(&stored).map_err(|e| {
            StoreError::new(Message::CannotEncode)
                .with_source(e.to_string())
                .context(Message::StoringStructure)
        })
    }

    /// Read a stored structure, checking that its nodes are one tree.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when `bytes` do not decode, are of another
    /// format version, or are nodes that are not one tree.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let stored: Stored = musli::storage::from_slice(bytes).map_err(|e| {
            StoreError::new(Message::NotDecoded)
                .with_source(e.to_string())
                .context(Message::DecodingStructure)
        })?;

        if stored.format != FORMAT {
            return Err(StoreError::new(Message::WrongFormat {
                found: stored.format,
                reads: FORMAT,
            })
            .context(Message::DecodingStructure));
        }

        let mut parents = vec![None; stored.nodes.len()];

        for (at, node) in stored.nodes.iter().enumerate() {
            let at = NodeId(u32::try_from(at).map_err(|_| invalid(Invalid::TooManyNodes, None))?);

            for child in node.children() {
                let slot = parents
                    .get_mut(child.index())
                    .ok_or_else(|| invalid(Invalid::PartPastEnd, Some(at)))?;

                if slot.is_some() || child == stored.root {
                    return Err(invalid(Invalid::PartTwice, Some(child)));
                }

                *slot = Some(at);
            }
        }

        if stored.nodes.get(stored.root.index()).is_none() {
            return Err(invalid(Invalid::RootPastEnd, None));
        }

        // Each node but the root has one parent, so reaching every node
        // from the root means there is no cycle.
        if post_order(&stored.nodes, stored.root).len() != stored.nodes.len() {
            return Err(invalid(Invalid::UnusedNodes, None));
        }

        Ok(Self {
            nodes: stored.nodes,
            parents,
            root: stored.root,
        })
    }

    /// The expression's nodes alone, renumbered in post order.
    fn compact(&self) -> Option<(Vec<Node>, NodeId)> {
        let order = self.post_order();
        let mut ids = vec![None; self.nodes.len()];
        let mut nodes = Vec::with_capacity(order.len());

        for id in order {
            let node = self
                .get(id)?
                .0
                .clone()
                .renumber(|c| ids.get(c.index()).copied().flatten())?;
            *ids.get_mut(id.index())? = Some(NodeId(u32::try_from(nodes.len()).ok()?));
            nodes.push(Node(node));
        }

        let root = NodeId(u32::try_from(nodes.len().checked_sub(1)?).ok()?);
        Some((nodes, root))
    }
}

impl Repr {
    /// The same node with its parts' ids mapped.
    fn renumber(self, f: impl Fn(NodeId) -> Option<NodeId>) -> Option<Self> {
        let map = |ids: Vec<NodeId>| ids.into_iter().map(&f).collect::<Option<Vec<_>>>();

        Some(match self {
            Self::Unary { op, operand } => Self::Unary {
                op,
                operand: f(operand)?,
            },
            Self::Binary { op, left, right } => Self::Binary {
                op,
                left: f(left)?,
                right: f(right)?,
            },
            Self::Call { function, args } => Self::Call {
                function,
                args: map(args)?,
            },
            Self::Method {
                receiver,
                name,
                args,
            } => Self::Method {
                receiver: f(receiver)?,
                name,
                args: map(args)?,
            },
            Self::Field { receiver, name } => Self::Field {
                receiver: f(receiver)?,
                name,
            },
            Self::Index { receiver, index } => Self::Index {
                receiver: f(receiver)?,
                index: f(index)?,
            },
            Self::List { entries } => Self::List {
                entries: map(entries)?,
            },
            Self::Unwrap { operand } => Self::Unwrap {
                operand: f(operand)?,
            },
            leaf => leaf,
        })
    }

    /// Whether the two are the same but for their parts.
    fn same_shape(&self, other: &Repr) -> bool {
        match (self, other) {
            (Self::Unary { op: a, .. }, Self::Unary { op: b, .. }) => a == b,
            (Self::Binary { op: a, .. }, Self::Binary { op: b, .. }) => a == b,
            (
                Self::Call {
                    function: a,
                    args: x,
                },
                Self::Call {
                    function: b,
                    args: y,
                },
            ) => a == b && x.len() == y.len(),
            (
                Self::Method {
                    name: a, args: x, ..
                },
                Self::Method {
                    name: b, args: y, ..
                },
            ) => a == b && x.len() == y.len(),
            (Self::Field { name: a, .. }, Self::Field { name: b, .. }) => a == b,
            (Self::Index { .. }, Self::Index { .. }) => true,
            (Self::Unwrap { .. }, Self::Unwrap { .. }) => true,
            (Self::List { entries: x }, Self::List { entries: y }) => x.len() == y.len(),
            (a, b) => a == b,
        }
    }
}

/// Structures are equal when their expressions are, however their nodes
/// are numbered.
impl PartialEq for Structure {
    fn eq(&self, other: &Self) -> bool {
        let mut stack = vec![(self.root, other.root)];

        while let Some((a, b)) = stack.pop() {
            let (Some(a), Some(b)) = (self.get(a), other.get(b)) else {
                return false;
            };

            if !a.0.same_shape(&b.0) {
                return false;
            }

            stack.extend(a.children().into_iter().zip(b.children()));
        }

        true
    }
}

impl Eq for Structure {}

impl fmt::Display for Structure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

/// A stored structure that is not one tree, at `node` when about one.
#[cold]
fn invalid(invalid: Invalid, node: Option<NodeId>) -> StoreError {
    let mut error = StoreError::from(invalid);

    if let Some(node) = node {
        error = error.context(Message::AtNode { node: node.0 });
    }

    error.context(Message::DecodingStructure)
}

/// The nodes of the expression at `root`, parts before what holds them.
/// Stops at nodes seen before, so it ends on any table.
fn post_order(nodes: &[Node], root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut seen = vec![false; nodes.len()];
    let mut stack = vec![(root, false)];

    while let Some((id, exit)) = stack.pop() {
        if exit {
            out.push(id);
            continue;
        }

        let Some(seen) = seen.get_mut(id.index()) else {
            continue;
        };

        if *seen {
            continue;
        }

        *seen = true;
        stack.push((id, true));

        if let Some(node) = nodes.get(id.index()) {
            stack.extend(node.children().into_iter().rev().map(|c| (c, false)));
        }
    }

    out
}

/// Compile the expression at `root`.
pub(crate) fn compile_node(
    nodes: &[Node],
    root: NodeId,
    env: &Environment,
    options: &Options,
) -> Result<Program, StructureErrors> {
    Ok(compile_node_debug(nodes, root, env, options)?.0)
}

/// Compile the expression at `root`, with its debug information.
fn compile_node_debug(
    nodes: &[Node],
    root: NodeId,
    env: &Environment,
    options: &Options,
) -> Result<(Program, DebugInfo), StructureErrors> {
    let (text, spans) = render(nodes, root);
    let mut list = Vec::new();

    for id in post_order(nodes, root) {
        let Some(node) = nodes.get(id.index()) else {
            continue;
        };

        let kind = if node.is_hole() {
            StructureKind::Hole
        } else {
            let name = match node {
                Node(Repr::Name(name)) | Node(Repr::Call { function: name, .. }) => {
                    Some(name).filter(|n| !is_value_name(n) || is_reserved(n))
                }
                Node(Repr::Method {
                    name: Some(name), ..
                })
                | Node(Repr::Field {
                    name: Some(name), ..
                }) => Some(name).filter(|n| !is_field_name(n) || is_reserved(n)),
                _ => None,
            };

            match name {
                Some(name) => StructureKind::InvalidName(name.as_str().into()),
                None => continue,
            }
        };

        list.push(StructureError { node: id, kind });
    }

    if !list.is_empty() {
        // In the order they are written.
        list.sort_by_key(|entry| {
            spans
                .iter()
                .find(|(id, _)| *id == entry.node)
                .map_or(u32::MAX, |(_, s)| s.start)
        });

        return Err(StructureErrors { list });
    }

    let error = |errors: Diagnostics| StructureErrors {
        list: errors
            .into_iter()
            .map(|diagnostic| StructureError {
                node: locate(&spans, diagnostic.span()).unwrap_or(root),
                kind: StructureKind::Compile(Diagnostics::one(diagnostic)),
            })
            .collect(),
    };

    let parsed = parse_with(&text, options).map_err(error)?;
    compile_parsed(&text, parsed, env).map_err(error)
}

/// Names that are written as other tokens, or as a hole.
pub(crate) fn is_reserved(name: &str) -> bool {
    matches!(name, "true" | "false" | HOLE)
}

/// The innermost node whose text holds `span`.
fn locate(spans: &[(NodeId, Span<u32>)], span: Span<u32>) -> Option<NodeId> {
    let mut best: Option<(NodeId, u32)> = None;

    for (id, s) in spans {
        if s.start <= span.start && span.end <= s.end {
            let len = s.end.saturating_sub(s.start);

            if best.is_none_or(|(_, b)| len <= b) {
                best = Some((*id, len));
            }
        }
    }

    best.map(|(id, _)| id)
}

enum Emit {
    Node(NodeId, bool),
    Text(&'static str),
    Owned(String),
    /// The end of a node's text.
    /// The end of the node whose span is `spans[usize]`, which started at
    /// the second `usize`.
    End(usize, usize),
}

/// The text of the expression at `root`, and each node's span in it,
/// parentheses included, outer nodes first.
fn render(nodes: &[Node], root: NodeId) -> (String, Vec<(NodeId, Span<u32>)>) {
    let mut out = String::new();
    let mut spans = Vec::new();
    let mut stack = vec![Emit::Node(root, false)];
    let mut steps = 0usize;
    let get = |id: NodeId| nodes.get(id.index());

    while let Some(emit) = stack.pop() {
        // A table with a cycle renders until it runs out of steps.
        steps = steps.saturating_add(1);

        if steps > nodes.len().saturating_mul(16).saturating_add(16) {
            break;
        }

        let (id, paren) = match emit {
            Emit::Text(text) => {
                out.push_str(text);
                continue;
            }
            Emit::Owned(text) => {
                out.push_str(&text);
                continue;
            }
            Emit::End(n, at) => {
                if let Some((_, s)) = spans.get_mut(n) {
                    *s = crate::lexer::span(at, out.len());
                }

                continue;
            }
            Emit::Node(id, paren) => (id, paren),
        };

        let Some(node) = get(id) else {
            out.push_str(HOLE);
            continue;
        };

        let span_at = spans.len();
        spans.push((id, crate::lexer::span(out.len(), out.len())));
        let start = out.len();

        if paren {
            out.push('(');
        }

        // What follows the node's first characters, pushed in reverse.
        let mut then = Vec::new();
        let power = |id: NodeId| get(id).map_or(ATOM, |n| n.0.power());

        match node {
            Node(Repr::Hole) => out.push_str(HOLE),
            Node(Repr::Int(n)) => out.push_str(&n.to_string()),
            Node(Repr::Bool(b)) => out.push_str(if *b { "true" } else { "false" }),
            Node(Repr::String(s)) => escape(&mut out, s),
            Node(Repr::Name(name)) => out.push_str(name),
            Node(Repr::Unary { op, operand }) => {
                out.push_str(op.symbol());
                // `-(12)`, since `-12` is the integer.
                let literal = *op == UnaryOp::Neg
                    && matches!(get(*operand), Some(Node(Repr::Int(n))) if *n >= 0);
                then.push(Emit::Node(*operand, literal || power(*operand) < PREFIX));
            }
            Node(Repr::Binary { op, left, right }) => {
                let p = op.power();
                let lp = power(*left);
                then.push(Emit::Node(*left, lp < p || (p == COMPARE && lp == COMPARE)));
                then.push(Emit::Text(match op {
                    BinaryOp::Add => " + ",
                    BinaryOp::Sub => " - ",
                    BinaryOp::Mul => " * ",
                    BinaryOp::Div => " / ",
                    BinaryOp::Rem => " % ",
                    BinaryOp::Eq => " == ",
                    BinaryOp::Ne => " != ",
                    BinaryOp::Lt => " < ",
                    BinaryOp::Le => " <= ",
                    BinaryOp::Gt => " > ",
                    BinaryOp::Ge => " >= ",
                    BinaryOp::And => " && ",
                    BinaryOp::Or => " || ",
                }));
                then.push(Emit::Node(*right, power(*right) <= p));
            }
            Node(Repr::Call { function, args }) => {
                out.push_str(function);
                then.push(Emit::Text("("));
                separated(&mut then, args);
                then.push(Emit::Text(")"));
            }
            Node(Repr::Method {
                receiver,
                name,
                args,
            }) => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("."));
                then.push(name_emit(name.as_deref()));
                then.push(Emit::Text("("));
                separated(&mut then, args);
                then.push(Emit::Text(")"));
            }
            Node(Repr::Field { receiver, name }) => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("."));
                then.push(name_emit(name.as_deref()));
            }
            Node(Repr::Index { receiver, index }) => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("["));
                then.push(Emit::Node(*index, false));
                then.push(Emit::Text("]"));
            }
            Node(Repr::List { entries }) => {
                out.push('[');
                separated(&mut then, entries);
                then.push(Emit::Text("]"));
            }
            Node(Repr::Unwrap { operand }) => {
                then.push(Emit::Node(*operand, power(*operand) < ATOM));
                then.push(Emit::Text("!"));
            }
        }

        if paren {
            then.push(Emit::Text(")"));
        }

        stack.push(Emit::End(span_at, start));
        stack.extend(then.into_iter().rev());
    }

    (out, spans)
}

fn name_emit(name: Option<&str>) -> Emit {
    match name {
        None => Emit::Text(HOLE),
        Some(name) => Emit::Owned(name.to_owned()),
    }
}

fn separated(then: &mut Vec<Emit>, ids: &[NodeId]) {
    for (n, id) in ids.iter().enumerate() {
        if n > 0 {
            then.push(Emit::Text(", "));
        }

        then.push(Emit::Node(*id, false));
    }
}

/// Write `s` as a string literal.
pub(crate) fn escape(out: &mut String, s: &str) {
    out.push('"');

    for c in s.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }

        out.push(c);
    }

    out.push('"');
}

/// Build a structure from a parse of `source`.
fn from_tree(source: &str, tree: &Tree) -> Result<Structure, Diagnostic> {
    let text = |node: N<'_>| source.get(node.range()).unwrap_or_default();
    let internal = |message: &str| {
        Diagnostic::new(
            Span::new(0, 0),
            crate::error::ErrorKind::Internal(message.to_owned()),
        )
    };

    let root = tree
        .first()
        .and_then(|root| exprs(root).next())
        .ok_or_else(|| internal("a tree without an expression"))?;

    let mut s = Structure {
        nodes: Vec::new(),
        parents: Vec::new(),
        // No node, until the expression is built.
        root: NodeId(u32::MAX),
    };

    let mut stack = vec![(root, false)];
    let mut done: Vec<NodeId> = Vec::new();
    let edit = |e: EditError| internal(&e.to_string());

    while let Some((node, exit)) = stack.pop() {
        // A negative integer, or a parenthesis, is one node of the
        // structure; everything else has its parts first.
        let negative = node.value() == Syntax::Unary
            && operator(node) == Some(Syntax::Minus)
            && exprs(node)
                .next()
                .is_some_and(|n| n.value() == Syntax::Integer);

        if !exit && !negative && node.value() != Syntax::Paren {
            stack.push((node, true));
            stack.extend(
                exprs(node)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .map(|n| (n, false)),
            );
            continue;
        }

        if node.value() == Syntax::Paren {
            if let Some(inner) = exprs(node).next() {
                stack.push((inner, false));
            }

            continue;
        }

        let n = exprs(node).count();
        let at = done
            .len()
            .checked_sub(if negative { 0 } else { n })
            .ok_or_else(|| internal("fewer parts than expected"))?;
        let parts = done.split_off(at);
        let member = || {
            name_of(node)
                .map(text)
                .filter(|n| *n != HOLE)
                .map(str::to_owned)
        };

        let built = match node.value() {
            Syntax::Unary if negative => {
                let n = exprs(node)
                    .next()
                    .map(text)
                    .and_then(|t| format!("-{t}").parse::<i64>().ok())
                    .ok_or_else(|| Diagnostic::new(*node.span(), ErrorKind::IntegerTooLarge))?;
                Node(Repr::Int(n))
            }
            Syntax::Integer => {
                Node(Repr::Int(text(node).parse().map_err(|_| {
                    Diagnostic::new(*node.span(), ErrorKind::IntegerTooLarge)
                })?))
            }
            Syntax::Bool => Node(Repr::Bool(text(node) == "true")),
            Syntax::String => Node(Repr::String(
                unescape(text(node)).ok_or_else(|| internal("a malformed string literal"))?,
            )),
            Syntax::Name if text(node) == HOLE => Node(Repr::Hole),
            Syntax::Name => Node(Repr::Name(text(node).to_owned())),
            Syntax::Unary => Node(Repr::Unary {
                op: match operator(node) {
                    Some(Syntax::Minus) => UnaryOp::Neg,
                    _ => UnaryOp::Not,
                },
                operand: *parts.first().ok_or_else(|| internal("a lone operator"))?,
            }),
            Syntax::Binary => {
                let [left, right] = parts.as_slice() else {
                    return Err(internal("a binary operator without two operands"));
                };

                Node(Repr::Binary {
                    op: operator(node)
                        .and_then(BinaryOp::from_syntax)
                        .ok_or_else(|| internal("an unknown binary operator"))?,
                    left: *left,
                    right: *right,
                })
            }
            Syntax::Call => {
                let function = node
                    .children()
                    .find(|n| n.value() == Syntax::Name)
                    .map(text)
                    .ok_or_else(|| internal("a call without a name"))?;
                Node(Repr::Call {
                    function: function.to_owned(),
                    args: parts,
                })
            }
            Syntax::Method => {
                let mut parts = parts.into_iter();
                Node(Repr::Method {
                    receiver: parts.next().ok_or_else(|| internal("no receiver"))?,
                    name: member(),
                    args: parts.collect(),
                })
            }
            Syntax::Field => Node(Repr::Field {
                receiver: *parts.first().ok_or_else(|| internal("no receiver"))?,
                name: member(),
            }),
            Syntax::Index => {
                let [receiver, index] = parts.as_slice() else {
                    return Err(internal("an index without two parts"));
                };

                Node(Repr::Index {
                    receiver: *receiver,
                    index: *index,
                })
            }
            Syntax::List => Node(Repr::List { entries: parts }),
            Syntax::Unwrap => Node(Repr::Unwrap {
                operand: *parts.first().ok_or_else(|| internal("no operand"))?,
            }),
            _ => return Err(internal("not an expression")),
        };

        done.push(s.add(built).map_err(edit)?);
    }

    let [root] = done.as_slice() else {
        return Err(internal("not one expression"));
    };

    s.root = *root;
    Ok(s)
}

fn is_expr(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::Integer
            | Syntax::String
            | Syntax::Bool
            | Syntax::Name
            | Syntax::Paren
            | Syntax::Unary
            | Syntax::Binary
            | Syntax::Call
            | Syntax::Method
            | Syntax::Field
            | Syntax::Index
            | Syntax::Unwrap
            | Syntax::List
    )
}

/// The expressions of the structure directly inside `node`, in the order
/// they are written: not a call's function name nor a method's or field's
/// name, which are part of the node itself.
fn exprs<'a>(node: N<'a>) -> impl Iterator<Item = N<'a>> {
    let kind = node.value();

    // A call's name, and a method's or field's name after its `.`, are
    // part of the node itself.
    let direct = node
        .children()
        .take_while(move |n| {
            !(matches!(kind, Syntax::Method | Syntax::Field) && n.value() == Syntax::Dot)
        })
        .filter(move |n| is_expr(n.value()) && kind != Syntax::Call);

    let args = node
        .children()
        .filter(|n| n.value() == Syntax::Args)
        .flat_map(|args| args.children().filter(|n| is_expr(n.value())));

    direct.chain(args)
}

fn operator(node: N<'_>) -> Option<Syntax> {
    node.children()
        .map(|n| n.value())
        .find(|kind| crate::syntax::is_operator(*kind))
}

/// The `Name` after a method's or field's `.`.
fn name_of(node: N<'_>) -> Option<N<'_>> {
    node.children()
        .skip_while(|n| n.value() != Syntax::Dot)
        .find(|n| n.value() == Syntax::Name)
}

#[cfg(test)]
pub(crate) fn test_id(n: u32) -> NodeId {
    NodeId(n)
}

#[cfg(test)]
pub(crate) fn test_stored(nodes: Vec<Node>, root: NodeId) -> Vec<u8> {
    musli::storage::to_vec(&Stored {
        format: FORMAT,
        nodes,
        root,
    })
    .unwrap_or_default()
}
