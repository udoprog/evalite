//! Signatures of the builtin functions and methods: the one place their
//! parameter and result types are written, generic over one type `T` where
//! they take any type. The checker resolves calls with them, completions
//! offer from them, and errors and docs print them.

use std::fmt;

use crate::Type;
use crate::error::Arity;
use crate::types::Kind;

/// A parameter's or result's type, which may hold the type variable `T`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Int,
    Bool,
    /// The type variable `T`, bound by the first argument that holds it.
    Var,
    List(&'static Shape),
    /// `Option<T>`.
    Option(&'static Shape),
}

impl Shape {
    /// Whether it holds `T`.
    pub(crate) fn is_generic(self) -> bool {
        match self {
            Self::Int | Self::Bool => false,
            Self::Var => true,
            Self::List(inner) | Self::Option(inner) => inner.is_generic(),
        }
    }

    /// Whether `ty` is of this shape, binding `T` in `t` the first time it
    /// is met, and requiring the bound type every time after.
    pub(crate) fn bind(self, ty: &Type, t: &mut Option<Type>) -> bool {
        match (self, ty) {
            (Self::Int, Type(Kind::Int)) | (Self::Bool, Type(Kind::Bool)) => true,
            (Self::Var, ty) => match t {
                Some(bound) => bound == ty,
                None => {
                    *t = Some(ty.clone());
                    true
                }
            },
            (Self::List(inner), Type(Kind::List(entry))) => inner.bind(entry, t),
            (Self::Option(inner), Type(Kind::Option(value))) => inner.bind(value, t),
            _ => false,
        }
    }

    /// The type it is with `T` as `t`, or `None` when it holds `T` and `t`
    /// is not known.
    pub(crate) fn resolve(self, t: Option<&Type>) -> Option<Type> {
        Some(match self {
            Self::Int => Type::int(),
            Self::Bool => Type::bool(),
            Self::Var => t?.clone(),
            Self::List(inner) => Type::list(inner.resolve(t)?),
            Self::Option(inner) => Type::option(inner.resolve(t)?),
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
            Self::Var => f.write_str("T"),
            Self::List(inner) => write!(f, "List<{inner}>"),
            Self::Option(inner) => write!(f, "Option<{inner}>"),
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
    /// The argument at this index, from 0, is not of the type described.
    Argument(usize, String),
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
    /// one more for a repeated one, as `min(_, _)`.
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

    /// The result of calling it with `args`, `t` already bound by a
    /// method's receiver, or why `args` do not fit.
    pub(crate) fn call(&self, args: &[Type], mut t: Option<Type>) -> Result<Type, Mismatch> {
        let fits = match self.arity() {
            Arity::Exactly(n) => args.len() == n,
            Arity::AtLeast(n) => args.len() >= n,
        };

        if !fits {
            return Err(Mismatch::Count(self.arity()));
        }

        for (n, arg) in args.iter().enumerate() {
            let Some(param) = self.param(n) else {
                return Err(Mismatch::Count(self.arity()));
            };

            if !param.bind(arg, &mut t) {
                return Err(Mismatch::Argument(n, param.describe(t.as_ref())));
            }
        }

        self.ret
            .resolve(t.as_ref())
            .ok_or_else(|| Mismatch::Count(self.arity()))
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

    /// It in full, called `name`: `lookup<T>(Int, List<T>) -> Option<T>`.
    pub(crate) fn describe(&self, name: &str) -> String {
        let generic = if self.is_generic() { "<T>" } else { "" };
        format!("{name}{generic}{} -> {}", self.describe_params(), self.ret)
    }
}
