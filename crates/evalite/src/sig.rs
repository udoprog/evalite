//! Signatures of the types' functions and methods: the one place their
//! parameter and result types are written, generic over one type `T` where
//! they take any type. The checker resolves calls with them, completions
//! offer from them, and errors and docs print them.

use std::fmt;

use crate::Type;
use crate::error::Arity;
use crate::types::{Kind, TypeId, TypeNode, Types};

/// A type as a [`Shape`] matches it: its outer node, and what it holds.
/// Both a [`Type`] and a [`TypeId`] in its table are one, so the checker
/// matches the ids it has without building the types they name, which
/// may be large.
pub(crate) trait View: Copy + PartialEq {
    fn view(self) -> Viewed<Self>;
}

/// The outer node of a [`View`].
pub(crate) enum Viewed<V> {
    Int,
    Bool,
    String,
    List(V),
    Option(V),
    Map(V, V),
    /// A record, which no shape matches but `T`.
    Record,
}

impl View for &Type {
    fn view(self) -> Viewed<Self> {
        match &self.0 {
            Kind::Int => Viewed::Int,
            Kind::Bool => Viewed::Bool,
            Kind::String => Viewed::String,
            Kind::List(entry) => Viewed::List(entry),
            Kind::Option(value) => Viewed::Option(value),
            Kind::Map(key, value) => Viewed::Map(key, value),
            Kind::Record(_) => Viewed::Record,
        }
    }
}

/// A type in a table. Tables hold each type once, so two are equal when
/// their ids are, in constant time.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Id<'a>(pub(crate) &'a Types, pub(crate) TypeId);

impl PartialEq for Id<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

impl View for Id<'_> {
    fn view(self) -> Viewed<Self> {
        let Self(types, id) = self;

        match types.get(id) {
            Some(TypeNode::Int) => Viewed::Int,
            Some(TypeNode::Bool) => Viewed::Bool,
            Some(TypeNode::String) => Viewed::String,
            Some(TypeNode::List(entry)) => Viewed::List(Self(types, *entry)),
            Some(TypeNode::Option(value)) => Viewed::Option(Self(types, *value)),
            Some(TypeNode::Map { key, value }) => {
                Viewed::Map(Self(types, *key), Self(types, *value))
            }
            Some(TypeNode::Record(_)) | None => Viewed::Record,
        }
    }
}

/// A parameter's or result's type, which may hold the type variable `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Int,
    Bool,
    String,
    /// The type variable `T`, bound by the first argument that holds it.
    Var,
    List(&'static Shape),
    /// `Option<T>`.
    Option(&'static Shape),
    /// `Map<K, V>`, its key's shape then its value's.
    Map(&'static Shape, &'static Shape),
}

impl Shape {
    /// Whether it holds `T`.
    pub(crate) fn is_generic(self) -> bool {
        match self {
            Self::Int | Self::Bool | Self::String => false,
            Self::Var => true,
            Self::List(inner) | Self::Option(inner) => inner.is_generic(),
            Self::Map(key, value) => key.is_generic() || value.is_generic(),
        }
    }

    /// Whether `ty` is of this shape, binding `T` in `t` the first time it
    /// is met, and requiring the bound type every time after.
    pub(crate) fn bind<V: View>(self, ty: V, t: &mut Option<V>) -> bool {
        if self == Self::Var {
            return match t {
                Some(bound) => *bound == ty,
                None => {
                    *t = Some(ty);
                    true
                }
            };
        }

        match (self, ty.view()) {
            (Self::Int, Viewed::Int)
            | (Self::Bool, Viewed::Bool)
            | (Self::String, Viewed::String) => true,
            (Self::List(inner), Viewed::List(entry)) => inner.bind(entry, t),
            (Self::Option(inner), Viewed::Option(value)) => inner.bind(value, t),
            (Self::Map(k, v), Viewed::Map(key, value)) => k.bind(key, t) && v.bind(value, t),
            _ => false,
        }
    }

    /// The type it is with `T` as `t`, or `None` when it holds `T` and `t`
    /// is not known.
    pub(crate) fn resolve(self, t: Option<&Type>) -> Option<Type> {
        Some(match self {
            Self::Int => Type::int(),
            Self::Bool => Type::bool(),
            Self::String => Type::string(),
            Self::Var => t?.clone(),
            Self::List(inner) => Type::list(inner.resolve(t)?),
            Self::Option(inner) => Type::option(inner.resolve(t)?),
            Self::Map(key, value) => Type::map(key.resolve(t)?, value.resolve(t)?),
        })
    }

    /// It as an error or a completion shows it: with `T` as `t` when known,
    /// and as `T` when not.
    pub(crate) fn describe(self, t: Option<&Type>) -> String {
        match self.resolve(t) {
            Some(ty) => ty.to_string(),
            None => self.to_string(),
        }
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int => f.write_str("Int"),
            Self::Bool => f.write_str("Bool"),
            Self::String => f.write_str("String"),
            Self::Var => f.write_str("T"),
            Self::List(inner) => write!(f, "List<{inner}>"),
            Self::Option(inner) => write!(f, "Option<{inner}>"),
            Self::Map(key, value) => write!(f, "Map<{key}, {value}>"),
        }
    }
}

/// What a function or method takes and gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Signature {
    pub(crate) params: &'static [Shape],
    /// The last parameter may be given any number of times, at least once.
    pub(crate) repeats: bool,
    pub(crate) ret: Shape,
}

/// Why arguments do not fit a [`Signature`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mismatch {
    Count(Arity),
    /// Too few arguments for a repeated parameter of a caller's function,
    /// which takes what is described, such as `one or more Int`.
    Takes(String),
    /// The argument at this index, from 0, is not of the type described.
    Argument(usize, String),
}

/// Why arguments do not fit a [`Signature`], before it is described:
/// describing the parameter an argument misses may build the type `T` is,
/// so only a reported one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unfit<V> {
    Mismatch(Mismatch),
    /// The argument at this index, from 0, is not of this parameter, with
    /// `T` as bound so far.
    Argument(usize, Shape, Option<V>),
}

impl<V> Unfit<V> {
    pub(crate) fn map<W>(self, f: impl FnOnce(V) -> W) -> Unfit<W> {
        match self {
            Self::Mismatch(mismatch) => Unfit::Mismatch(mismatch),
            Self::Argument(n, param, t) => Unfit::Argument(n, param, t.map(f)),
        }
    }
}

impl Signature {
    pub(crate) const fn new(params: &'static [Shape], ret: Shape) -> Self {
        Self {
            params,
            repeats: false,
            ret,
        }
    }

    pub(crate) const fn repeating(params: &'static [Shape], ret: Shape) -> Self {
        Self {
            params,
            repeats: true,
            ret,
        }
    }

    /// Whether it holds `T`.
    pub(crate) fn is_generic(&self) -> bool {
        self.params.iter().any(|p| p.is_generic()) || self.ret.is_generic()
    }

    /// The arguments it takes.
    pub(crate) fn arity(&self) -> Arity {
        if self.repeats {
            Arity::AtLeast(self.params.len())
        } else {
            Arity::Exactly(self.params.len())
        }
    }

    /// How many holes a completion of a call leaves: one per parameter, and
    /// one more for a repeated one, as `Int::min(_, _)`.
    pub(crate) fn holes(&self) -> usize {
        self.params.len().saturating_add(usize::from(self.repeats))
    }

    /// The parameter argument `n` is given for.
    pub(crate) fn param(&self, n: usize) -> Option<Shape> {
        match self.params.get(n) {
            Some(param) => Some(*param),
            None if self.repeats => self.params.last().copied(),
            None => None,
        }
    }

    /// The `T` calling it with `args` binds, `t` already bound by a
    /// method's receiver, or why `args` do not fit. Its result is
    /// [`ret`](Self::ret) with that `T`.
    pub(crate) fn call<V: View>(
        &self,
        args: &[V],
        mut t: Option<V>,
    ) -> Result<Option<V>, Unfit<V>> {
        let fits = match self.arity() {
            Arity::Exactly(n) => args.len() == n,
            Arity::AtLeast(n) => args.len() >= n,
        };

        if !fits {
            return Err(Unfit::Mismatch(Mismatch::Count(self.arity())));
        }

        for (n, arg) in args.iter().enumerate() {
            let Some(param) = self.param(n) else {
                return Err(Unfit::Mismatch(Mismatch::Count(self.arity())));
            };

            if !param.bind(*arg, &mut t) {
                return Err(Unfit::Argument(n, param, t));
            }
        }

        Ok(t)
    }

    /// Its parameters as an error lists them, such as `(Int, List<T>)`, or
    /// `(Int, ...)` for a repeated one.
    pub(crate) fn describe_params(&self) -> String {
        let mut params = self.params.iter().map(Shape::to_string).collect::<Vec<_>>();

        if self.repeats {
            params.push("...".to_owned());
        }

        format!("({})", params.join(", "))
    }

    /// It in full, called `name`: `Int::max(Int, ...) -> Int`.
    pub(crate) fn describe(&self, name: &str) -> String {
        let generic = if self.is_generic() { "<T>" } else { "" };
        format!("{name}{generic}{} -> {}", self.describe_params(), self.ret)
    }
}
