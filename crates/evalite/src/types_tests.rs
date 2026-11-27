//! Tests of the type table: interning finds each type once, however it is
//! reached.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::types::{FieldNode, TypeNode, Types};
use crate::*;

fn record(a: Type) -> Type {
    Type::record([("a", a), ("b", Type::list(Type::option(Type::int())))])
}

#[test]
fn intern_finds_what_it_added() {
    let mut types = Types::new();

    let first = types.intern(&record(Type::int())).unwrap();
    let len = types.nodes().len();

    assert_eq!(types.intern(&record(Type::int())), Some(first));
    assert_eq!(types.nodes().len(), len);

    let other = types.intern(&record(Type::bool())).unwrap();
    assert_ne!(first, other);
    assert_eq!(types.intern(&record(Type::bool())), Some(other));

    let renamed = Type::record([
        ("a", Type::int()),
        ("c", Type::list(Type::option(Type::int()))),
    ]);
    let renamed = types.intern(&renamed).unwrap();
    assert_ne!(renamed, first);

    assert_eq!(types.to_type(first), Some(record(Type::int())));
    assert_eq!(types.to_type(other), Some(record(Type::bool())));
}

#[test]
fn add_and_intern_agree() {
    let mut types = Types::new();
    let int = types.add(TypeNode::Int).unwrap();
    let node = TypeNode::Record(vec![FieldNode {
        name: "a".to_owned(),
        ty: int,
    }]);

    let added = types.add(node.clone()).unwrap();
    assert_eq!(types.find(&node), Some(added));
    assert_eq!(
        types.intern(&Type::record([("a", Type::int())])),
        Some(added)
    );
    assert_eq!(types.add(node), Some(added));
}

#[test]
fn intern_refuses_too_deep() {
    let mut ty = Type::int();

    for _ in 0..crate::types::MAX_TYPE_DEPTH {
        ty = Type::list(ty);
    }

    let mut types = Types::new();
    assert_eq!(types.intern(&ty), None);
}
