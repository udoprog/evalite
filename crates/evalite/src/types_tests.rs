//! Tests of the type table: interning finds each type once, however it is
//! reached.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::types::{FieldNode, TypeId, TypeNode, Types};
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

/// A map interns as its key and value, each once, and builds back.
#[test]
fn maps_intern() {
    let mut types = Types::new();
    let ty = Type::map(Type::int(), Type::map(Type::string(), Type::int()));
    let id = types.intern(&ty).unwrap();

    assert_eq!(types.intern(&ty), Some(id));
    assert_eq!(types.to_type(id), Some(ty));
    assert_eq!(types.size(id), 5);

    let other = types
        .intern(&Type::map(Type::string(), Type::int()))
        .unwrap();
    assert!(other < id);
    assert_eq!(
        types.get(id),
        Some(&TypeNode::Map {
            key: types.find(&TypeNode::Int).unwrap(),
            value: other,
        })
    );

    // The key and value are not interchangeable.
    let swapped = types
        .intern(&Type::map(Type::string(), Type::string()))
        .unwrap();
    assert_ne!(swapped, other);
}

/// A stored map must be keyed by `Int` or `String`.
#[test]
fn stored_maps_need_scalar_keys() {
    let int = TypeId(0);
    let bool = TypeId(1);

    let ok = vec![
        TypeNode::Int,
        TypeNode::Bool,
        TypeNode::Map {
            key: int,
            value: bool,
        },
    ];
    assert!(Types::from_stored(ok).is_ok());

    for key in [bool, TypeId(2)] {
        let nodes = vec![
            TypeNode::Int,
            TypeNode::Bool,
            TypeNode::List(int),
            TypeNode::Map { key, value: int },
        ];
        let error = Types::from_stored(nodes).unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "the type is a map keyed by other than Int or String: at type 3"
        );
    }

    let later = vec![
        TypeNode::Int,
        TypeNode::Map {
            key: int,
            value: TypeId(1),
        },
    ];
    assert_eq!(
        format!("{:#}", Types::from_stored(later).unwrap_err()),
        "the type refers to a later type: at type 1"
    );
}
