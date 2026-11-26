//! Tests of the type checker, compiler and stored programs.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::string_slice,
    clippy::indexing_slicing
)]

use std::ops::Range;

use crate::env::DeclareKind;
use crate::error::ErrorKind;
use crate::message::Message;
use crate::program::{Constant, FORMAT, Global, HostFunction, Instr, Scalar, Stored};
use crate::tests::Rng;
use crate::types::{FieldNode, TypeId, TypeNode};
use crate::*;

/// A track as the sheet's environment declares one: a record of its
/// boxes and how many are at each state or worse.
pub(crate) fn track() -> Type {
    Type::record([("boxes", Type::int()), ("count", Type::list(Type::int()))])
}

/// The environment most tests compile against.
pub(crate) fn env() -> Environment {
    let mut env = Environment::new();

    for name in [
        "wits",
        "dexterity",
        "athletics",
        "strength",
        "blood_potency",
        "primal_urge",
        "min",
    ] {
        env.value(name, Type::int()).unwrap();
    }

    env.value("flag", Type::bool()).unwrap();
    env.value("bonus", Type::option(Type::int())).unwrap();
    env.value("maybe", Type::option(Type::bool())).unwrap();
    env.value("levels", Type::list(Type::int())).unwrap();
    env.value("grid", Type::list(Type::list(Type::int())))
        .unwrap();
    env.value("name", Type::string()).unwrap();
    env.value("health", track()).unwrap();
    env.value("tracks", Type::list(track())).unwrap();
    env.value(
        "specialties",
        Type::record([
            ("athletics", Type::int()),
            ("brawl", Type::option(Type::int())),
        ]),
    )
    .unwrap();
    env.value(
        "m",
        Type::record([
            ("Brawl (bar)", Type::int()),
            ("a b", track()),
            ("inner", Type::record([("deep", Type::bool())])),
        ]),
    )
    .unwrap();
    env.value("maybe_track", Type::option(track())).unwrap();

    env.function("count", [track()], Type::int()).unwrap();
    env.function("count", [track(), Type::int()], Type::int())
        .unwrap();
    env.function("boxes", [track()], Type::int()).unwrap();
    env.function("chart", [Type::int(), Type::string()], Type::int())
        .unwrap();
    env.function("label", [Type::int()], Type::option(Type::int()))
        .unwrap();
    env.function("pick", [], Type::bool()).unwrap();
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
        Err(errors) => {
            assert!(
                !errors
                    .iter()
                    .any(|e| matches!(e.kind(), ErrorKind::Internal(..))),
                "{source:?}: {errors}"
            );
            let first = errors.iter().next().map(|e| e.span().range());
            (errors.to_string(), first.unwrap_or_default())
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
        ("Int::min(wits)", "Int"),
        ("Int::min(wits, dexterity)", "Int"),
        ("Int::max(dexterity, wits, 7)", "Int"),
        ("[10, 11, 12].get(wits)", "Option<Int>"),
        ("levels.get(wits)", "Option<Int>"),
        ("grid.get(wits)", "Option<List<Int>>"),
        ("[true, false].get(1)", "Option<Bool>"),
        ("tracks.get(1)", "Option<{boxes: Int, count: List<Int>}>"),
        ("[bonus].get(1)", "Option<Option<Int>>"),
        ("[maybe, maybe].get(1)", "Option<Option<Bool>>"),
        ("[bonus].get(1)!", "Option<Int>"),
        ("[bonus].get(1)!!", "Int"),
        ("[bonus].get(1).unwrap_or(label(3))", "Option<Int>"),
        ("levels.get(1).unwrap_or(0)", "Int"),
        (r#"chart(primal_urge, "Essence (1e)")"#, "Int"),
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
        ("Int::min(wits, dexterity) + athletics", "Int"),
        ("Int::max (wits)", "Int"),
        ("[10, 11, 12 , 13].get(blood_potency)", "Option<Int>"),
        ("[10, 11, 12, 13].get(blood_potency + 5)", "Option<Int>"),
        ("[10, 11].get(-wits)", "Option<Int>"),
        ("[wits + 1].get(0)", "Option<Int>"),
        (r#"chart(primal_urge, "Essence (1e)")"#, "Int"),
        (r#"chart(blood_potency, "Vitae \"2e\"")"#, "Int"),
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
        ("health.count.get(-4)", "Option<Int>"),
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
            "Int::min(bonus)",
            &format!("argument 1 of `Int::min` must be Int, found Option<Int>{HINT}"),
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
            "1 + Int::max",
            "`Int::max` is a function, call it with `Int::max(...)`",
            "Int::max",
        ),
        (
            "Int::nope(1)",
            "Int has no function `nope`, its functions are `abs`, `clamp`, `max` and `min`",
            "nope",
        ),
        (
            "Foo::min(1)",
            "`Foo` is not a type with functions, such as `Int`",
            "Foo",
        ),
        (
            "Int::(1)",
            "expected a function name after `::`, found `(`",
            "(",
        ),
        ("f(1)", "unknown function `f`", "f"),
        (
            "cout(health)",
            "unknown function `cout`, did you mean `count`?",
            "cout",
        ),
        (
            "Count()",
            "unknown function `Count`, did you mean `count`?",
            "Count",
        ),
        ("wits(1)", "`wits` is a value, not a function", "wits"),
        // Arguments are not checked before the function is found.
        (
            "f(nothing)",
            "unknown function `f`\nunknown name `nothing`",
            "f",
        ),
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
        (
            "Int::min()",
            "`Int::min` takes at least 1 argument, found 0",
            "()",
        ),
        (
            "Int::max()",
            "`Int::max` takes at least 1 argument, found 0",
            "()",
        ),
        ("Int::abs()", "`Int::abs` takes 1 argument, found 0", "()"),
        (
            "Int::clamp(1, true, 2)",
            "argument 2 of `Int::clamp` must be Int, found Bool",
            "true",
        ),
        (
            "Option::unwrap_or(wits, 0)",
            "argument 1 of `Option::unwrap_or` must be Option<T>, found Int",
            "wits",
        ),
        (
            "Int::min(1, true)",
            "argument 2 of `Int::min` must be Int, found Bool",
            "true",
        ),
        (
            "Int::max(flag)",
            "argument 1 of `Int::max` must be Int, found Bool",
            "flag",
        ),
        ("chart(1)", "`chart` takes 2 arguments, found 1", "(1)"),
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
        ("Int::min(1, x)", "unknown name `x`", "x"),
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
            "List<Int> has no method `abs`, its methods are `get` and `len`",
            "abs",
        ),
        (
            "levels.get(true)",
            "argument 1 of `get` on List<Int> must be Int, found Bool",
            "true",
        ),
        (
            "levels.get()",
            "`get` on List<Int> takes 1 argument, found 0",
            "()",
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
            "Int has no method `nope`, its methods are `abs`, `clamp`, `max` and `min`\nunknown name `x`",
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
            "wits!",
            "`!` unwraps an optional value, found Int, which is never absent",
            "wits!",
        ),
        (
            "1 + levels!",
            "`!` unwraps an optional value, found List<Int>, which is never absent",
            "levels!",
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
            "[].get(1)",
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
            "wits < strength < dexterity",
            "comparisons do not chain, combine them with `&&` or add parentheses",
            "<",
        ),
    ]);

    // Checking goes on around a syntax error.
    let (message, _) = error("a < b < c");
    assert_eq!(
        message,
        "unknown name `a`\nunknown name `b`\ncomparisons do not chain, combine them with `&&` or add parentheses\nunknown name `c`"
    );

    let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    let (message, _) = error(&deep);
    assert_eq!(
        message,
        "this parenthesis nests too deeply, the limit is 32 levels of nesting"
    );

    let options = Options::new().max_depth(40);
    let program = compile_with(&deep, &env(), &options).unwrap();
    assert_eq!(program.result_type(), Type::int());
}

#[test]
fn empty_environment() {
    let env = Environment::new();

    assert_eq!(compile("1 + 2", &env).unwrap().result_type(), Type::int());
    assert_eq!(error_in("min(1)", &env).0, "unknown function `min`");
    assert_eq!(error_in("wits", &env).0, "unknown name `wits`");
}

#[test]
fn declarations() {
    let mut env = Environment::new();

    for name in ["a", "_", "_a1", "wits", "blood_potency", "x9"] {
        assert_eq!(env.value(name, Type::int()), Ok(()), "{name}");
    }

    for name in ["", "A", "Wits", "9a", "a-b", "a b", "é", "a.b"] {
        assert_eq!(
            env.value(name, Type::int()),
            Err(DeclareError::new(DeclareKind::InvalidName, name)),
            "{name}"
        );
        assert_eq!(
            env.function(name, [], Type::int()),
            Err(DeclareError::new(DeclareKind::InvalidName, name)),
            "{name}"
        );
    }

    // Options nest, anywhere a type may be written.
    assert_eq!(
        env.value("x", Type::option(Type::option(Type::int()))),
        Ok(())
    );
    assert_eq!(
        env.value(
            "y",
            Type::list(Type::record([(
                "a",
                Type::option(Type::option(Type::int()))
            )]))
        ),
        Ok(())
    );
    assert_eq!(
        env.function(
            "f",
            [Type::option(Type::option(Type::int()))],
            Type::option(Type::option(Type::bool()))
        ),
        Ok(())
    );

    let mut deep = Type::int();

    for _ in 1..MAX_TYPE_DEPTH {
        deep = Type::list(deep);
    }

    assert_eq!(env.value("ok", deep.clone()), Ok(()));
    assert_eq!(
        env.value("x", Type::list(deep)),
        Err(DeclareError::new(DeclareKind::TypeTooDeep, "x"))
    );

    let messages = [
        (
            DeclareError::new(DeclareKind::InvalidName, "A"),
            "`A` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`",
        ),
        (
            DeclareError::new(DeclareKind::TypeTooDeep, "x"),
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
    env.value("a", Type::int()).unwrap();
    env.value("a", Type::bool()).unwrap();
    assert_eq!(env.value_type("a"), Some(&Type::bool()));
    assert_eq!(compile("a", &env).unwrap().result_type(), Type::bool());

    env.function("f", [Type::int()], Type::int()).unwrap();
    env.function("f", [Type::int()], Type::bool()).unwrap();
    assert_eq!(compile("f(1)", &env).unwrap().result_type(), Type::bool());
    assert_eq!(
        error_in("f(true)", &env).0,
        "argument 1 of `f` must be Int, found Bool"
    );

    env.function("f", [Type::bool()], Type::int()).unwrap();
    assert_eq!(compile("f(true)", &env).unwrap().result_type(), Type::int());

    // A caller's function may share a name with a type's function.
    env.function("max", [Type::int(), Type::int()], Type::bool())
        .unwrap();
    assert_eq!(
        compile("max(1, 2)", &env).unwrap().result_type(),
        Type::bool()
    );
    assert_eq!(
        compile("Int::max(1, 2, 3)", &env).unwrap().result_type(),
        Type::int()
    );
    assert_eq!(
        error_in("Int::min()", &env).0,
        "`Int::min` takes at least 1 argument, found 0"
    );
}

#[test]
fn type_display() {
    let cases = [
        (Type::int(), "Int"),
        (Type::list(Type::option(Type::bool())), "List<Option<Bool>>"),
        (Type::record::<&str>([]), "{}"),
        (
            Type::record([("b", Type::int()), ("a", Type::string())]),
            "{a: String, b: Int}",
        ),
        (
            Type::record([
                ("x y", Type::int()),
                ("_1", Type::int()),
                ("1", Type::int()),
            ]),
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
        ("Int::min(1, 2, 3, 4)", (4, 0, 0)),
        ("Int::max(wits)", (1, 0, 0)),
        ("wits.clamp(1, 2)", (3, 0, 0)),
        ("bonus.unwrap_or(0) + 1", (3, 0, 0)),
        ("[1, 2, 3]", (1, 0, 3)),
        ("[-1, -2]", (1, 0, 2)),
        ("[true, false]", (1, 0, 2)),
        ("[10, 11, 12, 13].get(wits)", (2, 0, 4)),
        ("[[1, 2], [3]]", (1, 0, 5)),
        ("[wits, 2]", (2, 2, 0)),
        ("[[wits], [3]]", (2, 3, 1)),
        ("[wits + 1].get(0)", (2, 1, 0)),
        // Strings take a word per 8 bytes, and a repeat is stored once.
        (r#""""#, (1, 0, 0)),
        (r#""Essence (1e)""#, (1, 0, 2)),
        (r#"[name, "Essence (1e)", "Essence (1e)"]"#, (3, 3, 2)),
        (r#"chart(1, "abcdefgh") + chart(2, "abcdefgh")"#, (3, 0, 1)),
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
            ("wits", Type::int()),
            ("bonus", Type::option(Type::int())),
        ]
    );

    assert_eq!(
        program.functions(),
        [
            ("count", vec![track()], Type::int()),
            ("count", vec![track(), Type::int()], Type::int()),
        ]
    );

    // Builtins are the machine's own, not the caller's.
    assert!(
        compile("Int::min(1) + [1].get(1).unwrap_or(0)", &env())
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
    assert_eq!(code("[1].get(wits)"), [Const(0), WITS, Get(TypeId(0))]);
    assert_eq!(code("Int::min(1, wits, 3)"), [Int(1), WITS, Int(3), Min(3)]);
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
        "Int::min(wits, dexterity) + athletics",
        "[10, 11, 12, 13].get(blood_potency)",
        r#"chart(primal_urge, "Essence (1e)")"#,
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
        Err(error) if matches!(error.message(), Message::Invalid(..)) => format!("{error:#}"),
        Err(error) => panic!("expected an invalid program, found {error:#}"),
    }
}

#[test]
fn tampered_programs() {
    let source = "wits + [2, 3].get(1).unwrap_or(0) + count(health)";

    // Unchanged, it loads.
    assert!(load(&stored(source)).is_ok());

    type Tamper = Box<dyn Fn(&mut Stored)>;

    let cases: Vec<(&str, Tamper, &str)> = vec![
        (
            source,
            Box::new(|s| s.memory.stack -= 1),
            "it declares 7 words of memory but uses 8: while verifying the program",
        ),
        (
            source,
            Box::new(|s| s.memory.stack += 1),
            "it declares 9 words of memory but uses 8: while verifying the program",
        ),
        (
            source,
            Box::new(|s| s.memory.constants = 0),
            "it declares 6 words of memory but uses 8: while verifying the program",
        ),
        (
            "bonus",
            Box::new(|s| s.memory.globals = 1),
            "it declares 3 words of memory but uses 4: while verifying the program",
        ),
        (
            "[wits]",
            Box::new(|s| s.memory.heap = 0),
            "it declares 2 words of memory but uses 3: while verifying the program",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[2] = Instr::Lt),
            "it ends with 1 value on the stack, not one of its result type: while verifying the program",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code.pop().map(|_| ()).unwrap()),
            "it ends with 2 values on the stack, not one of its result type: while verifying the program",
        ),
        (
            "1",
            Box::new(|s| s.code = vec![Instr::Add]),
            "the instruction pops an empty stack: at instruction 0: while verifying the program",
        ),
        (
            "1",
            Box::new(|s| s.code = vec![]),
            "it ends with 0 values on the stack, not one of its result type: while verifying the program",
        ),
        (
            "wits",
            Box::new(|s| {
                s.code[0] = Instr::Global {
                    at: 1,
                    ty: TypeId(0),
                }
            }),
            "the instruction names a value there is not: at instruction 0: while verifying the program",
        ),
        (
            "wits",
            Box::new(|s| {
                s.code[0] = Instr::Global {
                    at: 0,
                    ty: TypeId(1),
                }
            }),
            "the instruction names a value there is not: at instruction 0: while verifying the program",
        ),
        (
            "[1]",
            Box::new(|s| s.code[0] = Instr::Const(7)),
            "the instruction names a constant there is not: at instruction 0: while verifying the program",
        ),
        (
            "count(health)",
            Box::new(|s| s.code[1] = Instr::Call(1)),
            "the instruction calls a function there is not: at instruction 1: while verifying the program",
        ),
        (
            "flag && flag",
            Box::new(|s| s.code[1] = Instr::And(2)),
            "the instruction jumps past the end: at instruction 1: while verifying the program",
        ),
        (
            "flag && flag && flag",
            Box::new(|s| s.code[1] = Instr::And(0)),
            "the instruction is reached with two different stacks: at instruction 2: while verifying the program",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[2] = Instr::Min(0)),
            "the instruction takes no operands: at instruction 2: while verifying the program",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 0,
                    entry: TypeId(0),
                }
            }),
            "the instruction makes a list of no entries: at instruction 1: while verifying the program",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 2,
                    entry: TypeId(0),
                }
            }),
            "the instruction makes a list of entries it does not find: at instruction 1: while verifying the program",
        ),
        (
            "[wits]",
            Box::new(|s| {
                s.code[1] = Instr::List {
                    len: 1,
                    entry: TypeId(1),
                }
            }),
            "the instruction makes a list of entries it does not find: at instruction 1: while verifying the program",
        ),
        (
            "specialties.athletics",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 9,
                    ty: TypeId(0),
                }
            }),
            "the instruction reads a field the record does not have: at instruction 1: while verifying the program",
        ),
        (
            "wits.abs()",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 0,
                    ty: TypeId(0),
                }
            }),
            "the instruction reads a field of other than a record: at instruction 1: while verifying the program",
        ),
        (
            "1 + 2",
            Box::new(|s| s.code[0] = Instr::Bool(true)),
            "the instruction finds an operand of the wrong type: at instruction 2: while verifying the program",
        ),
        (
            "bonus.unwrap_or(1)",
            Box::new(|s| s.code[1] = Instr::Bool(true)),
            "the instruction finds an operand of the wrong type: at instruction 2: while verifying the program",
        ),
        (
            "bonus.unwrap_or(1)",
            Box::new(|s| s.code[2] = Instr::UnwrapOr(TypeId(0))),
            "the instruction unwraps other than an option type: at instruction 2: while verifying the program",
        ),
        (
            "bonus!",
            Box::new(|s| s.code[1] = Instr::Unwrap(TypeId(0))),
            "the instruction unwraps other than an option type: at instruction 1: while verifying the program",
        ),
        (
            "bonus.is_none()",
            Box::new(|s| {
                s.types.push(TypeNode::Option(TypeId(1)));
                s.code[1] = Instr::IsNone(TypeId(3));
            }),
            "the instruction finds an operand of the wrong type: at instruction 1: while verifying the program",
        ),
        (
            "levels[0]",
            Box::new(|s| s.code[2] = Instr::Index(TypeId(1))),
            "the instruction indexes other than a list of its entry type: at instruction 2: while verifying the program",
        ),
        (
            "levels.get(0)",
            Box::new(|s| s.code[2] = Instr::Get(TypeId(1))),
            "the instruction gets from other than a list of its entry type: at instruction 2: while verifying the program",
        ),
        (
            "specialties.brawl",
            Box::new(|s| {
                s.code[1] = Instr::Field {
                    at: 1,
                    ty: TypeId(0),
                }
            }),
            "the instruction reads a field the record does not have: at instruction 1: while verifying the program",
        ),
        (
            "levels.len()",
            Box::new(|s| s.code[1] = Instr::IsSome(TypeId(2))),
            "the instruction tests other than an option type: at instruction 1: while verifying the program",
        ),
        (
            "1 == 1",
            Box::new(|s| s.code = vec![Instr::Const(0), Instr::Const(0), Instr::Eq]),
            "the instruction names a constant there is not: at instruction 0: while verifying the program",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![Scalar::Bool(true)],
                }
            }),
            "the constant has an entry of another type: at constant 0: while verifying the program",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![],
                }
            }),
            "the constant is an empty list: at constant 0: while verifying the program",
        ),
        (
            "[1]",
            Box::new(|s| {
                s.constants[0] = Constant::List {
                    entry: TypeId(0),
                    items: vec![Scalar::Const(0)],
                }
            }),
            "the constant refers to a later constant: at constant 0: while verifying the program",
        ),
        (
            "wits",
            Box::new(|s| s.globals[0].name = "Wits".into()),
            "the value's name is invalid or repeated: at value 0: while verifying the program",
        ),
        (
            "wits + dexterity",
            Box::new(|s| s.globals[1].name = "wits".into()),
            "the value's name is invalid or repeated: at value 1: while verifying the program",
        ),
        (
            "count(health)",
            Box::new(|s| s.functions.push(s.functions[0].clone())),
            "the function's name is invalid or its signature repeated: at function 1: while verifying the program",
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
            "a type is not in the type table: at function 0: while verifying the program",
        ),
        (
            "wits",
            Box::new(|s| s.result = TypeId(1)),
            "it ends with 1 value on the stack, not one of its result type: while verifying the program",
        ),
        (
            "wits",
            Box::new(|s| {
                s.globals[0] = Global {
                    name: "wits".into(),
                    ty: TypeId(5),
                }
            }),
            "a type is not in the type table: at value 0: while verifying the program",
        ),
        (
            "levels",
            Box::new(|s| s.types[2] = TypeNode::List(TypeId(2))),
            "the type refers to a later type: at type 2: while verifying the program",
        ),
        (
            "levels",
            Box::new(|s| s.types.push(TypeNode::Int)),
            "the type repeats an earlier type: at type 3: while verifying the program",
        ),
        (
            "specialties",
            Box::new(|s| {
                if let Some(TypeNode::Record(fields)) = s.types.last_mut() {
                    fields.reverse();
                }
            }),
            "the type has fields that are not sorted and unique: at type 3: while verifying the program",
        ),
        (
            "1",
            Box::new(|s| {
                for n in 0..64 {
                    s.types.push(TypeNode::List(TypeId(n + 1)));
                }
            }),
            "the type nests too deeply: at type 65: while verifying the program",
        ),
        (
            "1",
            Box::new(|s| s.types.clear()),
            "a type is not in the type table: while verifying the program",
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
            Err(error) if *error.message() == Message::NotDecoded => {}
            other => panic!("{len}: {other:?}"),
        }
    }

    let mut stored = stored("wits + 1");
    stored.format = FORMAT + 1;

    assert_eq!(
        format!("{:#}", load(&stored).unwrap_err()),
        "it was stored in format 4, this evalite reads format 3: while reading a stored program"
    );

    let error = Program::from_bytes(&[]).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "the bytes do not decode: while reading a stored program"
    );
    assert!(std::error::Error::source(&error).is_some());

    stored.format = FORMAT;
    stored.memory.stack = 0;
    let error = load(&stored).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "it declares 1 word of memory but uses 3: while verifying the program"
    );

    // The chain, frame by frame, as labels show it.
    let labels: Vec<(String, bool)> = error
        .labels()
        .map(|l| (l.message().to_string(), l.is_primary()))
        .collect();
    assert_eq!(
        labels,
        [
            ("it declares 1 word of memory but uses 3".to_owned(), true),
            ("while verifying the program".to_owned(), false),
        ]
    );

    // Equal totals spent otherwise read as such.
    stored.memory.stack = 1;
    stored.memory.globals = 2;
    assert_eq!(
        load(&stored).unwrap_err().to_string(),
        "it declares 3 words of memory, but uses them otherwise"
    );
}

/// Flipping bytes of stored programs never panics, and whatever loads is
/// a valid program.
#[test]
fn random_corruption() {
    let mut rng = Rng(0xc0de);
    let env = env();

    let sources = [
        "[10, 11, 12, 13].get(blood_potency).unwrap_or(0) + count(health, 2)",
        "flag && (wits > 1 || !maybe.unwrap_or(false))",
        r#"[[wits], [1, 2]][0][0] + m["Brawl (bar)"] + chart(1, "Essence")"#,
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
                    let f = rng.pick(&["Int::min", "Int::max"]);
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
                    let l = generate(rng, "List<Int>", b / 3);
                    let d = generate(rng, "Int", b / 3);
                    format!("{l}.get({i}).unwrap_or({d})")
                }
                9 => {
                    let l = generate(rng, "List<Int>", b / 2);
                    let i = generate(rng, "Int", b / 2);
                    format!("({l})[{i}]")
                }
                10 => format!("({}).len()", generate(rng, "List<Int>", b)),
                11 => format!("count(health, {})", generate(rng, "Int", b)),
                12 => format!(r#"chart({}, "Essence")"#, generate(rng, "Int", b)),
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
        "Option<Int>" => match rng.below(4) {
            0 => "bonus".to_owned(),
            1 => "specialties.brawl".to_owned(),
            2 => format!("label({})", generate(rng, "Int", budget.saturating_sub(1))),
            _ => {
                let b = budget.saturating_sub(1);
                let i = generate(rng, "Int", b / 2);
                let l = generate(rng, "List<Int>", b / 2);
                format!("{l}.get({i})")
            }
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
        "get",
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

        if let Err(errors) = compile(&source, &env) {
            for error in &errors {
                assert!(
                    !matches!(error.kind(), ErrorKind::Internal(..)),
                    "{source:?}: {error}"
                );
                let range = error.span().range();
                assert!(range.end <= source.len(), "{source:?}");
            }
        }
    }
}

#[test]
fn long_chains_do_not_recurse() {
    let roomy = Options::new().max_len(usize::MAX);
    let source = vec!["wits"; 50_000].join(" + ");
    let program = compile_with(&source, &env(), &roomy).unwrap();
    assert_eq!(program.memory().stack, 2);

    let source = format!("wits{}", ".abs()".repeat(50_000));
    assert_eq!(
        compile_with(&source, &env(), &roomy)
            .unwrap()
            .memory()
            .stack,
        1
    );

    let source = vec!["flag"; 50_000].join(" && ");
    assert_eq!(
        compile_with(&source, &env(), &roomy)
            .unwrap()
            .memory()
            .stack,
        1
    );
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

/// Suggesting a name for an unknown one costs nothing for an input name far
/// longer than every declared one, however long, so a huge name cannot make
/// compiling slow. Without the length check this is about 10^10 steps.
#[test]
fn long_unknown_names_are_cheap() {
    let mut env = Environment::new();

    for n in 0..1_000 {
        env.value(&format!("name_{n:04}"), Type::int()).unwrap();
    }

    env.value("strength", Type::int()).unwrap();

    let long = "a".repeat(10_000_000);
    let error = compile_with(&long, &env, &Options::new().max_len(usize::MAX)).unwrap_err();
    assert!(error.to_string().starts_with("unknown name"), "{error}");

    // A near miss is still suggested.
    let error = compile("strngth", &env).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unknown name `strngth`, did you mean `strength`?"
    );
}

/// Every diagnostic compiling `source` gives: its message and the text it
/// points at.
fn diagnostics_in(source: &str, env: &Environment) -> Vec<(String, String)> {
    let Err(errors) = compile(source, env) else {
        panic!("{source:?} compiled");
    };

    errors
        .iter()
        .map(|e| (e.to_string(), source[e.span().range()].to_owned()))
        .collect()
}

#[test]
fn several_errors_in_order() {
    let env = env();

    let found = diagnostics_in(
        "nothing + wits.nope() + levels[true] + [1, flag, name].len() + boxes()",
        &env,
    );

    let found = found
        .iter()
        .map(|(m, at)| (m.as_str(), at.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        found,
        [
            ("unknown name `nothing`", "nothing"),
            (
                "Int has no method `nope`, its methods are `abs`, `clamp`, `max` and `min`",
                "nope",
            ),
            ("a List<Int> is indexed by an Int, found Bool", "true"),
            (
                "entry 2 of this list is Bool, but the first is Int, a list's entries have one type",
                "flag",
            ),
            (
                "entry 3 of this list is String, but the first is Int, a list's entries have one type",
                "name",
            ),
            ("`boxes` takes 1 argument, found 0", "()"),
        ]
    );

    // Arguments and receivers of what is in error are still checked.
    assert_eq!(diagnostics_in("nope(nothing, wits)", &env).len(), 2);
    assert_eq!(diagnostics_in("nothing.abs(true + 1)", &env).len(), 2);
    assert_eq!(diagnostics_in("nothing[flag + 1]", &env).len(), 2);
    assert_eq!(diagnostics_in("m[nothing]", &env).len(), 2);
    assert_eq!(diagnostics_in("wits[-flag]", &env).len(), 2);
}

#[test]
fn one_error_is_reported_once() {
    let env = env();

    for source in [
        "wits + true + bonus",
        "(-(wits + true) * 2 < 3) == flag",
        "!nothing && flag || flag",
        "nothing == 1",
        "nothing.abs().max(1)",
        "nothing.boxes + 1",
        "nothing[0] + 1",
        "nothing[\"x\"] + 1",
        "nothing! + 1",
        "nothing.unwrap_or(1) + 2",
        "Int::min(nothing, 1) + 1",
        "count(nothing) + 1",
        "label(nothing).unwrap_or(0)",
        "[nothing, 1][0] + 1",
        "[1, nothing][0] + 1",
        "levels[nothing] + 1",
        "levels.get(nothing)!",
        "nope(1).abs()",
        "wits.nope(1).abs()",
        "(nothing)",
    ] {
        let found = diagnostics_in(source, &env);
        assert_eq!(found.len(), 1, "{source:?}: {found:?}");
    }
}

#[test]
fn every_kind_keeps_going() {
    let mut env = env();
    let mut deep = Type::int();

    for _ in 1..MAX_TYPE_DEPTH {
        deep = Type::list(deep);
    }

    env.value("deep", deep).unwrap();

    for (source, at) in [
        ("nope", "nope"),
        ("count", "count"),
        ("nope(1)", "nope"),
        ("wits(1)", "wits"),
        ("wits + true", "wits + true"),
        ("-flag", "-flag"),
        ("boxes()", "()"),
        ("label(true)", "true"),
        ("count(1)", "count(1)"),
        ("wits.nope()", "nope"),
        ("wits.x", "x"),
        ("health.nope", "nope"),
        ("wits[0]", "wits"),
        ("wits!", "wits!"),
        ("levels[true]", "true"),
        ("m[1]", "1"),
        ("[1, true]", "true"),
        ("[]", "[]"),
        ("[deep]", "[deep]"),
    ] {
        let source = format!("({source}) + nothing");
        let found = diagnostics_in(&source, &env);

        assert_eq!(found.len(), 2, "{source:?}: {found:?}");
        assert_eq!(found[0].1, at, "{source:?}: {found:?}");
        assert_eq!(found[1].0, "unknown name `nothing`", "{source:?}");
    }
}

#[test]
fn diagnostics_are_capped() {
    let env = Environment::new();
    let options = Options::new().max_len(usize::MAX);

    let at = |n: usize| (n * 4) as u32;

    for (count, more) in [
        (100, None),
        (101, Some(1)),
        (150, Some(50)),
        (5000, Some(4900)),
    ] {
        let source = vec!["x"; count].join(" + ");
        let errors = compile_with(&source, &env, &options).unwrap_err();
        let errors = errors.iter().collect::<Vec<_>>();

        let kept = errors.len().min(MAX_DIAGNOSTICS);

        for (n, error) in errors.iter().take(kept).enumerate() {
            assert_eq!(error.to_string(), "unknown name `x`");
            assert_eq!(error.span().start, at(n));
        }

        match more {
            None => assert_eq!(errors.len(), count),
            Some(more) => {
                assert_eq!(errors.len(), MAX_DIAGNOSTICS + 1);
                let last = errors[MAX_DIAGNOSTICS];
                assert_eq!(last.kind(), &ErrorKind::MoreErrors(more));
                assert_eq!(last.span().start, at(MAX_DIAGNOSTICS));
                let s = if more == 1 { "error" } else { "errors" };
                assert_eq!(last.to_string(), format!("and {more} more {s}"));
            }
        }
    }
}

#[test]
fn capped_diagnostics_keep_the_earliest() {
    // `[...]` entries are found after what is inside them, so the list's own
    // mistakes come later than the names past it would: the earliest are
    // kept whatever order they are found in.
    let env = env();
    let entries = vec!["true"; 150].join(", ");
    let names = vec!["x"; 150].join(" + ");
    let source = format!("[1, {entries}].len() + {names}");

    let errors = compile(&source, &env).unwrap_err();
    let errors = errors.iter().collect::<Vec<_>>();

    assert_eq!(errors.len(), MAX_DIAGNOSTICS + 1);

    for pair in errors.windows(2) {
        assert!(pair[0].span().start <= pair[1].span().start);
    }

    assert!(
        errors[..MAX_DIAGNOSTICS]
            .iter()
            .all(|e| matches!(e.kind(), ErrorKind::ListEntry { .. }))
    );
    assert_eq!(errors[MAX_DIAGNOSTICS].kind(), &ErrorKind::MoreErrors(200));
}
