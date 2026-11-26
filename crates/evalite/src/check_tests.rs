//! Tests of the type checker, compiler and stored programs.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::string_slice,
    clippy::indexing_slicing
)]

use std::ops::Range;

use crate::error::ErrorKind;
use crate::program::{Constant, FORMAT, Global, HostFunction, Instr, Scalar, Stored};
use crate::tests::Rng;
use crate::types::{FieldNode, TypeId, TypeNode};
use crate::*;

/// A track as the sheet's environment declares one: a record of its
/// boxes and how many are at each state or worse.
pub(crate) fn track() -> Type {
    Type::record([("boxes", Type::Int), ("count", Type::list(Type::Int))])
}

/// The environment most tests compile against.
pub(crate) fn env() -> Environment {
    let mut env = Environment::with_builtins();

    for name in [
        "wits",
        "dexterity",
        "athletics",
        "strength",
        "blood_potency",
        "primal_urge",
        "min",
    ] {
        env.value(name, Type::Int).unwrap();
    }

    env.value("flag", Type::Bool).unwrap();
    env.value("bonus", Type::option(Type::Int)).unwrap();
    env.value("maybe", Type::option(Type::Bool)).unwrap();
    env.value("levels", Type::list(Type::Int)).unwrap();
    env.value("grid", Type::list(Type::list(Type::Int)))
        .unwrap();
    env.value("name", Type::String).unwrap();
    env.value("health", track()).unwrap();
    env.value("tracks", Type::list(track())).unwrap();
    env.value(
        "specialties",
        Type::record([("athletics", Type::Int), ("brawl", Type::option(Type::Int))]),
    )
    .unwrap();
    env.value(
        "m",
        Type::record([
            ("Brawl (bar)", Type::Int),
            ("a b", track()),
            ("inner", Type::record([("deep", Type::Bool)])),
        ]),
    )
    .unwrap();
    env.value("maybe_track", Type::option(track())).unwrap();

    env.function("count", [track()], Type::Int).unwrap();
    env.function("count", [track(), Type::Int], Type::Int)
        .unwrap();
    env.function("boxes", [track()], Type::Int).unwrap();
    env.function("lookup", [Type::Int, Type::String], Type::Int)
        .unwrap();
    env.function("label", [Type::Int], Type::option(Type::Int))
        .unwrap();
    env.function("pick", [], Type::Bool).unwrap();
    env
}

fn type_of(source: &str) -> String {
    match compile(source, &env()) {
        Ok(program) => program.result_type().to_string(),
        Err(error) => panic!("{source:?}: {error}"),
    }
}

fn error(source: &str) -> (String, Range<usize>) {
    error_in(source, &env())
}

fn error_in(source: &str, env: &Environment) -> (String, Range<usize>) {
    match compile(source, env) {
        Ok(program) => panic!("{source:?} compiled to {}", program.result_type()),
        Err(error) => {
            assert!(
                !matches!(error.kind(), ErrorKind::Internal(..)),
                "{source:?}: {error}"
            );
            (error.to_string(), error.span().range())
        }
    }
}

/// Each source with its type.
fn types(cases: &[(&str, &str)]) {
    for (source, expected) in cases {
        assert_eq!(type_of(source), *expected, "{source:?}");
    }
}

/// Each source with its error and the bytes it points at.
fn errors(cases: &[(&str, &str, &str)]) {
    for (source, message, at) in cases {
        let (actual, range) = error(source);
        assert_eq!(actual, *message, "{source:?}");
        assert_eq!(&source[range.clone()], *at, "{source:?}: {range:?}");
    }
}

#[test]
fn literals_and_names() {
    types(&[
        ("12", "Int"),
        ("-12", "Int"),
        ("true", "Bool"),
        ("false", "Bool"),
        (r#""Essence (1e)""#, "String"),
        ("wits", "Int"),
        ("flag", "Bool"),
        ("bonus", "Option<Int>"),
        ("levels", "List<Int>"),
        ("grid", "List<List<Int>>"),
        ("health", "{boxes: Int, count: List<Int>}"),
        ("specialties", "{athletics: Int, brawl: Option<Int>}"),
        (
            "m",
            r#"{"Brawl (bar)": Int, "a b": {boxes: Int, count: List<Int>}, inner: {deep: Bool}}"#,
        ),
        // Values and functions are named apart: `min` is a value here.
        ("min + 1", "Int"),
        ("(wits)", "Int"),
        ("((flag))", "Bool"),
    ]);
}

#[test]
fn operators() {
    types(&[
        ("wits + 1", "Int"),
        ("wits - 1", "Int"),
        ("wits * 2", "Int"),
        ("wits / 2", "Int"),
        ("wits % 2", "Int"),
        ("-wits", "Int"),
        ("--wits", "Int"),
        ("!flag", "Bool"),
        ("!!flag", "Bool"),
        ("wits < 1", "Bool"),
        ("wits <= 1", "Bool"),
        ("wits > 1", "Bool"),
        ("wits >= 1", "Bool"),
        ("wits == 1", "Bool"),
        ("wits != 1", "Bool"),
        ("flag == true", "Bool"),
        ("flag != false", "Bool"),
        ("flag && true", "Bool"),
        ("flag || false", "Bool"),
        ("wits > 1 && flag || !flag", "Bool"),
        ("(wits > 1) == flag", "Bool"),
    ]);
}

#[test]
fn every_method() {
    types(&[
        ("wits.abs()", "Int"),
        ("wits.min(1)", "Int"),
        ("wits.max(1)", "Int"),
        ("wits.clamp(0, 5)", "Int"),
        ("(-3).abs().max(1).min(9).clamp(0, 5)", "Int"),
        ("bonus.unwrap_or(0)", "Int"),
        ("bonus.is_some()", "Bool"),
        ("bonus.is_none()", "Bool"),
        ("maybe.unwrap_or(false)", "Bool"),
        ("maybe.is_some()", "Bool"),
        (
            "maybe_track.unwrap_or(health)",
            "{boxes: Int, count: List<Int>}",
        ),
        ("levels.len()", "Int"),
        ("grid.len()", "Int"),
        ("tracks.len()", "Int"),
        ("[1, 2].len()", "Int"),
        ("specialties.brawl.unwrap_or(0).max(1)", "Int"),
        ("label(1).unwrap_or(wits)", "Int"),
    ]);
}

#[test]
fn functions() {
    types(&[
        ("min(wits)", "Int"),
        ("min(wits, dexterity)", "Int"),
        ("max(dexterity, wits, 7)", "Int"),
        ("lookup(wits, [10, 11, 12])", "Int"),
        ("lookup(wits, levels)", "Int"),
        ("lookup(wits, grid)", "List<Int>"),
        ("lookup(1, [true, false])", "Bool"),
        ("lookup(1, tracks)", "{boxes: Int, count: List<Int>}"),
        (r#"lookup(primal_urge, "Essence (1e)")"#, "Int"),
        ("count(health)", "Int"),
        ("count(health, 2)", "Int"),
        ("boxes(health)", "Int"),
        (r#"count(m["a b"])"#, "Int"),
        ("count(tracks[0])", "Int"),
        ("label(wits)", "Option<Int>"),
        ("pick()", "Bool"),
    ]);
}

#[test]
fn fields_indices_and_lists() {
    types(&[
        ("specialties.athletics", "Int"),
        ("specialties.brawl", "Option<Int>"),
        (r#"specialties["athletics"]"#, "Int"),
        (r#"m["Brawl (bar)"]"#, "Int"),
        (r#"m["a b"]"#, "{boxes: Int, count: List<Int>}"),
        ("m.inner.deep", "Bool"),
        (r#"m["inner"].deep"#, "Bool"),
        ("levels[2]", "Int"),
        ("levels[wits + 1]", "Int"),
        ("grid[0]", "List<Int>"),
        ("grid[0][1]", "Int"),
        ("[1, 2, 3]", "List<Int>"),
        ("[wits, 2]", "List<Int>"),
        ("[true]", "List<Bool>"),
        (r#"["a", "b"]"#, "List<String>"),
        ("[[1], [2, 3]]", "List<List<Int>>"),
        ("[levels, [1]]", "List<List<Int>>"),
        ("[health]", "List<{boxes: Int, count: List<Int>}>"),
        ("[bonus, label(1)]", "List<Option<Int>>"),
        ("[1, 2][0]", "Int"),
        (
            "[specialties]",
            "List<{athletics: Int, brawl: Option<Int>}>",
        ),
    ]);
}

/// Every form `crates/api/src/expr.rs` accepts type checks, with a track
/// declared as a record and its boxes read with the functions `count` and
/// `boxes` the sheet declares today.
#[test]
fn current_expressions() {
    types(&[
        ("12", "Int"),
        ("dexterity + wits - 1", "Int"),
        ("-dexterity + 1", "Int"),
        ("min(wits, dexterity) + athletics", "Int"),
        ("max (wits)", "Int"),
        ("lookup(blood_potency, [10, 11, 12 , 13])", "Int"),
        ("lookup(blood_potency + 5, [10, 11, 12, 13])", "Int"),
        ("lookup(-wits, [10, 11])", "Int"),
        ("lookup(0, [wits + 1])", "Int"),
        (r#"lookup(primal_urge, "Essence (1e)")"#, "Int"),
        (r#"lookup(blood_potency, "Vitae \"2e\"")"#, "Int"),
        (
            r#"specialties.athletics + levels[2] + m["Brawl (bar)"]"#,
            "Int",
        ),
        ("count(health) + count( health , 1 + 1)", "Int"),
        (r#"boxes(health) - boxes(m["a b"])"#, "Int"),
        ("count(health, -4)", "Int"),
        // The same, read from the track's record.
        ("health.boxes", "Int"),
        ("health.count[2]", "Int"),
        ("lookup(-4, health.count)", "Int"),
        (r#"health.boxes - m["a b"].boxes"#, "Int"),
        ("9223372036854775807 + 1", "Int"),
    ]);
}

#[test]
fn unwrap_hint() {
    const HINT: &str = ", unwrap the optional value first with `.unwrap_or(...)`";

    errors(&[
        (
            "bonus + 1",
            &format!("`+` takes Int and Int, found Option<Int> and Int{HINT}"),
            "bonus + 1",
        ),
        (
            "-bonus",
            &format!("`-` takes an Int, found Option<Int>{HINT}"),
            "-bonus",
        ),
        (
            "!maybe",
            &format!("`!` takes a Bool, found Option<Bool>{HINT}"),
            "!maybe",
        ),
        (
            "min(bonus)",
            &format!("argument 1 of `min` must be Int, found Option<Int>{HINT}"),
            "bonus",
        ),
        (
            "bonus.max(1)",
            &format!("Option<Int> has no method `max`{HINT}"),
            "max",
        ),
        (
            "specialties.brawl.abs()",
            &format!("Option<Int> has no method `abs`{HINT}"),
            "abs",
        ),
        (
            "bonus.value",
            &format!("Option<Int> has no fields, so no `.value`{HINT}"),
            "value",
        ),
        (
            "bonus[0]",
            &format!("Option<Int> cannot be indexed, only a List or a record can{HINT}"),
            "bonus",
        ),
    ]);
}

#[test]
fn error_names() {
    errors(&[
        ("x", "unknown name `x`", "x"),
        ("Wits", "unknown name `Wits`, did you mean `wits`?", "Wits"),
        (
            "WITS + 1",
            "unknown name `WITS`, did you mean `wits`?",
            "WITS",
        ),
        ("wit", "unknown name `wit`, did you mean `wits`?", "wit"),
        (
            "witts",
            "unknown name `witts`, did you mean `wits`?",
            "witts",
        ),
        (
            "dexterty + 1",
            "unknown name `dexterty`, did you mean `dexterity`?",
            "dexterty",
        ),
        ("zzzzzz", "unknown name `zzzzzz`", "zzzzzz"),
        (
            "count + 1",
            "`count` is a function, call it with `count(...)`",
            "count",
        ),
        (
            "1 + max",
            "`max` is a function, call it with `max(...)`",
            "max",
        ),
        ("f(1)", "unknown function `f`", "f"),
        (
            "cout(health)",
            "unknown function `cout`, did you mean `count`?",
            "cout",
        ),
        (
            "Min(1)",
            "unknown function `Min`, did you mean `min`?",
            "Min",
        ),
        ("wits(1)", "`wits` is a value, not a function", "wits"),
        // Arguments are not checked before the function is found.
        ("f(nothing)", "unknown function `f`", "f"),
    ]);
}

#[test]
fn error_operators() {
    errors(&[
        (
            "1 + true",
            "`+` takes Int and Int, found Int and Bool",
            "1 + true",
        ),
        (
            "true - 1",
            "`-` takes Int and Int, found Bool and Int",
            "true - 1",
        ),
        (
            "flag * flag",
            "`*` takes Int and Int, found Bool and Bool",
            "flag * flag",
        ),
        (
            "1 / levels",
            "`/` takes Int and Int, found Int and List<Int>",
            "1 / levels",
        ),
        (
            r#"1 % "a""#,
            "`%` takes Int and Int, found Int and String",
            r#"1 % "a""#,
        ),
        (
            "flag < 1",
            "`<` takes Int and Int, found Bool and Int",
            "flag < 1",
        ),
        (
            "1 <= flag",
            "`<=` takes Int and Int, found Int and Bool",
            "1 <= flag",
        ),
        (
            "flag > flag",
            "`>` takes Int and Int, found Bool and Bool",
            "flag > flag",
        ),
        (
            "health >= 1",
            "`>=` takes Int and Int, found {boxes: Int, count: List<Int>} and Int",
            "health >= 1",
        ),
        (
            "1 == true",
            "`==` takes two Ints or two Bools, found Int and Bool",
            "1 == true",
        ),
        (
            r#""a" != "a""#,
            "`!=` takes two Ints or two Bools, found String and String",
            r#""a" != "a""#,
        ),
        (
            "levels == levels",
            "`==` takes two Ints or two Bools, found List<Int> and List<Int>",
            "levels == levels",
        ),
        (
            "1 && flag",
            "`&&` takes Bool and Bool, found Int and Bool",
            "1 && flag",
        ),
        (
            "flag || 0",
            "`||` takes Bool and Bool, found Bool and Int",
            "flag || 0",
        ),
        ("-flag", "`-` takes an Int, found Bool", "-flag"),
        ("!1", "`!` takes a Bool, found Int", "!1"),
        ("!(wits + 1)", "`!` takes a Bool, found Int", "!(wits + 1)"),
        (
            "wits + (flag)",
            "`+` takes Int and Int, found Int and Bool",
            "wits + (flag)",
        ),
        // The innermost mistake is the one reported.
        (
            "1 + (2 * true) + 3",
            "`*` takes Int and Int, found Int and Bool",
            "2 * true",
        ),
    ]);
}

#[test]
fn error_calls() {
    errors(&[
        ("min()", "`min` takes at least 1 argument, found 0", "()"),
        ("max()", "`max` takes at least 1 argument, found 0", "()"),
        (
            "min(1, true)",
            "argument 2 of `min` must be Int, found Bool",
            "true",
        ),
        (
            "max(flag)",
            "argument 1 of `max` must be Int, found Bool",
            "flag",
        ),
        (
            "lookup(1)",
            "no `lookup` takes (Int), it takes (Int, List<T>) or (Int, String)",
            "lookup(1)",
        ),
        (
            "lookup(true, [1])",
            "no `lookup` takes (Bool, List<Int>), it takes (Int, List<T>) or (Int, String)",
            "lookup(true, [1])",
        ),
        (
            "lookup(1, 2)",
            "no `lookup` takes (Int, Int), it takes (Int, List<T>) or (Int, String)",
            "lookup(1, 2)",
        ),
        (
            "count(1)",
            "no `count` takes (Int), it takes ({boxes: Int, count: List<Int>}) or ({boxes: Int, count: List<Int>}, Int)",
            "count(1)",
        ),
        (
            "count()",
            "no `count` takes (), it takes ({boxes: Int, count: List<Int>}) or ({boxes: Int, count: List<Int>}, Int)",
            "count()",
        ),
        (
            "count(health, flag)",
            "no `count` takes ({boxes: Int, count: List<Int>}, Bool), it takes ({boxes: Int, count: List<Int>}) or ({boxes: Int, count: List<Int>}, Int)",
            "count(health, flag)",
        ),
        ("boxes()", "`boxes` takes 1 argument, found 0", "()"),
        (
            "boxes(health, 1)",
            "`boxes` takes 1 argument, found 2",
            "(health, 1)",
        ),
        (
            "boxes(1)",
            "argument 1 of `boxes` must be {boxes: Int, count: List<Int>}, found Int",
            "1",
        ),
        (
            "boxes(maybe_track)",
            "argument 1 of `boxes` must be {boxes: Int, count: List<Int>}, found Option<{boxes: Int, count: List<Int>}>, unwrap the optional value first with `.unwrap_or(...)`",
            "maybe_track",
        ),
        ("pick(1)", "`pick` takes no arguments, found 1", "(1)"),
        (
            "label(flag)",
            "argument 1 of `label` must be Int, found Bool",
            "flag",
        ),
        // An argument's own error comes first.
        ("min(1, x)", "unknown name `x`", "x"),
    ]);
}

#[test]
fn error_methods() {
    errors(&[
        (
            "wits.unwrap_or(0)",
            "Int has no method `unwrap_or`, its methods are `abs`, `clamp`, `max` and `min`",
            "unwrap_or",
        ),
        (
            "bonus.len()",
            "Option<Int> has no method `len`, unwrap the optional value first with `.unwrap_or(...)`",
            "len",
        ),
        (
            "bonus.unwrapor(0)",
            "Option<Int> has no method `unwrapor`, unwrap the optional value first with `.unwrap_or(...)`",
            "unwrapor",
        ),
        (
            "levels.abs()",
            "List<Int> has no method `abs`, its methods are `len`",
            "abs",
        ),
        (
            "flag.abs()",
            "Bool has no method `abs`, it has no methods",
            "abs",
        ),
        (
            r#""a".len()"#,
            "String has no method `len`, it has no methods",
            "len",
        ),
        (
            "health.count()",
            "{boxes: Int, count: List<Int>} has no method `count`, it has no methods",
            "count",
        ),
        (
            "specialties.len()",
            "{athletics: Int, brawl: Option<Int>} has no method `len`, it has no methods",
            "len",
        ),
        (
            "wits.abs(1)",
            "`abs` on Int takes no arguments, found 1",
            "(1)",
        ),
        ("wits.min()", "`min` on Int takes 1 argument, found 0", "()"),
        (
            "wits.max(1, 2)",
            "`max` on Int takes 1 argument, found 2",
            "(1, 2)",
        ),
        (
            "wits.clamp(1)",
            "`clamp` on Int takes 2 arguments, found 1",
            "(1)",
        ),
        (
            "wits.clamp(1, flag)",
            "argument 2 of `clamp` on Int must be Int, found Bool",
            "flag",
        ),
        (
            "wits.min(bonus)",
            "argument 1 of `min` on Int must be Int, found Option<Int>, unwrap the optional value first with `.unwrap_or(...)`",
            "bonus",
        ),
        (
            "bonus.unwrap_or(flag)",
            "argument 1 of `unwrap_or` on Option<Int> must be Int, found Bool",
            "flag",
        ),
        (
            "bonus.unwrap_or(bonus)",
            "argument 1 of `unwrap_or` on Option<Int> must be Int, found Option<Int>, unwrap the optional value first with `.unwrap_or(...)`",
            "bonus",
        ),
        (
            "bonus.unwrap_or()",
            "`unwrap_or` on Option<Int> takes 1 argument, found 0",
            "()",
        ),
        (
            "bonus.is_some(1)",
            "`is_some` on Option<Int> takes no arguments, found 1",
            "(1)",
        ),
        (
            "bonus.is_none(1)",
            "`is_none` on Option<Int> takes no arguments, found 1",
            "(1)",
        ),
        (
            "levels.len(1)",
            "`len` on List<Int> takes no arguments, found 1",
            "(1)",
        ),
        // The method is checked before its arguments.
        (
            "wits.nope(x)",
            "Int has no method `nope`, its methods are `abs`, `clamp`, `max` and `min`",
            "nope",
        ),
    ]);
}

#[test]
fn error_fields_and_indices() {
    errors(&[
        (
            "wits.athletics",
            "Int has no fields, so no `.athletics`",
            "athletics",
        ),
        (
            "wits.abs",
            "Int has no fields, so no `.abs`, `abs` is a method, call it with `.abs(...)`",
            "abs",
        ),
        (
            "levels.len",
            "List<Int> has no fields, so no `.len`, `len` is a method, call it with `.len(...)`",
            "len",
        ),
        (
            "specialties.occult",
            r#"this record has no field "occult", its fields are "athletics" and "brawl""#,
            "occult",
        ),
        (
            r#"m["Brawl"]"#,
            r#"this record has no field "Brawl", its fields are "Brawl (bar)", "a b" and "inner""#,
            r#""Brawl""#,
        ),
        (
            "m.inner.shallow",
            r#"this record has no field "shallow", its fields are "deep""#,
            "shallow",
        ),
        (
            "specialties[0]",
            "a record is indexed by a string literal naming its field, such as `m[\"name\"]`",
            "0",
        ),
        (
            "specialties[name]",
            "a record is indexed by a string literal naming its field, such as `m[\"name\"]`",
            "name",
        ),
        (
            "wits[0]",
            "Int cannot be indexed, only a List or a record can",
            "wits",
        ),
        (
            r#""abc"[0]"#,
            "String cannot be indexed, only a List or a record can",
            r#""abc""#,
        ),
        (
            "levels[flag]",
            "a List<Int> is indexed by an Int, found Bool",
            "flag",
        ),
        (
            r#"levels["a"]"#,
            "a List<Int> is indexed by an Int, found String",
            r#""a""#,
        ),
        (
            "levels[bonus]",
            "a List<Int> is indexed by an Int, found Option<Int>",
            "bonus",
        ),
    ]);
}

#[test]
fn error_lists() {
    errors(&[
        (
            "[1, true]",
            "entry 2 of this list is Bool, but the first is Int, a list's entries have one type",
            "true",
        ),
        (
            "[1, 2, bonus]",
            "entry 3 of this list is Option<Int>, but the first is Int, a list's entries have one type",
            "bonus",
        ),
        (
            "[[1], [true]]",
            "entry 2 of this list is List<Bool>, but the first is List<Int>, a list's entries have one type",
            "[true]",
        ),
        (
            "[]",
            "an empty list `[]` has no entry type, give it at least one entry",
            "[]",
        ),
        (
            "lookup(1, [])",
            "an empty list `[]` has no entry type, give it at least one entry",
            "[]",
        ),
        (
            "[[]]",
            "an empty list `[]` has no entry type, give it at least one entry",
            "[]",
        ),
    ]);
}

#[test]
fn parse_errors_pass_through() {
    errors(&[
        (
            "1 +",
            "expected an expression, found the end of the expression",
            "",
        ),
        (
            "a < b < c",
            "comparisons do not chain, combine them with `&&` or add parentheses",
            "<",
        ),
    ]);

    let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    let (message, _) = error(&deep);
    assert_eq!(
        message,
        "this parenthesis nests too deeply, the limit is 32 levels of nesting"
    );

    let options = Options::new().max_depth(40);
    let program = compile_with(&deep, &env(), &options).unwrap();
    assert_eq!(program.result_type(), Type::Int);
}

#[test]
fn empty_environment() {
    let env = Environment::new();

    assert_eq!(compile("1 + 2", &env).unwrap().result_type(), Type::Int);
    assert_eq!(error_in("min(1)", &env).0, "unknown function `min`");
    assert_eq!(error_in("wits", &env).0, "unknown name `wits`");
}

#[test]
fn declarations() {
    let mut env = Environment::new();

    for name in ["a", "_", "_a1", "wits", "blood_potency", "x9"] {
        assert_eq!(env.value(name, Type::Int), Ok(()), "{name}");
    }

    for name in ["", "A", "Wits", "9a", "a-b", "a b", "é", "a.b"] {
        assert_eq!(
            env.value(name, Type::Int),
            Err(DeclareError::InvalidName(name.to_owned())),
            "{name}"
        );
        assert_eq!(
            env.function(name, [], Type::Int),
            Err(DeclareError::InvalidName(name.to_owned())),
            "{name}"
        );
    }

    assert_eq!(
        env.value("x", Type::option(Type::option(Type::Int))),
        Err(DeclareError::NestedOption("x".to_owned()))
    );
    assert_eq!(
        env.value(
            "x",
            Type::list(Type::record([("a", Type::option(Type::option(Type::Int)))]))
        ),
        Err(DeclareError::NestedOption("x".to_owned()))
    );
    assert_eq!(
        env.function("f", [Type::option(Type::option(Type::Int))], Type::Int),
        Err(DeclareError::NestedOption("f".to_owned()))
    );
    assert_eq!(
        env.function("f", [], Type::option(Type::option(Type::Int))),
        Err(DeclareError::NestedOption("f".to_owned()))
    );

    let mut deep = Type::Int;

    for _ in 1..MAX_TYPE_DEPTH {
        deep = Type::list(deep);
    }

    assert_eq!(env.value("ok", deep.clone()), Ok(()));
    assert_eq!(
        env.value("x", Type::list(deep)),
        Err(DeclareError::TypeTooDeep("x".to_owned()))
    );

    let messages = [
        (
            DeclareError::InvalidName("A".into()),
            "`A` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`",
        ),
        (
            DeclareError::NestedOption("x".into()),
            "the type of `x` has an option of an option, which is not a type",
        ),
        (
            DeclareError::TypeTooDeep("x".into()),
            "the type of `x` nests more than 64 levels",
        ),
    ];

    for (error, message) in messages {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn redeclaring() {
    let mut env = Environment::new();
    env.value("a", Type::Int).unwrap();
    env.value("a", Type::Bool).unwrap();
    assert_eq!(env.value_type("a"), Some(&Type::Bool));
    assert_eq!(compile("a", &env).unwrap().result_type(), Type::Bool);

    env.function("f", [Type::Int], Type::Int).unwrap();
    env.function("f", [Type::Int], Type::Bool).unwrap();
    assert_eq!(compile("f(1)", &env).unwrap().result_type(), Type::Bool);
    assert_eq!(
        error_in("f(true)", &env).0,
        "argument 1 of `f` must be Int, found Bool"
    );

    env.function("f", [Type::Bool], Type::Int).unwrap();
    assert_eq!(compile("f(true)", &env).unwrap().result_type(), Type::Int);

    // Adding a builtin twice keeps one.
    env.builtin(Builtin::Min);
    env.builtin(Builtin::Min);
    assert_eq!(
        error_in("min()", &env).0,
        "`min` takes at least 1 argument, found 0"
    );

    // The first matching declaration wins.
    env.function("lookup", [Type::Int, Type::list(Type::Int)], Type::Bool)
        .unwrap();
    env.builtin(Builtin::Lookup);
    assert_eq!(
        compile("lookup(1, [1])", &env).unwrap().result_type(),
        Type::Bool
    );
    assert_eq!(
        compile("lookup(1, [true])", &env).unwrap().result_type(),
        Type::Bool
    );
    assert_eq!(
        compile("lookup(1, [[1]])", &env).unwrap().result_type(),
        Type::list(Type::Int)
    );
}

#[test]
fn type_display() {
    let cases = [
        (Type::Int, "Int"),
        (Type::list(Type::option(Type::Bool)), "List<Option<Bool>>"),
        (Type::record::<&str>([]), "{}"),
        (
            Type::record([("b", Type::Int), ("a", Type::String)]),
            "{a: String, b: Int}",
        ),
        (
            Type::record([("x y", Type::Int), ("_1", Type::Int), ("1", Type::Int)]),
            r#"{"1": Int, _1: Int, "x y": Int}"#,
        ),
    ];

    for (ty, text) in cases {
        assert_eq!(ty.to_string(), text);
    }
}

fn memory(source: &str) -> (u32, u32, u32) {
    let m = compile(source, &env()).unwrap().memory();
    (m.stack, m.heap, m.constants)
}

#[test]
fn memory_bounds() {
    let cases = [
        ("1", (1, 0, 0)),
        ("-1", (1, 0, 0)),
        ("wits", (1, 0, 0)),
        ("1 + 2", (2, 0, 0)),
        ("((1 + 2) + 3) + 4", (2, 0, 0)),
        ("1 + (2 + (3 + 4))", (4, 0, 0)),
        ("1 * 2 + 3 * 4", (3, 0, 0)),
        ("min(1, 2, 3, 4)", (4, 0, 0)),
        ("max(wits)", (1, 0, 0)),
        ("wits.clamp(1, 2)", (3, 0, 0)),
        ("bonus.unwrap_or(0) + 1", (3, 0, 0)),
        ("[1, 2, 3]", (1, 0, 3)),
        ("[-1, -2]", (1, 0, 2)),
        ("[true, false]", (1, 0, 2)),
        ("lookup(wits, [10, 11, 12, 13])", (2, 0, 4)),
        ("[[1, 2], [3]]", (1, 0, 5)),
        ("[wits, 2]", (2, 2, 0)),
        ("[[wits], [3]]", (2, 3, 1)),
        ("lookup(0, [wits + 1])", (3, 1, 0)),
        // Strings take a word per 8 bytes, and a repeat is stored once.
        (r#""""#, (1, 0, 0)),
        (r#""Essence (1e)""#, (1, 0, 2)),
        (r#"[name, "Essence (1e)", "Essence (1e)"]"#, (3, 3, 2)),
        (
            r#"lookup(1, "abcdefgh") + lookup(2, "abcdefgh")"#,
            (3, 0, 1),
        ),
        // The right operand of `&&` runs on what the left leaves.
        ("flag && wits > 1", (2, 0, 0)),
        ("flag || (wits > 1 && (1 + (2 + 3)) > 0)", (3, 0, 0)),
        // An `Option<Int>` is two words wherever it is held.
        ("specialties.brawl.unwrap_or(0)", (3, 0, 0)),
        ("bonus", (2, 0, 0)),
        ("bonus.is_some()", (2, 0, 0)),
        ("[bonus]", (2, 2, 0)),
        ("[bonus, bonus, bonus][2]", (6, 6, 0)),
        ("maybe.unwrap_or(true)", (2, 0, 0)),
        (r#"m["Brawl (bar)"]"#, (1, 0, 0)),
        ("levels[wits]", (2, 0, 0)),
        ("count(health, 1 + 2)", (3, 0, 0)),
    ];

    for (source, expected) in cases {
        assert_eq!(memory(source), expected, "{source:?}");
    }

    // The values a program reads are counted as they are held.
    assert_eq!(compile("[wits, 2]", &env()).unwrap().memory().total(), 5);
    assert_eq!(
        compile("bonus.unwrap_or(wits)", &env())
            .unwrap()
            .memory()
            .globals,
        3
    );
}

#[test]
fn globals_and_functions() {
    let program = compile(
        "count(health) + count(health, wits) + count(health) + wits + bonus.unwrap_or(wits)",
        &env(),
    )
    .unwrap();

    assert_eq!(
        program.globals(),
        [
            ("health", track()),
            ("wits", Type::Int),
            ("bonus", Type::option(Type::Int)),
        ]
    );

    assert_eq!(
        program.functions(),
        [
            ("count", vec![track()], Type::Int),
            ("count", vec![track(), Type::Int], Type::Int),
        ]
    );

    // Builtins are the machine's own, not the caller's.
    assert!(
        compile("min(1) + lookup(1, [1])", &env())
            .unwrap()
            .functions()
            .is_empty()
    );
}

#[test]
fn code() {
    let code = |source: &str| compile(source, &env()).unwrap().code;

    use Instr::*;

    const WITS: Instr = Global {
        at: 0,
        ty: TypeId(0),
    };

    assert_eq!(code("1 + 2 * 3"), [Int(1), Int(2), Int(3), Mul, Add]);
    assert_eq!(code("-5"), [Int(-5)]);
    assert_eq!(code("-(5)"), [Int(-5)]);
    assert_eq!(code("--5"), [Int(5)]);
    assert_eq!(
        code("-wits"),
        [
            Global {
                at: 0,
                ty: TypeId(0)
            },
            Neg
        ]
    );
    assert_eq!(code("!true"), [Bool(true), Not]);
    assert_eq!(
        code("flag && wits > 1"),
        [
            Global {
                at: 0,
                ty: TypeId(1)
            },
            And(3),
            Global {
                at: 1,
                ty: TypeId(0)
            },
            Int(1),
            Gt
        ]
    );
    assert_eq!(
        code("flag || flag && flag"),
        [
            Global {
                at: 0,
                ty: TypeId(1)
            },
            Or(3),
            Global {
                at: 0,
                ty: TypeId(1)
            },
            And(1),
            Global {
                at: 0,
                ty: TypeId(1)
            }
        ]
    );
    assert_eq!(code("[1, 2]"), [Const(0)]);
    assert_eq!(
        code("[wits, 2]"),
        [
            WITS,
            Int(2),
            List {
                len: 2,
                entry: TypeId(0)
            }
        ]
    );
    assert_eq!(
        code("lookup(wits, [1])"),
        [WITS, Const(0), Lookup(TypeId(0))]
    );
    assert_eq!(code("min(1, wits, 3)"), [Int(1), WITS, Int(3), Min(3)]);
    assert_eq!(code("wits.max(2)"), [WITS, Int(2), Max(2)]);
    assert_eq!(code("wits.clamp(1, 2)"), [WITS, Int(1), Int(2), Clamp]);
    assert_eq!(
        code("bonus.unwrap_or(3)"),
        [
            Global {
                at: 0,
                ty: TypeId(2)
            },
            Int(3),
            UnwrapOr(TypeId(2))
        ]
    );
    assert_eq!(
        code("specialties.brawl"),
        [
            Global {
                at: 0,
                ty: TypeId(3)
            },
            Field {
                at: 1,
                ty: TypeId(2)
            }
        ]
    );
    assert_eq!(
        code(r#"m["inner"].deep"#),
        [
            Global {
                at: 0,
                ty: TypeId(5)
            },
            Field {
                at: 2,
                ty: TypeId(4)
            },
            Field {
                at: 0,
                ty: TypeId(1)
            }
        ]
    );
    assert_eq!(
        code("levels[1]"),
        [
            Global {
                at: 0,
                ty: TypeId(2)
            },
            Int(1),
            Index(TypeId(0))
        ]
    );
    assert_eq!(
        code("count(health, 1)"),
        [
            Global {
                at: 0,
                ty: TypeId(3)
            },
            Int(1),
            Call(0)
        ]
    );
}

#[test]
fn store_round_trip() {
    let sources = [
        "1",
        "min(wits, dexterity) + athletics",
        "lookup(blood_potency, [10, 11, 12, 13])",
        r#"lookup(primal_urge, "Essence (1e)")"#,
        "count(health) + count(health, 2) + boxes(health)",
        "specialties.brawl.unwrap_or(0).max(1)",
        "flag && (wits > 1 || !maybe.unwrap_or(false))",
        "[[wits], [1, 2]][0][0]",
        r#"m["inner"].deep == true"#,
        "label(1).is_some()",
    ];

    for source in sources {
        let program = compile(source, &env()).unwrap();
        let bytes = program.to_bytes().unwrap();
        let loaded = Program::from_bytes(&bytes).unwrap();

        assert_eq!(loaded.code, program.code, "{source:?}");
        assert_eq!(loaded.constants, program.constants, "{source:?}");
        assert_eq!(loaded.memory(), program.memory(), "{source:?}");
        assert_eq!(loaded.result_type(), program.result_type(), "{source:?}");
        assert_eq!(loaded.globals(), program.globals(), "{source:?}");
        assert_eq!(loaded.functions(), program.functions(), "{source:?}");
        assert_eq!(loaded.to_bytes().unwrap(), bytes, "{source:?}");
    }
}

fn stored(source: &str) -> Stored {
    let bytes = compile(source, &env()).unwrap().to_bytes().unwrap();
    musli::storage::from_slice(&bytes).unwrap()
}

fn load(stored: &Stored) -> Result<Program, StoreError> {
    Program::from_bytes(&musli::storage::to_vec(stored).unwrap())
}

fn invalid(stored: &Stored) -> String {
    match load(stored) {
        Ok(_) => panic!("a tampered program loaded"),
        Err(StoreError::Invalid(reason)) => reason,
        Err(error) => panic!("expected an invalid program, found {error}"),
    }
}

#[test]
fn tampered_programs() {
    let source = "wits + lookup(1, [2, 3]) + count(health)";

    // Unchanged, it loads.
    assert!(load(&stored(source)).is_ok());

    type Tamper = Box<dyn Fn(&mut Stored)>;

    let cases: Vec<(&str, Tamper, &str)> = vec![
        (
            source,
            Box::new(|s| s.memory.stack -= 1),
            "it declares Memory { stack: 2, heap: 0, constants: 2, globals: 2 } of memory but uses Memory { stack: 3, heap: 0, constants: 2, globals: 2 }",
        ),
        (
            source,
            Box::new(|s| s.memory.stack += 1),
            "it declares Memory { stack: 4, heap: 0, constants: 2, globals: 2 } of memory but uses Memory { stack: 3, heap: 0, constants: 2, globals: 2 }",
        ),
        (
            source,
            Box::new(|s| s.memory.constants = 0),
            "it declares Memory { stack: 3, heap: 0, constants: 0, globals: 2 } of memory but uses Memory { stack: 3, heap: 0, constants: 2, globals: 2 }",
        ),
        (
            "bonus",
            Box::new(|s| s.memory.globals = 1),
            "it declares Memory { stack: 2, heap: 0, constants: 0, globals: 1 } of memory but uses Memory { stack: 2, heap: 0, constants: 0, globals: 2 }",
        ),
        (
            "[wits]",
            Box::new(|s| s.memory.heap = 0),
            "it declares Memory { stack: 1, heap: 0, constants: 0, globals: 1 } of memory but uses Memory { stack: 1, heap: 1, constants: 0, globals: 1 }",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[2] = Instr::Lt),
            "it ends with 1 values on the stack, not one of its result type",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code.pop().map(|_| ()).unwrap()),
            "it ends with 2 values on the stack, not one of its result type",
        ),
        (
            "1",
            Box::new(|s| s.code = vec![Instr::Add]),
            "instruction 0 (Add) pops an empty stack",
        ),
        (
            "1",
            Box::new(|s| s.code = vec![]),
            "it ends with 0 values on the stack, not one of its result type",
        ),
        (
            "wits",
            Box::new(|s| {
                s.code[0] = Instr::Global {
                    at: 1,
                    ty: TypeId(0),
                }
            }),
            "instruction 0 (Global { at: 1, ty: TypeId(0) }) names a global there is not",
        ),
        (
            "wits",
            Box::new(|s| {
                s.code[0] = Instr::Global {
                    at: 0,
                    ty: TypeId(1),
                }
            }),
            "instruction 0 (Global { at: 0, ty: TypeId(1) }) names a global there is not",
        ),
        (
            "[1]",
            Box::new(|s| s.code[0] = Instr::Const(7)),
            "instruction 0 (Const(7)) names a constant there is not",
        ),
        (
            "count(health)",
            Box::new(|s| s.code[1] = Instr::Call(1)),
            "instruction 1 (Call(1)) calls a function there is not",
        ),
        (
            "flag && flag",
            Box::new(|s| s.code[1] = Instr::And(2)),
            "instruction 1 (And(2)) jumps past the end",
        ),
        (
            "flag && flag && flag",
            Box::new(|s| s.code[1] = Instr::And(0)),
            "instruction 2 (Global { at: 0, ty: TypeId(1) }) is reached with two different stacks",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[2] = Instr::Min(0)),
            "instruction 2 (Min(0)) takes no operands",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 0,
                    entry: TypeId(0),
                }
            }),
            "instruction 1 (List { len: 0, entry: TypeId(0) }) makes a list of no entries",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 2,
                    entry: TypeId(0),
                }
            }),
            "instruction 1 (List { len: 2, entry: TypeId(0) }) makes a list of entries it does not find",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 1,
                    entry: TypeId(1),
                }
            }),
            "instruction 1 (List { len: 1, entry: TypeId(1) }) makes a list of entries it does not find",
        ),
        (
            "specialties.athletics",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 9,
                    ty: TypeId(0),
                }
            }),
            "instruction 1 (Field { at: 9, ty: TypeId(0) }) reads a field the record does not have",
        ),
        (
            "wits.abs()",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 0,
                    ty: TypeId(0),
                }
            }),
            "instruction 1 (Field { at: 0, ty: TypeId(0) }) reads a field of other than a record",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[0] = Instr::Bool(true)),
            "instruction 2 (Add) finds an operand of the wrong type",
        ),
        (
            "bonus.unwrap_or(1)",
            Box::new(|s| s.code[1] = Instr::Bool(true)),
            "instruction 2 (UnwrapOr(TypeId(2))) finds an operand of the wrong type",
        ),
        (
            "bonus.unwrap_or(1)",
            Box::new(|s| s.code[2] = Instr::UnwrapOr(TypeId(0))),
            "instruction 2 (UnwrapOr(TypeId(0))) unwraps other than an option type",
        ),
        (
            "bonus.is_none()",
            Box::new(|s| {
                s.types.push(TypeNode::Option(TypeId(1)));
                s.code[1] = Instr::IsNone(TypeId(3));
            }),
            "instruction 1 (IsNone(TypeId(3))) finds an operand of the wrong type",
        ),
        (
            "levels[0]",
            Box::new(|s| s.code[2] = Instr::Index(TypeId(1))),
            "instruction 2 (Index(TypeId(1))) indexes other than a list of its entry type",
        ),
        (
            "lookup(0, levels)",
            Box::new(|s| s.code[2] = Instr::Lookup(TypeId(1))),
            "instruction 2 (Lookup(TypeId(1))) looks up in other than a list of its entry type",
        ),
        (
            "specialties.brawl",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 1,
                    ty: TypeId(0),
                }
            }),
            "instruction 1 (Field { at: 1, ty: TypeId(0) }) reads a field the record does not have",
        ),
        (
            "levels.len()",
            Box::new(|s| s.code[1] = Instr::IsSome(TypeId(2))),
            "instruction 1 (IsSome(TypeId(2))) tests other than an option type",
        ),
        (
            "1 == 1",
            Box::new(|s| s.code = vec![Instr::Const(0), Instr::Const(0), Instr::Eq]),
            "instruction 0 (Const(0)) names a constant there is not",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![Scalar::Bool(true)],
                }
            }),
            "constant 0 has an entry of another type",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![],
                }
            }),
            "constant 0 is an empty list",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![Scalar::Const(0)],
                }
            }),
            "constant 0 refers to a later constant",
        ),
        (
            "wits",
            Box::new(|s| s.globals[0].name = "Wits".into()),
            "global \"Wits\" is invalid or repeated",
        ),
        (
            "wits + dexterity",
            Box::new(|s| s.globals[1].name = "wits".into()),
            "global \"wits\" is invalid or repeated",
        ),
        (
            "count(health)",
            Box::new(|s| s.functions.push(s.functions[0].clone())),
            "function \"count\" is invalid or repeated",
        ),
        (
            "count(health)",
            Box::new(|s| {
                s.functions[0] = HostFunction {
                    name: "count".into(),
                    params: vec![TypeId(99)],
                    ret: TypeId(0),
                }
            }),
            "type 99 is not in its table",
        ),
        (
            "wits",
            Box::new(|s| s.result = TypeId(1)),
            "it ends with 1 values on the stack, not one of its result type",
        ),
        (
            "wits",
            Box::new(|s| {
                s.globals[0] = Global {
                    name: "wits".into(),
                    ty: TypeId(5),
                }
            }),
            "type 5 is not in its table",
        ),
        (
            "levels",
            Box::new(|s| s.types[2] = TypeNode::List(TypeId(2))),
            "type 2 refers to a later type 2",
        ),
        (
            "levels",
            Box::new(|s| s.types.push(TypeNode::Int)),
            "type 3 is a repeat of an earlier type",
        ),
        (
            "bonus",
            Box::new(|s| s.types.push(TypeNode::Option(TypeId(2)))),
            "type 3 is an option of an option",
        ),
        (
            "specialties",
            Box::new(|s| {
                if let Some(TypeNode::Record(fields)) = s.types.last_mut() {
                    fields.reverse();
                }
            }),
            "type 3 has fields that are not sorted and unique",
        ),
        (
            "1",
            Box::new(|s| {
                for n in 0..64 {
                    s.types.push(TypeNode::List(TypeId(n + 1)));
                }
            }),
            "type 65 nests too deeply",
        ),
        (
            "1",
            Box::new(|s| s.types.clear()),
            "type 0 is not in its table",
        ),
    ];

    for (source, tamper, reason) in cases {
        let mut stored = stored(source);
        tamper(&mut stored);
        assert_eq!(invalid(&stored), reason, "{source:?}");
    }
}

#[test]
fn stored_bytes_errors() {
    let bytes = compile("wits + 1", &env()).unwrap().to_bytes().unwrap();

    for len in 0..bytes.len() {
        match Program::from_bytes(&bytes[..len]) {
            Err(StoreError::Decode(_)) => {}
            other => panic!("{len}: {other:?}"),
        }
    }

    let mut stored = stored("wits + 1");
    stored.format = FORMAT + 1;

    assert_eq!(
        load(&stored).unwrap_err().to_string(),
        "the program was stored in format 3, this evalite reads format 2"
    );

    assert!(
        Program::from_bytes(&[])
            .unwrap_err()
            .to_string()
            .starts_with("these bytes are not a stored program: ")
    );

    stored.format = FORMAT;
    stored.memory.stack = 0;
    assert_eq!(
        load(&stored).unwrap_err().to_string(),
        "the stored program is not valid: it declares Memory { stack: 0, heap: 0, constants: 0, globals: 1 } of memory but uses Memory { stack: 2, heap: 0, constants: 0, globals: 1 }"
    );
}

/// Flipping bytes of stored programs never panics, and whatever loads is
/// a valid program.
#[test]
fn random_corruption() {
    let mut rng = Rng(0xc0de);
    let env = env();

    let sources = [
        "lookup(blood_potency, [10, 11, 12, 13]) + count(health, 2)",
        "flag && (wits > 1 || !maybe.unwrap_or(false))",
        r#"[[wits], [1, 2]][0][0] + m["Brawl (bar)"] + lookup(1, "Essence")"#,
    ];

    for source in sources {
        let bytes = compile(source, &env).unwrap().to_bytes().unwrap();

        for _ in 0..3_000 {
            let mut bytes = bytes.clone();

            for _ in 0..1 + rng.below(3) {
                let at = rng.below(bytes.len());
                bytes[at] = rng.next() as u8;
            }

            if let Ok(program) = Program::from_bytes(&bytes) {
                let again = program.to_bytes().unwrap();
                let reloaded = Program::from_bytes(&again).unwrap();
                assert_eq!(reloaded.memory(), program.memory());
            }
        }
    }
}

/// A random well-typed expression of type `ty`: Int, Bool, Option<Int> or
/// List<Int>. Operands of operators are parenthesized so precedence never
/// changes what was built.
pub(crate) fn generate(rng: &mut Rng, ty: &str, budget: usize) -> String {
    let leaf = budget == 0 || rng.below(4) == 0;

    match ty {
        "Int" if leaf => match rng.below(6) {
            0 => rng.below(100).to_string(),
            1 => format!("-{}", rng.below(100)),
            2 => rng
                .pick(&["wits", "dexterity", "athletics", "min"])
                .to_owned(),
            3 => "specialties.athletics".to_owned(),
            4 => r#"m["Brawl (bar)"]"#.to_owned(),
            _ => "levels.len()".to_owned(),
        },
        "Int" => {
            let b = budget - 1;

            match rng.below(14) {
                0 => {
                    let op = rng.pick(&["+", "-", "*", "/", "%"]);
                    let a = generate(rng, "Int", b / 2);
                    let c = generate(rng, "Int", b / 2);
                    format!("({a}) {op} ({c})")
                }
                1 => format!("-({})", generate(rng, "Int", b)),
                2 | 3 => {
                    let f = rng.pick(&["min", "max"]);
                    let n = 1 + rng.below(4);
                    let args = (0..n)
                        .map(|_| generate(rng, "Int", b / n))
                        .collect::<Vec<_>>();
                    format!("{f}({})", args.join(", "))
                }
                4 => format!("({}).abs()", generate(rng, "Int", b)),
                5 => {
                    let m = rng.pick(&["min", "max"]);
                    let a = generate(rng, "Int", b / 2);
                    let c = generate(rng, "Int", b / 2);
                    format!("({a}).{m}({c})")
                }
                6 => {
                    let a = generate(rng, "Int", b / 3);
                    let lo = generate(rng, "Int", b / 3);
                    let hi = generate(rng, "Int", b / 3);
                    format!("({a}).clamp({lo}, {hi})")
                }
                7 => {
                    let o = generate(rng, "Option<Int>", b / 2);
                    let d = generate(rng, "Int", b / 2);
                    format!("({o}).unwrap_or({d})")
                }
                8 => {
                    let i = generate(rng, "Int", b / 2);
                    let l = generate(rng, "List<Int>", b / 2);
                    format!("lookup({i}, {l})")
                }
                9 => {
                    let l = generate(rng, "List<Int>", b / 2);
                    let i = generate(rng, "Int", b / 2);
                    format!("({l})[{i}]")
                }
                10 => format!("({}).len()", generate(rng, "List<Int>", b)),
                11 => format!("count(health, {})", generate(rng, "Int", b)),
                12 => format!(r#"lookup({}, "Essence")"#, generate(rng, "Int", b)),
                _ => rng
                    .pick(&["count(health)", "boxes(health)", r#"boxes(m["a b"])"#])
                    .to_owned(),
            }
        }
        "Bool" if leaf => rng.pick(&["true", "false", "flag", "pick()"]).to_owned(),
        "Bool" => {
            let b = budget - 1;

            match rng.below(6) {
                0 => {
                    let op = rng.pick(&["<", "<=", ">", ">=", "==", "!="]);
                    let a = generate(rng, "Int", b / 2);
                    let c = generate(rng, "Int", b / 2);
                    format!("({a}) {op} ({c})")
                }
                1 => {
                    let op = rng.pick(&["&&", "||", "==", "!="]);
                    let a = generate(rng, "Bool", b / 2);
                    let c = generate(rng, "Bool", b / 2);
                    format!("({a}) {op} ({c})")
                }
                2 => format!("!({})", generate(rng, "Bool", b)),
                3 => {
                    let m = rng.pick(&["is_some", "is_none"]);
                    format!("({}).{m}()", generate(rng, "Option<Int>", b))
                }
                4 => format!("maybe.unwrap_or({})", generate(rng, "Bool", b)),
                _ => format!("m.inner.deep || ({})", generate(rng, "Bool", b)),
            }
        }
        "Option<Int>" => match rng.below(3) {
            0 => "bonus".to_owned(),
            1 => "specialties.brawl".to_owned(),
            _ => format!("label({})", generate(rng, "Int", budget.saturating_sub(1))),
        },
        "List<Int>" if leaf => "levels".to_owned(),
        "List<Int>" => {
            let b = budget - 1;
            let n = 1 + rng.below(4);

            match rng.below(3) {
                0 => {
                    let items = (0..n)
                        .map(|_| rng.below(50).to_string())
                        .collect::<Vec<_>>();
                    format!("[{}]", items.join(", "))
                }
                1 => {
                    let items = (0..n)
                        .map(|_| generate(rng, "Int", b / n))
                        .collect::<Vec<_>>();
                    format!("[{}]", items.join(", "))
                }
                _ => format!("grid[{}]", generate(rng, "Int", b)),
            }
        }
        _ => unreachable!("{ty}"),
    }
}

#[test]
fn random_well_typed() {
    let mut rng = Rng(0x7e57);
    let env = env();
    let options = Options::new().max_depth(10_000);

    for round in 0..3_000 {
        let ty = rng.pick(&["Int", "Bool", "Option<Int>", "List<Int>"]);
        let budget = rng.below(40);
        let source = generate(&mut rng, ty, budget);

        let program = match compile_with(&source, &env, &options) {
            Ok(program) => program,
            Err(error) => panic!("{round}: {source:?}: {error}"),
        };

        assert_eq!(program.result_type().to_string(), ty, "{source:?}");

        // The bound is no more than two words per token of the source (an
        // `Option<Int>` takes two), plus a constant string's bytes.
        let tokens = source
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '"')
            .filter(|t| !t.is_empty())
            .count()
            + source.matches(['(', '[', '-', '!']).count();
        let memory = program.memory();
        assert!(
            memory.stack as usize <= 2 * tokens && (memory.heap as usize) <= 2 * tokens,
            "{source:?}: {memory:?} for {tokens} tokens"
        );
        assert!(memory.stack >= 1);

        let bytes = program.to_bytes().unwrap();
        let loaded = Program::from_bytes(&bytes).unwrap();
        assert_eq!(loaded.memory(), memory, "{source:?}");
        assert_eq!(loaded.code, program.code, "{source:?}");
        assert_eq!(loaded.result_type(), program.result_type(), "{source:?}");
    }
}

/// Random token soup never makes the compiler fault: it compiles or gives
/// an error inside the input.
#[test]
fn random_ill_typed() {
    const TOKENS: &[&str] = &[
        "wits",
        "flag",
        "bonus",
        "levels",
        "grid",
        "health",
        "specialties",
        "m",
        "min",
        "max",
        "lookup",
        "count",
        "boxes",
        "label",
        "pick",
        "x",
        "1",
        "-",
        "+",
        "*",
        "<",
        "==",
        "&&",
        "!",
        "(",
        ")",
        "[",
        "]",
        ",",
        ".",
        "abs",
        "unwrap_or",
        "len",
        "brawl",
        "\"a b\"",
        "true",
        " ",
    ];

    let mut rng = Rng(0xbad);
    let env = env();

    for _ in 0..20_000 {
        let len = rng.below(16);
        let source = (0..len).map(|_| rng.pick(TOKENS)).collect::<String>();

        if let Err(error) = compile(&source, &env) {
            assert!(
                !matches!(error.kind(), ErrorKind::Internal(..)),
                "{source:?}: {error}"
            );
            let range = error.span().range();
            assert!(range.end <= source.len(), "{source:?}");
        }
    }
}

#[test]
fn long_chains_do_not_recurse() {
    let source = vec!["wits"; 50_000].join(" + ");
    let program = compile(&source, &env()).unwrap();
    assert_eq!(program.memory().stack, 2);

    let source = format!("wits{}", ".abs()".repeat(50_000));
    assert_eq!(compile(&source, &env()).unwrap().memory().stack, 1);

    let source = vec!["flag"; 50_000].join(" && ");
    assert_eq!(compile(&source, &env()).unwrap().memory().stack, 1);
}

#[test]
fn field_node_ids() {
    // Records' fields are kept in name order in the type table.
    let program = compile("specialties", &env()).unwrap();
    let Some(TypeNode::Record(fields)) = program.types.get(program.result) else {
        panic!("not a record");
    };

    assert_eq!(
        fields
            .iter()
            .map(|FieldNode { name, .. }| name.as_str())
            .collect::<Vec<_>>(),
        ["athletics", "brawl"]
    );
}
