//! The methods each type has: the one table the checker resolves method
//! calls with and completions offer methods from.

use crate::Type;
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
    /// Its instruction, given the receiver's type.
    pub(crate) instr: fn(TypeId) -> Instr,
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
    instr: fn(TypeId) -> Instr,
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

/// Every method, by receiver, in name order.
pub(crate) const METHODS: &[MethodSig] = &[
    method(Shape::Int, "abs", Signature::new(&[], Shape::Int), |_| {
        Instr::Abs
    }),
    method(
        Shape::Int,
        "clamp",
        Signature::new(&[Shape::Int, Shape::Int], Shape::Int),
        |_| Instr::Clamp,
    ),
    method(
        Shape::Int,
        "max",
        Signature::new(&[Shape::Int], Shape::Int),
        |_| Instr::Max(2),
    ),
    method(
        Shape::Int,
        "min",
        Signature::new(&[Shape::Int], Shape::Int),
        |_| Instr::Min(2),
    ),
    method(
        OPTION,
        "is_none",
        Signature::new(&[], Shape::Bool),
        Instr::IsNone,
    ),
    method(
        OPTION,
        "is_some",
        Signature::new(&[], Shape::Bool),
        Instr::IsSome,
    ),
    method(
        OPTION,
        "unwrap_or",
        Signature::new(&[T], T),
        Instr::UnwrapOr,
    ),
    method(LIST, "len", Signature::new(&[], Shape::Int), |_| Instr::Len),
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
