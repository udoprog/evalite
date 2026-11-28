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

#[test]
fn matches_compares_without_building() {
    let samples = [
        Type::int(),
        Type::bool(),
        Type::string(),
        Type::list(Type::int()),
        Type::option(Type::int()),
        Type::option(Type::option(Type::bool())),
        Type::map(Type::string(), Type::int()),
        Type::map(Type::int(), Type::string()),
        record(Type::int()),
        record(Type::bool()),
        Type::record([("a", Type::int())]),
        Type::record([("b", Type::int())]),
        Type::record([("a", Type::int()), ("c", Type::int())]),
        Type::list(record(Type::option(Type::string()))),
    ];

    let mut types = Types::new();
    let ids: Vec<TypeId> = samples.iter().map(|t| types.intern(t).unwrap()).collect();

    let mut stack = Vec::new();

    for (id, a) in ids.iter().zip(&samples) {
        for b in &samples {
            assert_eq!(types.matches(*id, b, &mut stack), a == b, "{a} against {b}");
        }
    }

    let past = TypeId(u32::try_from(types.nodes().len()).unwrap());
    assert!(!types.matches(past, &samples[0], &mut stack));
    assert!(stack.is_empty());
}

#[test]
fn copy_from_copies_and_caches_shared_types() {
    let samples = [
        Type::int(),
        Type::bool(),
        Type::string(),
        Type::option(Type::int()),
        Type::list(Type::option(Type::int())),
        Type::map(Type::string(), Type::list(Type::int())),
        Type::record([
            ("a", Type::option(Type::int())),
            ("b", Type::option(Type::int())),
            ("c", Type::list(Type::int())),
        ]),
    ];

    let mut src = Types::new();
    let src_ids: Vec<TypeId> = samples.iter().map(|t| src.intern(t).unwrap()).collect();

    let mut dst = Types::new();
    // Seed a different ordering in dst so ids do not trivially match src.
    dst.intern(&Type::bool()).unwrap();
    dst.intern(&Type::int()).unwrap();

    let mut map = Vec::new();

    for (src_id, ty) in src_ids.iter().zip(&samples) {
        let copied = dst.copy_from(&src, *src_id, &mut map).unwrap();
        assert_eq!(dst.to_type(copied).as_ref(), Some(ty));
        // Second call hits the cache and returns the same id.
        assert_eq!(dst.copy_from(&src, *src_id, &mut map), Some(copied));
    }

    let past = TypeId(u32::try_from(src.nodes().len()).unwrap());
    assert_eq!(dst.copy_from(&src, past, &mut map), None);
}

#[test]
fn fx_hasher_mixes_high_bits_into_low_buckets() {
    use crate::types::FxBuildHasher;
    use std::collections::HashSet;
    use std::hash::BuildHasher;

    for seed in [0u64, 0x1234_5678_9abc_def0, u64::MAX] {
        let bh = FxBuildHasher::new(seed);

        let u32_buckets: HashSet<u64> =
            (0u32..1024).map(|i| bh.hash_one(i << 20) & 0x3ff).collect();
        assert!(
            u32_buckets.len() > 500,
            "u32 high-bit keys collapsed to {} buckets for seed {seed:#x}",
            u32_buckets.len()
        );

        let u64_buckets: HashSet<u64> =
            (0u64..1024).map(|i| bh.hash_one(i << 52) & 0x3ff).collect();
        assert!(
            u64_buckets.len() > 500,
            "u64 high-bit keys collapsed to {} buckets for seed {seed:#x}",
            u64_buckets.len()
        );
    }
}
