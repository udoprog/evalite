//! Tests of the machine: every instruction, every runtime error with its
//! message, each limit at its boundary, and random well-typed programs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::types::Kind;
use crate::value::Repr;
use std::collections::HashSet;
use std::mem::discriminant;

use crate::check_tests::{env, generate};
use crate::message::Message;
use crate::program::{Constant, Instr, Memory, Stored};
use crate::tests::Rng;
use crate::types::{TypeId, TypeNode, Types};
use crate::*;

/// A track of `boxes` boxes, `boxes` of them at the first state.
fn track(boxes: i64) -> Value {
    Value::record([
        ("boxes", Value::int(boxes)),
        ("count", Value::list(vec![Value::int(boxes)])),
    ])
}

/// The value bound to each of [`env`]'s names, and the words its parts
/// take.
fn binding(name: &str) -> (Value, usize) {
    match name {
        "wits" => (Value::int(2), 0),
        "dexterity" => (Value::int(3), 0),
        "athletics" => (Value::int(1), 0),
        "strength" => (Value::int(4), 0),
        "blood_potency" => (Value::int(2), 0),
        "primal_urge" => (Value::int(1), 0),
        "min" => (Value::int(5), 0),
        "flag" => (Value::bool(true), 0),
        "bonus" => (Value::some(Value::int(2)), 0),
        "maybe" => (Value::none(), 0),
        "levels" => (
            Value::list(vec![Value::int(10), Value::int(20), Value::int(30)]),
            3,
        ),
        "grid" => (
            Value::list(vec![
                Value::list(vec![Value::int(1), Value::int(2)]),
                Value::list(vec![Value::int(3)]),
            ]),
            5,
        ),
        "name" => (Value::from("Essence"), 1),
        "health" => (track(1), 3),
        "tracks" => (Value::list(vec![track(2), track(3)]), 8),
        "specialties" => (
            Value::record([
                ("athletics", Value::int(2)),
                ("brawl", Value::some(Value::int(1))),
            ]),
            3,
        ),
        "m" => (
            Value::record([
                ("Brawl (bar)", Value::int(4)),
                ("a b", track(4)),
                ("inner", Value::record([("deep", Value::bool(false))])),
            ]),
            7,
        ),
        "maybe_track" => (Value::some(track(5)), 3),
        // Two words an entry: the key, then the record.
        "ids" => (
            Value::map([
                (int(0x1A2B_3C4D), Value::record([("dice", int(3))])),
                (int(-4), Value::record([("dice", int(1))])),
                (int(7), Value::record([("dice", int(2))])),
            ]),
            9,
        ),
        // Three words an entry, an `Option<Int>` being tagged, and a word
        // for each key's bytes.
        "tags" => (
            Value::map([
                (Value::from("brawl"), Value::some(int(2))),
                (Value::from("athletics"), Value::none()),
                (Value::from("athletics (long name)"), Value::some(int(5))),
                (Value::from(""), Value::some(int(0))),
            ]),
            17,
        ),
        "empty" => (Value::map([]), 0),
        _ => panic!("no binding for {name}"),
    }
}

/// [`env`]'s functions: `count(t)` is its boxes, `count(t, n)` is `n`,
/// `boxes` is 5, `chart(i, table)` is `i`, `label(i)` is `some(i)` for
/// even `i`, `pick()` is `true`.
fn host(name: &str, args: Vec<Value>) -> Result<Value, String> {
    Ok(match (name, args.as_slice()) {
        ("count", [Value(Repr::Record(t))]) => t["boxes"].clone(),
        ("count", [Value(Repr::Record(_)), Value(Repr::Int(n))]) => Value::int(*n),
        ("boxes", [Value(Repr::Record(_))]) => Value::int(5),
        ("chart", [Value(Repr::Int(i)), Value(Repr::String(table))]) => {
            assert_eq!(table, "Essence");
            Value::int(*i)
        }
        ("label", [Value(Repr::Int(i))]) if i % 2 == 0 => Value::some(Value::int(*i)),
        ("label", [Value(Repr::Int(_))]) => Value::none(),
        ("pick", []) => Value::bool(true),
        _ => return Err(format!("unexpected call {name}{args:?}")),
    })
}

fn compile_in(source: &str, extra: &[(&str, Type, Value)]) -> Program {
    let mut env = env();

    for (name, ty, _) in extra {
        env.value(name, ty.clone()).unwrap();
    }

    env.function("fail", [], Type::int()).unwrap();
    env.function("wrong", [], Type::int()).unwrap();
    env.function("nested", [], Type::list(Type::record([("a", Type::int())])))
        .unwrap();
    env.function("many", [Type::int()], Type::list(Type::int()))
        .unwrap();
    env.value("ids", ids_type()).unwrap();
    env.value("tags", tags_type()).unwrap();
    env.value("empty", Type::map(Type::string(), Type::bool()))
        .unwrap();
    env.function("lookup", [], ids_type()).unwrap();

    compile(source, &env).unwrap_or_else(|e| panic!("{source:?}: {e}"))
}

/// `ids`: records by `Int` key.
fn ids_type() -> Type {
    Type::map(Type::int(), Type::record([("dice", Type::int())]))
}

/// `tags`: optional values by `String` key.
fn tags_type() -> Type {
    Type::map(Type::string(), Type::option(Type::int()))
}

/// `host` and some functions that misbehave.
fn test_host(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match (name, args.as_slice()) {
        ("fail", []) => Err("the track is gone".to_owned()),
        ("wrong", []) => Ok(Value::bool(true)),
        ("nested", []) => Ok(Value::list(vec![
            Value::record([("a", Value::int(1))]),
            Value::record([("a", Value::none())]),
        ])),
        ("many", [Value(Repr::Int(n))]) => Ok(Value::list(vec![Value::int(0); *n as usize])),
        ("lookup", []) => Ok(Value::map([
            (int(2), Value::record([("dice", int(20))])),
            (int(1), Value::record([("dice", int(10))])),
        ])),
        _ => host(name, args),
    }
}

/// Bind every value `program` reads, from `extra` first, then [`binding`].
fn bind_all(run: &mut Run<'_>, extra: &[(&str, Type, Value)]) -> Result<(), RunError> {
    for (name, _) in run.globals() {
        let value = match extra.iter().find(|(n, ..)| *n == name) {
            Some((.., value)) => value.clone(),
            None => binding(name).0,
        };

        run.bind(name, &value)?;
    }

    Ok(())
}

fn eval_in(
    source: &str,
    extra: &[(&str, Type, Value)],
    limits: Limits,
) -> Result<Outcome, RunError> {
    let program = compile_in(source, extra);
    let mut machine = Machine::new(limits);
    let mut run = machine.load(&program)?;
    bind_all(&mut run, extra)?;
    run.run_with(&mut test_host)
}

fn eval(source: &str) -> Value {
    eval_in(source, &[], Limits::new())
        .unwrap_or_else(|e| panic!("{source:?}: {e}"))
        .value
}

/// Every label of `error`, with the names `program` gives, joined by
/// `: `.
pub(crate) fn chain(error: &RunError, program: &Program) -> String {
    error
        .labels(program, None)
        .map(|l| l.message().to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// The primary message of `error`, with the names `program` gives.
pub(crate) fn primary(error: &RunError, program: &Program) -> String {
    error
        .labels(program, None)
        .next()
        .map(|l| l.message().to_string())
        .unwrap_or_default()
}

/// The whole chain of how `source` fails.
fn fails_chain(source: &str) -> String {
    let program = compile_in(source, &[]);
    let mut machine = Machine::new(Limits::new());
    let mut run = machine.load(&program).unwrap();
    bind_all(&mut run, &[]).unwrap();
    chain(&run.run_with(&mut test_host).unwrap_err(), &program)
}

fn fails_in(source: &str, extra: &[(&str, Type, Value)]) -> String {
    let program = compile_in(source, extra);
    let mut machine = Machine::new(Limits::new());
    let result = machine.load(&program).and_then(|mut run| {
        bind_all(&mut run, extra)?;
        run.run_with(&mut test_host)
    });

    match result {
        Ok(outcome) => panic!("{source:?} gave {}", outcome.value),
        Err(error) => {
            assert!(!error.is_fault(), "{source:?}: {error}");
            primary(&error, &program)
        }
    }
}

fn fails(source: &str) -> String {
    fails_in(source, &[])
}

fn int(n: i64) -> Value {
    Value::int(n)
}

fn ints(ns: &[i64]) -> Value {
    Value::list(ns.iter().map(|n| Value::int(*n)))
}

/// One or more expressions per instruction, with what each gives.
const CASES: &[(&str, &str)] = &[
    // Int, Bool, Const, Global
    ("7", "7"),
    ("-7", "-7"),
    ("true", "true"),
    ("false", "false"),
    ("[4, 5, 6]", "[4, 5, 6]"),
    ("[[1], [2, 3]]", "[[1], [2, 3]]"),
    (r#"chart(3, "Essence")"#, "3"),
    ("wits", "2"),
    // Neg, Not
    ("-wits", "-2"),
    ("-(-wits)", "2"),
    ("!flag", "false"),
    ("!!flag", "true"),
    // Add, Sub, Mul, Div, Rem
    ("wits + dexterity", "5"),
    ("wits - dexterity", "-1"),
    ("wits * dexterity", "6"),
    ("strength / dexterity", "1"),
    ("-strength / dexterity", "-1"),
    ("strength % dexterity", "1"),
    ("-strength % dexterity", "-1"),
    // Eq, Ne on Ints and Bools
    ("wits == 2", "true"),
    ("wits == 3", "false"),
    ("wits != 2", "false"),
    ("flag == true", "true"),
    ("flag != true", "false"),
    // Lt, Le, Gt, Ge
    ("wits < dexterity", "true"),
    ("wits < wits", "false"),
    ("wits <= wits", "true"),
    ("dexterity <= wits", "false"),
    ("dexterity > wits", "true"),
    ("wits > wits", "false"),
    ("wits >= wits", "true"),
    ("wits >= dexterity", "false"),
    // And, Or: both outcomes and the skipped right operand
    ("flag && wits == 2", "true"),
    ("flag && wits == 3", "false"),
    ("!flag && wits / 0 == 0", "false"),
    ("flag || wits / 0 == 0", "true"),
    ("!flag || wits == 2", "true"),
    ("!flag || wits == 3", "false"),
    ("(!flag && true) || flag", "true"),
    // Field
    ("specialties.athletics", "2"),
    (r#"m["Brawl (bar)"]"#, "4"),
    ("m.inner.deep", "false"),
    ("m.inner", "{deep: false}"),
    // Index
    ("levels[0]", "10"),
    ("levels[2]", "30"),
    ("[4, 5, 6][1]", "5"),
    ("grid[1][0]", "3"),
    ("[wits, dexterity][1]", "3"),
    // List
    ("[wits, dexterity]", "[2, 3]"),
    ("[[wits], levels]", "[[2], [10, 20, 30]]"),
    ("[name]", r#"["Essence"]"#),
    (
        "[bonus, specialties.brawl, label(3)]",
        "[some(2), some(1), none]",
    ),
    // Min, Max (functions and methods)
    ("Int::min(wits)", "2"),
    ("Int::min(dexterity, wits, strength)", "2"),
    ("Int::max(dexterity, wits, strength)", "4"),
    ("wits.min(dexterity)", "2"),
    ("Int :: max(wits)", "2"),
    // A method by path, the receiver first
    ("Int::abs(-wits)", "2"),
    ("Int::clamp(wits, 3, 5)", "3"),
    ("Option::unwrap_or(bonus, 0)", "2"),
    ("List::len([1, 2, 3])", "3"),
    ("wits.max(dexterity)", "3"),
    // Abs
    ("(-wits).abs()", "2"),
    ("wits.abs()", "2"),
    // Clamp
    ("strength.clamp(1, 3)", "3"),
    ("athletics.clamp(2, 3)", "2"),
    ("wits.clamp(1, 3)", "2"),
    ("wits.clamp(2, 2)", "2"),
    // Get: an option, none out of range
    ("levels.get(1)", "some(20)"),
    ("levels.get(0)", "some(10)"),
    ("levels.get(-1)", "none"),
    ("levels.get(-5)", "none"),
    ("levels.get(3)", "none"),
    ("levels.get(99)", "none"),
    ("[10, 11, 12, 13].get(blood_potency)", "some(12)"),
    ("[wits].get(0)", "some(2)"),
    ("levels.get(9).unwrap_or(0)", "0"),
    ("levels.get(1).unwrap_or(0)", "20"),
    ("levels.get(1).is_some()", "true"),
    ("levels.get(3).is_none()", "true"),
    ("[true, false].get(0)", "some(true)"),
    ("[true, false].get(2)", "none"),
    ("[levels].get(0)", "some([10, 20, 30])"),
    ("[levels].get(1)", "none"),
    // A list of options gives an option of an option, telling an absent
    // entry from one out of range
    ("[bonus, label(3)].get(0)", "some(some(2))"),
    ("[bonus, label(3)].get(1)", "some(none)"),
    ("[bonus, label(3)].get(2)", "none"),
    ("[maybe].get(0)", "some(none)"),
    ("[maybe].get(1)", "none"),
    ("[maybe_track].get(0)!!.boxes", "5"),
    ("[[maybe_track]].get(0)!", "[some({boxes: 5, count: [5]})]"),
    // Unwrap, `!`
    ("bonus!", "2"),
    ("bonus! + 1", "3"),
    ("-bonus!", "-2"),
    ("specialties.brawl!", "1"),
    ("label(4)!", "4"),
    ("levels.get(1)!", "20"),
    ("maybe_track!.boxes", "5"),
    ("maybe_track!.count[0]", "5"),
    ("[bonus, label(8)][1]!", "8"),
    ("[true].get(0)!", "true"),
    ("[\"a\"].get(0)!", "\"a\""),
    // UnwrapOr, IsSome, IsNone
    ("bonus.unwrap_or(0)", "2"),
    ("maybe.unwrap_or(true)", "true"),
    ("specialties.brawl.unwrap_or(0)", "1"),
    ("label(4).unwrap_or(0)", "4"),
    ("label(3).unwrap_or(0)", "0"),
    ("bonus.is_some()", "true"),
    ("maybe.is_some()", "false"),
    ("bonus.is_none()", "false"),
    ("maybe.is_none()", "true"),
    ("bonus", "some(2)"),
    ("maybe", "none"),
    ("label(3)", "none"),
    ("maybe_track", "some({boxes: 5, count: [5]})"),
    // Len
    ("levels.len()", "3"),
    ("[1].len()", "1"),
    ("grid[0].len()", "2"),
    // Call
    ("count(health)", "1"),
    ("count(tracks[1])", "3"),
    ("count(health, 9)", "9"),
    ("boxes(m[\"a b\"])", "5"),
    ("pick()", "true"),
    ("health", "{boxes: 1, count: [1]}"),
    ("tracks", "[{boxes: 2, count: [2]}, {boxes: 3, count: [3]}]"),
    ("specialties", "{athletics: 2, brawl: some(1)}"),
    (
        "m",
        r#"{"Brawl (bar)": 4, "a b": {boxes: 4, count: [4]}, inner: {deep: false}}"#,
    ),
    ("name", r#""Essence""#),
    // MapIndex, MapGet, and Len of a map
    ("ids[439041101].dice", "3"),
    ("ids[-4].dice", "1"),
    ("ids[7]", "{dice: 2}"),
    ("ids[wits + 5].dice", "2"),
    ("ids.get(7)", "some({dice: 2})"),
    ("ids.get(8)", "none"),
    ("ids.len()", "3"),
    ("Map::len(ids)", "3"),
    ("Map::get(ids, -4)", "some({dice: 1})"),
    ("Map::get(tags, \"brawl\")", "some(some(2))"),
    ("tags[\"brawl\"]", "some(2)"),
    ("tags[\"athletics\"]", "none"),
    ("tags[\"athletics (long name)\"]!", "5"),
    ("tags[\"\"]", "some(0)"),
    ("tags.get(\"brawl\")", "some(some(2))"),
    ("tags.get(\"athletics\")", "some(none)"),
    ("tags.get(\"athletic\")", "none"),
    ("tags.get(\"athleticsz\")", "none"),
    ("tags.get(\"\")", "some(some(0))"),
    ("tags.get(\"zzz\").is_none()", "true"),
    ("tags.len()", "4"),
    ("empty.len()", "0"),
    ("empty.get(\"a\")", "none"),
    ("lookup()[2].dice", "20"),
    ("lookup().get(3)", "none"),
    ("[ids][0][7].dice", "2"),
    ("ids", "{-4: {dice: 1}, 7: {dice: 2}, 439041101: {dice: 3}}"),
    (
        "tags",
        r#"{"": some(0), "athletics": none, "athletics (long name)": some(5), "brawl": some(2)}"#,
    ),
    ("empty", "{}"),
];

#[test]
fn every_instruction() {
    for (source, expected) in CASES {
        if expected.is_empty() {
            continue;
        }

        assert_eq!(eval(source).to_string(), *expected, "{source:?}");
    }
}

#[test]
fn every_instruction_is_covered() {
    let all = [
        Instr::Int(0),
        Instr::Bool(false),
        Instr::Const(0),
        Instr::Global {
            at: 0,
            ty: TypeId(0),
        },
        Instr::Neg,
        Instr::Not,
        Instr::Add,
        Instr::Sub,
        Instr::Mul,
        Instr::Div,
        Instr::Rem,
        Instr::Eq,
        Instr::Ne,
        Instr::Lt,
        Instr::Le,
        Instr::Gt,
        Instr::Ge,
        Instr::And(0),
        Instr::Or(0),
        Instr::Field {
            at: 0,
            ty: TypeId(0),
        },
        Instr::Index(TypeId(0)),
        Instr::List {
            len: 0,
            entry: TypeId(0),
        },
        Instr::Min(0),
        Instr::Max(0),
        Instr::Abs,
        Instr::Clamp,
        Instr::Get(TypeId(0)),
        Instr::UnwrapOr(TypeId(0)),
        Instr::IsSome(TypeId(0)),
        Instr::IsNone(TypeId(0)),
        Instr::Unwrap(TypeId(0)),
        Instr::Len,
        Instr::Call(0),
        Instr::MapIndex(TypeId(0)),
        Instr::MapGet(TypeId(0)),
    ];

    let mut seen = HashSet::new();

    for (source, expected) in CASES {
        if expected.is_empty() {
            continue;
        }

        for instr in &compile_in(source, &[]).code {
            seen.insert(discriminant(instr));
        }
    }

    for instr in all {
        assert!(seen.contains(&discriminant(&instr)), "{instr:?} is not run");
    }
}

#[test]
fn values_come_back_whole() {
    // Lists of strings, options and records survive the round trip
    // through memory, whether constant, bound, built or returned.
    let list = Value::list(vec![
        Value::from("a"),
        Value::from("bcdefghij"),
        Value::from(""),
    ]);
    let extra = [("words", Type::list(Type::string()), list.clone())];
    assert_eq!(eval_in("words", &extra, Limits::new()).unwrap().value, list);
    assert_eq!(
        eval_in("[words, words][1]", &extra, Limits::new())
            .unwrap()
            .value,
        list
    );

    let unicode = Value::from("Ærø \"é\" \\ 🐉 and some more bytes");
    let extra = [("s", Type::string(), unicode.clone())];
    assert_eq!(eval_in("s", &extra, Limits::new()).unwrap().value, unicode);
    assert_eq!(
        unicode.to_string(),
        r#""Ærø \"é\" \\ 🐉 and some more bytes""#
    );

    let option_list = Value::some(ints(&[1, 2]));
    let extra = [("ol", Type::option(Type::list(Type::int())), option_list)];
    assert_eq!(
        eval_in("ol.unwrap_or([9])", &extra, Limits::new())
            .unwrap()
            .value,
        ints(&[1, 2])
    );

    let extra = [("ol", Type::option(Type::list(Type::int())), Value::none())];
    assert_eq!(
        eval_in("ol.unwrap_or([9])[0]", &extra, Limits::new())
            .unwrap()
            .value,
        int(9)
    );

    let empty = [("e", Type::list(Type::int()), ints(&[]))];
    assert_eq!(
        eval_in("e.len()", &empty, Limits::new()).unwrap().value,
        int(0)
    );
}

#[test]
fn options() {
    // Every way an option is made, and every way it is used.
    let cases: &[(&str, Value, &str)] = &[
        ("o.unwrap_or(7)", Value::none(), "7"),
        ("o.unwrap_or(7)", Value::some(int(3)), "3"),
        ("o.is_some()", Value::none(), "false"),
        ("o.is_some()", Value::some(int(3)), "true"),
        ("o.is_none()", Value::none(), "true"),
        ("o.is_none()", Value::some(int(3)), "false"),
        ("o", Value::none(), "none"),
        ("o", Value::some(int(3)), "some(3)"),
        ("[o, o][0]", Value::some(int(0)), "some(0)"),
        ("o.unwrap_or(-1).max(0)", Value::none(), "0"),
    ];

    for (source, value, expected) in cases {
        let extra = [("o", Type::option(Type::int()), value.clone())];
        let outcome = eval_in(source, &extra, Limits::new()).unwrap();
        assert_eq!(
            outcome.value.to_string(),
            *expected,
            "{source:?} with {value}"
        );
    }

    // An option of a record, and a record's optional field.
    let ty = Type::option(Type::record([("x", Type::option(Type::int()))]));
    let extra = [(
        "r",
        ty.clone(),
        Value::some(Value::record([("x", Value::none())])),
    )];
    assert_eq!(
        eval_in("r", &extra, Limits::new())
            .unwrap()
            .value
            .to_string(),
        "some({x: none})"
    );
    assert_eq!(
        eval_in("r.is_some()", &extra, Limits::new()).unwrap().value,
        Value::bool(true)
    );

    // Options from the caller's functions.
    assert_eq!(eval("label(2).is_some()"), Value::bool(true));
    assert_eq!(eval("label(1).is_none()"), Value::bool(true));
}

#[test]
fn arithmetic_errors() {
    let big = [
        ("big", Type::int(), int(i64::MAX)),
        ("small", Type::int(), int(i64::MIN)),
    ];

    let cases = [
        ("big + 1", "`9223372036854775807 + 1` overflows an Int"),
        ("small - 1", "`-9223372036854775808 - 1` overflows an Int"),
        ("big * 2", "`9223372036854775807 * 2` overflows an Int"),
        ("small / -1", "`-9223372036854775808 / -1` overflows an Int"),
        ("small % -1", "`-9223372036854775808 % -1` overflows an Int"),
        ("-small", "`-(-9223372036854775808)` overflows an Int"),
        (
            "small.abs()",
            "`(-9223372036854775808).abs()` overflows an Int",
        ),
        ("wits / 0", "`2 / 0` divides by zero"),
        ("wits % 0", "`2 % 0` divides by zero"),
        ("wits / (wits - 2)", "`2 / 0` divides by zero"),
    ];

    for (source, message) in cases {
        assert_eq!(fails_in(source, &big), message, "{source:?}");
    }

    // Right at the edges, nothing overflows.
    assert_eq!(
        eval_in("big - 1 + 1", &big, Limits::new()).unwrap().value,
        int(i64::MAX)
    );
    assert_eq!(
        eval_in("small + 1 - 1", &big, Limits::new()).unwrap().value,
        int(i64::MIN)
    );
    assert_eq!(
        eval_in("(small + 1).abs()", &big, Limits::new())
            .unwrap()
            .value,
        int(i64::MAX)
    );
    assert_eq!(
        eval_in("small / 1", &big, Limits::new()).unwrap().value,
        int(i64::MIN)
    );
}

#[test]
fn get_errors() {
    let cases = [
        (
            "levels[3]",
            "index 3 is out of range of a list of 3 entries",
        ),
        (
            "levels[-1]",
            "index -1 is out of range of a list of 3 entries",
        ),
        ("[wits][1]", "index 1 is out of range of a list of 1 entry"),
        ("grid[1][1]", "index 1 is out of range of a list of 1 entry"),
        (
            "athletics.clamp(3, 2)",
            "`clamp` needs its low bound 3 to be at most its high bound 2",
        ),
        ("maybe!", "`!` found no value: the optional value is none"),
        (
            "label(3)!",
            "`!` found no value: the optional value is none",
        ),
        (
            "levels.get(3)! + 1",
            "`!` found no value: the optional value is none",
        ),
        (
            "[maybe_track].get(1)!!.boxes",
            "`!` found no value: the optional value is none",
        ),
    ];

    for (source, message) in cases {
        assert_eq!(fails(source), message, "{source:?}");
    }

    // Right at the ends, indexing works.
    assert_eq!(eval("levels[2]"), int(30));
    assert_eq!(eval("levels[0]"), int(10));

    let empty = [("e", Type::list(Type::int()), ints(&[]))];
    assert_eq!(
        fails_in("e[0]", &empty),
        "index 0 is out of range of a list of 0 entries"
    );

    // `get` gives none instead, of an empty list too.
    let outcome = eval_in("e.get(0)", &empty, Limits::new()).unwrap();
    assert_eq!(outcome.value, Value::none());
    assert_eq!(eval("levels.get(3)"), Value::none());
    assert_eq!(eval("levels.get(2)"), Value::some(int(30)));
}

#[test]
fn host_errors() {
    assert_eq!(
        fails_chain("fail()"),
        "the function failed: while calling `fail`: while running instruction 0: while running the program"
    );
    assert_eq!(
        fails_chain("wrong()"),
        "expected Int, found Bool: while reading what `wrong` returned: while running instruction 0: while running the program"
    );
    assert_eq!(
        fails_chain("nested()"),
        "expected Int, found none: at `nested(..)[1].a`: while reading what `nested` returned: while running instruction 0: while running the program"
    );

    // Without a host, any call fails.
    let program = compile_in("count(health)", &[]);
    let mut machine = Machine::default();
    let mut run = machine.load(&program).unwrap();
    bind_all(&mut run, &[]).unwrap();
    let error = run.run().unwrap_err();
    assert_eq!(primary(&error, &program), "the function failed");
    assert_eq!(
        std::error::Error::source(&error).unwrap().to_string(),
        "no functions are provided"
    );
}

#[test]
fn host_result_memory() {
    // `many(n)` returns n words of parts; the program needs 1 stack word
    // and nothing else, so a machine of 1 + k words takes a list of k.
    let program = compile_in("many(3).len()", &[]);
    assert_eq!(program.memory().total(), 1);

    let run = |memory: u32| {
        let mut machine = Machine::new(Limits::new().memory(memory));
        machine.load(&program).unwrap().run_with(&mut test_host)
    };

    assert_eq!(run(4).unwrap().value, int(3));
    assert_eq!(run(4).unwrap().memory, 4);
    assert_eq!(
        chain(&run(3).unwrap_err(), &program),
        "the value needs 3 words of memory, 2 are left: while reading what `many` returned: while running instruction 1: while running the program"
    );
}

#[test]
fn binding_errors() {
    let program = compile_in(
        r#"wits + bonus.unwrap_or(0) + levels[0] + grid[0][0] + m["Brawl (bar)"] + count(health) + count(maybe_track.unwrap_or(health))"#,
        &[],
    );
    let mut machine = Machine::default();

    let cases: &[(&str, Value, &str)] = &[
        (
            "wits",
            Value::bool(true),
            "expected Int, found Bool: while binding `wits`",
        ),
        (
            "wits",
            Value::some(int(1)),
            "expected Int, found some(..): while binding `wits`",
        ),
        (
            "wits",
            Value::none(),
            "expected Int, found none: while binding `wits`",
        ),
        (
            "bonus",
            int(1),
            "expected Option<Int>, found Int: while binding `bonus`",
        ),
        (
            "bonus",
            Value::some(Value::bool(true)),
            "expected Option<Int>, found some(Bool): while binding `bonus`",
        ),
        (
            "levels",
            Value::list(vec![int(1), Value::bool(false)]),
            "expected Int, found Bool: at `levels[1]`: while binding `levels`",
        ),
        (
            "levels",
            int(1),
            "expected List<Int>, found Int: while binding `levels`",
        ),
        (
            "grid",
            Value::list(vec![ints(&[1]), int(2)]),
            "expected List<Int>, found Int: at `grid[1]`: while binding `grid`",
        ),
        (
            "m",
            Value::record([("Brawl (bar)", int(1)), ("a b", track(1))]),
            r#"expected {"Brawl (bar)": Int, "a b": {boxes: Int, count: List<Int>}, inner: {deep: Bool}}, found a record without `inner`: while binding `m`"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", int(1)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", Value::bool(true))])),
                ("zzz", int(0)),
            ]),
            r#"expected {"Brawl (bar)": Int, "a b": {boxes: Int, count: List<Int>}, inner: {deep: Bool}}, found a record with an extra field: while binding `m`"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", Value::bool(true)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", Value::bool(true))])),
            ]),
            r#"expected Int, found Bool: at `m["Brawl (bar)"]`: while binding `m`"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", int(1)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", int(0))])),
            ]),
            "expected Bool, found Int: at `m.inner.deep`: while binding `m`",
        ),
        (
            "health",
            Value::record([("boxes", int(1))]),
            "expected {boxes: Int, count: List<Int>}, found a record without `count`: while binding `health`",
        ),
        (
            "health",
            Value::record([
                ("boxes", int(1)),
                ("count", Value::list(vec![Value::none()])),
            ]),
            "expected Int, found none: at `health.count[0]`: while binding `health`",
        ),
        (
            "maybe_track",
            track(1),
            "expected Option<{boxes: Int, count: List<Int>}>, found a record: while binding `maybe_track`",
        ),
        (
            "strength",
            int(1),
            "the program reads no value of that name",
        ),
        ("nothing", int(1), "the program reads no value of that name"),
    ];

    for (name, value, message) in cases {
        let mut run = machine.load(&program).unwrap();
        let error = run.bind(name, value).unwrap_err();
        assert_eq!(chain(&error, &program), *message, "{name} = {value}");
    }

    // Binding twice is an error, and so is not binding at all.
    let mut run = machine.load(&program).unwrap();
    run.bind("wits", &int(1)).unwrap();
    assert_eq!(
        chain(&run.bind("wits", &int(1)).unwrap_err(), &program),
        "`wits` is already bound: while binding `wits`"
    );

    let mut run = machine.load(&program).unwrap();
    run.bind("wits", &int(1)).unwrap();
    assert_eq!(
        chain(&run.run_with(&mut test_host).unwrap_err(), &program),
        "`bonus` is not bound, bind an Option<Int> before running: while running the program"
    );

    // A failed binding leaves the value unbound, and a good one binds.
    let mut run = machine.load(&program).unwrap();
    run.bind("wits", &Value::bool(true)).unwrap_err();
    run.bind("wits", &int(1)).unwrap();
}

#[test]
fn memory_budget() {
    // `[10, 11, 12, 13].get(i)` takes 2 stack words, 4 constant words
    // and a word for `i`.
    let mut env = Environment::new();
    env.value("i", Type::int()).unwrap();
    let program = compile("[10, 11, 12, 13].get(i)", &env).unwrap();
    assert_eq!(
        program.memory(),
        Memory {
            stack: 2,
            heap: 0,
            constants: 4,
            globals: 1,
        }
    );

    let mut machine = Machine::new(Limits::new().memory(7));
    let mut run = machine.load(&program).unwrap();
    run.bind("i", &int(2)).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, Value::some(int(12)));
    assert_eq!(outcome.memory, 7);

    let mut machine = Machine::new(Limits::new().memory(6));
    assert_eq!(
        machine.load(&program).unwrap_err().to_string(),
        "the program needs 7 words of memory, more than the 6 the machine allows"
    );

    // Lists built as the program runs take heap: `[i, i, i]` is 3 stack, 3
    // heap and 1 for `i`.
    let program = compile("[i, i, i]", &env).unwrap();
    assert_eq!(program.memory().total(), 7);
    assert!(Machine::new(Limits::new().memory(7)).load(&program).is_ok());
    assert!(
        Machine::new(Limits::new().memory(6))
            .load(&program)
            .is_err()
    );

    let mut machine = Machine::new(Limits::new().memory(7));
    let mut run = machine.load(&program).unwrap();
    run.bind("i", &int(1)).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, ints(&[1, 1, 1]));
    assert!(outcome.memory <= 7, "{outcome:?}");

    // The default machine refuses a program past its memory.
    let source = format!("[{}]", vec!["i"; 600].join(", "));
    let program = compile_with(&source, &env, &Options::new()).unwrap();
    assert_eq!(
        Machine::default().load(&program).unwrap_err().message(),
        &Message::TooMuchMemory {
            needed: 1201,
            allowed: DEFAULT_MEMORY
        }
    );
}

#[test]
fn binding_memory() {
    // `l.len()` takes 1 stack word and 1 for `l`, so a machine of 2 + k
    // words takes a bound list of k entries, and no more.
    let mut env = Environment::new();
    env.value("l", Type::list(Type::int())).unwrap();
    env.value("s", Type::string()).unwrap();
    let program = compile("l.len()", &env).unwrap();
    assert_eq!(program.memory().total(), 2);

    let mut machine = Machine::new(Limits::new().memory(5));
    let mut run = machine.load(&program).unwrap();
    run.bind("l", &ints(&[1, 2, 3])).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, int(3));
    assert_eq!(outcome.memory, 5);

    let mut run = machine.load(&program).unwrap();
    assert_eq!(
        chain(&run.bind("l", &ints(&[1, 2, 3, 4])).unwrap_err(), &program),
        "the value needs 4 words of memory, 3 are left: while binding `l`"
    );

    // A string takes a word per 8 bytes.
    let program = compile(r#"[s, s].len()"#, &env).unwrap();
    let needed = program.memory().total();
    let mut machine = Machine::new(Limits::new().memory(needed as u32 + 2));
    let mut run = machine.load(&program).unwrap();
    run.bind("s", &Value::from("0123456789abcdef")).unwrap();
    assert_eq!(run.run().unwrap().value, int(2));

    let mut run = machine.load(&program).unwrap();
    assert_eq!(
        chain(
            &run.bind("s", &Value::from("0123456789abcdef!"))
                .unwrap_err(),
            &program
        ),
        "the value needs 3 words of memory, 2 are left: while binding `s`"
    );
}

#[test]
fn fuel() {
    // `wits + dexterity` is three instructions, so three steps.
    let outcome = eval_in("wits + dexterity", &[], Limits::new()).unwrap();
    assert_eq!(outcome.steps, 3);
    assert!(eval_in("wits + dexterity", &[], Limits::new().fuel(3)).is_ok());
    assert_eq!(
        eval_in("wits + dexterity", &[], Limits::new().fuel(2))
            .unwrap_err()
            .to_string(),
        "the expression took too long: it used all of its 2 steps"
    );
    assert_eq!(
        eval_in("wits", &[], Limits::new().fuel(0))
            .unwrap_err()
            .message(),
        &Message::OutOfFuel { fuel: 0 }
    );

    // A list of n entries, `min` and `max` of n take n more steps, a call
    // one more per word it returns, and a map lookup one more per key it
    // probes and per word of a string key it compares.
    for (source, steps) in [
        ("[wits, wits, wits]", 7),
        ("Int::min(wits, wits, wits)", 7),
        ("Int::max(wits, wits)", 5),
        ("many(4)", 6),
        ("wits.min(wits)", 5),
        ("flag || flag", 2),
        ("!flag || flag", 4),
        ("ids[7]", 4),
        ("ids.get(-4)", 5),
        ("tags[\"brawl\"]", 7),
        ("tags.get(\"athletics (long name)\")", 7),
    ] {
        let outcome = eval_in(source, &[], Limits::new()).unwrap();
        assert_eq!(outcome.steps, steps, "{source:?}");
        assert!(eval_in(source, &[], Limits::new().fuel(steps)).is_ok());
        assert_eq!(
            eval_in(source, &[], Limits::new().fuel(steps - 1))
                .unwrap_err()
                .message(),
            &Message::OutOfFuel { fuel: steps - 1 },
            "{source:?}"
        );
    }

    // The default is low: a sum of 501 terms takes 1001 steps, past it,
    // which a raised limit allows.
    assert_eq!(DEFAULT_FUEL, 1000);
    let source = vec!["wits"; 501].join(" + ");
    let raised = Limits::new().fuel(1001);
    assert_eq!(eval_in(&source, &[], raised).unwrap().value, int(1002));
    assert_eq!(
        eval_in(&source, &[], Limits::new()).unwrap_err().message(),
        &Message::OutOfFuel { fuel: 1000 }
    );
    let source = vec!["wits"; 500].join(" + ");
    assert_eq!(eval_in(&source, &[], Limits::new()).unwrap().steps, 999);
}

#[test]
fn long_string_keys_spend_fuel() {
    // Two keys of a MiB, alike but for their last byte, so each lookup
    // compares about 2^17 words a probe.
    let long = |last: char| format!("{}{last}", "x".repeat(1 << 20));
    let map = Value::map([
        (Value::string(long('a')), int(1)),
        (Value::string(long('b')), int(2)),
    ]);
    let extra = [
        ("m", Type::map(Type::string(), Type::int()), map),
        ("k", Type::string(), Value::string(long('a'))),
    ];

    let source = vec!["m[k]"; 9_000].join(" + ");
    let limits = Limits::new().memory(1 << 20).fuel(1 << 20);

    let start = std::time::Instant::now();
    let error = eval_in(&source, &extra, limits).unwrap_err();
    let took = start.elapsed();
    assert!(
        took < std::time::Duration::from_secs(5),
        "running took {took:?}"
    );
    assert_eq!(error.message(), &Message::OutOfFuel { fuel: 1 << 20 });

    // One lookup fits.
    let limits = Limits::new().memory(1 << 20).fuel(1 << 20);
    assert_eq!(eval_in("m[k]", &extra, limits).unwrap().value, int(1));
}

#[test]
fn stored_programs_run() {
    for (source, expected) in CASES {
        let program = compile_in(source, &[]);
        let loaded = Program::from_bytes(&program.to_bytes().unwrap()).unwrap();
        let mut machine = Machine::default();
        let mut run = machine.load(&loaded).unwrap();
        bind_all(&mut run, &[]).unwrap();
        let outcome = run.run_with(&mut test_host).unwrap();

        if !expected.is_empty() {
            assert_eq!(outcome.value.to_string(), *expected, "{source:?}");
        }
    }
}

#[test]
fn machines_are_reused() {
    let program = compile_in("[wits, dexterity]", &[]);
    let mut machine = Machine::new(Limits::new().memory(6));

    for n in 0..10 {
        let mut run = machine.load(&program).unwrap();
        run.bind("wits", &int(n)).unwrap();
        run.bind("dexterity", &int(-n)).unwrap();
        assert_eq!(run.run().unwrap().value, ints(&[n, -n]));
    }
}

/// A program that evalite would not make gives a fault or a wrong value,
/// never a panic: memory is untyped, so what the verifier would refuse
/// but stays inside memory runs on whatever words it finds.
#[test]
fn faulty_programs() {
    let mut types = Types::new();
    let int = types.add(TypeNode::Int).unwrap();

    let faulty = |code: Vec<Instr>, stack: u32| Program {
        types: types.clone(),
        constants: Vec::new(),
        globals: Vec::new(),
        functions: Vec::new(),
        code,
        result: int,
        memory: Memory {
            stack,
            heap: 0,
            constants: 0,
            globals: 0,
        },
        global_types: Vec::new(),
        global_at: Vec::new(),
        constant_at: Vec::new(),
    };

    let global = Instr::Global { at: 0, ty: int };

    let faults = [
        (vec![Instr::Add], 2),
        (vec![Instr::Int(1), Instr::Int(2)], 1),
        (vec![Instr::Int(1), Instr::Int(2)], 2),
        (vec![Instr::Const(3)], 1),
        (vec![Instr::Int(1), Instr::Field { at: 0, ty: int }], 1),
        (vec![Instr::Int(1), Instr::List { len: 1, entry: int }], 1),
        (vec![Instr::Call(0)], 1),
        (vec![], 0),
        (vec![Instr::Int(-1), Instr::Int(0), Instr::Index(int)], 2),
        (
            vec![Instr::Global {
                at: u32::MAX,
                ty: int,
            }],
            1,
        ),
    ];

    for (code, stack) in faults {
        let program = faulty(code.clone(), stack);
        let mut machine = Machine::default();
        let error = machine.load(&program).unwrap().run().unwrap_err();
        assert!(error.is_fault(), "{code:?}: {error}");
    }

    let wrong = [
        (vec![Instr::Bool(true), Instr::Neg], 1, Value::int(-1)),
        (vec![global], 1, Value::int(0)),
        (vec![Instr::Int(1), Instr::Len], 1, Value::int(0)),
        (
            vec![Instr::Bool(true), Instr::Or(u32::MAX)],
            1,
            Value::int(1),
        ),
    ];

    for (code, stack, value) in wrong {
        let program = faulty(code.clone(), stack);
        let mut machine = Machine::default();
        let outcome = machine.load(&program).unwrap().run().unwrap();
        assert_eq!(outcome.value, value, "{code:?}");
    }
}

/// Random well-typed programs run without panicking, use no more memory
/// than they declare plus their bindings, and fail only for the reasons a
/// program may.
#[test]
fn random_programs() {
    let mut rng = Rng(0x5eed_f00d);
    let options = Options::new().max_depth(10_000);
    let env = env();
    let mut ran = 0;

    for round in 0..3_000 {
        let ty = rng.pick(&["Int", "Bool", "Option<Int>", "List<Int>"]);
        let budget = rng.below(40);
        let source = generate(&mut rng, ty, budget);
        let program = compile_with(&source, &env, &options).unwrap();

        // Exactly the memory it needs: its own, the values it reads and
        // the parts of those values.
        let parts = program
            .globals()
            .map(|(name, _)| binding(name).1 as u64)
            .sum::<u64>();
        let needed = program.memory().total() + parts;

        let mut machine = Machine::new(Limits::new().memory(needed as u32).fuel(u64::MAX));
        let mut run = machine.load(&program).unwrap();
        bind_all(&mut run, &[]).unwrap();

        match run.run_with(&mut host) {
            Ok(outcome) => {
                ran += 1;
                assert!(outcome.memory <= needed, "{source:?}: {outcome:?}");
                assert!(outcome.steps <= program.code.len() as u64 * 2, "{source:?}");
                assert_eq!(
                    describe(&outcome.value),
                    ty,
                    "{round}: {source:?} gave {}",
                    outcome.value
                );
            }
            Err(error)
                if matches!(
                    error.message(),
                    Message::Overflow { .. }
                        | Message::DivideByZero { .. }
                        | Message::IndexOutOfRange { .. }
                        | Message::ClampBounds { .. }
                ) => {}
            Err(error) => panic!("{round}: {source:?}: {error}"),
        }

        // With a step too few it runs out of fuel or fails as before,
        // never faults.
        let mut machine = Machine::new(
            Limits::new()
                .memory(needed as u32)
                .fuel(rng.below(20) as u64),
        );
        let mut run = machine.load(&program).unwrap();
        bind_all(&mut run, &[]).unwrap();

        if let Err(error) = run.run_with(&mut host)
            && error.is_fault()
        {
            panic!("{round}: {source:?}: {error:#}");
        }

        // A word less is refused before it runs, or a binding is.
        {
            let mut machine = Machine::new(Limits::new().memory(needed as u32 - 1));

            if let Ok(mut run) = machine.load(&program) {
                assert!(bind_all(&mut run, &[]).is_err(), "{source:?}");
            }
        }
    }

    assert!(ran > 1_000, "only {ran} programs ran to a value");
}

/// The type a generated program's value has, as [`generate`] names it.
fn describe(value: &Value) -> &'static str {
    match value {
        Value(Repr::Int(_)) => "Int",
        Value(Repr::Bool(_)) => "Bool",
        Value(Repr::None) | Value(Repr::Some(..)) => "Option<Int>",
        Value(Repr::List(items)) if items.iter().all(|i| matches!(i, Value(Repr::Int(_)))) => {
            "List<Int>"
        }
        _ => "?",
    }
}

/// Whether `value` is of type `ty`.
fn conforms(value: &Value, ty: &Type) -> bool {
    match (ty, value) {
        (Type(Kind::Int), Value(Repr::Int(_))) | (Type(Kind::Bool), Value(Repr::Bool(_))) => true,
        (Type(Kind::String), Value(Repr::String(_))) => true,
        (Type(Kind::Option(_)), Value(Repr::None)) => true,
        (Type(Kind::Option(inner)), Value(Repr::Some(value))) => conforms(value, inner),
        (Type(Kind::List(entry)), Value(Repr::List(items))) => {
            items.iter().all(|i| conforms(i, entry))
        }
        (Type(Kind::Record(fields)), Value(Repr::Record(map))) => {
            fields.len() == map.len()
                && fields
                    .iter()
                    .all(|(name, ty)| map.get(name).is_some_and(|v| conforms(v, ty)))
        }
        (Type(Kind::Map(key, value)), Value(Repr::Map(entries))) => entries
            .iter()
            .all(|(k, v)| conforms(k, key) && conforms(v, value)),
        _ => false,
    }
}

/// A value of type `ty`, from the edges of each type's range.
fn value_of(rng: &mut Rng, ty: &Type) -> Value {
    match ty {
        Type(Kind::Int) => int([0, 1, -1, 7, i64::MIN, i64::MAX][rng.below(6)]),
        Type(Kind::Bool) => Value::bool(rng.below(2) == 0),
        Type(Kind::String) => Value::from(rng.pick(&["", "Essence", "0123456789abcdefg"])),
        Type(Kind::Option(inner)) => match rng.below(3) {
            0 => Value::none(),
            _ => Value::some(value_of(rng, inner)),
        },
        Type(Kind::List(entry)) => Value::list((0..rng.below(4)).map(|_| value_of(rng, entry))),
        Type(Kind::Record(fields)) => Value::record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), value_of(rng, ty))),
        ),
        Type(Kind::Map(key, value)) => {
            Value::map((0..rng.below(4)).map(|_| (value_of(rng, key), value_of(rng, value))))
        }
    }
}

/// Every option layout, through every place a value is held: bound,
/// returned, built into a list, read from a record's field and given back
/// as the result. An `Option<Int>` is two words; every other option is
/// one word with a value no `Bool` or reference is for absent.
#[test]
fn option_layouts() {
    let record = Type::record([("a", Type::int()), ("b", Type::option(Type::int()))]);

    let cases: Vec<(Type, Vec<Value>)> = vec![
        (
            Type::int(),
            vec![int(0), int(-1), int(1), int(i64::MIN), int(i64::MAX)],
        ),
        (Type::bool(), vec![Value::bool(false), Value::bool(true)]),
        (
            Type::string(),
            vec![
                Value::from(""),
                Value::from("\u{ff}\u{ff}\u{ff}\u{ff}"),
                Value::from("0123456789abcdefg"),
            ],
        ),
        (
            Type::list(Type::int()),
            vec![ints(&[-1]), ints(&[i64::MIN, i64::MAX, 0])],
        ),
        (
            record.clone(),
            vec![
                Value::record([("a", int(-1)), ("b", Value::none())]),
                Value::record([("a", int(i64::MAX)), ("b", Value::some(int(-1)))]),
            ],
        ),
    ];

    for (ty, values) in cases {
        let option = Type::option(ty.clone());
        let holder = Type::record([("o", option.clone()), ("z", Type::int())]);

        let mut env = Environment::new();
        env.value("o", option.clone()).unwrap();
        env.value("d", ty.clone()).unwrap();
        env.value("r", holder.clone()).unwrap();
        env.value("l", Type::list(option.clone())).unwrap();
        env.function("give", [], option.clone()).unwrap();
        env.function("take", [option.clone(), Type::int()], Type::int())
            .unwrap();

        for value in &values {
            for o in [Value::none(), Value::some(value.clone())] {
                let default = values[0].clone();
                let unwrapped = match &o {
                    Value(Repr::Some(v)) => (**v).clone(),
                    _ => default.clone(),
                };

                let expected: &[(&str, Value)] = &[
                    ("o", o.clone()),
                    ("o.is_some()", Value::bool(o != Value::none())),
                    ("o.is_none()", Value::bool(o == Value::none())),
                    ("o.unwrap_or(d)", unwrapped.clone()),
                    ("[o, o, o][2]", o.clone()),
                    ("[o, give()]", Value::list(vec![o.clone(), o.clone()])),
                    ("[give(), o].get(1)", Value::some(o.clone())),
                    ("[give(), o].get(1)!", o.clone()),
                    ("[give(), o].get(9)", Value::none()),
                    ("r.o", o.clone()),
                    ("r.z", int(-5)),
                    ("l[1]", o.clone()),
                    ("l.len()", int(3)),
                    ("give().unwrap_or(d)", unwrapped.clone()),
                    ("take(o, 4)", int(4)),
                    ("take(give(), -1)", int(-1)),
                    ("r", Value::record([("o", o.clone()), ("z", int(-5))])),
                ];

                for (source, want) in expected {
                    let program = compile(source, &env).unwrap();
                    let mut machine = Machine::default();
                    let mut run = machine.load(&program).unwrap();

                    for (name, _) in run.globals() {
                        let bound = match name {
                            "o" => o.clone(),
                            "d" => default.clone(),
                            "r" => Value::record([("o", o.clone()), ("z", int(-5))]),
                            "l" => Value::list(vec![
                                Value::none(),
                                o.clone(),
                                Value::some(default.clone()),
                            ]),
                            _ => unreachable!(),
                        };

                        run.bind(name, &bound).unwrap();
                    }

                    let given = o.clone();
                    let mut host = |name: &str, args: Vec<Value>| match (name, args.as_slice()) {
                        ("give", []) => Ok(given.clone()),
                        ("take", [arg, Value(Repr::Int(n))]) => {
                            assert_eq!(*arg, given, "{source:?}");
                            Ok(int(*n))
                        }
                        _ => Err(format!("unexpected {name}{args:?}")),
                    };

                    let outcome = run.run_with(&mut host).unwrap();
                    assert_eq!(outcome.value, *want, "{source:?} of {option} with o = {o}");
                }
            }
        }
    }
}

/// A list of `Option<Int>` takes two words an entry, wherever it is.
#[test]
fn list_of_optional_ints() {
    let mut env = Environment::new();
    env.value("l", Type::list(Type::option(Type::int())))
        .unwrap();
    env.value("o", Type::option(Type::int())).unwrap();
    env.function("many", [Type::int()], Type::list(Type::option(Type::int())))
        .unwrap();

    let l = Value::list(vec![
        Value::none(),
        Value::some(int(i64::MIN)),
        Value::some(int(i64::MAX)),
        Value::some(int(-1)),
        Value::some(int(0)),
    ]);

    let cases: &[(&str, &str)] = &[
        (
            "l",
            "[none, some(-9223372036854775808), some(9223372036854775807), some(-1), some(0)]",
        ),
        ("l[0]", "none"),
        ("l[1].unwrap_or(5)", "-9223372036854775808"),
        ("l[2].unwrap_or(5)", "9223372036854775807"),
        ("l[3].is_some()", "true"),
        ("l[3].unwrap_or(5)", "-1"),
        ("l[0].unwrap_or(5)", "5"),
        ("l.get(-9)", "none"),
        ("l.get(99)", "none"),
        ("l.get(0)", "some(none)"),
        ("l.get(1)", "some(some(-9223372036854775808))"),
        ("l.get(4)", "some(some(0))"),
        ("l.get(4)!", "some(0)"),
        ("l.get(4)!!", "0"),
        ("l.get(0)!.is_none()", "true"),
        ("l.get(0).is_some()", "true"),
        ("l.get(9).is_some()", "false"),
        ("l.get(0).unwrap_or(o)", "none"),
        ("l.get(9).unwrap_or(o)", "some(-1)"),
        ("l.get(2).unwrap_or(o)", "some(9223372036854775807)"),
        (
            "[l.get(0), l.get(9), l.get(3)]",
            "[some(none), none, some(some(-1))]",
        ),
        ("l.len()", "5"),
        (
            "[l[2], o, l[0]]",
            "[some(9223372036854775807), some(-1), none]",
        ),
        ("[l[2], o, l[0]][1].unwrap_or(3)", "-1"),
        ("[l, [o]][1]", "[some(-1)]"),
        ("many(2)", "[some(0), some(1)]"),
        ("many(3)[2].unwrap_or(-7)", "2"),
    ];

    for (source, expected) in cases {
        let program = compile(source, &env).unwrap();
        let mut machine = Machine::default();
        let mut run = machine.load(&program).unwrap();

        for (name, _) in run.globals() {
            match name {
                "l" => run.bind("l", &l).unwrap(),
                _ => run.bind("o", &Value::some(int(-1))).unwrap(),
            }
        }

        let mut host = |_: &str, args: Vec<Value>| match args.as_slice() {
            [Value(Repr::Int(n))] => Ok(Value::list((0..*n).map(|i| Value::some(int(i))))),
            _ => Err("unexpected".to_owned()),
        };

        let outcome = run.run_with(&mut host).unwrap();
        assert_eq!(outcome.value.to_string(), *expected, "{source:?}");
    }

    assert_eq!(
        compile("l[1]", &env).unwrap().memory(),
        Memory {
            stack: 2,
            heap: 0,
            constants: 0,
            globals: 1,
        }
    );
    assert_eq!(
        compile("[o, o, o]", &env).unwrap().memory(),
        Memory {
            stack: 6,
            heap: 6,
            constants: 0,
            globals: 2,
        }
    );
}

/// Memory is counted in words at the budget's boundary: a program's own,
/// what it binds and what the caller's functions return.
#[test]
fn word_budget() {
    let mut env = Environment::new();
    env.value("l", Type::list(Type::option(Type::int())))
        .unwrap();
    env.value("o", Type::option(Type::int())).unwrap();
    env.value("s", Type::string()).unwrap();
    env.function("many", [Type::int()], Type::list(Type::option(Type::int())))
        .unwrap();

    // A program's own memory: `[o, o, o]` is 6 stack, 6 heap and 2 for `o`.
    let program = compile("[o, o, o]", &env).unwrap();
    assert_eq!(program.memory().total(), 14);

    let mut machine = Machine::new(Limits::new().memory(14));
    let mut run = machine.load(&program).unwrap();
    run.bind("o", &Value::some(int(i64::MIN))).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(
        outcome.value.to_string(),
        "[some(-9223372036854775808), some(-9223372036854775808), some(-9223372036854775808)]"
    );
    assert_eq!(outcome.memory, 14);

    assert_eq!(
        Machine::new(Limits::new().memory(13))
            .load(&program)
            .unwrap_err()
            .to_string(),
        "the program needs 14 words of memory, more than the 13 the machine allows"
    );

    // A bound list of k `Option<Int>`s takes 2k words beside `l.len()`'s 2.
    let program = compile("l.len()", &env).unwrap();
    assert_eq!(program.memory().total(), 2);

    for k in [1usize, 2, 7] {
        let list = Value::list(vec![Value::some(int(-1)); k]);
        let words = 2 + 2 * k as u32;

        let mut machine = Machine::new(Limits::new().memory(words));
        let mut run = machine.load(&program).unwrap();
        run.bind("l", &list).unwrap();
        let outcome = run.run().unwrap();
        assert_eq!(outcome.value, int(k as i64));
        assert_eq!(outcome.memory, u64::from(words));

        let mut machine = Machine::new(Limits::new().memory(words - 1));
        let mut run = machine.load(&program).unwrap();
        assert_eq!(
            chain(&run.bind("l", &list).unwrap_err(), &program),
            format!(
                "the value needs {} words of memory, {} are left: while binding `l`",
                2 * k,
                2 * k - 1
            )
        );
    }

    // A string takes a word per 8 bytes, the empty string none.
    let program = compile("[s].get(0)", &env).unwrap();
    let base = program.memory().total() as u32;

    for (s, words) in [("", 0), ("12345678", 1), ("123456789", 2)] {
        let mut machine = Machine::new(Limits::new().memory(base + words));
        let mut run = machine.load(&program).unwrap();
        run.bind("s", &Value::from(s)).unwrap();
        assert_eq!(run.run().unwrap().value, Value::some(Value::from(s)));

        if words > 0 {
            let mut machine = Machine::new(Limits::new().memory(base + words - 1));
            let mut run = machine.load(&program).unwrap();
            assert!(matches!(
                run.bind("s", &Value::from(s)).unwrap_err().message(),
                Message::TooLarge { .. }
            ));
        }
    }

    // What a function returns: `many(k)` gives 2k words of parts, and
    // costs a step a word.
    let program = compile("many(3).len()", &env).unwrap();
    assert_eq!(program.memory().total(), 1);

    let run = |memory: u32| {
        let mut machine = Machine::new(Limits::new().memory(memory));
        let mut host = |_: &str, args: Vec<Value>| match args.as_slice() {
            [Value(Repr::Int(n))] => Ok(Value::list(vec![Value::none(); *n as usize])),
            _ => Err("unexpected".to_owned()),
        };
        machine.load(&program).unwrap().run_with(&mut host)
    };

    let outcome = run(7).unwrap();
    assert_eq!(outcome.value, int(3));
    assert_eq!(outcome.memory, 7);
    assert_eq!(outcome.steps, 3 + 6);
    assert_eq!(
        run(6).unwrap_err().to_string(),
        "the value needs 6 words of memory, 5 are left"
    );
}

/// An instruction of any kind, with operands near the ones programs use.
fn instruction(rng: &mut Rng, types: u32) -> Instr {
    let n = |rng: &mut Rng| rng.below(5) as u32;
    let ty = |rng: &mut Rng| TypeId(rng.below(types as usize + 2) as u32);

    match rng.below(36) {
        0 => Instr::Int([0, 1, -1, i64::MIN, i64::MAX][rng.below(5)]),
        1 => Instr::Bool(rng.below(2) == 0),
        2 => Instr::Const(n(rng)),
        3 => Instr::Global {
            at: n(rng),
            ty: ty(rng),
        },
        4 => Instr::Neg,
        5 => Instr::Not,
        6 => Instr::Add,
        7 => Instr::Sub,
        8 => Instr::Mul,
        9 => Instr::Div,
        10 => Instr::Rem,
        11 => Instr::Eq,
        12 => Instr::Ne,
        13 => Instr::Lt,
        14 => Instr::Le,
        15 => Instr::Gt,
        16 => Instr::Ge,
        17 => Instr::And(n(rng)),
        18 => Instr::Or(n(rng)),
        19 => Instr::Field {
            at: n(rng),
            ty: ty(rng),
        },
        20 => Instr::Index(ty(rng)),
        21 => Instr::List {
            len: n(rng),
            entry: ty(rng),
        },
        22 => Instr::Min(n(rng)),
        23 => Instr::Max(n(rng)),
        24 => Instr::Abs,
        25 => Instr::Clamp,
        26 => Instr::Get(ty(rng)),
        27 => Instr::UnwrapOr(ty(rng)),
        28 => Instr::IsSome(ty(rng)),
        29 => Instr::IsNone(ty(rng)),
        30 => Instr::Len,
        31 => Instr::Unwrap(ty(rng)),
        32 => Instr::MapIndex(ty(rng)),
        33 => Instr::MapGet(ty(rng)),
        34 => Instr::Map {
            keys: n(rng),
            map: ty(rng),
        },
        _ => Instr::Call(n(rng)),
    }
}

/// `instr` with one operand moved a little.
fn nudge(rng: &mut Rng, instr: Instr) -> Instr {
    let mut d = |n: u32| match rng.below(3) {
        0 => n.wrapping_add(1),
        1 => n.wrapping_sub(1),
        _ => n.wrapping_add(rng.below(4) as u32),
    };

    match instr {
        Instr::Const(c) => Instr::Const(d(c)),
        Instr::Global { at, ty } if at % 2 == 0 => Instr::Global { at: d(at), ty },
        Instr::Global { at, ty } => Instr::Global {
            at,
            ty: TypeId(d(ty.0)),
        },
        Instr::And(n) => Instr::And(d(n)),
        Instr::Or(n) => Instr::Or(d(n)),
        Instr::Field { at, ty } if at % 2 == 0 => Instr::Field { at: d(at), ty },
        Instr::Field { at, ty } => Instr::Field {
            at,
            ty: TypeId(d(ty.0)),
        },
        Instr::Index(ty) => Instr::Index(TypeId(d(ty.0))),
        Instr::List { len, entry } => Instr::List { len: d(len), entry },
        Instr::Min(n) => Instr::Min(d(n)),
        Instr::Max(n) => Instr::Max(d(n)),
        Instr::Get(ty) => Instr::Get(TypeId(d(ty.0))),
        Instr::UnwrapOr(ty) => Instr::UnwrapOr(TypeId(d(ty.0))),
        Instr::IsSome(ty) => Instr::IsNone(TypeId(d(ty.0))),
        Instr::IsNone(ty) => Instr::IsSome(TypeId(d(ty.0))),
        Instr::Unwrap(ty) => Instr::Unwrap(TypeId(d(ty.0))),
        Instr::MapIndex(ty) => Instr::MapGet(TypeId(d(ty.0))),
        Instr::MapGet(ty) => Instr::MapIndex(TypeId(d(ty.0))),
        Instr::Map { keys, map } if keys % 2 == 0 => Instr::Map { keys: d(keys), map },
        Instr::Map { keys, map } => Instr::Map {
            keys,
            map: TypeId(d(map.0)),
        },
        Instr::Call(f) => Instr::Call(d(f)),
        Instr::Int(n) => Instr::Int(n.wrapping_add(1)),
        Instr::Bool(b) => Instr::Bool(!b),
        other => other,
    }
}

/// Tamper with a stored program: its instructions, their operands and the
/// type ids it names.
fn tamper(rng: &mut Rng, stored: &mut Stored) {
    let types = stored.types.len() as u32;
    let len = stored.code.len();

    match rng.below(11) {
        0..=2 if len > 0 => {
            let at = rng.below(len);
            stored.code[at] = nudge(rng, stored.code[at]);
        }
        9 if !stored.constants.is_empty() => {
            // A constant list's entries reordered or repeated, which a map's
            // keys must not be.
            let at = rng.below(stored.constants.len());

            if let Constant::List { items, .. } = &mut stored.constants[at] {
                match rng.below(2) {
                    0 => items.reverse(),
                    _ => {
                        let first = items[0];
                        items.push(first);
                    }
                }
            }
        }
        3 | 4 if len > 0 => {
            let at = rng.below(len);
            stored.code[at] = instruction(rng, types);
        }
        5 => {
            let at = rng.below(len + 1);
            stored.code.insert(at, instruction(rng, types));
        }
        6 if len > 0 => {
            stored.code.remove(rng.below(len));
        }
        7 if len > 1 => {
            let (a, b) = (rng.below(len), rng.below(len));
            stored.code.swap(a, b);
        }
        8 => {
            let ty = TypeId(rng.below(types as usize + 1) as u32);

            match rng.below(3) {
                0 if !stored.globals.is_empty() => {
                    let at = rng.below(stored.globals.len());
                    stored.globals[at].ty = ty;
                }
                1 if !stored.functions.is_empty() => {
                    let at = rng.below(stored.functions.len());
                    stored.functions[at].ret = ty;
                }
                _ => stored.result = ty,
            }
        }
        _ => {
            let id = |rng: &mut Rng| TypeId(rng.below(types as usize) as u32);

            let fresh = match rng.below(5) {
                0 => TypeNode::Int,
                1 => TypeNode::Option(id(rng)),
                2 => TypeNode::List(id(rng)),
                3 => TypeNode::Map {
                    key: id(rng),
                    value: id(rng),
                },
                _ => TypeNode::Record(vec![]),
            };
            stored.types.push(fresh);
        }
    }

    // A verified program declares the memory it uses, so a tamper that
    // changes it is caught there; recount it as a forger would, so the
    // code itself is what gets checked.
    if let Ok(program) = Program::verify(
        Types::from_stored(stored.types.clone()).unwrap_or_default(),
        stored.constants.clone(),
        stored.globals.clone(),
        stored.functions.clone(),
        stored.code.clone(),
        stored.result,
    ) {
        stored.memory = program.memory();
    }
}

/// Stored programs tampered with are refused when they load, or run
/// without a fault or a panic, giving a value of their result type and
/// handing the caller's functions arguments of their declared types: the
/// verifier's proof is what the untyped machine relies on.
#[test]
fn tampered_programs_never_fault() {
    let mut rng = Rng(0x7a4b_e5ed);
    let options = Options::new().max_depth(10_000);
    let env = env();
    let (mut refused, mut ran) = (0, 0);

    for round in 0..3_000 {
        let ty = rng.pick(&["Int", "Bool", "Option<Int>", "List<Int>"]);
        let budget = rng.below(30);
        let source = generate(&mut rng, ty, budget);
        let program = compile_with(&source, &env, &options).unwrap();
        let bytes = program.to_bytes().unwrap();

        for _ in 0..4 {
            let mut stored: Stored = musli::storage::from_slice(&bytes).unwrap();

            for _ in 0..=rng.below(3) {
                tamper(&mut rng, &mut stored);
            }

            let tampered = musli::storage::to_vec(&stored).unwrap();

            let Ok(program) = Program::from_bytes(&tampered) else {
                refused += 1;
                continue;
            };

            let limit = u32::try_from(program.memory().total()).unwrap() + 4096;
            let mut machine = Machine::new(Limits::new().memory(limit).fuel(100_000));
            let mut run = machine.load(&program).unwrap();

            for (name, ty) in run.globals() {
                let value = value_of(&mut rng, ty);
                run.bind(name, &value).unwrap();
            }

            let functions = program.functions();
            let mut host_rng = Rng(rng.next());
            let mut host = |name: &str, args: Vec<Value>| {
                let (_, _, ret) = functions
                    .iter()
                    .find(|(n, params, _)| {
                        *n == name
                            && params.len() == args.len()
                            && params.iter().zip(&args).all(|(p, a)| conforms(a, p))
                    })
                    .unwrap_or_else(|| panic!("{round}: {name}{args:?} was not declared"));
                Ok(value_of(&mut host_rng, ret))
            };

            match run.run_with(&mut host) {
                Ok(outcome) => {
                    ran += 1;
                    assert!(
                        conforms(&outcome.value, &program.result_type()),
                        "{round}: {source:?} tampered as {:?} gave {}",
                        stored.code,
                        outcome.value
                    );
                }
                Err(fault) if fault.is_fault() => {
                    panic!(
                        "{round}: {source:?} tampered as {:?}: {fault:#}",
                        stored.code
                    )
                }
                Err(_) => ran += 1,
            }
        }
    }

    assert!(
        refused > 1_000,
        "only {refused} tampered programs were refused"
    );
    assert!(ran > 1_000, "only {ran} tampered programs ran");
}

/// Options of options, of every kind of value, bind, read back, and work
/// with every option operation; a tagged option takes a word more than its
/// value, an option of a value with a niche one word.
#[test]
fn nested_options() {
    let o = Type::option;
    let int_values = [int(-1), int(0), int(i64::MIN)];
    let bool_values = [Value::bool(true), Value::bool(false)];
    let string_values = [Value::from(""), Value::from("Essence")];

    let cases: Vec<(Type, &[Value], usize)> = vec![
        (o(o(Type::int())), &int_values, 3),
        (o(o(o(Type::int()))), &int_values, 4),
        (o(o(Type::bool())), &bool_values, 2),
        (o(o(o(Type::bool()))), &bool_values, 3),
        (o(o(Type::string())), &string_values, 2),
    ];

    for (ty, inner, words) in cases {
        let mut env = env();
        env.value("x", ty.clone()).unwrap();
        let program = compile("x", &env).unwrap();
        assert_eq!(program.memory().globals(), words as u32, "{ty}");

        // Every value of the type: none, and each inner value wrapped in as
        // many `some`s as it can take, and every depth of `none` between.
        let mut depth = 0;
        let mut t = &ty;

        while let Some(inner) = t.as_option() {
            depth += 1;
            t = inner;
        }

        let mut values = Vec::new();

        for d in 0..depth {
            let mut v = Value::none();

            for _ in 0..d {
                v = Value::some(v);
            }

            values.push(v);
        }

        for v in inner {
            let mut v = v.clone();

            for _ in 0..depth {
                v = Value::some(v);
            }

            values.push(v);
        }

        for value in values {
            let extra = [("y", ty.clone(), value.clone())];
            let run = |source: &str| eval_in(source, &extra, Limits::new());

            assert_eq!(run("y").unwrap().value, value, "{ty} {value}");

            let present = value != Value::none();
            assert_eq!(
                run("y.is_some()").unwrap().value,
                Value::bool(present),
                "{ty} {value}"
            );
            assert_eq!(
                run("y.is_none()").unwrap().value,
                Value::bool(!present),
                "{ty} {value}"
            );
            assert_eq!(run("[y, y][1]").unwrap().value, value, "{ty} {value}");
            assert_eq!(
                run("[y].get(0)").unwrap().value,
                Value::some(value.clone()),
                "{ty} {value}"
            );
            assert_eq!(
                run("[y].get(1)").unwrap().value,
                Value::none(),
                "{ty} {value}"
            );

            match value.as_option() {
                Some(Some(inner)) => {
                    assert_eq!(run("y!").unwrap().value, *inner, "{ty} {value}");
                    assert_eq!(
                        run("y.unwrap_or(y!)").unwrap().value,
                        *inner,
                        "{ty} {value}"
                    );
                }
                _ => {
                    let error = run("y!").unwrap_err();
                    assert_eq!(
                        error.to_string(),
                        "`!` found no value: the optional value is none"
                    );
                }
            }
        }
    }
}

#[test]
fn hexadecimal_integers_evaluate() {
    assert_eq!(eval("0x1F + 1").as_int(), Some(32));
    assert_eq!(eval("0Xff").as_int(), Some(255));
    assert_eq!(eval("-0x10").as_int(), Some(-16));
    assert_eq!(eval("0x7FFFFFFFFFFFFFFF").as_int(), Some(i64::MAX));
    assert_eq!(eval("0x1A2B3C4D").as_int(), Some(0x1A2B_3C4D));
    assert_eq!(eval("0x0010 == 16").as_bool(), Some(true));
}

/// A missing key fails a run naming the key, as an index out of range
/// does; `get` gives `none` instead.
#[test]
fn map_lookup_errors() {
    assert_eq!(fails("ids[8].dice"), "the map has no key 8");
    assert_eq!(
        fails("ids[-9223372036854775807 - 1]"),
        "the map has no key -9223372036854775808"
    );
    assert_eq!(fails("tags[name]"), r#"the map has no key "Essence""#);
    assert_eq!(fails(r#"empty["a"]"#), r#"the map has no key "a""#);
    assert_eq!(
        fails_chain("ids[wits]"),
        "the map has no key 2: while running instruction 2: while running the program"
    );

    // A long key is cut short where it is named.
    let long = "k".repeat(100);
    let message = fails_in(&format!("tags[{long:?}]"), &[]);
    assert_eq!(
        message,
        format!("the map has no key \"{}…\"", "k".repeat(64))
    );
}

/// A bound map is checked entry by entry, keys first, naming where a value
/// is mistyped by its key.
#[test]
fn map_binding_errors() {
    let program = compile_in("ids.len() + tags.len()", &[]);
    let mut machine = Machine::default();
    let dice = |n| Value::record([("dice", int(n))]);

    let cases: &[(&str, Value, &str)] = &[
        (
            "ids",
            Value::list([]),
            "expected Map<Int, {dice: Int}>, found a list: while binding `ids`",
        ),
        (
            "ids",
            Value::record([("dice", int(1))]),
            "expected Map<Int, {dice: Int}>, found a record: while binding `ids`",
        ),
        (
            "ids",
            Value::map([(int(1), dice(1)), (int(5), int(5))]),
            "expected {dice: Int}, found Int: at `ids[5]`: while binding `ids`",
        ),
        (
            "ids",
            Value::map([(int(1), dice(1)), (Value::from("x"), dice(2))]),
            "expected Int, found String: at `ids[key #1]`: while binding `ids`",
        ),
        (
            "tags",
            Value::map([(Value::from("a b"), Value::some(Value::bool(true)))]),
            r#"expected Option<Int>, found some(Bool): at `tags["a b"]`: while binding `tags`"#,
        ),
        (
            "tags",
            Value::map([(int(1), Value::none())]),
            "expected String, found Int: at `tags[key #0]`: while binding `tags`",
        ),
    ];

    for (name, value, expected) in cases {
        let mut run = machine.load(&program).unwrap();
        let error = run.bind(name, value).unwrap_err();
        assert!(!error.is_fault(), "{name}: {error:#}");
        assert_eq!(chain(&error, &program), *expected, "{name} = {value}");
    }

    // The entries' words: three entries of a key and a one-word record,
    // and each record's field.
    let mut run = machine.load(&program).unwrap();
    let (ids, words) = binding("ids");
    assert_eq!(words, 9);
    run.bind("ids", &ids).unwrap();
    run.bind("tags", &binding("tags").0).unwrap();
    assert_eq!(run.run().unwrap().value, int(7));
}

/// A map binds and reads back in key order however its entries were
/// given, a key given twice keeping its last value, and the lookup finds
/// every key of a large map and none between them.
#[test]
fn maps_bind_in_key_order() {
    let ty = Type::map(Type::int(), Type::int());
    let keys: Vec<i64> = (0..200).map(|n| n * 3 - 300).collect();
    let map = Value::map(keys.iter().rev().map(|k| (int(*k), int(k * 2))));
    let extra = [("big", ty.clone(), map.clone())];
    let limits = Limits::new().memory(4096);

    let outcome = eval_in("big", &extra, limits).unwrap();
    assert_eq!(outcome.value, map);

    for k in [
        -300,
        -297,
        0,
        297,
        294,
        -301,
        -299,
        1,
        298,
        i64::MIN + 1,
        i64::MAX,
    ] {
        let got = eval_in(&format!("big.get({k})"), &extra, limits)
            .unwrap()
            .value;
        let expected = if keys.contains(&k) {
            Value::some(int(k * 2))
        } else {
            Value::none()
        };
        assert_eq!(got, expected, "{k}");
    }

    let twice = Value::map([(int(1), int(1)), (int(0), int(0)), (int(1), int(2))]);
    assert_eq!(twice.to_string(), "{0: 0, 1: 2}");
    assert_eq!(
        Value::map([(int(2), int(0)), (int(1), int(0))]),
        Value::map([(int(1), int(0)), (int(2), int(0))])
    );

    // Strings are ordered by their bytes, past the first word of them.
    let ty = Type::map(Type::string(), Type::int());
    let names = [
        "",
        "a",
        "ab",
        "abcdefgh",
        "abcdefghi",
        "abcdefgg",
        "b",
        "é",
        "\u{ff}",
    ];
    let map = Value::map(
        names
            .iter()
            .enumerate()
            .map(|(n, s)| (Value::from(*s), int(n as i64))),
    );
    let extra = [("named", ty, map)];

    for (n, s) in names.iter().enumerate() {
        let got = eval_in(&format!("named[{s:?}]"), &extra, limits)
            .unwrap()
            .value;
        assert_eq!(got, int(n as i64), "{s:?}");
    }

    for s in ["abc", "abcdefgha", "c", "aa", "abcdefg"] {
        let got = eval_in(&format!("named.get({s:?})"), &extra, limits)
            .unwrap()
            .value;
        assert_eq!(got, Value::none(), "{s:?}");
    }
}

/// Programs over maps, tampered with, are refused or run without a fault.
#[test]
fn tampered_map_programs_never_fault() {
    let sources = [
        "ids[7].dice",
        "ids.get(wits).is_some()",
        "tags[\"brawl\"].unwrap_or(0) + tags.len()",
        "tags.get(name)",
        "lookup()[1]",
        "[ids.len(), empty.len()]",
    ];
    let mut rng = Rng(0x3a95_00c1);
    let (mut refused, mut ran) = (0, 0);

    for round in 0..2_000 {
        let source = sources[rng.below(sources.len())];
        let bytes = compile_in(source, &[]).to_bytes().unwrap();
        let mut stored: Stored = musli::storage::from_slice(&bytes).unwrap();

        for _ in 0..=rng.below(3) {
            tamper(&mut rng, &mut stored);
        }

        let Ok(program) = Program::from_bytes(&musli::storage::to_vec(&stored).unwrap()) else {
            refused += 1;
            continue;
        };

        let limit = u32::try_from(program.memory().total()).unwrap() + 4096;
        let mut machine = Machine::new(Limits::new().memory(limit).fuel(100_000));
        let mut run = machine.load(&program).unwrap();

        for (name, ty) in run.globals() {
            run.bind(name, &value_of(&mut rng, ty)).unwrap();
        }

        let functions = program.functions();
        let mut host_rng = Rng(rng.next());
        let mut host = |name: &str, _: Vec<Value>| {
            let (_, _, ret) = functions.iter().find(|(n, ..)| *n == name).unwrap();
            Ok(value_of(&mut host_rng, ret))
        };

        match run.run_with(&mut host) {
            Ok(outcome) => {
                ran += 1;
                assert!(
                    conforms(&outcome.value, &program.result_type()),
                    "{round}: {source:?}"
                );
            }
            Err(error) if error.is_fault() => panic!("{round}: {source:?}: {error:#}"),
            Err(_) => ran += 1,
        }
    }

    assert!(refused > 500, "only {refused} were refused");
    assert!(ran > 100, "only {ran} ran");
}

/// A map literal evaluates to its entries in key order, which its keys,
/// decimal or hexadecimal, find.
#[test]
fn map_literals_evaluate() {
    assert_eq!(eval("{1: 10, 0x2A: 20}[42]").as_int(), Some(20));
    assert_eq!(eval("{0x2A: 20, 1: 10}[1]").as_int(), Some(10));
    assert_eq!(eval("{-1: 1, -0x10: 2, 3: 3}[-16]").as_int(), Some(2));
    assert_eq!(eval("{1: wits + 1}[1]").as_int(), Some(3));
    assert_eq!(eval(r#"{"b": 2, "a": 1, "ab": 3}["ab"]"#).as_int(), Some(3));
    assert_eq!(eval(r#"{"b": 2, "": 1}[""]"#).as_int(), Some(1));
    assert_eq!(eval("{3: 1, 1: 2, 2: 3}.len()").as_int(), Some(3));
    assert_eq!(eval("Map::len({3: 1})").as_int(), Some(1));
    assert_eq!(eval("{1: true}.get(2).is_none()").as_bool(), Some(true));
    // Values of every width: tagged options take two words, records many.
    assert_eq!(eval("{2: bonus, 1: label(1)}[2]"), eval("bonus"));
    assert_eq!(eval("{2: bonus, 1: label(1)}[1]"), eval("label(1)"));
    assert_eq!(
        eval("{2: health, 1: health}[1].boxes"),
        eval("health.boxes")
    );
    assert_eq!(
        eval(r#"{"x": {1: [1, 2]}, "y": {2: [3]}}["y"][2][0]"#).as_int(),
        Some(3)
    );

    // In key order, however written.
    assert_eq!(
        eval("{3: 30, -0x1: 10, 2: 20}").to_string(),
        "{-1: 10, 2: 20, 3: 30}"
    );
    assert_eq!(
        eval(r#"{"b": true, "a": false}"#),
        Value::map([
            (Value::from("a"), Value::bool(false)),
            (Value::from("b"), Value::bool(true)),
        ])
    );

    assert_eq!(fails("{1: 2}[3]"), "the map has no key 3");
    assert_eq!(fails(r#"{"a": 2}["b"]"#), r#"the map has no key "b""#);
    assert_eq!(
        fails_chain("{1: 2, 0x10: 3}[wits]"),
        "the map has no key 2: while running instruction 4: while running the program"
    );
}

/// A map literal's values are evaluated in key order: the failing call,
/// at key 1, runs before the division by zero, at key 2.
#[test]
fn map_literal_values_in_key_order() {
    assert_eq!(fails("{2: 1 / 0, 1: fail()}[1]"), "the function failed");
    assert_eq!(fails("{1: 1 / 0, 2: fail()}[1]"), fails("1 / 0"));
}
