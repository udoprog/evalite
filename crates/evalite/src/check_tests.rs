//! Tests of the type checker, compiler and stored programs.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::string_slice,
    clippy::indexing_slicing
)]

use std::ops::Range;
use std::time::{Duration, Instant};

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
            &format!("Option<Int> cannot be indexed, only a List, a Map or a record can{HINT}"),
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
            "Int cannot be indexed, only a List, a Map or a record can",
            "wits",
        ),
        (
            r#""abc"[0]"#,
            "String cannot be indexed, only a List, a Map or a record can",
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
        ("[]", EMPTY_LIST, "[]"),
        ("[].get(1)", EMPTY_LIST, "[]"),
        ("[[]]", EMPTY_LIST, "[]"),
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
        env.value("z", Type::list(deep)),
        Err(DeclareError::new(DeclareKind::TypeTooDeep, "z"))
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

/// A record of `fields` `Int` fields, `fields + 1` nodes written out.
fn wide(fields: usize) -> Type {
    Type::record((0..fields).map(|i| (format!("f{i}"), Type::int())))
}

#[test]
fn types_with_too_many_nodes_are_refused_at_declaration() {
    let mut env = Environment::new();

    // The most a type may have is accepted, one more is refused, whatever
    // its depth.
    assert_eq!(env.value("x", wide(MAX_TYPE_NODES - 1)), Ok(()));
    assert_eq!(
        env.value("y", wide(MAX_TYPE_NODES)),
        Err(DeclareError::new(DeclareKind::TypeTooLarge, "y"))
    );
    assert_eq!(
        env.value("y", wide(70_000)),
        Err(DeclareError::new(DeclareKind::TypeTooLarge, "y"))
    );
    assert_eq!(env.value_type("y"), None);

    assert_eq!(
        env.function("f", [wide(70_000)], Type::int()),
        Err(DeclareError::new(DeclareKind::TypeTooLarge, "f"))
    );
    assert_eq!(
        env.function("f", [], wide(70_000)),
        Err(DeclareError::new(DeclareKind::TypeTooLarge, "f"))
    );
    assert_eq!(
        env.function("f", [], Type::list(wide(MAX_TYPE_NODES - 1))),
        Err(DeclareError::new(DeclareKind::TypeTooLarge, "f"))
    );

    assert_eq!(
        DeclareError::new(DeclareKind::TypeTooLarge, "y").to_string(),
        "the type of `y` has more than 65536 parts once written out, which is the most a type may have"
    );

    // What is declared compiles, so the compiler never meets a type it
    // cannot hold.
    assert_eq!(
        compile("x.f0 + x.f65534", &env).unwrap().result_type(),
        Type::int()
    );
}

#[test]
fn a_value_over_the_type_limit_is_its_own_error() {
    let mut env = Environment::new();
    env.value("x", wide(MAX_TYPE_NODES - 1)).unwrap();

    let (message, at) = error_in("[x]", &env);
    assert_eq!(
        message,
        "this value's type has more than 65536 parts once written out, which is the most a type may have"
    );
    assert_eq!(at, 0..3);

    // Not a nesting error.
    let (message, _) = error_in("[x].len()", &env);
    assert!(!message.contains("nests"), "{message}");
}

#[test]
fn globals_over_the_total_are_a_diagnostic() {
    let mut env = Environment::new();
    let names = (0..40).map(|i| format!("g{i}")).collect::<Vec<_>>();

    for name in &names {
        env.value(name, wide(60_000)).unwrap();
    }

    let source = names
        .iter()
        .map(|name| format!("{name}.f0"))
        .collect::<Vec<_>>()
        .join(" + ");

    let errors = diagnostics_in(&source, &env);
    let max = crate::program::MAX_GLOBAL_TYPE_NODES;

    // Each is 60001 nodes, so the 35th is the first to pass 2^21.
    let (message, at) = errors.first().unwrap();
    assert_eq!(*at, "g34");
    assert_eq!(
        message,
        &format!(
            "the types of the values and functions used so far have more than {max} parts together once written out, which is the most an expression's may have"
        )
    );
    assert!(
        errors.iter().all(|(m, _)| !m.contains("internal")),
        "{errors:?}"
    );

    // Under the total, the same globals compile.
    let ok = names
        .iter()
        .take(34)
        .map(|name| format!("{name}.f0"))
        .collect::<Vec<_>>()
        .join(" + ");
    assert_eq!(compile(&ok, &env).unwrap().result_type(), Type::int());
}

#[test]
fn functions_count_to_the_total_too() {
    let mut env = Environment::new();
    let names = (0..40).map(|i| format!("f{i}")).collect::<Vec<_>>();

    for name in &names {
        env.function(name, [], wide(60_000)).unwrap();
    }

    let source = names
        .iter()
        .map(|name| format!("{name}().f0"))
        .collect::<Vec<_>>()
        .join(" + ");

    let errors = diagnostics_in(&source, &env);
    let (message, at) = errors.first().unwrap();
    assert!(
        message.starts_with("the types of the values and functions"),
        "{message}"
    );
    assert_eq!(*at, "f34()");
    assert!(
        errors.iter().all(|(m, _)| !m.contains("internal")),
        "{errors:?}"
    );
}

#[test]
fn redeclaring() {
    let mut env = Environment::new();
    env.value("a", Type::int()).unwrap();

    // A value's slot never moves, so its name is declared once, whatever
    // the type, and the first declaration stands.
    for ty in [Type::bool(), Type::int()] {
        assert_eq!(
            env.value("a", ty),
            Err(DeclareError::new(DeclareKind::Redeclared, "a"))
        );
    }

    assert_eq!(env.value_type("a"), Some(&Type::int()));
    assert_eq!(compile("a", &env).unwrap().result_type(), Type::int());
    assert_eq!(env.slots.len(), 1);

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

    // The values a program reads are in the store, not the machine: only
    // the words it pushes them to count.
    assert_eq!(compile("[wits, 2]", &env()).unwrap().memory().total(), 4);
}

#[test]
fn globals_and_functions() {
    let program = compile(
        "count(health) + count(health, wits) + count(health) + wits + bonus.unwrap_or(wits)",
        &env(),
    )
    .unwrap();

    assert_eq!(
        program.globals().collect::<Vec<_>>(),
        [
            ("health", &track()),
            ("wits", &Type::int()),
            ("bonus", &Type::option(Type::int())),
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
                at: 7,
                ty: TypeId(1)
            },
            And(3),
            Global {
                at: 0,
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
                at: 7,
                ty: TypeId(1)
            },
            Or(3),
            Global {
                at: 7,
                ty: TypeId(1)
            },
            And(1),
            Global {
                at: 7,
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
                at: 8,
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
                at: 16,
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
                at: 17,
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
                at: 11,
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
                at: 14,
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
        assert_eq!(
            loaded.globals().collect::<Vec<_>>(),
            program.globals().collect::<Vec<_>>(),
            "{source:?}"
        );
        assert_eq!(loaded.functions(), program.functions(), "{source:?}");
        assert_eq!(loaded.to_bytes().unwrap(), bytes, "{source:?}");
    }
}

fn stored(source: &str) -> Stored {
    let bytes = compile(source, &env()).unwrap().to_bytes().unwrap();
    crate::framed::body(&bytes)
}

fn load(stored: &Stored) -> Result<Program, StoreError> {
    Program::from_bytes(&crate::framed::to_bytes(FORMAT, stored).unwrap())
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
            "it declares 5 words of memory but uses 6: while verifying the program",
        ),
        (
            source,
            Box::new(|s| s.memory.stack += 1),
            "it declares 7 words of memory but uses 6: while verifying the program",
        ),
        (
            source,
            Box::new(|s| s.memory.constants = 0),
            "it declares 4 words of memory but uses 6: while verifying the program",
        ),
        (
            "[wits]",
            Box::new(|s| s.memory.heap = 0),
            "it declares 1 word of memory but uses 2: while verifying the program",
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
            // A global moved to another slot, which the code still reads at
            // the old one.
            "wits",
            Box::new(|s| s.globals[0].slot = 3),
            "the instruction names a value there is not: at instruction 0: while verifying the program",
        ),
        (
            // `bonus` is two words from 8, and `dexterity` moved into its
            // second, so binding one would change what the other reads.
            "bonus.unwrap_or(dexterity)",
            Box::new(|s| {
                let at = s
                    .globals
                    .iter()
                    .position(|g| g.name == "dexterity")
                    .unwrap();
                s.globals[at].slot = 9;

                for instr in &mut s.code {
                    if let Instr::Global { at: 1, ty } = *instr {
                        *instr = Instr::Global { at: 9, ty };
                    }
                }
            }),
            "the value is held in words another value is held in: at value 1: while verifying the program",
        ),
        (
            "bonus.unwrap_or(dexterity)",
            Box::new(|s| {
                let at = s.globals.iter().position(|g| g.name == "bonus").unwrap();
                s.globals[at].slot = 0;
                let at = s
                    .globals
                    .iter()
                    .position(|g| g.name == "dexterity")
                    .unwrap();
                s.globals[at].slot = 0;
            }),
            "the value is held in words another value is held in: at value 0: while verifying the program",
        ),
        (
            // A slot ending past the last word.
            "bonus",
            Box::new(|s| {
                s.globals[0].slot = u32::MAX;
                s.code[0] = Instr::Global {
                    at: u32::MAX,
                    ty: s.globals[0].ty,
                };
            }),
            "it uses too much memory to count: at value 0: while verifying the program",
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
                    slot: 0,
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
                s.types.push(TypeNode::Map {
                    key: TypeId(1),
                    value: TypeId(0),
                })
            }),
            "the type is a map keyed by other than Int or String: at type 2: while verifying the program",
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
            // Each record holds the one before twice, so 62 of them stand
            // for about 2^62 nodes, and are refused at the first past the
            // limit without being written out.
            "1",
            Box::new(|s| {
                for n in 0..62 {
                    let part = TypeId(n + 1);
                    s.types.push(TypeNode::Record(vec![
                        FieldNode {
                            name: "a".into(),
                            ty: part,
                        },
                        FieldNode {
                            name: "b".into(),
                            ty: part,
                        },
                    ]));
                }
            }),
            "the type is too large: at type 17: while verifying the program",
        ),
        (
            // Large types are fine one by one, but not as many globals.
            "1",
            Box::new(|s| {
                for n in 0..15 {
                    let part = TypeId(n + 1);
                    s.types.push(TypeNode::Record(vec![
                        FieldNode {
                            name: "a".into(),
                            ty: part,
                        },
                        FieldNode {
                            name: "b".into(),
                            ty: part,
                        },
                    ]));
                }

                for n in 0..40 {
                    s.globals.push(Global {
                        name: format!("g{n}"),
                        ty: TypeId(16),
                        slot: n,
                    });
                }
            }),
            "the type is too large: while verifying the program",
        ),
        (
            // Each global is small, and so are all of them together, but
            // four chains of 20 records over one large type (32767 nodes)
            // are 20 large types each to hold while building.
            "1",
            Box::new(|s| {
                for n in 0..14 {
                    let part = TypeId(n + 1);
                    s.types.push(TypeNode::Record(vec![
                        FieldNode {
                            name: "a".into(),
                            ty: part,
                        },
                        FieldNode {
                            name: "b".into(),
                            ty: part,
                        },
                    ]));
                }

                let base = u32::try_from(s.types.len()).unwrap_or(u32::MAX) - 1;
                let mut next = base + 1;

                for chain in 0..4 {
                    let mut part = TypeId(base);

                    for _ in 0..20 {
                        s.types.push(TypeNode::Record(vec![FieldNode {
                            name: format!("c{chain}"),
                            ty: part,
                        }]));
                        part = TypeId(next);
                        next += 1;
                    }

                    s.globals.push(Global {
                        name: format!("g{chain}"),
                        ty: part,
                        slot: chain,
                    });
                }
            }),
            "the type is too large: while verifying the program",
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

/// Load `stored`, failing when it takes longer than a few seconds. Inputs
/// are sized so that work quadratic in them would take far longer.
fn load_quickly(stored: &Stored) -> Result<Program, StoreError> {
    let bytes = crate::framed::to_bytes(FORMAT, stored).unwrap();
    let start = Instant::now();
    let loaded = Program::from_bytes(&bytes);
    let took = start.elapsed();
    assert!(took < Duration::from_secs(5), "loading took {took:?}");
    loaded
}

#[test]
fn deep_stacks_with_many_jumps_verify_quickly() {
    const DEPTH: usize = 100_000;
    const JUMPS: usize = 100_000;

    // A deep stack of `Bool`s, the top one `&&`ed many times, then folded
    // with `==`: each jump lands with the whole stack.
    let mut deep = stored("1 == 1");
    deep.code = std::iter::repeat_n(Instr::Bool(true), DEPTH + 1)
        .chain((0..JUMPS).flat_map(|_| [Instr::And(1), Instr::Bool(true)]))
        .chain(std::iter::repeat_n(Instr::Eq, DEPTH))
        .collect();
    deep.memory.stack = u32::try_from(DEPTH + 1).unwrap();

    load_quickly(&deep).unwrap();

    // Going on with another type than a jump over it keeps is found where
    // they meet.
    let at = DEPTH + 1 + JUMPS + 1;
    deep.code[at] = Instr::Int(1);
    let error = load_quickly(&deep).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        format!(
            "the instruction is reached with two different stacks: at instruction {}: while verifying the program",
            at + 1
        )
    );

    // Two jumps landing at the end with deep stacks of one depth, which
    // differ near the top, disagree.
    let mut disagree = stored("1 == 1");
    disagree.code = std::iter::repeat_n(Instr::Bool(true), DEPTH + 1)
        .chain([
            Instr::And(4),
            Instr::Eq,
            Instr::Int(1),
            Instr::Bool(true),
            Instr::And(0),
        ])
        .collect();
    let error = load_quickly(&disagree).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        format!(
            "the instruction jumps where another jump lands with another stack: at instruction {}: while verifying the program",
            DEPTH + 5
        )
    );
}

#[test]
fn many_reads_of_a_wide_record_compile_and_verify_quickly() {
    // With the record itself, the most nodes a type may have.
    const FIELDS: usize = 65_535;
    const READS: usize = 50_000;

    let mut env = Environment::new();
    let fields = (0..FIELDS).map(|n| (format!("f{n:05}"), Type::int()));
    env.value("x", Type::record(fields)).unwrap();

    // The last field, which a scan of the fields finds last.
    let source = vec![format!("x.f{:05}", FIELDS - 1); READS].join(" + ");
    let options = Options::new().max_len(usize::MAX);

    let start = Instant::now();
    let program = compile_with(&source, &env, &options).unwrap();
    let took = start.elapsed();
    assert!(took < Duration::from_secs(5), "compiling took {took:?}");

    let bytes = program.to_bytes().unwrap();
    load_quickly(&crate::framed::body(&bytes)).unwrap();
}

/// The id of the next type pushed onto a stored table.
fn next_type(stored: &Stored) -> TypeId {
    TypeId(u32::try_from(stored.types.len()).unwrap())
}

/// The id of `node` in a stored table.
fn stored_type(stored: &Stored, node: &TypeNode) -> TypeId {
    let at = stored.types.iter().position(|n| n == node).unwrap();
    TypeId(u32::try_from(at).unwrap())
}

#[test]
fn functions_types_are_bounded_together() {
    // Each record holds the one before twice, over an `Int`, so the last
    // of 15 is 65 535 nodes written out, which one type may be. Unused
    // functions, unique by name, returning it are as many such types to
    // hand out from `Program::functions`.
    let declaring = |functions: usize| {
        let mut stored = stored("1");
        let mut part = stored_type(&stored, &TypeNode::Int);

        for _ in 0..15 {
            let record = next_type(&stored);
            stored.types.push(TypeNode::Record(vec![
                FieldNode {
                    name: "a".into(),
                    ty: part,
                },
                FieldNode {
                    name: "b".into(),
                    ty: part,
                },
            ]));
            part = record;
        }

        for n in 0..functions {
            stored.functions.push(HostFunction {
                name: format!("f{n}"),
                params: Vec::new(),
                ret: part,
            });
        }

        stored
    };

    let loaded = load_quickly(&declaring(2)).unwrap();
    assert_eq!(loaded.functions().len(), 2);

    let many = declaring(100_000);
    assert_eq!(
        format!("{:#}", load_quickly(&many).unwrap_err()),
        "the type is too large: while verifying the program"
    );

    // Many small types in a large table are fine, and listing them walks
    // the table once, not once a function.
    const TYPES: usize = 100_000;
    let mut small = stored("1");
    let int = stored_type(&small, &TypeNode::Int);
    let first = next_type(&small);

    for n in 0..TYPES {
        small.types.push(TypeNode::Record(vec![FieldNode {
            name: format!("f{n}"),
            ty: int,
        }]));
    }

    for n in 0..TYPES {
        small.functions.push(HostFunction {
            name: format!("f{n}"),
            params: vec![int],
            ret: TypeId(first.0 + u32::try_from(n).unwrap()),
        });
    }

    let loaded = load_quickly(&small).unwrap();
    let start = Instant::now();
    let functions = loaded.functions();
    let took = start.elapsed();
    assert!(took < Duration::from_secs(5), "listing took {took:?}");
    assert_eq!(functions.len(), TYPES);
    assert_eq!(
        functions[TYPES - 1],
        (
            "f99999",
            vec![Type::int()],
            Type::record([("f99999", Type::int())])
        )
    );
}

#[test]
fn shared_map_keys_verify_quickly() {
    const MAPS: usize = 100_000;

    // Two keys of 8 MiB, alike but for their last byte, so checking that
    // they are sorted compares them whole. Comparing is fast, so they are
    // long enough for comparing them once a map to show.
    let long = |last: char| format!("{}{last}", "x".repeat(8 << 20));
    let source = format!("{{{:?}: 1, {:?}: 2}}.len()", long('a'), long('b'));
    let options = Options::new().max_len(usize::MAX);
    let program = compile_with(&source, &Environment::new(), &options).unwrap();
    let one: Stored = crate::framed::body(&program.to_bytes().unwrap());

    // Many maps of the one key constant, their lengths summed.
    let mut many: Stored = crate::framed::body(&program.to_bytes().unwrap());
    many.code = one
        .code
        .iter()
        .copied()
        .chain((0..MAPS).flat_map(|_| one.code.iter().copied().chain([Instr::Add])))
        .collect();
    many.memory.stack = one.memory.stack + 1;
    many.memory.heap = one.memory.heap * u32::try_from(MAPS + 1).unwrap();

    load_quickly(&many).unwrap();
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

    // A later format is refused by its number, before its body is read,
    // whatever that body's layout.
    let mut newer = musli::storage::to_vec(&(FORMAT + 1)).unwrap();
    newer.extend([0xff, 0x00, 0x13, 0x37]);

    assert_eq!(
        format!("{:#}", Program::from_bytes(&newer).unwrap_err()),
        format!(
            "it was stored in format {}, this evalite reads format {FORMAT}: while reading a stored program",
            FORMAT + 1
        )
    );

    let error = Program::from_bytes(&[]).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "the bytes do not decode: while reading a stored program"
    );
    assert!(std::error::Error::source(&error).is_some());

    let mut stored = stored("wits + 1");
    stored.memory.stack = 0;
    let error = load(&stored).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "it declares 0 words of memory but uses 2: while verifying the program"
    );

    // The chain, frame by frame, as labels show it.
    let labels: Vec<(String, bool)> = error
        .labels()
        .map(|l| (l.message().to_string(), l.is_primary()))
        .collect();
    assert_eq!(
        labels,
        [
            ("it declares 0 words of memory but uses 2".to_owned(), true),
            ("while verifying the program".to_owned(), false),
        ]
    );

    // Equal totals spent otherwise read as such.
    stored.memory.stack = 1;
    stored.memory.heap = 1;
    assert_eq!(
        load(&stored).unwrap_err().to_string(),
        "it declares 2 words of memory, but uses them otherwise"
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

            match rng.below(15) {
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
                13 => {
                    // Keys in any order, distinct, decimal or hex.
                    let n = 1 + rng.below(3);
                    let start = rng.below(4) as i64 - 2;
                    let entries = (0..n)
                        .map(|k| {
                            let key = start + 3 * k as i64 * if rng.below(2) == 0 { 1 } else { -1 };
                            let key = match rng.below(2) {
                                0 if key >= 0 => format!("0x{key:X}"),
                                _ => key.to_string(),
                            };
                            format!("{key}: {}", generate(rng, "Int", b / (n + 1)))
                        })
                        .collect::<Vec<_>>();
                    let at = generate(rng, "Int", b / (n + 1));
                    format!("{{{}}}.get({at}).unwrap_or(0)", entries.join(", "))
                }
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
        "{",
        "}",
        ":",
        "0x1",
        "-0x1",
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

/// A syntax error after a map does not hide the map's own errors: only a
/// key's own text is looked into for syntax errors.
#[test]
fn map_errors_before_a_syntax_error() {
    let env = env();

    let found = diagnostics_in("[{1: true, 2: 3}, {1 + wits: 1}, (]", &env);
    let found = found
        .iter()
        .map(|(m, at)| (m.as_str(), at.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        found,
        [
            (
                "the value of entry 2 of this map is Int, but the first is Bool, a map's values have one type",
                "3",
            ),
            (
                "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
                "1 + wits",
            ),
            ("expected an expression, found `]`", "]"),
        ]
    );
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

/// The environment of the map tests: records by `Int` key and options by
/// `String` key.
fn map_env() -> Environment {
    let mut env = env();
    let dice = Type::record([("dice", Type::int())]);
    env.value("ids", Type::map(Type::int(), dice)).unwrap();
    env.value("tags", Type::map(Type::string(), Type::option(Type::int())))
        .unwrap();
    env
}

/// `m[k]` gives a map's value type and `get` an option of it, the key of
/// its key type; a map is no record, and has no other methods.
#[test]
fn maps() {
    let env = map_env();
    let type_in = |source: &str| match compile(source, &env) {
        Ok(program) => program.result_type().to_string(),
        Err(error) => panic!("{source:?}: {error}"),
    };

    for (source, expected) in [
        ("ids", "Map<Int, {dice: Int}>"),
        ("ids[1]", "{dice: Int}"),
        ("ids[wits].dice", "Int"),
        ("ids.get(1)", "Option<{dice: Int}>"),
        ("ids.get(1)!.dice", "Int"),
        ("ids.len()", "Int"),
        ("Map::get(ids, 2)", "Option<{dice: Int}>"),
        ("Map::len(tags)", "Int"),
        ("tags[\"a\"]", "Option<Int>"),
        ("tags[name]", "Option<Int>"),
        ("tags.get(name)", "Option<Option<Int>>"),
        ("[ids, ids][0][3]", "{dice: Int}"),
    ] {
        assert_eq!(type_in(source), expected, "{source:?}");
    }

    for (source, message, at) in [
        (
            "ids[\"a\"]",
            "a Map<Int, {dice: Int}> is indexed by its key, an Int, found String",
            "\"a\"",
        ),
        (
            "tags[1]",
            "a Map<String, Option<Int>> is indexed by its key, a String, found Int",
            "1",
        ),
        (
            "ids.get(name)",
            "argument 1 of `get` on Map<Int, {dice: Int}> must be Int, found String",
            "name",
        ),
        (
            "Map::get(tags, 1)",
            "argument 2 of `Map::get` must be String, found Int",
            "1",
        ),
        (
            "Map::len(5)",
            "argument 1 of `Map::len` must be Map<K, V>, found Int",
            "5",
        ),
        (
            "ids.dice",
            "Map<Int, {dice: Int}> has no fields, so no `.dice`",
            "dice",
        ),
        (
            "ids.keys()",
            "Map<Int, {dice: Int}> has no method `keys`, its methods are `get` and `len`",
            "keys",
        ),
        (
            "ids == ids",
            "`==` takes two Ints or two Bools, found Map<Int, {dice: Int}> and Map<Int, {dice: Int}>",
            "ids == ids",
        ),
    ] {
        let (actual, range) = error_in(source, &env);
        assert_eq!(actual, message, "{source:?}");
        assert_eq!(&source[range.clone()], at, "{source:?}: {range:?}");
    }
}

/// A map is keyed by `Int` or `String`, and any other key is refused where
/// the type is declared.
#[test]
fn map_keys_are_int_or_string() {
    let mut env = Environment::new();
    let message = "the type of `m` has a map keyed by other than Int or String, a map's keys are Int or String";

    for key in [
        Type::bool(),
        Type::list(Type::int()),
        Type::option(Type::int()),
        Type::record([("a", Type::int())]),
        Type::map(Type::int(), Type::int()),
    ] {
        let ty = Type::map(key.clone(), Type::int());
        assert_eq!(
            env.value("m", ty.clone()).unwrap_err().to_string(),
            message,
            "{key}"
        );
        assert_eq!(
            env.function("m", [ty.clone()], Type::int())
                .unwrap_err()
                .to_string(),
            message
        );
        assert_eq!(
            env.function("m", [], Type::list(ty))
                .unwrap_err()
                .to_string(),
            message
        );
    }

    env.value(
        "m",
        Type::map(Type::int(), Type::map(Type::string(), Type::int())),
    )
    .unwrap();
}

/// A map literal: keys are integer or string literals, of one type and
/// each given once; values are of one type.
#[test]
fn map_literals() {
    types(&[
        ("{1: wits}", "Map<Int, Int>"),
        ("{1: wits, 0x2A: 2, -3: -0x4}", "Map<Int, Int>"),
        (r#"{"a": flag, "b c": true}"#, "Map<String, Bool>"),
        (r#"{1: "x"}[1]"#, "String"),
        ("{1: health}[1].boxes", "Int"),
        ("{1: [1, 2]}.get(1)", "Option<List<Int>>"),
        ("{1: {\"a\": bonus}}", "Map<Int, Map<String, Option<Int>>>"),
        ("{1: 2}.len()", "Int"),
        ("Map::len({1: 2})", "Int"),
        ("[{1: 2}, {3: 4}]", "List<Map<Int, Int>>"),
        (
            "{9223372036854775807: 1, -9223372036854775807: 2}",
            "Map<Int, Int>",
        ),
    ]);

    errors(&[
        ("{}", EMPTY_MAP, "{}"),
        ("{1: {}}", EMPTY_MAP, "{}"),
        (
            "{1 + 2: 1}",
            "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "1 + 2",
        ),
        (
            "{(1): 1}",
            "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "(1)",
        ),
        (
            "{--1: 1}",
            "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "--1",
        ),
        (
            "{true: 1}",
            "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "true",
        ),
        (
            "{-\"a\": 1}",
            "a map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "-\"a\"",
        ),
        (
            "{1: 2, \"a\": 3}",
            "key 2 of this map is String, but the first is Int, a map's keys have one type",
            "\"a\"",
        ),
        (
            "{\"a\": 2, 1: 3, 2: 4}",
            "key 2 of this map is Int, but the first is String, a map's keys have one type\nkey 3 of this map is Int, but the first is String, a map's keys have one type",
            "1",
        ),
        (
            "{1: 2, 0x1: 3}",
            "the key 1 is given twice in this map",
            "0x1",
        ),
        (
            "{16: 2, 3: 4, 0x10: 5}",
            "the key 16 is given twice in this map",
            "0x10",
        ),
        (
            "{-1: 2, -0x1: 3}",
            "the key -1 is given twice in this map",
            "-0x1",
        ),
        (
            r#"{"a": 1, "b": 2, "a": 3}"#,
            "the key \"a\" is given twice in this map",
            "\"a\"",
        ),
        (
            "{1: 1, 1: 2, 1: 3}",
            "the key 1 is given twice in this map\nthe key 1 is given twice in this map",
            "1",
        ),
        (
            "{1: true, 2: 3}",
            "the value of entry 2 of this map is Int, but the first is Bool, a map's values have one type",
            "3",
        ),
        // In error by key order or not, the first written is the first.
        (
            "{2: true, 1: 3, 3: 4}",
            "the value of entry 2 of this map is Int, but the first is Bool, a map's values have one type\nthe value of entry 3 of this map is Int, but the first is Bool, a map's values have one type",
            "3",
        ),
        (
            "{1: 2}[\"a\"]",
            "a Map<Int, Int> is indexed by its key, an Int, found String",
            "\"a\"",
        ),
        (
            "{1: 2} == {1: 2}",
            "`==` takes two Ints or two Bools, found Map<Int, Int> and Map<Int, Int>",
            "{1: 2} == {1: 2}",
        ),
        // Values are checked when keys are wrong, keys only when literal.
        (
            "{1: nope, wits + 1: 2}",
            "unknown name `nope`\na map's keys are integer or string literals, such as `1`, `-0x2A` or `\"x\"`",
            "nope",
        ),
        (
            "{1: nope, 1: 2}",
            "unknown name `nope`\nthe key 1 is given twice in this map",
            "nope",
        ),
        // A syntax error is not reported again.
        (
            "{1, 2: x}",
            "expected an operator or `:` after this map key, found `,`\nunknown name `x`",
            ",",
        ),
        (
            "{9223372036854775808: 1}",
            "this integer is too large, the largest is 9223372036854775807",
            "9223372036854775808",
        ),
        (
            "{-9223372036854775808: 1}",
            "this integer is too large, the largest is 9223372036854775807",
            "9223372036854775808",
        ),
    ]);
}

#[test]
fn record_literal_types() {
    types(&[
        ("{b: 1, a: true}", "{a: Bool, b: Int}"),
        (
            "{boxes: wits, count: levels}",
            "{boxes: Int, count: List<Int>}",
        ),
        ("{x: {y: [1]}}", "{x: {y: List<Int>}}"),
        ("{x: {y: [1]}}.x.y", "List<Int>"),
        ("{name: name}", "{name: String}"),
        // Each field is expected to be the field of the record expected.
        (
            "[specialties, {brawl: None, athletics: 1}]",
            "List<{athletics: Int, brawl: Option<Int>}>",
        ),
        ("[{a: Some(1)}, {a: None}]", "List<{a: Option<Int>}>"),
        ("count({count: [1], boxes: 2})", "Int"),
        (
            "Some({a: Some(true), b: 2}).unwrap_or({b: 1, a: None})",
            "{a: Option<Bool>, b: Int}",
        ),
        // String keys still make a map.
        ("{\"a\": 1}", "Map<String, Int>"),
    ]);
}

#[test]
fn record_literal_errors() {
    errors(&[
        (
            "{a: 1, 2: nope}",
            "this key is no name, but the first is, which makes this a record, whose fields are all names, such as `{a: 1, b: 2}`\nunknown name `nope`",
            "2",
        ),
        (
            "{a: 1, \"b\": 2}",
            "this key is no name, but the first is, which makes this a record, whose fields are all names, such as `{a: 1, b: 2}`",
            "\"b\"",
        ),
        (
            "{1: 1, a: 2}",
            "this key is a name, but the first is a literal, which makes this a map, whose keys are all integer or string literals, such as `{\"a\": 1}`",
            "a",
        ),
        (
            "{a: 1, b: 2, a: 3}",
            "the field `a` is given twice in this record",
            "a",
        ),
        (
            "{a: None}",
            "`None` needs a type from where it is used, such as beside `Some(...)` in a list or as an argument of an option",
            "None",
        ),
        (
            "count({boxes: 1})",
            "a {boxes: Int, count: List<Int>} is expected here, but this record lacks its field `count`",
            "{boxes: 1}",
        ),
        (
            "count({boxes: 1, count: [1], dots: 2, extra: 3})",
            "a {boxes: Int, count: List<Int>} is expected here, which has no field `dots`\na {boxes: Int, count: List<Int>} is expected here, which has no field `extra`",
            "dots",
        ),
        (
            "[specialties, {other: 1}]",
            "a {athletics: Int, brawl: Option<Int>} is expected here, but this record lacks its fields `athletics`, `brawl`\na {athletics: Int, brawl: Option<Int>} is expected here, which has no field `other`",
            "{other: 1}",
        ),
        (
            "count({boxes: true, count: [1]})",
            "no `count` takes ({boxes: Bool, count: List<Int>}), it takes ({boxes: Int, count: List<Int>}) or ({boxes: Int, count: List<Int>}, Int)",
            "count({boxes: true, count: [1]})",
        ),
        (
            "{a: 1}.b",
            "this record has no field \"b\", its fields are \"a\"",
            "b",
        ),
        (
            "{a: 1} == {a: 1}",
            "`==` takes two Ints or two Bools, found {a: Int} and {a: Int}",
            "{a: 1} == {a: 1}",
        ),
        ("{a: nope, b: 1}.b", "unknown name `nope`", "nope"),
    ]);
}

/// A repeated field points at itself and at where it is first given.
#[test]
fn record_literal_repeated_field_labels() {
    let source = "{b: 1, a: 2, b: 3}";
    let errors = compile(source, &env()).unwrap_err();
    let error = errors.iter().next().unwrap();
    let labels = error
        .labels()
        .map(|l| {
            let at = l.span().map(|s| &source[s.range()]);
            (l.message().to_string(), at, l.is_primary())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            (
                "the field `b` is given twice in this record".to_owned(),
                Some("b"),
                true
            ),
            ("given first here".to_owned(), Some("b"), false),
        ]
    );
    let spans = error
        .labels()
        .filter_map(|l| l.span())
        .map(|s| s.range().start)
        .collect::<Vec<_>>();
    assert_eq!(spans, [13, 1]);
}

/// A record literal's values are compiled in name order, then made into
/// the record.
#[test]
fn record_literal_code() {
    use Instr::{Global, Int, Record};

    let program = compile("{b: wits, a: 1}", &env()).unwrap();
    let record = program.result;
    assert_eq!(
        program.code,
        [
            Int(1),
            Global {
                at: 0,
                ty: TypeId(0)
            },
            Record(record),
        ]
    );
}

/// A repeated key points at itself and at where it is first given.
#[test]
fn map_literal_repeated_key_labels() {
    let source = "{0x10: 1, 2: 3, 16: 4}";
    let errors = compile(source, &env()).unwrap_err();
    let error = errors.iter().next().unwrap();
    assert_eq!(
        error.kind(),
        &ErrorKind::RepeatedKey {
            key: "16".to_owned(),
            first: crate::lexer::span(1, 5),
        }
    );

    let labels = error
        .labels()
        .map(|l| {
            let at = l.span().map(|s| &source[s.range()]);
            (l.message().to_string(), at, l.is_primary())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            (
                "the key 16 is given twice in this map".to_owned(),
                Some("16"),
                true
            ),
            ("given first here".to_owned(), Some("0x10"), false),
        ]
    );
}

/// A map literal's values are compiled in key order, then made into the
/// map with its keys, a constant list in key order.
#[test]
fn map_literal_code() {
    use Instr::{Global, Int, Map};

    let program = compile("{2: wits, 0x1: 3, -5: 4}", &env()).unwrap();
    let map = program.result;
    assert_eq!(
        program.types.get(map),
        Some(&TypeNode::Map {
            key: TypeId(0),
            value: TypeId(0)
        })
    );
    assert_eq!(
        program.code,
        [
            Int(4),
            Int(3),
            Global {
                at: 0,
                ty: TypeId(0)
            },
            Map { keys: 0, map },
        ]
    );
    assert_eq!(
        program.constants,
        [Constant::List {
            entry: TypeId(0),
            items: vec![Scalar::Int(-5), Scalar::Int(1), Scalar::Int(2)],
        }]
    );
    // Each entry takes its key's word and its value's words.
    assert_eq!(program.memory().heap, 6);
    assert_eq!(program.memory().constants, 3);

    // String keys are ordered by their bytes.
    let program = compile(r#"{"b": bonus, "ab": bonus, "a": bonus}"#, &env()).unwrap();
    let Some(Map { keys, .. }) = program.code.last() else {
        panic!("{:?}", program.code);
    };
    let Constant::List { items, .. } = &program.constants[*keys as usize] else {
        panic!("{:?}", program.constants);
    };
    let keys = items
        .iter()
        .map(|item| match item {
            Scalar::Const(c) => match &program.constants[*c as usize] {
                Constant::String(s) => s.as_str(),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, ["a", "ab", "b"]);
    // An `Option<Int>` is two words, so three words an entry.
    assert_eq!(program.memory().heap, 9);
}

/// The verifier checks a map's instruction against its type, its keys and
/// the stack.
#[test]
fn tampered_map_literals() {
    let source = "{1: wits, 2: 3}";
    assert!(load(&stored(source)).is_ok());

    type Tamper = Box<dyn Fn(&mut Stored)>;

    fn map(s: &Stored) -> TypeId {
        match s.code.last() {
            Some(Instr::Map { map, .. }) => *map,
            other => panic!("{other:?}"),
        }
    }

    fn keys(s: &mut Stored) -> &mut Vec<Scalar> {
        match &mut s.constants[0] {
            Constant::List { items, .. } => items,
            other => panic!("{other:?}"),
        }
    }

    let at = "at instruction 2: while verifying the program";

    let cases: Vec<(Tamper, String)> = vec![
        (
            Box::new(|s| *keys(s) = vec![Scalar::Int(2), Scalar::Int(1)]),
            format!("the instruction makes a map of keys that are not sorted and unique: {at}"),
        ),
        (
            Box::new(|s| *keys(s) = vec![Scalar::Int(1), Scalar::Int(1)]),
            format!("the instruction makes a map of keys that are not sorted and unique: {at}"),
        ),
        (
            Box::new(|s| keys(s).push(Scalar::Int(3))),
            format!("the instruction makes a map of entries it does not find: {at}"),
        ),
        (
            Box::new(|s| {
                keys(s).pop();
            }),
            "it ends with 2 values on the stack, not one of its result type: while verifying the program"
                .to_owned(),
        ),
        (
            Box::new(|s| {
                let map = map(s);
                *s.code.last_mut().unwrap() = Instr::Map { keys: 1, map };
            }),
            format!("the instruction names a constant there is not: {at}"),
        ),
        (
            Box::new(|s| {
                *s.code.last_mut().unwrap() = Instr::Map {
                    keys: 0,
                    map: TypeId(0),
                };
            }),
            format!("the instruction makes a map of other than a map type: {at}"),
        ),
        (
            Box::new(|s| s.code[1] = Instr::Bool(true)),
            format!("the instruction makes a map of entries it does not find: {at}"),
        ),
        (
            Box::new(|s| s.memory.heap -= 1),
            "it declares 7 words of memory but uses 8: while verifying the program".to_owned(),
        ),
    ];

    for (n, (tamper, expected)) in cases.into_iter().enumerate() {
        let mut s = stored(source);
        tamper(&mut s);
        assert_eq!(invalid(&s), expected, "case {n}");
    }

    // String keys are checked by their bytes, and must be strings.
    let source = r#"{"a": 1, "b": 2}"#;
    assert!(load(&stored(source)).is_ok());

    let mut s = stored(source);
    let Constant::List { items, .. } = &mut s.constants[2] else {
        panic!("{:?}", s.constants);
    };
    items.reverse();
    assert_eq!(
        invalid(&s),
        format!("the instruction makes a map of keys that are not sorted and unique: {at}")
    );

    // Keys that are no list of the map's key type.
    let mut s = stored(source);
    let map = map(&s);
    *s.code.last_mut().unwrap() = Instr::Map { keys: 0, map };
    assert_eq!(
        invalid(&s),
        format!(
            "the instruction makes a map of keys that are no constant list of its key type: {at}"
        )
    );
}

/// A long string key given twice is cut short where it is named.
#[test]
fn map_literal_long_repeated_key() {
    let long = "k".repeat(100);
    let source = format!("{{{long:?}: 1, {long:?}: 2}}");
    let (message, _) = error(&source);
    assert_eq!(
        message,
        format!("the key \"{}…\" is given twice in this map", "k".repeat(64))
    );
}

/// A map entry without its `:` is reported once, at the first thing that is
/// not its key's end, and a key with a syntax error in it is not also
/// reported as not a literal.
#[test]
fn map_entry_syntax_errors_are_reported_once() {
    let key_end =
        |found: &str| format!("expected an operator or `:` after this map key, found {found}");

    errors(&[
        ("{1 2}", &key_end("an integer"), "2"),
        ("{1::2}", &key_end("`::`"), "::"),
        ("{1 2, 3: 4}", &key_end("an integer"), "2"),
        ("{1 2 3}", &key_end("an integer"), "2"),
        (
            "{1 2, 3 4}",
            &format!("{0}\n{0}", key_end("an integer")),
            "2",
        ),
        ("{\"a\" + : 1}", "expected an expression, found `:`", ":"),
        ("{-: 1}", "expected an expression, found `:`", ":"),
        ("{(1 +): 1}", "expected an expression, found `)`", ")"),
    ]);
}

/// Each value is given the next slot as it is declared, whatever its name,
/// and every program compiled from one environment reads it there.
#[test]
fn slots_are_the_environments() {
    let env = env();
    let at = |name: &str| env.slot(name).unwrap().at;

    // Declaration order, a word each but for `bonus`'s two.
    assert_eq!(at("wits"), 0);
    assert_eq!(at("min"), 6);
    assert_eq!(at("flag"), 7);
    assert_eq!(at("bonus"), 8);
    assert_eq!(at("maybe"), 10);
    assert_eq!(at("maybe_track"), 18);

    let reads = |source: &str| {
        let program = compile(source, &env).unwrap();
        let globals = program
            .globals
            .iter()
            .map(|g| (g.name.clone(), g.slot))
            .collect::<Vec<_>>();
        let code = program
            .code
            .iter()
            .filter_map(|i| match i {
                Instr::Global { at, .. } => Some(*at),
                _ => None,
            })
            .collect::<Vec<_>>();
        (globals, code)
    };

    // In whichever order and with whatever else a program reads them, the
    // slots are the same.
    assert_eq!(
        reads("bonus.unwrap_or(wits)"),
        (vec![("bonus".into(), 8), ("wits".into(), 0)], vec![8, 0])
    );
    assert_eq!(
        reads("wits + maybe_track.unwrap_or(health).boxes + bonus.unwrap_or(0)").0,
        [
            ("wits".into(), 0),
            ("maybe_track".into(), 18),
            ("health".into(), 14),
            ("bonus".into(), 8)
        ]
    );
    assert_eq!(
        reads("wits + wits + bonus.unwrap_or(wits)"),
        (
            vec![("wits".into(), 0), ("bonus".into(), 8)],
            vec![0, 0, 8, 0]
        )
    );

    // Stored and read back, it keeps its slots and fingerprint.
    let program = compile("bonus.unwrap_or(wits)", &env).unwrap();
    let loaded = Program::from_bytes(&program.to_bytes().unwrap()).unwrap();
    assert_eq!(loaded.globals, program.globals);
    assert_eq!(loaded.fingerprint(), env.fingerprint());
    assert_eq!(loaded.memory(), program.memory());
}

/// The fingerprint is a fixed function of the names and types in
/// declaration order, the same in every process.
#[test]
fn fingerprints() {
    let fingerprint = |values: &[(&str, Type)]| {
        let mut env = Environment::new();

        for (name, ty) in values {
            env.value(name, ty.clone()).unwrap();
        }

        env.fingerprint()
    };

    let record = Type::record([
        ("x", Type::int()),
        (
            "y",
            Type::option(Type::map(Type::string(), Type::list(Type::bool()))),
        ),
    ]);

    // FNV-1a over each name's length and bytes, then its type in
    // pre-order. Worked out apart from evalite, so a change to the
    // algorithm, which would orphan every stored program, fails here.
    assert_eq!(fingerprint(&[]), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fingerprint(&[("a", Type::int())]), 0x519e_0cc8_9e06_158d);
    assert_eq!(
        fingerprint(&[("a", Type::int()), ("rec", record.clone())]),
        0x0e0b_ec65_28e6_d8c8
    );

    // Functions are not part of the layout.
    let mut env = Environment::new();
    env.value("a", Type::int()).unwrap();
    env.function("f", [Type::int()], Type::int()).unwrap();
    assert_eq!(env.fingerprint(), 0x519e_0cc8_9e06_158d);

    // A change of name, type, order or field changes it.
    let layouts = [
        vec![("a", Type::int())],
        vec![("b", Type::int())],
        vec![("a", Type::bool())],
        vec![("a", Type::option(Type::int()))],
        vec![("a", Type::list(Type::int()))],
        vec![("a", Type::int()), ("b", Type::int())],
        vec![("b", Type::int()), ("a", Type::int())],
        vec![("a", Type::int()), ("b", Type::bool())],
        vec![("ab", Type::int())],
        vec![("a", Type::record([("b", Type::int())]))],
        vec![("a", Type::record([("c", Type::int())]))],
        vec![("a", Type::record([("b", Type::bool())]))],
        vec![("a", Type::record([("b", Type::int()), ("c", Type::int())]))],
        vec![(
            "a",
            Type::record([("b", Type::record([("c", Type::int())]))]),
        )],
        vec![("a", Type::map(Type::int(), Type::int()))],
        vec![("a", Type::map(Type::string(), Type::int()))],
        vec![("a", Type::map(Type::int(), Type::bool()))],
        vec![("a", record.clone())],
    ];

    let mut seen = std::collections::HashMap::new();

    for layout in &layouts {
        if let Some(before) = seen.insert(fingerprint(layout), layout) {
            panic!("{before:?} and {layout:?} share a fingerprint");
        }
    }
}

/// A program is accepted by an environment of its layout, or by one that
/// holds each value it reads in the same slot with the same type.
#[test]
fn accepted_layouts() {
    let mut env = Environment::new();
    env.value("wits", Type::int()).unwrap();
    env.value("bonus", Type::option(Type::int())).unwrap();
    let program = compile("bonus.unwrap_or(wits)", &env).unwrap();
    assert!(env.accepts(&program));

    // Values declared after the ones it reads move nothing.
    let mut more = env.clone();
    more.value("flag", Type::bool()).unwrap();
    assert_ne!(more.fingerprint(), program.fingerprint());
    assert!(more.accepts(&program));

    // So does a value it does not read, of another type, before them.
    let mut other = Environment::new();
    other.value("wits", Type::int()).unwrap();
    other.value("bonus", Type::option(Type::int())).unwrap();
    let only_wits = compile("wits", &other).unwrap();
    let mut changed = Environment::new();
    changed.value("wits", Type::int()).unwrap();
    changed.value("bonus", Type::bool()).unwrap();
    assert!(changed.accepts(&only_wits));
    assert!(!changed.accepts(&program));

    // A value it reads in another slot, of another type, or not declared.
    let mut swapped = Environment::new();
    swapped.value("bonus", Type::option(Type::int())).unwrap();
    swapped.value("wits", Type::int()).unwrap();
    assert!(!swapped.accepts(&program));

    let mut retyped = Environment::new();
    retyped.value("wits", Type::bool()).unwrap();
    retyped.value("bonus", Type::option(Type::int())).unwrap();
    assert!(!retyped.accepts(&program));

    let mut missing = Environment::new();
    missing.value("wits", Type::int()).unwrap();
    assert!(!missing.accepts(&program));
    assert!(!Environment::new().accepts(&program));

    // A program reading nothing fits anywhere.
    assert!(Environment::new().accepts(&compile("1", &env).unwrap()));
}

/// A fingerprint that matches is no proof of the layout: a compiled
/// program whose fingerprint collides with another layout's is checked
/// read by read, and refused where it would read a slot as another type.
#[test]
fn colliding_fingerprints_are_not_accepted() {
    let mut env = Environment::new();
    env.value("wits", Type::int()).unwrap();
    let mut program = compile("wits + 1", &env).unwrap();

    let mut other = Environment::new();
    other.value("wits", Type::string()).unwrap();
    // As if FNV-1a had collided: the hashes match, the layouts do not.
    program.layout = other.fingerprint();
    assert_eq!(program.fingerprint(), other.fingerprint());

    assert!(!other.accepts(&program));
    let globals = Globals::new(&other);
    assert!(!globals.accepts(&program));

    let mut machine = Machine::default();
    let error = machine.load(&program, &globals).unwrap_err();
    assert_eq!(error.message(), &Message::OtherLayout { global: Some(0) });

    // The layout it was compiled against still takes it.
    assert!(env.accepts(&program));
}

/// The environment works out a slot's width from the type as the type
/// table does from its id.
#[test]
fn slot_widths() {
    let o = Type::option;
    let types = [
        Type::int(),
        Type::bool(),
        Type::string(),
        Type::list(Type::int()),
        track(),
        Type::map(Type::int(), Type::int()),
        o(Type::int()),
        o(Type::bool()),
        o(Type::string()),
        o(track()),
        o(Type::list(o(Type::int()))),
        o(o(Type::int())),
        o(o(o(Type::int()))),
        o(o(Type::bool())),
        o(o(o(Type::string()))),
    ];

    for ty in types {
        let mut table = crate::types::Types::new();
        let id = table.intern(&ty).unwrap();
        assert_eq!(crate::env::width(&ty), table.width(id), "{ty}");
    }

    let mut env = Environment::new();
    env.value("a", o(o(Type::int()))).unwrap();
    env.value("b", Type::int()).unwrap();
    assert_eq!(env.slot("b").unwrap().at, 3);
}

/// A run reads each value from its slot in the store, with slots it does
/// not read, set or not, left between.
#[test]
fn reading_from_slots() {
    let mut env = Environment::new();
    env.value("a", Type::option(Type::int())).unwrap();
    env.value("b", Type::list(Type::int())).unwrap();
    env.value("c", Type::int()).unwrap();

    let program = compile("c + b.len()", &env).unwrap();
    let mut globals = Globals::new(&env);
    globals
        .set("b", &Value::list([Value::int(1), Value::int(2)]))
        .unwrap();
    globals.set("c", &Value::int(40)).unwrap();

    let mut machine = Machine::default();
    let run = machine.load(&program, &globals).unwrap();
    assert_eq!(run.run().unwrap().value(), &Value::int(42));

    // Setting a value it does not read changes nothing it reads.
    globals.set("a", &Value::some(Value::int(7))).unwrap();
    let run = machine.load(&program, &globals).unwrap();
    assert_eq!(run.run().unwrap().value(), &Value::int(42));
    assert_eq!(globals.get("a"), Some(Value::some(Value::int(7))));
}

#[test]
fn option_literals() {
    types(&[
        ("Some(1)", "Option<Int>"),
        ("Some(Some(true))", "Option<Option<Bool>>"),
        ("Some(health)", "Option<{boxes: Int, count: List<Int>}>"),
        ("[Some(1), None]", "List<Option<Int>>"),
        ("[None, Some(1)]", "List<Option<Int>>"),
        ("[None, None, bonus, None]", "List<Option<Int>>"),
        ("[[Some(1)], [None]]", "List<List<Option<Int>>>"),
        ("[Some(Some(1)), Some(None)]", "List<Option<Option<Int>>>"),
        ("{1: None, 2: Some(3)}", "Map<Int, Option<Int>>"),
        ("{2: Some(3), 1: None}", "Map<Int, Option<Int>>"),
        (
            r#"{"a": [bonus], "b": [None]}"#,
            "Map<String, List<Option<Int>>>",
        ),
        ("Some(bonus).unwrap_or(None)", "Option<Int>"),
        ("Some(bonus).unwrap_or(Some(1))", "Option<Int>"),
        ("[Some(1)].get(0).unwrap_or(None)", "Option<Int>"),
        ("count(health, 1)", "Int"),
    ]);
}

#[test]
fn option_literals_from_host_functions() {
    let mut env = env();
    env.function("bump", [Type::option(Type::int())], Type::int())
        .unwrap();
    env.function("either", [Type::option(Type::int())], Type::int())
        .unwrap();
    env.function("either", [Type::option(Type::bool())], Type::int())
        .unwrap();

    let program = compile("bump(None) + bump(Some(2))", &env).unwrap();
    assert_eq!(program.result_type(), Type::int());

    let (message, _) = error_in("either(None)", &env);
    assert!(message.starts_with("`None` needs a type"), "{message}");
    assert_eq!(
        compile("either(Some(true))", &env).unwrap().result_type(),
        Type::int()
    );
}

#[test]
fn option_literal_errors() {
    let untyped = "`None` needs a type from where it is used, such as beside `Some(...)` in a list or as an argument of an option";

    errors(&[
        ("None", untyped, "None"),
        ("[None]", untyped, "None"),
        // Only a sibling gives a waiting `None` its type, not a sibling
        // of what holds it.
        ("[Some(None), Some(Some(1))]", untyped, "None"),
        ("Some(None)", untyped, "None"),
        ("None.is_some()", untyped, "None"),
        ("(None)", untyped, "None"),
        (
            "[1, None]",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        (
            "[None, 1]",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        (
            "[Some(1), Some(None)]",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        (
            "wits.min(None)",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        (
            "bonus.unwrap_or(None)",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        (
            "chart(None, name)",
            "`None` is an option, but an Int is expected here",
            "None",
        ),
        ("Some()", "`Some` takes 1 argument, found 0", "()"),
        ("Some(1, 2)", "`Some` takes 1 argument, found 2", "(1, 2)"),
        (
            "[Some(1), Some(true)]",
            "entry 2 of this list is Option<Bool>, but the first is Option<Int>, a list's entries have one type",
            "Some(true)",
        ),
        (
            "Some(1) + 1",
            "`+` takes Int and Int, found Option<Int> and Int, unwrap the optional value first with `.unwrap_or(...)`",
            "Some(1) + 1",
        ),
    ]);
}

const EMPTY_LIST: &str = "an empty list `[]` needs a type from where it is used, such as beside a typed entry in a list or as an argument that takes a list, or give it at least one entry";
const EMPTY_MAP: &str = "an empty map `{}` needs a type from where it is used, such as beside a typed entry in a list or as an argument that takes a map, or give it at least one entry";

#[test]
fn empty_literals() {
    types(&[
        ("[[], [1]]", "List<List<Int>>"),
        ("[[1], []]", "List<List<Int>>"),
        ("[[], [], levels]", "List<List<Int>>"),
        ("[{}, {1: true}]", "List<Map<Int, Bool>>"),
        (r#"{"a": [], "b": [name]}"#, "Map<String, List<String>>"),
        (r#"{1: {}, 2: {"x": 1}}"#, "Map<Int, Map<String, Int>>"),
        ("Some([1]).unwrap_or([])", "List<Int>"),
        ("Some({1: 2}).unwrap_or({})", "Map<Int, Int>"),
        ("[levels, ([])]", "List<List<Int>>"),
    ]);
}

#[test]
fn empty_literals_from_host_functions() {
    let mut env = env();
    env.function("total", [Type::list(Type::int())], Type::int())
        .unwrap();
    env.function(
        "size",
        [Type::map(Type::string(), Type::int())],
        Type::int(),
    )
    .unwrap();

    let program = compile("total([]) + size({})", &env).unwrap();
    assert_eq!(program.result_type(), Type::int());
}

#[test]
fn empty_literal_errors() {
    errors(&[
        ("[]", EMPTY_LIST, "[]"),
        ("[].len()", EMPTY_LIST, "[]"),
        ("{}", EMPTY_MAP, "{}"),
        ("{}.len()", EMPTY_MAP, "{}"),
        ("[[]]", EMPTY_LIST, "[]"),
        // Only a sibling gives a waiting `[]` its type, not a sibling of
        // what holds it.
        ("[[[]], [[1]]]", EMPTY_LIST, "[]"),
        (
            "[1, []]",
            "`[]` is a list, but an Int is expected here",
            "[]",
        ),
        (
            "[[], 1]",
            "`[]` is a list, but an Int is expected here",
            "[]",
        ),
        (
            "[levels, {}]",
            "`{}` is a map, but a List<Int> is expected here",
            "{}",
        ),
        (
            "[{}, levels]",
            "`{}` is a map, but a List<Int> is expected here",
            "{}",
        ),
        (
            "wits.min([])",
            "`[]` is a list, but an Int is expected here",
            "[]",
        ),
        (
            "Some(levels).unwrap_or({})",
            "`{}` is a map, but a List<Int> is expected here",
            "{}",
        ),
    ]);
}

/// Checking `None`s, `[]`s and `{}`s that wait for a sibling's type, and
/// `Some`s nested deep, is linear: each waiting literal is given its type
/// once.
#[test]
fn option_literals_are_linear() {
    let options = Options::new().max_len(usize::MAX).max_depth(usize::MAX);

    let shapes: [fn(usize) -> String; 5] = [
        |n| format!("[{}Some(1)]", "None, ".repeat(n)),
        |n| {
            format!(
                "{{{}0: Some(1)}}",
                (1..=n).map(|k| format!("{k}: None, ")).collect::<String>()
            )
        },
        |n| format!("{}None{}", "[Some(1), Some(".repeat(n), ")]".repeat(n)),
        |n| format!("[{}[1]]", "[], ".repeat(n)),
        |n| format!("[{}{{1: 2}}]", "{}, ".repeat(n)),
    ];

    for shape in shapes {
        let mut times = Vec::new();

        for n in [5_000usize, 20_000] {
            let source = shape(n);
            let start = Instant::now();
            let _ = compile_with(&source, &env(), &options);
            times.push(start.elapsed().as_secs_f64());
        }

        // Four times the input takes four times as long, not sixteen.
        assert!(times[1] < times[0] * 10.0 + 0.05, "{times:?}");
    }
}

/// Record literals check in about linear time: many fields, repeated
/// fields, fields checked against an expected record, and nesting.
#[test]
fn record_literals_are_linear() {
    let options = Options::new().max_len(usize::MAX).max_depth(usize::MAX);

    let shapes: [fn(usize) -> String; 4] = [
        |n| {
            format!(
                "{{{}}}",
                (0..n).map(|k| format!("f{k}: None, ")).collect::<String>()
            )
        },
        |n| format!("{{{}}}", "a: 1, ".repeat(n)),
        |n| {
            format!(
                "[specialties, {{{}}}]",
                (0..n).map(|k| format!("f{k}: 1, ")).collect::<String>()
            )
        },
        |n| format!("{}1{}", "{a: ".repeat(n), "}".repeat(n)),
    ];

    for shape in shapes {
        let mut times = Vec::new();

        for n in [5_000usize, 20_000] {
            let source = shape(n);
            let start = Instant::now();
            let _ = compile_with(&source, &env(), &options);
            times.push(start.elapsed().as_secs_f64());
        }

        // Four times the input takes four times as long, not sixteen.
        assert!(times[1] < times[0] * 10.0 + 0.05, "{times:?}");
    }
}
