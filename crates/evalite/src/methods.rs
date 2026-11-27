//! The methods and functions each type has: the one table the checker
//! resolves method calls and paths such as `Int::min(a, b)` with, and
//! completions offer them from.

use crate::Type;
use crate::error::Arity;
use crate::program::Instr;
use crate::sig::{Mismatch, Shape, Signature};
use crate::types::TypeId;

const T: Shape = Shape::Var;

#[derive(Debug)]
pub(crate) struct MethodSig {
    /// The type it is a method of, which binds `T`.
    pub(crate) receiver: Shape,
    pub(crate) name: &'static str,
    pub(crate) sig: Signature,
    /// Its instruction, given the receiver's type and the type of what
    /// it holds, if it is a list or an option; `None` when that is
    /// missing.
    pub(crate) instr: fn(TypeId, Option<TypeId>) -> Option<Instr>,
}

impl MethodSig {
    /// The `T` a `receiver` binds, if it has this method.
    fn bind(&self, receiver: &Type) -> Option<Option<Type>> {
        let mut t = None;
        self.receiver.bind(receiver, &mut t).then_some(t)
    }

    /// Its parameters' types and its result's, called on a `receiver` it
    /// is a method of.
    pub(crate) fn signature(&self, receiver: &Type) -> (Vec<Type>, Type) {
        let t = self.bind(receiver).flatten();
        let resolve = |shape: Shape| shape.resolve(t.as_ref()).unwrap_or_else(Type::int);
        let params = self.sig.params.iter().map(|p| resolve(*p)).collect();
        (params, resolve(self.sig.ret))
    }

    /// What calling it on `receiver` with `args` gives, or why it does
    /// not take them.
    pub(crate) fn call(&self, receiver: &Type, args: &[Type]) -> Result<Type, Mismatch> {
        self.sig.call(args, self.bind(receiver).flatten())
    }

    /// What calling it by path with `args`, the receiver first, gives, or
    /// why it does not take them.
    fn call_by_path(&self, args: &[Type]) -> Result<Type, Mismatch> {
        let count = || Mismatch::Count(Arity::Exactly(self.sig.params.len().saturating_add(1)));

        let Some((receiver, rest)) = args.split_first() else {
            return Err(count());
        };

        if rest.len() != self.sig.params.len() {
            return Err(count());
        }

        let Some(t) = self.bind(receiver) else {
            // A map's methods exist once per key type, so any key type will do.
            let expected = match self.receiver {
                Shape::Map(..) => String::from("Map<K, V>"),
                receiver => receiver.to_string(),
            };
            return Err(Mismatch::Argument(0, expected));
        };

        self.sig.call(rest, t).map_err(|mismatch| match mismatch {
            Mismatch::Count(_) | Mismatch::Takes(_) => count(),
            Mismatch::Argument(n, ty) => Mismatch::Argument(n.saturating_add(1), ty),
        })
    }

    /// It in full: `Option<T>.unwrap_or(T) -> T`.
    pub(crate) fn describe(&self) -> String {
        format!(
            "{}.{}{} -> {}",
            self.receiver,
            self.name,
            self.sig.describe_params(),
            self.sig.ret
        )
    }
}

const fn method(
    receiver: Shape,
    name: &'static str,
    sig: Signature,
    instr: fn(TypeId, Option<TypeId>) -> Option<Instr>,
) -> MethodSig {
    MethodSig {
        receiver,
        name,
        sig,
        instr,
    }
}

const OPTION: Shape = Shape::Option(&T);
const LIST: Shape = Shape::List(&T);
const INT_MAP: Shape = Shape::Map(&Shape::Int, &T);
const STRING_MAP: Shape = Shape::Map(&Shape::String, &T);

/// Every method, by receiver, in name order.
pub(crate) const METHODS: &[MethodSig] = &[
    method(
        Shape::Int,
        "abs",
        Signature::new(&[], Shape::Int),
        |_, _| Some(Instr::Abs),
    ),
    method(
        Shape::Int,
        "clamp",
        Signature::new(&[Shape::Int, Shape::Int], Shape::Int),
        |_, _| Some(Instr::Clamp),
    ),
    method(
        Shape::Int,
        "max",
        Signature::new(&[Shape::Int], Shape::Int),
        |_, _| Some(Instr::Max(2)),
    ),
    method(
        Shape::Int,
        "min",
        Signature::new(&[Shape::Int], Shape::Int),
        |_, _| Some(Instr::Min(2)),
    ),
    method(
        OPTION,
        "is_none",
        Signature::new(&[], Shape::Bool),
        |option, _| Some(Instr::IsNone(option)),
    ),
    method(
        OPTION,
        "is_some",
        Signature::new(&[], Shape::Bool),
        |option, _| Some(Instr::IsSome(option)),
    ),
    method(OPTION, "unwrap_or", Signature::new(&[T], T), |option, _| {
        Some(Instr::UnwrapOr(option))
    }),
    method(
        LIST,
        "get",
        Signature::new(&[Shape::Int], Shape::Option(&T)),
        |_, entry| entry.map(Instr::Get),
    ),
    method(LIST, "len", Signature::new(&[], Shape::Int), |_, _| {
        Some(Instr::Len)
    }),
    // A map's methods, once per key type, as `T` is its value type.
    method(
        INT_MAP,
        "get",
        Signature::new(&[Shape::Int], Shape::Option(&T)),
        |map, _| Some(Instr::MapGet(map)),
    ),
    method(INT_MAP, "len", Signature::new(&[], Shape::Int), |_, _| {
        Some(Instr::Len)
    }),
    method(
        STRING_MAP,
        "get",
        Signature::new(&[Shape::String], Shape::Option(&T)),
        |map, _| Some(Instr::MapGet(map)),
    ),
    method(
        STRING_MAP,
        "len",
        Signature::new(&[], Shape::Int),
        |_, _| Some(Instr::Len),
    ),
];

/// The methods a value of type `ty` has, in name order.
pub(crate) fn methods_of(ty: &Type) -> impl Iterator<Item = &'static MethodSig> {
    METHODS.iter().filter(move |m| m.bind(ty).is_some())
}

/// The method `name` of a value of type `ty`.
pub(crate) fn find(ty: &Type, name: &str) -> Option<&'static MethodSig> {
    methods_of(ty).find(|m| m.name == name)
}

/// The names of the methods a value of type `ty` has, in name order.
pub(crate) fn method_names(ty: &Type) -> Vec<&'static str> {
    methods_of(ty).map(|m| m.name).collect()
}

/// A function of a type, called by its path, such as `Int::min(a, b)`.
#[derive(Debug)]
pub(crate) struct FunctionSig {
    pub(crate) ty: &'static str,
    pub(crate) name: &'static str,
    pub(crate) sig: Signature,
    /// Its instruction, given how many arguments it is called with.
    pub(crate) instr: fn(u32) -> Instr,
}

impl FunctionSig {
    /// It in full: `Int::min(Int, ...) -> Int`.
    pub(crate) fn describe(&self) -> String {
        self.sig.describe(&self.path())
    }

    /// Its path: `Int::min`.
    pub(crate) fn path(&self) -> String {
        format!("{}::{}", self.ty, self.name)
    }
}

/// Every function of a type, by type, in name order. A type's methods are
/// its functions too, taking the receiver first, unless one here has the
/// same name.
pub(crate) const FUNCTIONS: &[FunctionSig] = &[
    FunctionSig {
        ty: "Int",
        name: "max",
        sig: Signature::repeating(&[Shape::Int], Shape::Int),
        instr: Instr::Max,
    },
    FunctionSig {
        ty: "Int",
        name: "min",
        sig: Signature::repeating(&[Shape::Int], Shape::Int),
        instr: Instr::Min,
    },
];

/// What a path such as `Int::min` calls.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Associated {
    Function(&'static FunctionSig),
    /// A method, called with the receiver as the first argument.
    Method(&'static MethodSig),
}

impl Associated {
    /// The method of the same type and name as this one whose receiver is
    /// `receiver`, as a map's methods are once per key type; itself when
    /// none is, or it is a function.
    pub(crate) fn for_receiver(self, receiver: Option<&Type>) -> Self {
        let (Self::Method(method), Some(receiver)) = (self, receiver) else {
            return self;
        };

        METHODS
            .iter()
            .find(|m| {
                m.name == method.name
                    && type_name(m.receiver) == type_name(method.receiver)
                    && m.bind(receiver).is_some()
            })
            .map_or(self, Self::Method)
    }

    /// What calling it with `args` gives, or why it does not take them.
    pub(crate) fn call(self, args: &[Type]) -> Result<Type, Mismatch> {
        match self {
            Self::Function(function) => function.sig.call(args, None),
            Self::Method(method) => method.call_by_path(args),
        }
    }
}

/// The name a path calls a method receiving `shape` by: `Int` for
/// `Int::abs`, `Option` for `Option::unwrap_or`.
fn type_name(shape: Shape) -> Option<&'static str> {
    match shape {
        Shape::Int => Some("Int"),
        Shape::Bool => Some("Bool"),
        Shape::List(_) => Some("List"),
        Shape::Option(_) => Some("Option"),
        Shape::Map(..) => Some("Map"),
        Shape::String | Shape::Var => None,
    }
}

/// Why a path calls nothing.
pub(crate) enum PathError {
    /// No type of that name has functions.
    UnknownType,
    /// The type has no function of that name, only these.
    Unknown(&'static str, Vec<&'static str>),
}

/// What the path `ty::name` calls.
pub(crate) fn find_path(ty: &str, name: &str) -> Result<Associated, PathError> {
    if let Some(function) = FUNCTIONS.iter().find(|f| f.ty == ty && f.name == name) {
        return Ok(Associated::Function(function));
    }

    let methods = METHODS.iter().filter(|m| type_name(m.receiver) == Some(ty));

    if let Some(method) = methods.clone().find(|m| m.name == name) {
        return Ok(Associated::Method(method));
    }

    let functions = FUNCTIONS.iter().filter(|f| f.ty == ty);

    let Some(ty) = functions
        .clone()
        .map(|f| f.ty)
        .chain(methods.clone().filter_map(|m| type_name(m.receiver)))
        .next()
    else {
        return Err(PathError::UnknownType);
    };

    let mut names = functions
        .map(|f| f.name)
        .chain(methods.map(|m| m.name))
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    Err(PathError::Unknown(ty, names))
}
