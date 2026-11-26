//! Tests of the machine: every instruction, every runtime error with its
//! message, each limit at its boundary, and random well-typed programs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::HashSet;
use std::mem::discriminant;

use crate::check_tests::{env, generate};
use crate::program::{Instr, Memory};
use crate::tests::Rng;
use crate::types::{TypeNode, Types};
use crate::*;

fn track(handle: u64) -> Value {
    Value::opaque("Track", handle)
}

/// The value bound to each of [`env`]'s names, and the slots its parts
/// take.
fn binding(name: &str) -> (Value, usize) {
    match name {
        "wits" => (Value::Int(2), 0),
        "dexterity" => (Value::Int(3), 0),
        "athletics" => (Value::Int(1), 0),
        "strength" => (Value::Int(4), 0),
        "blood_potency" => (Value::Int(2), 0),
        "primal_urge" => (Value::Int(1), 0),
        "min" => (Value::Int(5), 0),
        "flag" => (Value::Bool(true), 0),
        "bonus" => (Value::some(Value::Int(2)), 0),
        "maybe" => (Value::None, 0),
        "levels" => (
            Value::List(vec![Value::Int(10), Value::Int(20), Value::Int(30)]),
            3,
        ),
        "grid" => (
            Value::List(vec![
                Value::List(vec![Value::Int(1), Value::Int(2)]),
                Value::List(vec![Value::Int(3)]),
            ]),
            5,
        ),
        "name" => (Value::from("Essence"), 1),
        "health" => (track(1), 0),
        "tracks" => (Value::List(vec![track(2), track(3)]), 2),
        "specialties" => (
            Value::record([
                ("athletics", Value::Int(2)),
                ("brawl", Value::some(Value::Int(1))),
            ]),
            2,
        ),
        "m" => (
            Value::record([
                ("Brawl (bar)", Value::Int(4)),
                ("a b", track(4)),
                ("inner", Value::record([("deep", Value::Bool(false))])),
            ]),
            4,
        ),
        "maybe_track" => (Value::some(track(5)), 0),
        _ => panic!("no binding for {name}"),
    }
}

/// [`env`]'s functions: `count(t)` is its handle, `count(t, n)` is `n`,
/// `boxes` is 5, `lookup(i, table)` is `i`, `label(i)` is `some(i)` for
/// even `i`, `pick()` is `true`.
fn host(name: &str, args: Vec<Value>) -> Result<Value, String> {
    Ok(match (name, args.as_slice()) {
        ("count", [Value::Opaque { handle, .. }]) => Value::Int(*handle as i64),
        ("count", [Value::Opaque { .. }, Value::Int(n)]) => Value::Int(*n),
        ("boxes", [Value::Opaque { .. }]) => Value::Int(5),
        ("lookup", [Value::Int(i), Value::String(table)]) => {
            assert_eq!(table, "Essence");
            Value::Int(*i)
        }
        ("label", [Value::Int(i)]) if i % 2 == 0 => Value::some(Value::Int(*i)),
        ("label", [Value::Int(_)]) => Value::None,
        ("pick", []) => Value::Bool(true),
        _ => return Err(format!("unexpected call {name}{args:?}")),
    })
}

fn compile_in(source: &str, extra: &[(&str, Type, Value)]) -> Program {
    let mut env = env();

    for (name, ty, _) in extra {
        env.value(name, ty.clone()).unwrap();
    }

    env.function("fail", [], Type::Int).unwrap();
    env.function("wrong", [], Type::Int).unwrap();
    env.function("nested", [], Type::list(Type::record([("a", Type::Int)])))
        .unwrap();
    env.function("many", [Type::Int], Type::list(Type::Int))
        .unwrap();

    compile(source, &env).unwrap_or_else(|e| panic!("{source:?}: {e}"))
}

/// `host` and some functions that misbehave.
fn test_host(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match (name, args.as_slice()) {
        ("fail", []) => Err("the track is gone".to_owned()),
        ("wrong", []) => Ok(Value::Bool(true)),
        ("nested", []) => Ok(Value::List(vec![
            Value::record([("a", Value::Int(1))]),
            Value::record([("a", Value::None)]),
        ])),
        ("many", [Value::Int(n)]) => Ok(Value::List(vec![Value::Int(0); *n as usize])),
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

        run.bind(name, value)?;
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

fn fails_in(source: &str, extra: &[(&str, Type, Value)]) -> String {
    match eval_in(source, extra, Limits::new()) {
        Ok(outcome) => panic!("{source:?} gave {}", outcome.value),
        Err(error) => {
            assert!(!matches!(error, RunError::Fault(..)), "{source:?}: {error}");
            error.to_string()
        }
    }
}

fn fails(source: &str) -> String {
    fails_in(source, &[])
}

fn int(n: i64) -> Value {
    Value::Int(n)
}

fn ints(ns: &[i64]) -> Value {
    Value::List(ns.iter().map(|n| Value::Int(*n)).collect())
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
    (r#"lookup(3, "Essence")"#, "3"),
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
    ("min(wits)", "2"),
    ("min(dexterity, wits, strength)", "2"),
    ("max(dexterity, wits, strength)", "4"),
    ("wits.min(dexterity)", "2"),
    ("wits.max(dexterity)", "3"),
    // Abs
    ("(-wits).abs()", "2"),
    ("wits.abs()", "2"),
    // Clamp
    ("strength.clamp(1, 3)", "3"),
    ("athletics.clamp(2, 3)", "2"),
    ("wits.clamp(1, 3)", "2"),
    ("wits.clamp(2, 2)", "2"),
    // Lookup, clamped
    ("lookup(1, levels)", "20"),
    ("lookup(-5, levels)", "10"),
    ("lookup(99, levels)", "30"),
    ("lookup(blood_potency, [10, 11, 12, 13])", "12"),
    ("lookup(9, [wits])", "2"),
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
    ("maybe_track", "some(<Track 5>)"),
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
    ("health", "<Track 1>"),
    ("tracks", "[<Track 2>, <Track 3>]"),
    ("specialties", "{athletics: 2, brawl: some(1)}"),
    (
        "m",
        r#"{"Brawl (bar)": 4, "a b": <Track 4>, inner: {deep: false}}"#,
    ),
    ("name", r#""Essence""#),
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
        Instr::Global(0),
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
        Instr::Field(0),
        Instr::Index,
        Instr::List(0),
        Instr::Min(0),
        Instr::Max(0),
        Instr::Abs,
        Instr::Clamp,
        Instr::Lookup,
        Instr::UnwrapOr,
        Instr::IsSome,
        Instr::IsNone,
        Instr::Len,
        Instr::Call(0),
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
    let list = Value::List(vec![
        Value::from("a"),
        Value::from("bcdefghij"),
        Value::from(""),
    ]);
    let extra = [("words", Type::list(Type::String), list.clone())];
    assert_eq!(eval_in("words", &extra, Limits::new()).unwrap().value, list);
    assert_eq!(
        eval_in("[words, words][1]", &extra, Limits::new())
            .unwrap()
            .value,
        list
    );

    let unicode = Value::from("Ærø \"é\" \\ 🐉 and some more bytes");
    let extra = [("s", Type::String, unicode.clone())];
    assert_eq!(eval_in("s", &extra, Limits::new()).unwrap().value, unicode);
    assert_eq!(
        unicode.to_string(),
        r#""Ærø \"é\" \\ 🐉 and some more bytes""#
    );

    let option_list = Value::some(ints(&[1, 2]));
    let extra = [("ol", Type::option(Type::list(Type::Int)), option_list)];
    assert_eq!(
        eval_in("ol.unwrap_or([9])", &extra, Limits::new())
            .unwrap()
            .value,
        ints(&[1, 2])
    );

    let extra = [("ol", Type::option(Type::list(Type::Int)), Value::None)];
    assert_eq!(
        eval_in("ol.unwrap_or([9])[0]", &extra, Limits::new())
            .unwrap()
            .value,
        int(9)
    );

    let empty = [("e", Type::list(Type::Int), ints(&[]))];
    assert_eq!(
        eval_in("e.len()", &empty, Limits::new()).unwrap().value,
        int(0)
    );
}

#[test]
fn options() {
    // Every way an option is made, and every way it is used.
    let cases: &[(&str, Value, &str)] = &[
        ("o.unwrap_or(7)", Value::None, "7"),
        ("o.unwrap_or(7)", Value::some(int(3)), "3"),
        ("o.is_some()", Value::None, "false"),
        ("o.is_some()", Value::some(int(3)), "true"),
        ("o.is_none()", Value::None, "true"),
        ("o.is_none()", Value::some(int(3)), "false"),
        ("o", Value::None, "none"),
        ("o", Value::some(int(3)), "some(3)"),
        ("[o, o][0]", Value::some(int(0)), "some(0)"),
        ("o.unwrap_or(-1).max(0)", Value::None, "0"),
    ];

    for (source, value, expected) in cases {
        let extra = [("o", Type::option(Type::Int), value.clone())];
        let outcome = eval_in(source, &extra, Limits::new()).unwrap();
        assert_eq!(
            outcome.value.to_string(),
            *expected,
            "{source:?} with {value}"
        );
    }

    // An option of a record, and a record's optional field.
    let ty = Type::option(Type::record([("x", Type::option(Type::Int))]));
    let extra = [(
        "r",
        ty.clone(),
        Value::some(Value::record([("x", Value::None)])),
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
        Value::Bool(true)
    );

    // Options from the caller's functions.
    assert_eq!(eval("label(2).is_some()"), Value::Bool(true));
    assert_eq!(eval("label(1).is_none()"), Value::Bool(true));
}

#[test]
fn arithmetic_errors() {
    let big = [
        ("big", Type::Int, int(i64::MAX)),
        ("small", Type::Int, int(i64::MIN)),
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
fn lookup_errors() {
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
    ];

    for (source, message) in cases {
        assert_eq!(fails(source), message, "{source:?}");
    }

    // Right at the ends, indexing works.
    assert_eq!(eval("levels[2]"), int(30));
    assert_eq!(eval("levels[0]"), int(10));

    let empty = [("e", Type::list(Type::Int), ints(&[]))];
    assert_eq!(
        fails_in("e[0]", &empty),
        "index 0 is out of range of a list of 0 entries"
    );
    assert_eq!(
        fails_in("lookup(0, e)", &empty),
        "`lookup` of an empty list has no entry to give"
    );
}

#[test]
fn host_errors() {
    assert_eq!(fails("fail()"), "`fail` failed: the track is gone");
    assert_eq!(fails("wrong()"), "`wrong` must return Int, returned Bool");
    assert_eq!(
        fails("nested()"),
        "`nested` returned a value whose `[1].a` must be Int, found none"
    );

    // Without a host, any call fails.
    let program = compile_in("count(health)", &[]);
    let mut machine = Machine::default();
    let mut run = machine.load(&program).unwrap();
    bind_all(&mut run, &[]).unwrap();
    assert_eq!(
        run.run().unwrap_err().to_string(),
        "`count` failed: no functions are provided"
    );
}

#[test]
fn host_result_memory() {
    // `many(n)` returns n slots of parts; the program needs 1 stack slot
    // and nothing else, so a machine of 1 + k slots takes a list of k.
    let program = compile_in("many(3).len()", &[]);
    assert_eq!(program.memory().total(), 1);

    let run = |memory: u32| {
        let mut machine = Machine::new(Limits::new().memory(memory));
        machine.load(&program).unwrap().run_with(&mut test_host)
    };

    assert_eq!(run(4).unwrap().value, int(3));
    assert_eq!(run(4).unwrap().memory, 4);
    assert_eq!(
        run(3).unwrap_err().to_string(),
        "the value `many` returned needs 3 slots of memory, 2 are left"
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
        ("wits", Value::Bool(true), "`wits` must be Int, found Bool"),
        (
            "wits",
            Value::some(int(1)),
            "`wits` must be Int, found some(..)",
        ),
        ("wits", Value::None, "`wits` must be Int, found none"),
        ("bonus", int(1), "`bonus` must be Option<Int>, found Int"),
        (
            "bonus",
            Value::some(Value::Bool(true)),
            "`bonus` must be Option<Int>, found some(Bool)",
        ),
        (
            "levels",
            Value::List(vec![int(1), Value::Bool(false)]),
            "`levels[1]` must be Int, found Bool",
        ),
        ("levels", int(1), "`levels` must be List<Int>, found Int"),
        (
            "grid",
            Value::List(vec![ints(&[1]), int(2)]),
            "`grid[1]` must be List<Int>, found Int",
        ),
        (
            "m",
            Value::record([("Brawl (bar)", int(1)), ("a b", track(1))]),
            r#"`m` must be {"Brawl (bar)": Int, "a b": Track, inner: {deep: Bool}}, found a record without `inner`"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", int(1)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", Value::Bool(true))])),
                ("zzz", int(0)),
            ]),
            r#"`m` must be {"Brawl (bar)": Int, "a b": Track, inner: {deep: Bool}}, found a record with an extra field `zzz`"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", Value::Bool(true)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", Value::Bool(true))])),
            ]),
            r#"`m["Brawl (bar)"]` must be Int, found Bool"#,
        ),
        (
            "m",
            Value::record([
                ("Brawl (bar)", int(1)),
                ("a b", track(1)),
                ("inner", Value::record([("deep", int(0))])),
            ]),
            "`m.inner.deep` must be Bool, found Int",
        ),
        (
            "health",
            Value::opaque("Box", 1),
            "`health` must be Track, found Box",
        ),
        (
            "maybe_track",
            track(1),
            "`maybe_track` must be Option<Track>, found Track",
        ),
        ("strength", int(1), "the program reads no value `strength`"),
        ("nothing", int(1), "the program reads no value `nothing`"),
    ];

    for (name, value, message) in cases {
        let mut run = machine.load(&program).unwrap();
        let error = run.bind(name, value.clone()).unwrap_err();
        assert_eq!(error.to_string(), *message, "{name} = {value}");
    }

    // Binding twice is an error, and so is not binding at all.
    let mut run = machine.load(&program).unwrap();
    run.bind("wits", int(1)).unwrap();
    assert_eq!(
        run.bind("wits", int(1)).unwrap_err().to_string(),
        "`wits` is already bound"
    );

    let mut run = machine.load(&program).unwrap();
    run.bind("wits", int(1)).unwrap();
    assert_eq!(
        run.run_with(&mut test_host).unwrap_err().to_string(),
        "`bonus` is not bound, bind a Option<Int> before running"
    );

    // A failed binding leaves the value unbound, and a good one binds.
    let mut run = machine.load(&program).unwrap();
    run.bind("wits", Value::Bool(true)).unwrap_err();
    run.bind("wits", int(1)).unwrap();
}

#[test]
fn memory_budget() {
    // `lookup(i, [10, 11, 12, 13])` takes 2 stack slots, 4 constant slots
    // and a slot for `i`.
    let mut env = Environment::with_builtins();
    env.value("i", Type::Int).unwrap();
    let program = compile("lookup(i, [10, 11, 12, 13])", &env).unwrap();
    assert_eq!(
        program.memory(),
        Memory {
            stack: 2,
            heap: 0,
            constants: 4
        }
    );

    let mut machine = Machine::new(Limits::new().memory(7));
    let mut run = machine.load(&program).unwrap();
    run.bind("i", int(2)).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, int(12));
    assert_eq!(outcome.memory, 7);

    let mut machine = Machine::new(Limits::new().memory(6));
    assert_eq!(
        machine.load(&program).unwrap_err().to_string(),
        "the program needs 7 slots of memory, more than the 6 the machine allows"
    );

    // Lists built as the program runs take heap: `[i, i, i]` is 3 stack, 3
    // heap and 1 for `i`.
    let program = compile("[i, i, i]", &env).unwrap();
    assert_eq!(program.memory().total(), 6);
    assert!(Machine::new(Limits::new().memory(7)).load(&program).is_ok());
    assert!(
        Machine::new(Limits::new().memory(6))
            .load(&program)
            .is_err()
    );

    let mut machine = Machine::new(Limits::new().memory(7));
    let mut run = machine.load(&program).unwrap();
    run.bind("i", int(1)).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, ints(&[1, 1, 1]));
    assert!(outcome.memory <= 7, "{outcome:?}");

    // The default machine refuses a program past its memory.
    let source = format!("[{}]", vec!["i"; 600].join(", "));
    let program = compile_with(&source, &env, &Options::new()).unwrap();
    assert_eq!(
        Machine::default().load(&program).unwrap_err(),
        RunError::TooMuchMemory {
            needed: 1201,
            allowed: DEFAULT_MEMORY
        }
    );
}

#[test]
fn binding_memory() {
    // `l.len()` takes 1 stack slot and 1 for `l`, so a machine of 2 + k
    // slots takes a bound list of k entries, and no more.
    let mut env = Environment::with_builtins();
    env.value("l", Type::list(Type::Int)).unwrap();
    env.value("s", Type::String).unwrap();
    let program = compile("l.len()", &env).unwrap();
    assert_eq!(program.memory().total(), 1);

    let mut machine = Machine::new(Limits::new().memory(5));
    let mut run = machine.load(&program).unwrap();
    run.bind("l", ints(&[1, 2, 3])).unwrap();
    let outcome = run.run().unwrap();
    assert_eq!(outcome.value, int(3));
    assert_eq!(outcome.memory, 5);

    let mut run = machine.load(&program).unwrap();
    assert_eq!(
        run.bind("l", ints(&[1, 2, 3, 4])).unwrap_err().to_string(),
        "binding `l` needs 4 slots of memory, 3 are left"
    );

    // A string takes a slot per 8 bytes.
    let program = compile(r#"[s, s].len()"#, &env).unwrap();
    let needed = program.memory().total() + 1;
    let mut machine = Machine::new(Limits::new().memory(needed as u32 + 2));
    let mut run = machine.load(&program).unwrap();
    run.bind("s", Value::from("0123456789abcdef")).unwrap();
    assert_eq!(run.run().unwrap().value, int(2));

    let mut run = machine.load(&program).unwrap();
    assert_eq!(
        run.bind("s", Value::from("0123456789abcdef!"))
            .unwrap_err()
            .to_string(),
        "binding `s` needs 3 slots of memory, 2 are left"
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
        eval_in("wits", &[], Limits::new().fuel(0)).unwrap_err(),
        RunError::OutOfFuel { fuel: 0 }
    );

    // A list of n entries, `min` and `max` of n take n more steps, and a
    // call one more per slot it returns.
    for (source, steps) in [
        ("[wits, wits, wits]", 7),
        ("min(wits, wits, wits)", 7),
        ("max(wits, wits)", 5),
        ("many(4)", 6),
        ("wits.min(wits)", 5),
        ("flag || flag", 2),
        ("!flag || flag", 4),
    ] {
        let outcome = eval_in(source, &[], Limits::new()).unwrap();
        assert_eq!(outcome.steps, steps, "{source:?}");
        assert!(eval_in(source, &[], Limits::new().fuel(steps)).is_ok());
        assert_eq!(
            eval_in(source, &[], Limits::new().fuel(steps - 1)).unwrap_err(),
            RunError::OutOfFuel { fuel: steps - 1 },
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
        eval_in(&source, &[], Limits::new()).unwrap_err(),
        RunError::OutOfFuel { fuel: 1000 }
    );
    let source = vec!["wits"; 500].join(" + ");
    assert_eq!(eval_in(&source, &[], Limits::new()).unwrap().steps, 999);
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
        run.bind("wits", int(n)).unwrap();
        run.bind("dexterity", int(-n)).unwrap();
        assert_eq!(run.run().unwrap().value, ints(&[n, -n]));
    }
}

/// A program that shex would not make gives a fault, not a panic.
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
        },
    };

    for (code, stack) in [
        (vec![Instr::Add], 2),
        (vec![Instr::Int(1), Instr::Int(2)], 1),
        (vec![Instr::Int(1), Instr::Int(2)], 2),
        (vec![Instr::Bool(true), Instr::Neg], 1),
        (vec![Instr::Const(3)], 1),
        (vec![Instr::Global(0)], 1),
        (vec![Instr::Int(1), Instr::Field(0)], 1),
        (vec![Instr::Int(1), Instr::Len], 1),
        (vec![Instr::Int(1), Instr::List(1)], 1),
        (vec![Instr::Call(0)], 1),
        (vec![Instr::Bool(true), Instr::Or(u32::MAX)], 1),
        (vec![], 0),
    ] {
        let program = faulty(code.clone(), stack);
        let mut machine = Machine::default();
        let error = machine.load(&program).unwrap().run().unwrap_err();
        assert!(matches!(error, RunError::Fault(..)), "{code:?}: {error}");
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

        // Exactly the memory it needs: its own, a slot per value it reads
        // and the parts of those values.
        let parts = program
            .globals()
            .iter()
            .map(|(name, _)| binding(name).1 as u64)
            .sum::<u64>();
        let needed = program.memory().total() + program.globals().len() as u64 + parts;

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
            Err(
                RunError::Overflow { .. }
                | RunError::DivideByZero { .. }
                | RunError::IndexOutOfRange { .. }
                | RunError::ClampBounds { .. },
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

        if let Err(RunError::Fault(error)) = run.run_with(&mut host) {
            panic!("{round}: {source:?}: {error}");
        }

        // A slot less is refused before it runs.
        if needed > program.globals().len() as u64 + parts {
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
        Value::Int(_) => "Int",
        Value::Bool(_) => "Bool",
        Value::None | Value::Some(..) => "Option<Int>",
        Value::List(items) if items.iter().all(|i| matches!(i, Value::Int(_))) => "List<Int>",
        _ => "?",
    }
}
