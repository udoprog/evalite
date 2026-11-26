//! The methods each type has: the one table the checker resolves method
//! calls with and completions offer methods from.

use crate::Type;
use crate::program::Instr;
use crate::types::TypeId;

/// The kinds of value that have methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Receiver {
    Int,
    /// `Option<T>`.
    Option,
    /// `List<T>`.
    List,
}

impl Receiver {
    /// The kind of `ty`, with the `T` of an option or list.
    pub(crate) fn of(ty: &Type) -> Option<(Self, Option<&Type>)> {
        match ty {
            Type::Int => Some((Self::Int, None)),
            Type::Option(t) => Some((Self::Option, Some(t))),
            Type::List(t) => Some((Self::List, Some(t))),
            _ => None,
        }
    }
}

/// A parameter's or result's type, in terms of the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Int,
    Bool,
    /// The receiver's `T`.
    Inner,
}

impl Slot {
    /// The type this is, `inner` being the receiver's `T`.
    pub(crate) fn resolve<T: Clone>(self, int: &T, bool: &T, inner: &T) -> T {
        match self {
            Self::Int => int.clone(),
            Self::Bool => bool.clone(),
            Self::Inner => inner.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct MethodSig {
    pub(crate) receiver: Receiver,
    pub(crate) name: &'static str,
    pub(crate) params: &'static [Slot],
    pub(crate) ret: Slot,
    /// Its instruction, given the receiver's type.
    pub(crate) instr: fn(TypeId) -> Instr,
}

impl MethodSig {
    /// Its parameters' types and its result's, called on a `receiver`.
    pub(crate) fn signature(&self, receiver: &Type) -> (Vec<Type>, Type) {
        let inner = match receiver {
            Type::Option(t) | Type::List(t) => (**t).clone(),
            other => other.clone(),
        };

        let resolve = |slot: Slot| slot.resolve(&Type::Int, &Type::Bool, &inner);
        let params = self.params.iter().map(|p| resolve(*p)).collect();
        (params, resolve(self.ret))
    }
}

const fn method(
    receiver: Receiver,
    name: &'static str,
    params: &'static [Slot],
    ret: Slot,
    instr: fn(TypeId) -> Instr,
) -> MethodSig {
    MethodSig {
        receiver,
        name,
        params,
        ret,
        instr,
    }
}

/// Every method, by receiver, in name order.
pub(crate) const METHODS: &[MethodSig] = &[
    method(Receiver::Int, "abs", &[], Slot::Int, |_| Instr::Abs),
    method(
        Receiver::Int,
        "clamp",
        &[Slot::Int, Slot::Int],
        Slot::Int,
        |_| Instr::Clamp,
    ),
    method(Receiver::Int, "max", &[Slot::Int], Slot::Int, |_| {
        Instr::Max(2)
    }),
    method(Receiver::Int, "min", &[Slot::Int], Slot::Int, |_| {
        Instr::Min(2)
    }),
    method(Receiver::Option, "is_none", &[], Slot::Bool, Instr::IsNone),
    method(Receiver::Option, "is_some", &[], Slot::Bool, Instr::IsSome),
    method(
        Receiver::Option,
        "unwrap_or",
        &[Slot::Inner],
        Slot::Inner,
        Instr::UnwrapOr,
    ),
    method(Receiver::List, "len", &[], Slot::Int, |_| Instr::Len),
];

/// The methods of a `receiver`, in name order.
pub(crate) fn methods(receiver: Receiver) -> impl Iterator<Item = &'static MethodSig> {
    METHODS.iter().filter(move |m| m.receiver == receiver)
}

/// The method `name` of a `receiver`.
pub(crate) fn find(receiver: Receiver, name: &str) -> Option<&'static MethodSig> {
    methods(receiver).find(|m| m.name == name)
}

/// The methods a value of type `ty` has, in name order.
pub(crate) fn method_names(ty: &Type) -> Vec<&'static str> {
    match Receiver::of(ty) {
        Some((receiver, _)) => methods(receiver).map(|m| m.name).collect(),
        None => Vec::new(),
    }
}
