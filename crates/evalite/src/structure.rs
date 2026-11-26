//! Expressions as data: the shape a form builds, which may have holes while
//! it is being edited, and which compiles through the same parser and
//! checker as text.

use std::fmt;

use musli::{Decode, Encode};
use syntree::{FlavorDefault, Node as TreeNode, Span};

use crate::check::compile_tree_debug;
use crate::debug::DebugInfo;
use crate::env::Environment;
use crate::error::{ErrorKind, ParseError};
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
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
        }
    }
}

/// An operator between two operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinaryOp {
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode)]
pub enum Node {
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
}

impl Node {
    /// Its parts, in the order they are written.
    pub fn children(&self) -> Vec<NodeId> {
        match self {
            Self::Hole | Self::Int(_) | Self::Bool(_) | Self::String(_) | Self::Name(_) => {
                Vec::new()
            }
            Self::Unary { operand, .. } => vec![*operand],
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
    pub fn is_hole(&self) -> bool {
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditError {
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

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "there is no node {id}"),
            Self::Attached(id) => write!(f, "node {id} is already part of another node"),
            Self::Cycle(id) => write!(f, "node {id} would be part of itself"),
            Self::NotAMember(id) => write!(f, "node {id} is not a field or a method"),
            Self::TooLarge => write!(f, "the structure has too many nodes"),
        }
    }
}

impl std::error::Error for EditError {}

/// Why stored bytes are not a structure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    Encode(String),
    Decode(String),
    /// A structure stored by another version of evalite.
    Format(u32),
    /// The bytes decode, but their nodes are not one tree.
    Invalid(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "the structure could not be stored: {e}"),
            Self::Decode(e) => write!(f, "these bytes are not a stored structure: {e}"),
            Self::Format(v) => write!(
                f,
                "the structure was stored in format {v}, this evalite reads format {FORMAT}"
            ),
            Self::Invalid(e) => write!(f, "the stored structure is not valid: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why a structure does not compile, and at which node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureError {
    node: NodeId,
    kind: StructureErrorKind,
}

/// What is wrong with a structure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructureErrorKind {
    /// A hole, which must be filled before the expression compiles.
    Hole,
    /// A name that cannot be written as one, such as `"Strength"` for a
    /// value or `"true"`. A record field of any name is read with
    /// [`Node::Index`] and a [`Node::String`].
    InvalidName(String),
    /// The error compiling [`Structure::to_text`]'s text gives: its span
    /// is in that text.
    Compile(ParseError),
}

impl StructureError {
    /// The node the error is about.
    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn kind(&self) -> &StructureErrorKind {
        &self.kind
    }
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StructureErrorKind::Hole => write!(f, "the expression is unfinished, fill in the `_`"),
            StructureErrorKind::InvalidName(name) => write!(
                f,
                "`{name}` cannot be written as a name, names are letters, digits and `_`, starting with a letter or `_`"
            ),
            StructureErrorKind::Compile(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for StructureError {}

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
/// let bonus = s.add(Node::Name("bonus".into()))?;
/// let zero = s.add(Node::Int(0))?;
/// let unwrap = s.add(Node::Method { receiver: bonus, name: Some("unwrap_or".into()), args: vec![zero] })?;
/// s.replace(s.root(), Node::Call { function: "max".into(), args: vec![unwrap] })?;
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
            nodes: vec![Node::Hole],
            parents: vec![None],
            root: NodeId(0),
        }
    }

    /// The whole expression's node.
    pub fn root(&self) -> NodeId {
        self.root
    }

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
    pub fn add(&mut self, node: Node) -> Result<NodeId, EditError> {
        let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| EditError::TooLarge)?);
        self.check_free(&node, None)?;
        self.attach(&node, id);
        self.nodes.push(node);
        self.parents.push(None);
        Ok(id)
    }

    /// Replace node `at` with `node`. `at`'s old parts are no longer part
    /// of it, and so may be used again, such as by `node`.
    pub fn replace(&mut self, at: NodeId, node: Node) -> Result<(), EditError> {
        let old = self.get(at).ok_or(EditError::UnknownNode(at))?;
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
    pub fn set_root(&mut self, id: NodeId) -> Result<(), EditError> {
        if self.get(id).is_none() {
            return Err(EditError::UnknownNode(id));
        }

        if id != self.root && self.parent(id).is_some() {
            return Err(EditError::Attached(id));
        }

        self.root = id;
        Ok(())
    }

    /// Copy the expression of `other` in place of node `at`.
    pub fn graft(&mut self, at: NodeId, other: &Structure) -> Result<(), EditError> {
        if self.get(at).is_none() {
            return Err(EditError::UnknownNode(at));
        }

        let mut ids = vec![None; other.nodes.len()];

        for id in other.post_order() {
            let node = other
                .get(id)
                .ok_or(EditError::UnknownNode(id))?
                .clone()
                .renumber(|child| ids.get(child.index()).copied().flatten())
                .ok_or(EditError::UnknownNode(id))?;

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
                return Err(EditError::UnknownNode(*child));
            }

            let reused = at.is_some_and(|(_, old)| old.contains(child));

            if *child == self.root
                || (self.parent(*child).is_some() && !reused)
                || children.iter().take(n).any(|c| c == child)
            {
                return Err(EditError::Attached(*child));
            }
        }

        if let Some((at, _)) = at {
            // Walk up from `at`: a part of `node` above it would hold
            // itself.
            let mut up = Some(at);
            let mut steps = 0usize;

            while let Some(id) = up {
                if children.contains(&id) {
                    return Err(EditError::Cycle(id));
                }

                steps = steps.saturating_add(1);

                if steps > self.nodes.len() {
                    return Err(EditError::Cycle(id));
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
    pub fn from_text(source: &str) -> Result<Self, ParseError> {
        Self::from_text_with(source, &Options::new())
    }

    /// Parse `source` into a structure.
    pub fn from_text_with(source: &str, options: &Options) -> Result<Self, ParseError> {
        let tree = parse_with(source, options)?;
        from_tree(source, &tree)
    }

    /// Compile against `env`, with the default [`Options`].
    pub fn compile(&self, env: &Environment) -> Result<Program, StructureError> {
        self.compile_with(env, &Options::new())
    }

    /// Compile against `env`: the same program, or the same error, as
    /// compiling [`to_text`](Self::to_text), once it has no holes.
    pub fn compile_with(
        &self,
        env: &Environment,
        options: &Options,
    ) -> Result<Program, StructureError> {
        compile_node(&self.nodes, self.root, env, options)
    }

    /// Compile against `env` as [`compile_with`](Self::compile_with) does,
    /// also giving the [`DebugInfo`] whose spans are into
    /// [`to_text`](Self::to_text).
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
    ) -> Result<(Program, DebugInfo), StructureError> {
        compile_node_debug(&self.nodes, self.root, env, options)
    }

    /// Compile the expression at node `id` alone.
    pub(crate) fn compile_at(
        &self,
        id: NodeId,
        env: &Environment,
    ) -> Result<Program, StructureError> {
        compile_node(&self.nodes, id, env, &Options::new())
    }

    /// The stored form, which [`Structure::from_bytes`] reads. Unused
    /// nodes are left out.
    pub fn to_bytes(&self) -> Result<Vec<u8>, LoadError> {
        let (nodes, root) = self.compact().ok_or_else(|| {
            LoadError::Encode("the structure's nodes are not one tree".to_owned())
        })?;

        let stored = Stored {
            format: FORMAT,
            nodes,
            root,
        };

        musli::storage::to_vec(&stored).map_err(|e| LoadError::Encode(e.to_string()))
    }

    /// Read a stored structure, checking that its nodes are one tree.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        let stored: Stored =
            musli::storage::from_slice(bytes).map_err(|e| LoadError::Decode(e.to_string()))?;

        if stored.format != FORMAT {
            return Err(LoadError::Format(stored.format));
        }

        let mut parents = vec![None; stored.nodes.len()];

        for (at, node) in stored.nodes.iter().enumerate() {
            let at = NodeId(u32::try_from(at).map_err(|_| invalid("too many nodes"))?);

            for child in node.children() {
                let slot = parents
                    .get_mut(child.index())
                    .ok_or_else(|| invalid(format!("node {at} has a part {child} past the end")))?;

                if slot.is_some() || child == stored.root {
                    return Err(invalid(format!("node {child} is a part twice")));
                }

                *slot = Some(at);
            }
        }

        if stored.nodes.get(stored.root.index()).is_none() {
            return Err(invalid("the root is past the end"));
        }

        // Each node but the root has one parent, so reaching every node
        // from the root means there is no cycle.
        if post_order(&stored.nodes, stored.root).len() != stored.nodes.len() {
            return Err(invalid("some nodes are not part of the expression"));
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
                .clone()
                .renumber(|c| ids.get(c.index()).copied().flatten())?;
            *ids.get_mut(id.index())? = Some(NodeId(u32::try_from(nodes.len()).ok()?));
            nodes.push(node);
        }

        let root = NodeId(u32::try_from(nodes.len().checked_sub(1)?).ok()?);
        Some((nodes, root))
    }
}

impl Node {
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
            leaf => leaf,
        })
    }

    /// Whether the two are the same but for their parts.
    fn same_shape(&self, other: &Node) -> bool {
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

            if !a.same_shape(b) {
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

fn invalid(message: impl Into<String>) -> LoadError {
    LoadError::Invalid(message.into())
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
) -> Result<Program, StructureError> {
    Ok(compile_node_debug(nodes, root, env, options)?.0)
}

/// Compile the expression at `root`, with its debug information.
fn compile_node_debug(
    nodes: &[Node],
    root: NodeId,
    env: &Environment,
    options: &Options,
) -> Result<(Program, DebugInfo), StructureError> {
    for id in post_order(nodes, root) {
        let Some(node) = nodes.get(id.index()) else {
            continue;
        };

        let error = |kind| Err(StructureError { node: id, kind });

        if node.is_hole() {
            return error(StructureErrorKind::Hole);
        }

        let name = match node {
            Node::Name(name) | Node::Call { function: name, .. } => {
                Some(name).filter(|n| !is_value_name(n) || is_reserved(n))
            }
            Node::Method {
                name: Some(name), ..
            }
            | Node::Field {
                name: Some(name), ..
            } => Some(name).filter(|n| !is_field_name(n) || is_reserved(n)),
            _ => None,
        };

        if let Some(name) = name {
            return error(StructureErrorKind::InvalidName(name.clone()));
        }
    }

    let (text, spans) = render(nodes, root);

    let error = |error: ParseError| {
        let node = locate(&spans, error.span()).unwrap_or(root);
        StructureError {
            node,
            kind: StructureErrorKind::Compile(error),
        }
    };

    let tree = parse_with(&text, options).map_err(error)?;
    compile_tree_debug(&text, &tree, env).map_err(error)
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
        let power = |id: NodeId| get(id).map_or(ATOM, Node::power);

        match node {
            Node::Hole => out.push_str(HOLE),
            Node::Int(n) => out.push_str(&n.to_string()),
            Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Node::String(s) => escape(&mut out, s),
            Node::Name(name) => out.push_str(name),
            Node::Unary { op, operand } => {
                out.push_str(op.symbol());
                // `-(12)`, since `-12` is the integer.
                let literal =
                    *op == UnaryOp::Neg && matches!(get(*operand), Some(Node::Int(n)) if *n >= 0);
                then.push(Emit::Node(*operand, literal || power(*operand) < PREFIX));
            }
            Node::Binary { op, left, right } => {
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
            Node::Call { function, args } => {
                out.push_str(function);
                then.push(Emit::Text("("));
                separated(&mut then, args);
                then.push(Emit::Text(")"));
            }
            Node::Method {
                receiver,
                name,
                args,
            } => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("."));
                then.push(name_emit(name.as_deref()));
                then.push(Emit::Text("("));
                separated(&mut then, args);
                then.push(Emit::Text(")"));
            }
            Node::Field { receiver, name } => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("."));
                then.push(name_emit(name.as_deref()));
            }
            Node::Index { receiver, index } => {
                then.push(Emit::Node(*receiver, power(*receiver) < ATOM));
                then.push(Emit::Text("["));
                then.push(Emit::Node(*index, false));
                then.push(Emit::Text("]"));
            }
            Node::List { entries } => {
                out.push('[');
                separated(&mut then, entries);
                then.push(Emit::Text("]"));
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
fn from_tree(source: &str, tree: &Tree) -> Result<Structure, ParseError> {
    let text = |node: N<'_>| source.get(node.range()).unwrap_or_default();
    let internal = |message: &str| {
        ParseError::new(
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
                    .ok_or_else(|| ParseError::new(*node.span(), ErrorKind::IntegerTooLarge))?;
                Node::Int(n)
            }
            Syntax::Integer => Node::Int(
                text(node)
                    .parse()
                    .map_err(|_| ParseError::new(*node.span(), ErrorKind::IntegerTooLarge))?,
            ),
            Syntax::Bool => Node::Bool(text(node) == "true"),
            Syntax::String => Node::String(
                unescape(text(node)).ok_or_else(|| internal("a malformed string literal"))?,
            ),
            Syntax::Name if text(node) == HOLE => Node::Hole,
            Syntax::Name => Node::Name(text(node).to_owned()),
            Syntax::Unary => Node::Unary {
                op: match operator(node) {
                    Some(Syntax::Minus) => UnaryOp::Neg,
                    _ => UnaryOp::Not,
                },
                operand: *parts.first().ok_or_else(|| internal("a lone operator"))?,
            },
            Syntax::Binary => {
                let [left, right] = parts.as_slice() else {
                    return Err(internal("a binary operator without two operands"));
                };

                Node::Binary {
                    op: operator(node)
                        .and_then(BinaryOp::from_syntax)
                        .ok_or_else(|| internal("an unknown binary operator"))?,
                    left: *left,
                    right: *right,
                }
            }
            Syntax::Call => {
                let function = node
                    .children()
                    .find(|n| n.value() == Syntax::Name)
                    .map(text)
                    .ok_or_else(|| internal("a call without a name"))?;
                Node::Call {
                    function: function.to_owned(),
                    args: parts,
                }
            }
            Syntax::Method => {
                let mut parts = parts.into_iter();
                Node::Method {
                    receiver: parts.next().ok_or_else(|| internal("no receiver"))?,
                    name: member(),
                    args: parts.collect(),
                }
            }
            Syntax::Field => Node::Field {
                receiver: *parts.first().ok_or_else(|| internal("no receiver"))?,
                name: member(),
            },
            Syntax::Index => {
                let [receiver, index] = parts.as_slice() else {
                    return Err(internal("an index without two parts"));
                };

                Node::Index {
                    receiver: *receiver,
                    index: *index,
                }
            }
            Syntax::List => Node::List { entries: parts },
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
