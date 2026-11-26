//! Completions: what fits in a structure's hole, by the type its place
//! takes.

use std::collections::HashSet;
use std::fmt;

use crate::Type;
use crate::env::{Environment, Overload};
use crate::error::Arity;
use crate::methods;
use crate::sig::Shape;
use crate::structure::{
    BinaryOp, EditError, EditKind, HOLE, Node, NodeId, Repr, Structure, StructureErrors, UnaryOp,
    escape, is_reserved,
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
    /// An optional value unwrapped with `!`, such as `bonus!`, which fails
    /// the run when it is absent.
    Unwrap,
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
    pub(crate) signature: Option<String>,
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
    ///
    /// // `lookup` gives an option of whatever its list holds: an
    /// // `Option<Int>` where one is wanted, and not known where anything is.
    /// let all = s.complete(&env, s.root(), Some(&Type::option(Type::int())))?;
    /// let lookup = all.iter().find(|c| c.label() == "lookup(_, _)").ok_or("not offered")?;
    /// assert_eq!(lookup.ty(), Some(&Type::option(Type::int())));
    ///
    /// let all = s.complete(&env, s.root(), None)?;
    /// let lookup = all.iter().find(|c| c.label() == "lookup(_, _)").ok_or("not offered")?;
    /// assert_eq!(lookup.ty(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn ty(&self) -> Option<&Type> {
        self.ty.as_ref()
    }

    /// The signature of the function or method it calls, generic over `T`
    /// where it takes any type, or `None` for anything else.
    ///
    /// ```
    /// use evalite::{Environment, Structure, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("bonus", Type::option(Type::int()))?;
    /// env.function("rating", [Type::string()], Type::int())?;
    ///
    /// let s = Structure::from_text("_")?;
    /// let all = s.complete(&env, s.root(), None)?;
    /// let signature = |label: &str| all.iter().find(|c| c.label() == label).and_then(|c| c.signature());
    /// assert_eq!(signature("lookup(_, _)"), Some("lookup<T>(Int, List<T>) -> Option<T>"));
    /// assert_eq!(signature("min(_, _)"), Some("min(Int, ...) -> Int"));
    /// assert_eq!(signature("rating(_)"), Some("rating(String) -> Int"));
    /// assert_eq!(signature("bonus"), None);
    ///
    /// // Where an `Int` is wanted, `bonus` is offered unwrapped.
    /// let ints = s.complete(&env, s.root(), Some(&Type::int()))?;
    /// let unwrap = ints.iter().find(|c| c.label() == "bonus.unwrap_or(_)").ok_or("not offered")?;
    /// assert_eq!(unwrap.signature(), Some("Option<T>.unwrap_or(T) -> T"));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }
}

/// Why a structure cannot be completed at a node.
///
/// ```
/// use evalite::{Environment, Structure};
///
/// let s = Structure::from_text("_._")?;
/// let error = s.complete(&Environment::new(), s.holes()[0], None).unwrap_err();
/// assert_eq!(error.to_string(), "the receiver's type is not known");
/// assert_eq!(
///     format!("{error:#}"),
///     "the receiver's type is not known: the expression is unfinished, fill in the `_`"
/// );
///
/// // The receiver's own error is the source.
/// let source = std::error::Error::source(&error).unwrap();
/// assert_eq!(source.to_string(), "the expression is unfinished, fill in the `_`");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteError {
    kind: CompleteKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompleteKind {
    UnknownNode(NodeId),
    /// The receiver of a member hole does not compile, so its members are
    /// not known.
    Receiver(StructureErrors),
}

impl CompleteError {
    #[cfg(test)]
    pub(crate) fn kind(&self) -> &CompleteKind {
        &self.kind
    }
}

impl From<CompleteKind> for CompleteError {
    #[inline]
    fn from(kind: CompleteKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for CompleteError {
    /// `{:#}` adds the receiver's error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CompleteKind::UnknownNode(id) => write!(f, "there is no node {id}"),
            CompleteKind::Receiver(e) if f.alternate() => {
                write!(f, "the receiver's type is not known: {e}")
            }
            CompleteKind::Receiver(_) => f.write_str("the receiver's type is not known"),
        }
    }
}

impl std::error::Error for CompleteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            CompleteKind::Receiver(e) => Some(e),
            CompleteKind::UnknownNode(_) => None,
        }
    }
}

/// A type a place takes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    Type(Type),
    /// Any `List<T>`.
    List,
    /// Any `Option<T>`.
    Option,
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
    /// An option unwrapped with `!`.
    OptionOf,
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
                Pattern::Option => matches!(ty, Type(Kind::Option(_))),
                Pattern::Record => matches!(ty, Type(Kind::Record(_))),
                Pattern::Method(name) => receiver_methods(ty).any(|m| m.name == name),
                Pattern::Field(name) => {
                    matches!(ty, Type(Kind::Record(fields)) if fields.contains_key(name))
                }
            }),
        }
    }

    /// Whether a result of `shape` can fit, its `T` not yet known: `None`
    /// when it cannot, else the type it gives, when that is known.
    fn fits_shape(&self, shape: Shape) -> Option<Option<Type>> {
        if let Some(ty) = shape.resolve(None) {
            return self.fits(&ty).then_some(Some(ty));
        }

        let Self::OneOf(patterns) = self else {
            return Some(None);
        };

        let mut fitting = patterns.iter().filter(|p| match p {
            Pattern::Type(ty) => shape.bind(ty, &mut None),
            Pattern::List => matches!(shape, Shape::List(_) | Shape::Var),
            Pattern::Option => matches!(shape, Shape::Option(_) | Shape::Var),
            Pattern::Record | Pattern::Field(_) => shape == Shape::Var,
            Pattern::Method(name) => {
                shape == Shape::Var
                    || shape
                        .resolve(Some(&Type::int()))
                        .is_some_and(|ty| receiver_methods(&ty).any(|m| m.name == name))
            }
        });

        match (fitting.next()?, fitting.next()) {
            (Pattern::Type(ty), None) => Some(Some(ty.clone())),
            _ => Some(None),
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

    /// Options of what this takes, which `!` unwraps to it.
    fn option_of(self) -> Self {
        match self {
            Self::Any => Self::OneOf(vec![Pattern::Option]),
            Self::OneOf(patterns) => {
                let mut out = Self::OneOf(Vec::new());

                for p in patterns {
                    out.push(match p {
                        Pattern::Type(t) => Pattern::Type(Type::option(t)),
                        _ => Pattern::Option,
                    });
                }

                out
            }
        }
    }

    /// What `_!` gives in this place, if anything fits it: an option's
    /// value may be of any type, options too, so anything that fits here
    /// does. Its type is known when this takes one type alone.
    fn unwrapped(&self) -> Option<Option<Type>> {
        match self {
            Self::Any => Some(None),
            Self::OneOf(patterns) => match patterns.as_slice() {
                [] => None,
                [Pattern::Type(ty)] => Some(Some(ty.clone())),
                _ => Some(None),
            },
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
    methods::methods_of(ty)
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
    /// # Errors
    ///
    /// Returns a [`CompleteError`] when `at` is not a node of the structure, or
    /// `at` is a member hole whose receiver does not compile, so its members
    /// are not known; the receiver's [`StructureErrors`] is then its
    /// [`source`](std::error::Error::source).
    ///
    /// # Examples
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
        let node = self.get(at).ok_or(CompleteKind::UnknownNode(at))?;
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
                    .map_err(CompleteKind::Receiver)?;
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
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when `at` is not a node of the structure, when
    /// `completion` is a field or method and `at` is neither, or when the edit
    /// would break the tree, as [`graft`](Self::graft) and
    /// [`replace`](Self::replace) do.
    pub fn apply(&mut self, at: NodeId, completion: &Completion) -> Result<(), EditError> {
        let (receiver, old_args) = match self.get(at) {
            Some(Node(Repr::Field { receiver, .. })) => (Some(*receiver), Vec::new()),
            Some(Node(Repr::Method { receiver, args, .. })) => (Some(*receiver), args.clone()),
            Some(_) => (None, Vec::new()),
            None => return Err(EditKind::UnknownNode(at).into()),
        };

        match &completion.insert {
            Insert::Expr(expr) => self.graft(at, expr),
            Insert::Field(name) => {
                let receiver = receiver.ok_or(EditKind::NotAMember(at))?;

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
                let receiver = receiver.ok_or(EditKind::NotAMember(at))?;

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
                        Some(methods::find(&ty, name)?.signature(&ty))
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
                Node(Repr::Unwrap { .. }) => {
                    steps_up.push(Up::OptionOf);
                    at = parent;
                    continue;
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
            Up::OptionOf => want.option_of(),
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
                Overload::Builtin(builtin) => {
                    let sig = builtin.signature();

                    let count = match sig.arity() {
                        Arity::Exactly(k) => args.len() == k,
                        Arity::AtLeast(k) => args.len() >= k,
                    };

                    // The `T` the other arguments bind, if they fit.
                    let mut t = None;
                    let others = (0..args.len()).all(|i| match (sig.param(i), known.get(i)) {
                        (Some(param), Some(Some(ty))) => param.bind(ty, &mut t),
                        (Some(_), _) => true,
                        (None, _) => false,
                    });

                    let Some(param) = sig.param(n).filter(|_| count && others) else {
                        continue;
                    };

                    match (param.resolve(t.as_ref()), param) {
                        (Some(ty), _) => want.push(Pattern::Type(ty)),
                        (None, Shape::List(_)) => want.push(Pattern::List),
                        (None, _) => return Want::Any,
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
                let (arity, ty) = match overload {
                    Overload::Builtin(builtin) => {
                        let sig = builtin.signature();

                        let Some(ty) = want.fits_shape(sig.ret) else {
                            continue;
                        };

                        (sig.holes(), ty)
                    }
                    Overload::Host { params, ret } => {
                        if !want.fits(ret) {
                            continue;
                        }

                        (params.len(), Some(ret.clone()))
                    }
                };

                let label = format!("{name}({})", holes(arity));
                let signature = overload.signature(name);
                expr_signed(&mut out, label, CompletionKind::Function, ty, signature);
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

        if let Some(ty) = want.unwrapped() {
            expr(&mut out, format!("{HOLE}!"), CompletionKind::Operator, ty);
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
            signature: None,
            insert: Insert::Expr(structure),
        });
    }
}

/// Offer the call or method call `label` is the text of, of `signature`.
fn expr_signed(
    out: &mut Vec<Completion>,
    label: String,
    kind: CompletionKind,
    ty: Option<Type>,
    signature: String,
) {
    expr(out, label, kind, ty);

    if let Some(last) = out.last_mut() {
        last.signature = Some(signature);
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

        if let Type(Kind::Option(value)) = &ty
            && want.fits(value)
        {
            let label = format!("{path}!");
            expr(out, label, CompletionKind::Unwrap, Some((**value).clone()));
        }

        for method in receiver_methods(&ty) {
            let (params, ret) = method.signature(&ty);

            if want.fits(&ret) {
                let label = format!("{path}.{}({})", method.name, holes(params.len()));
                expr_signed(
                    out,
                    label,
                    CompletionKind::Method,
                    Some(ret),
                    method.describe(),
                );
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
                    signature: None,
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
                signature: Some(method.describe()),
                insert: Insert::Method {
                    name: method.name.to_owned(),
                    args: params.len(),
                },
            });
        }
    }

    out
}
