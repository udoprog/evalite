#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

use crate::{DEFAULT_MAX_LEN, MAX_TYPE_DEPTH, Type, Value};

fn type_error(text: &str) -> (String, std::ops::Range<usize>) {
    let e = Type::parse(text).unwrap_err();
    (e.to_string(), e.span().range())
}

fn value_error(text: &str, ty: &str) -> (String, std::ops::Range<usize>) {
    let e = Value::parse(text, &Type::parse(ty).unwrap()).unwrap_err();
    (e.to_string(), e.span().range())
}

#[test]
fn types_round_trip() {
    let types = [
        Type::int(),
        Type::option(Type::option(Type::bool())),
        Type::list(Type::record([("a", Type::string())])),
        Type::record(Vec::<(String, Type)>::new()),
        Type::record([
            ("Brawl (bar)", Type::option(Type::int())),
            ("say \"hi\" \\ \n", Type::int()),
            ("_x1", Type::list(Type::list(Type::int()))),
        ]),
    ];

    for ty in types {
        assert_eq!(Type::parse(&ty.to_string()).unwrap(), ty, "{ty}");
    }

    assert_eq!(
        Type::parse("  List < Option<Int> >  ").unwrap(),
        Type::list(Type::option(Type::int()))
    );
}

#[test]
fn values_round_trip() {
    let ty = Type::parse(r#"{"a b": List<Option<String>>, n: Int, r: {}, t: Bool}"#).unwrap();
    let value = Value::record([
        (
            "a b",
            Value::list([Value::some(Value::string("x\"\\y")), Value::none()]),
        ),
        ("n", Value::int(i64::MIN)),
        ("r", Value::record(Vec::<(String, Value)>::new())),
        ("t", Value::bool(false)),
    ]);

    assert_eq!(Value::parse(&value.to_string(), &ty).unwrap(), value);
    assert_eq!(
        Value::parse("[]", &Type::list(Type::int())).unwrap(),
        Value::list([])
    );
    assert_eq!(
        Value::parse(&i64::MAX.to_string(), &Type::int()).unwrap(),
        Value::int(i64::MAX)
    );
}

#[test]
fn type_errors() {
    assert_eq!(
        type_error(""),
        ("expected a type, found the end".into(), 0..0)
    );
    assert_eq!(
        type_error("Int x"),
        ("expected the end, found `x`".into(), 4..5)
    );
    assert_eq!(
        type_error("List(Int)"),
        ("expected `<`, found `(`".into(), 4..5)
    );
    assert_eq!(
        type_error("{a Int}"),
        ("expected `:`, found `Int`".into(), 3..6)
    );
    assert_eq!(
        type_error("{a: Int b: Int}"),
        ("expected `}`, found `b`".into(), 8..9)
    );
    assert_eq!(
        type_error("{: Int}"),
        ("expected a field name, found `:`".into(), 1..2)
    );
    assert_eq!(
        type_error("{\"a: Int}"),
        (
            "this string is never closed, expected a `\"` before the end".into(),
            1..9
        )
    );
    assert_eq!(
        type_error(r#"{"\n": Int}"#),
        (
            "unknown escape `\\n` in a string, only `\\\"` and `\\\\` are allowed".into(),
            2..4
        )
    );
    assert_eq!(type_error("{a: Int, a: Int}").1, 9..10);
}

#[test]
fn type_depth_is_capped() {
    let at = |n: usize| format!("{}Int{}", "Option<".repeat(n - 1), ">".repeat(n - 1));

    assert!(Type::parse(&at(MAX_TYPE_DEPTH)).is_ok());
    assert_eq!(
        type_error(&at(MAX_TYPE_DEPTH + 1)).0,
        "this type nests more than 64 levels"
    );

    let records = format!(
        "{}Int{}",
        "{a: ".repeat(MAX_TYPE_DEPTH),
        "}".repeat(MAX_TYPE_DEPTH)
    );
    assert!(Type::parse(&records).is_err());

    // Deep input within the length limit fails at the depth cap, not the stack.
    let deep = "List<".repeat(DEFAULT_MAX_LEN / 5);
    assert!(Type::parse(&deep).is_err());
}

#[test]
fn length_is_capped() {
    let long = format!("[{}0]", "0, ".repeat(DEFAULT_MAX_LEN / 3));
    let e = Value::parse(&long, &Type::list(Type::int())).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "the text is {} bytes long, the limit is {DEFAULT_MAX_LEN}",
            long.len()
        )
    );
    assert!(Type::parse(&" ".repeat(DEFAULT_MAX_LEN + 1)).is_err());

    let fits = format!("[{}0]", "0,".repeat((DEFAULT_MAX_LEN - 3) / 2));
    assert!(Value::parse(&fits, &Type::list(Type::int())).is_ok());
}

#[test]
fn value_errors() {
    assert_eq!(
        value_error("- 1", "Int"),
        ("expected an Int, found `-`".into(), 0..1)
    );
    assert_eq!(
        value_error("12abc", "Int"),
        ("expected an Int, found `12abc`".into(), 0..5)
    );
    assert_eq!(
        value_error("-9223372036854775809", "Int"),
        (
            "this integer is too small, the smallest is -9223372036854775808".into(),
            0..20
        )
    );
    assert_eq!(
        value_error("yes", "Bool"),
        ("expected `true` or `false`, found `yes`".into(), 0..3)
    );
    assert_eq!(
        value_error("some(1", "Option<Int>"),
        ("expected `)`, found the end".into(), 6..6)
    );
    assert_eq!(
        value_error("maybe", "Option<Int>"),
        ("expected `none` or `some(..)`, found `maybe`".into(), 0..5)
    );
    assert_eq!(
        value_error("some(1)", "Option<Option<Int>>"),
        ("expected `none` or `some(..)`, found `1`".into(), 5..6)
    );
    assert_eq!(
        value_error("1", "List<Int>"),
        (
            "expected a List<Int> such as `[..]`, found `1`".into(),
            0..1
        )
    );
    assert_eq!(
        value_error("[1 2]", "List<Int>"),
        ("expected `]`, found `2`".into(), 3..4)
    );
    assert_eq!(
        value_error("[1,]", "List<Int>"),
        ("expected an Int, found `]`".into(), 3..4)
    );
    assert_eq!(
        value_error("7", "{a: Int}"),
        ("expected a {a: Int} such as `{..}`, found `7`".into(), 0..1)
    );
    assert_eq!(
        value_error("{b: 1}", "{a: Int}"),
        ("{a: Int} has no field `b`".into(), 1..2)
    );
    assert_eq!(
        value_error("{a: 1, a: 2}", "{a: Int}"),
        ("the field `a` is given twice".into(), 7..8)
    );
    assert_eq!(
        value_error("{}", "{a: Int}"),
        ("the field `a` has no value".into(), 1..2)
    );
    assert_eq!(
        value_error("\"ab", "String"),
        (
            "this string is never closed, expected a `\"` before the end".into(),
            0..3
        )
    );
    assert_eq!(
        value_error("", "String"),
        (
            "expected a string in quotes, such as `\"Brawl\"`, found the end".into(),
            0..0
        )
    );
}

#[test]
fn quotes_at_most_some_of_the_input() {
    let word = "x".repeat(1000);
    let (message, span) = value_error(&word, "Int");
    assert_eq!(
        message,
        format!("expected an Int, found `{}`", "x".repeat(32))
    );
    assert_eq!(span, 0..1000);
}
