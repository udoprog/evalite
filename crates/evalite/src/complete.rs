//! Completions: what fits in a structure's hole, by the type its place
//! takes.

use std::collections::HashSet;
use std::fmt;

use crate::Type;
use crate::env::{Builtin, Environment, Overload};
use crate::methods::{self, Receiver};
use crate::structure::{
    BinaryOp, EditError, HOLE, Node, NodeId, Repr, Structure, StructureError, UnaryOp, escape,
    is_reserved,
};
use crate::types::Kind;
use crate::types::is_field_name;

/// Types are walked at most this many steps when looking for a way to
/// reach a wanted type.
const MAX_STEPS: usize = 4096;

/// What a [`Completion`] offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompletionKind {
    /// A value of the environment, or a path into one.
    Value,
    /// A field of a member hole's receiver.
    Field,
    /// A method, of a member hole's receiver or at the end of a path.
    Method,
    /// A call of the environment's function.
    Function,
    /// A literal, such as `0`, `true` or `""`.
    Literal,
    /// An operator with holes for its operands, such as `_ + _`.
    Operator,
}

/// How [`Structure::apply`] fills a hole with a [`Completion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Insert {
    /// Put this expression, holes and all, in the node's place.
    Expr(Structure),
    /// Name a member hole's field, read as `["name"]` when it is not
    /// written as a name.
    Field(String),
    /// Name a member hole's method, which takes this many arguments.
    Method { name: String, args: usize },
}

/// Something that fits in a hole, from [`Structure::complete`], which
/// [`Structure::apply`] puts there.
///
/// It is read through its [`label`](Self::label), [`kind`](Self::kind)
/// and [`ty`](Self::ty); what it inserts stays inside, so it is only
/// applied as offered.
///
/// ```
/// use evalite::{CompletionKind, Environment, Structure, Type};
///
/// let mut env = Environment::with_builtins();
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let mut s = Structure::from_text("1 + _")?;
/// let hole = s.holes()[0];
/// let completions = s.complete(&env, hole, None)?;
///
/// let unwrap = completions
///     .iter()
///     .find(|c| c.label() == "bonus.unwrap_or(_)")
///     .ok_or("not offered")?;
/// assert_eq!(unwrap.kind(), CompletionKind::Method);
/// assert_eq!(unwrap.ty(), Some(&Type::int()));
///
/// s.apply(hole, unwrap)?;
/// assert_eq!(s.to_text(), "1 + bonus.unwrap_or(_)");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub(crate) label: String,
    pub(crate) kind: CompletionKind,
    pub(crate) ty: Option<Type>,
    pub(crate) insert: Insert,
}

impl Completion {
    /// What it looks like, as text, such as `bonus.unwrap_or(_)`. At a
    /// member hole it is the member alone: `unwrap_or(_)`, or a field's
    /// name, or `["Brawl (bar)"]` for one not written as a name.
    ///
    /// ```
    /// use evalite::{Environment, Structure, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("m", Type::record([("athletics", Type::int()), ("Brawl (bar)", Type::int())]))?;
    ///
    /// let s = Structure::from_text("m._")?;
    /// let labels = s
    ///     .complete(&env, s.holes()[0], None)?
    ///     .into_iter()
    ///     .map(|c| c.label().to_owned())
    ///     .collect::<Vec<_>>();
    /// assert!(labels.iter().any(|l| l == "athletics"));
    /// assert!(labels.iter().any(|l| l == r#"["Brawl (bar)"]"#));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn label(&self) -> &str {
        &self.label
    }

    /// What it offers: a value, field, method, call, literal or operator.
    ///
    /// ```
    /// use evalite::{CompletionKind, Environment, Structure, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("wits", Type::int())?;
    ///
    /// let s = Structure::from_text("_")?;
    /// let all = s.complete(&env, s.root(), Some(&Type::int()))?;
    /// let kind = |label: &str| all.iter().find(|c| c.label() == label).map(|c| c.kind());
    /// assert_eq!(kind("wits"), Some(CompletionKind::Value));
    /// assert_eq!(kind("0"), Some(CompletionKind::Literal));
    /// assert_eq!(kind("_ + _"), Some(CompletionKind::Operator));
    /// assert_eq!(kind("max(_, _)"), Some(CompletionKind::Function));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn kind(&self) -> CompletionKind {
        self.kind
    }

    /// The type it gives, when known before its holes are filled: `None`
    /// for one whose type depends on what fills them.
    ///
    /// ```
    /// use evalite::{Environment, Structure, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("bonus", Type::option(Type::int()))?;
    ///
    /// let s = Structure::from_text("_")?;
    /// let all = s.complete(&env, s.root(), Some(&Type::int()))?;
    /// let ty = |label: &str| all.iter().find(|c| c.label() == label).map(|c| c.ty().cloned());
    /// assert_eq!(ty("bonus.unwrap_or(_)"), Some(Some(Type::int())));
    /// // The builtin `lookup` gives whatever its list holds.
    /// assert_eq!(ty("lookup(_, _)"), Some(None));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn ty(&self) -> Option<&Type> {
        self.ty.as_ref()
    }
}

/// Why a structure cannot be completed at a node.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompleteError {
    UnknownNode(NodeId),
    /// The receiver of a member hole does not compile, so its members are
    /// not known.
    Receiver(StructureError),
}

impl fmt::Display for CompleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "there is no node {id}"),
            Self::Receiver(e) => write!(f, "the receiver's type is not known: {e}"),
        }
    }
}

impl std::error::Error for CompleteError {}

/// A type a place takes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    Type(Type),
    /// Any `List<T>`.
    List,
    /// Any record.
    Record,
    /// A type with this method.
    Method(String),
    /// A record with this field.
    Field(String),
}

/// A place between a node and the nearest one whose type is known.
enum Up {
    /// An entry of a list.
    Entry,
    /// A list indexed by an `Int`.
    ListOf,
}

/// The types a place takes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Want {
    Any,
    OneOf(Vec<Pattern>),
}

impl Want {
    fn exactly(ty: Type) -> Self {
        Self::OneOf(vec![Pattern::Type(ty)])
    }

    fn fits(&self, ty: &Type) -> bool {
        match self {
            Self::Any => true,
            Self::OneOf(patterns) => patterns.iter().any(|p| match p {
                Pattern::Type(t) => t == ty,
                Pattern::List => matches!(ty, Type(Kind::List(_))),
                Pattern::Record => matches!(ty, Type(Kind::Record(_))),
                Pattern::Method(name) => receiver_methods(ty).any(|m| m.name == name),
                Pattern::Field(name) => {
                    matches!(ty, Type(Kind::Record(fields)) if fields.contains_key(name))
                }
            }),
        }
    }

    /// Whether anything fits.
    fn is_open(&self) -> bool {
        !matches!(self, Self::OneOf(p) if p.is_empty())
    }

    /// Whether a value of `ty` fits, or reaches something that fits
    /// through its fields, entries and methods.
    fn reachable(&self, ty: &Type) -> bool {
        let mut stack = vec![ty.clone()];
        let mut steps = 0usize;

        while let Some(ty) = stack.pop() {
            steps = steps.saturating_add(1);

            if steps > MAX_STEPS {
                break;
            }

            if self.fits(&ty) {
                return true;
            }

            match &ty {
                Type(Kind::Record(fields)) => stack.extend(fields.values().cloned()),
                Type(Kind::List(entry)) => stack.push((**entry).clone()),
                _ => {}
            }

            for method in receiver_methods(&ty) {
                let (_, ret) = method.signature(&ty);

                // `Int`'s methods give an `Int`: nothing new to reach.
                if ret != ty {
                    stack.push(ret);
                }
            }
        }

        false
    }

    /// The entries of the lists this takes.
    fn entry(self) -> Self {
        match self {
            Self::Any => Self::Any,
            Self::OneOf(patterns) if patterns.contains(&Pattern::List) => Self::Any,
            Self::OneOf(patterns) => Self::OneOf(
                patterns
                    .into_iter()
                    .filter_map(|p| match p {
                        Pattern::Type(Type(Kind::List(t))) => Some(Pattern::Type(*t)),
                        _ => None,
                    })
                    .collect(),
            ),
        }
    }

    /// Lists of what this takes.
    fn list_of(self) -> Self {
        match self {
            Self::Any => Self::OneOf(vec![Pattern::List]),
            Self::OneOf(patterns) => {
                let mut out = Self::OneOf(Vec::new());

                for p in patterns {
                    out.push(match p {
                        Pattern::Type(t) => Pattern::Type(Type::list(t)),
                        _ => Pattern::List,
                    });
                }

                out
            }
        }
    }

    fn push(&mut self, pattern: Pattern) {
        if let Self::OneOf(patterns) = self
            && !patterns.contains(&pattern)
        {
            patterns.push(pattern);
        }
    }
}

fn receiver_methods(ty: &Type) -> impl Iterator<Item = &'static methods::MethodSig> {
    Receiver::of(ty)
        .map(|(receiver, _)| methods::methods(receiver))
        .into_iter()
        .flatten()
}

impl Structure {
    /// What fits at node `at` against `env`: values, paths into them,
    /// methods, calls, literals and operators whose type its place takes.
    /// The place's type comes from what holds it, and `root` is the type
    /// the whole expression must have, if any.
    ///
    /// At a member hole (a [`Node::field`] or [`Node::method`] without a
    /// name) these are its receiver's fields and methods; anywhere else,
    /// expressions to put in the node's place.
    ///
    /// ```
    /// use evalite::{Environment, Structure, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("bonus", Type::option(Type::int()))?;
    ///
    /// let mut s = Structure::from_text("1 + bonus._")?;
    /// let hole = s.holes()[0];
    ///
    /// let labels = s.complete(&env, hole, None)?.into_iter().map(|c| c.label().to_owned()).collect::<Vec<_>>();
    /// assert_eq!(labels, ["unwrap_or(_)"]);
    ///
    /// let completion = &s.complete(&env, hole, None)?[0];
    /// s.apply(hole, completion)?;
    /// assert_eq!(s.to_text(), "1 + bonus.unwrap_or(_)");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn complete(
        &self,
        env: &Environment,
        at: NodeId,
        root: Option<&Type>,
    ) -> Result<Vec<Completion>, CompleteError> {
        let node = self.get(at).ok_or(CompleteError::UnknownNode(at))?;
        let want = self.want(env, at, root);

        let mut out = match node {
            Node(Repr::Field {
                receiver,
                name: None,
            })
            | Node(Repr::Method {
                receiver,
                name: None,
                ..
            }) => {
                let program = self
                    .compile_at(*receiver, env)
                    .map_err(CompleteError::Receiver)?;
                members(&program.result_type(), &want)
            }
            _ => self.values(env, at, &want),
        };

        let mut seen = HashSet::new();
        out.retain(|c| seen.insert(c.label.clone()));
        Ok(out)
    }

    /// Fill node `at` with `completion`, one of [`complete`](Self::complete)'s
    /// for it.
    pub fn apply(&mut self, at: NodeId, completion: &Completion) -> Result<(), EditError> {
        let (receiver, old_args) = match self.get(at) {
            Some(Node(Repr::Field { receiver, .. })) => (Some(*receiver), Vec::new()),
            Some(Node(Repr::Method { receiver, args, .. })) => (Some(*receiver), args.clone()),
            Some(_) => (None, Vec::new()),
            None => return Err(EditError::UnknownNode(at)),
        };

        match &completion.insert {
            Insert::Expr(expr) => self.graft(at, expr),
            Insert::Field(name) => {
                let receiver = receiver.ok_or(EditError::NotAMember(at))?;

                if is_field_name(name) && !is_reserved(name) {
                    self.replace(
                        at,
                        Node(Repr::Field {
                            receiver,
                            name: Some(name.clone()),
                        }),
                    )
                } else {
                    let index = self.add(Node(Repr::String(name.clone())))?;
                    self.replace(at, Node(Repr::Index { receiver, index }))
                }
            }
            Insert::Method { name, args } => {
                let receiver = receiver.ok_or(EditError::NotAMember(at))?;

                let args = if old_args.len() == *args {
                    old_args
                } else {
                    (0..*args)
                        .map(|_| self.add(Node(Repr::Hole)))
                        .collect::<Result<Vec<_>, _>>()?
                };

                self.replace(
                    at,
                    Node(Repr::Method {
                        receiver,
                        name: Some(name.clone()),
                        args,
                    }),
                )
            }
        }
    }

    /// The type of the expression at `id`, if it compiles.
    fn type_of(&self, env: &Environment, id: NodeId) -> Option<Type> {
        Some(self.compile_at(id, env).ok()?.result_type())
    }

    /// The types node `at`'s place takes.
    fn want(&self, env: &Environment, mut at: NodeId, root: Option<&Type>) -> Want {
        // The places passed through on the way up to one whose type is
        // known: list entries without a sibling to tell their type, and
        // lists indexed by an `Int`.
        let mut steps_up = Vec::new();
        let mut steps = 0usize;
        let limit = self.post_order().len();

        let want = loop {
            steps = steps.saturating_add(1);

            let Some(parent) = self.parent(at) else {
                break match root {
                    Some(ty) if at == self.root() => Want::exactly(ty.clone()),
                    _ => Want::Any,
                };
            };

            if steps > limit {
                break Want::Any;
            }

            let Some(node) = self.get(parent) else {
                break Want::Any;
            };

            break match node {
                Node(Repr::Unary {
                    op: UnaryOp::Neg, ..
                }) => Want::exactly(Type::int()),
                Node(Repr::Unary {
                    op: UnaryOp::Not, ..
                }) => Want::exactly(Type::bool()),
                Node(Repr::Binary { op, left, right }) => match op {
                    BinaryOp::And | BinaryOp::Or => Want::exactly(Type::bool()),
                    BinaryOp::Eq | BinaryOp::Ne => {
                        let other = if *left == at { *right } else { *left };

                        match self.type_of(env, other) {
                            Some(ty @ (Type(Kind::Int) | Type(Kind::Bool))) => Want::exactly(ty),
                            _ => Want::OneOf(vec![
                                Pattern::Type(Type::int()),
                                Pattern::Type(Type::bool()),
                            ]),
                        }
                    }
                    _ => Want::exactly(Type::int()),
                },
                Node(Repr::Call { function, args }) => self.argument(env, function, args, at),
                Node(Repr::Method {
                    receiver,
                    name,
                    args,
                }) => {
                    let sig = name.as_deref().and_then(|name| {
                        let ty = self.type_of(env, *receiver)?;
                        let (kind, _) = Receiver::of(&ty)?;
                        Some(methods::find(kind, name)?.signature(&ty))
                    });

                    match (args.iter().position(|a| *a == at), sig, name) {
                        (Some(n), Some((params, _)), _) => params
                            .get(n)
                            .cloned()
                            .map_or(Want::OneOf(Vec::new()), Want::exactly),
                        (None, _, Some(name)) => Want::OneOf(vec![Pattern::Method(name.clone())]),
                        _ => Want::Any,
                    }
                }
                Node(Repr::Field {
                    name: Some(name), ..
                }) => Want::OneOf(vec![Pattern::Field(name.clone())]),
                Node(Repr::Index { receiver, index }) if *receiver == at => {
                    Want::OneOf(match self.get(*index) {
                        Some(Node(Repr::String(name))) => vec![Pattern::Field(name.clone())],
                        Some(Node(Repr::Hole)) => vec![Pattern::List, Pattern::Record],
                        _ => {
                            steps_up.push(Up::ListOf);
                            at = parent;
                            continue;
                        }
                    })
                }
                Node(Repr::Index { receiver, index }) if *index == at => {
                    match self.type_of(env, *receiver) {
                        Some(Type(Kind::List(_))) => Want::exactly(Type(Kind::Int)),
                        Some(Type(Kind::Record(_))) => Want::exactly(Type(Kind::String)),
                        _ => Want::Any,
                    }
                }
                Node(Repr::List { entries }) => {
                    let sibling = entries
                        .iter()
                        .filter(|e| **e != at)
                        .find_map(|e| self.type_of(env, *e));

                    match sibling {
                        Some(ty) => Want::exactly(ty),
                        None => {
                            steps_up.push(Up::Entry);
                            at = parent;
                            continue;
                        }
                    }
                }
                _ => Want::Any,
            };
        };

        steps_up.into_iter().rev().fold(want, |want, up| match up {
            Up::Entry => want.entry(),
            Up::ListOf => want.list_of(),
        })
    }

    /// The types argument `at` of a call of `function` takes, given the
    /// other arguments' types.
    fn argument(&self, env: &Environment, function: &str, args: &[NodeId], at: NodeId) -> Want {
        let Some(overloads) = env.functions.get(function) else {
            return Want::Any;
        };

        let Some(n) = args.iter().position(|a| *a == at) else {
            return Want::Any;
        };

        let known = args
            .iter()
            .map(|a| (*a != at).then(|| self.type_of(env, *a)).flatten())
            .collect::<Vec<_>>();

        let fits = |i: usize, f: &dyn Fn(&Type) -> bool| {
            known.get(i).is_none_or(|k| k.as_ref().is_none_or(f))
        };

        let mut want = Want::OneOf(Vec::new());

        for overload in overloads {
            match overload {
                Overload::Builtin(Builtin::Min | Builtin::Max) => {
                    if (0..args.len()).all(|i| fits(i, &|t| *t == Type::int())) {
                        want.push(Pattern::Type(Type::int()));
                    }
                }
                Overload::Builtin(Builtin::Lookup) => {
                    let list = |t: &Type| matches!(t, Type(Kind::List(_)));

                    if args.len() == 2 && fits(0, &|t| *t == Type::int()) && fits(1, &list) {
                        want.push(match n {
                            0 => Pattern::Type(Type::int()),
                            _ => Pattern::List,
                        });
                    }
                }
                Overload::Host { params, .. } => {
                    if params.len() == args.len()
                        && params.iter().enumerate().all(|(i, p)| fits(i, &|t| t == p))
                        && let Some(param) = params.get(n)
                    {
                        want.push(Pattern::Type(param.clone()));
                    }
                }
            }
        }

        want
    }

    /// Expressions to put in node `at`'s place.
    fn values(&self, env: &Environment, at: NodeId, want: &Want) -> Vec<Completion> {
        let mut out = Vec::new();

        if !want.is_open() {
            return out;
        }

        // A record's index: its fields' names.
        let record = match self.parent(at).and_then(|p| self.get(p)) {
            Some(Node(Repr::Index { receiver, index })) if *index == at => {
                match self.type_of(env, *receiver) {
                    Some(Type(Kind::Record(fields))) => Some(fields),
                    _ => None,
                }
            }
            _ => None,
        };

        if let Some(fields) = &record {
            for name in fields.keys() {
                let mut label = String::new();
                escape(&mut label, name);
                expr(
                    &mut out,
                    label,
                    CompletionKind::Literal,
                    Some(Type::string()),
                );
            }

            return out;
        }

        for (name, ty) in &env.values {
            paths(&mut out, name, ty, want);
        }

        for (name, overloads) in &env.functions {
            for overload in overloads {
                let (arity, ret) = match overload {
                    Overload::Builtin(Builtin::Min | Builtin::Max) => (2, Some(Type::int())),
                    Overload::Builtin(Builtin::Lookup) => (2, None),
                    Overload::Host { params, ret } => (params.len(), Some(ret.clone())),
                };

                if ret.as_ref().is_none_or(|ret| want.fits(ret)) {
                    let label = format!("{name}({})", holes(arity));
                    expr(&mut out, label, CompletionKind::Function, ret);
                }
            }
        }

        let literals = [
            ("0", Type::int()),
            ("true", Type::bool()),
            ("false", Type::bool()),
            ("\"\"", Type::string()),
        ];

        for (label, ty) in literals {
            if want.fits(&ty) {
                expr(
                    &mut out,
                    label.to_owned(),
                    CompletionKind::Literal,
                    Some(ty),
                );
            }
        }

        let list = match want {
            Want::Any => Some(None),
            Want::OneOf(patterns) => patterns.iter().find_map(|p| match p {
                Pattern::Type(ty @ Type(Kind::List(_))) => Some(Some(ty.clone())),
                Pattern::List => Some(None),
                _ => None,
            }),
        };

        if let Some(ty) = list {
            expr(&mut out, format!("[{HOLE}]"), CompletionKind::Literal, ty);
        }

        let operators = [
            (
                Type::int(),
                &[
                    BinaryOp::Add,
                    BinaryOp::Sub,
                    BinaryOp::Mul,
                    BinaryOp::Div,
                    BinaryOp::Rem,
                ][..],
                UnaryOp::Neg,
            ),
            (
                Type::bool(),
                &[
                    BinaryOp::Eq,
                    BinaryOp::Ne,
                    BinaryOp::Lt,
                    BinaryOp::Le,
                    BinaryOp::Gt,
                    BinaryOp::Ge,
                    BinaryOp::And,
                    BinaryOp::Or,
                ][..],
                UnaryOp::Not,
            ),
        ];

        for (ty, binary, unary) in operators {
            if !want.fits(&ty) {
                continue;
            }

            for op in binary {
                let label = format!("{HOLE} {} {HOLE}", op.symbol());
                expr(&mut out, label, CompletionKind::Operator, Some(ty.clone()));
            }

            let label = format!("{}{HOLE}", unary.symbol());
            expr(&mut out, label, CompletionKind::Operator, Some(ty.clone()));
        }

        out
    }
}

/// Offer the expression `label` is the text of.
fn expr(out: &mut Vec<Completion>, label: String, kind: CompletionKind, ty: Option<Type>) {
    if let Ok(structure) = Structure::from_text(&label) {
        out.push(Completion {
            label,
            kind,
            ty,
            insert: Insert::Expr(structure),
        });
    }
}

fn holes(n: usize) -> String {
    vec![HOLE; n].join(", ")
}

/// The value `name` of type `ty`, or the paths into it, and the methods at
/// their ends, that give what `want` takes.
fn paths(out: &mut Vec<Completion>, name: &str, ty: &Type, want: &Want) {
    let mut stack = vec![(name.to_owned(), ty.clone())];
    let mut steps = 0usize;

    while let Some((path, ty)) = stack.pop() {
        steps = steps.saturating_add(1);

        if steps > MAX_STEPS {
            break;
        }

        if want.fits(&ty) {
            expr(out, path, CompletionKind::Value, Some(ty));
            continue;
        }

        for method in receiver_methods(&ty) {
            let (params, ret) = method.signature(&ty);

            if want.fits(&ret) {
                let label = format!("{path}.{}({})", method.name, holes(params.len()));
                expr(out, label, CompletionKind::Method, Some(ret));
            }
        }

        let mut next = Vec::new();

        match &ty {
            Type(Kind::Record(fields)) => {
                for (field, ty) in fields {
                    if want.reachable(ty) {
                        next.push((format!("{path}{}", member(field)), ty.clone()));
                    }
                }
            }
            Type(Kind::List(entry)) if want.reachable(entry) => {
                next.push((format!("{path}[{HOLE}]"), (**entry).clone()));
            }
            _ => {}
        }

        stack.extend(next.into_iter().rev());
    }
}

/// How the field `name` is read: `.name`, or `["name"]`.
fn member(name: &str) -> String {
    if is_field_name(name) && !is_reserved(name) {
        format!(".{name}")
    } else {
        let mut out = String::from("[");
        escape(&mut out, name);
        out.push(']');
        out
    }
}

/// The fields and methods of a `receiver` that give, or lead to, what
/// `want` takes.
fn members(receiver: &Type, want: &Want) -> Vec<Completion> {
    let mut out = Vec::new();

    if let Type(Kind::Record(fields)) = receiver {
        for (name, ty) in fields {
            if want.reachable(ty) {
                let label = member(name);
                let label = label.strip_prefix('.').unwrap_or(&label).to_owned();

                out.push(Completion {
                    label,
                    kind: CompletionKind::Field,
                    ty: Some(ty.clone()),
                    insert: Insert::Field(name.clone()),
                });
            }
        }
    }

    for method in receiver_methods(receiver) {
        let (params, ret) = method.signature(receiver);

        if want.reachable(&ret) {
            out.push(Completion {
                label: format!("{}({})", method.name, holes(params.len())),
                kind: CompletionKind::Method,
                ty: Some(ret),
                insert: Insert::Method {
                    name: method.name.to_owned(),
                    args: params.len(),
                },
            });
        }
    }

    out
}
