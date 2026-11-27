//! Tests of the error core: every message's text with and without a
//! program, the context chain of each kind of failure, labels and their
//! spans, and debug info.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::string_slice
)]

use crate::message::{Fault, Found, Key, Location, Message, Op, PathStep, Root, Seg, show};
use crate::types::TypeId;
use crate::*;

fn env() -> Environment {
    let mut env = Environment::new();
    env.value("strength", Type::int()).unwrap();
    env.value(
        "health",
        Type::record([("boxes", Type::int()), ("count", Type::list(Type::int()))]),
    )
    .unwrap();
    env.value("bonus", Type::option(Type::int())).unwrap();
    env.function("count", [Type::int()], Type::record([("n", Type::int())]))
        .unwrap();
    env
}

/// Each label's message, and its span's text in `source`.
fn labels(
    error: &RunError,
    program: &Program,
    debug: Option<&DebugInfo>,
    source: &str,
) -> Vec<(String, Option<String>)> {
    error
        .labels(program, debug)
        .map(|l| {
            let span = l.span().map(|s| source[s.range()].to_owned());
            (l.message().to_string(), span)
        })
        .collect()
}

fn messages(error: &RunError, program: &Program) -> Vec<String> {
    error
        .labels(program, None)
        .map(|l| l.message().to_string())
        .collect()
}

#[test]
fn every_message_with_and_without_a_program() {
    let env = env();
    let program = compile(
        "strength + health.boxes + count(1).n + bonus.unwrap_or(0)",
        &env,
    )
    .unwrap();
    let int = program
        .globals
        .iter()
        .find(|g| g.name == "strength")
        .unwrap()
        .ty;
    let health = program
        .globals
        .iter()
        .find(|g| g.name == "health")
        .unwrap()
        .ty;
    let g = |name: &str| program.globals.iter().position(|g| g.name == name).unwrap() as u32;
    let strength = g("strength");

    let cases: Vec<(Message, &str, &str)> = vec![
        (
            Message::TooMuchMemory {
                needed: 8,
                allowed: 2,
            },
            "the program needs 8 words of memory, more than the 2 the machine allows",
            "the program needs 8 words of memory, more than the 2 the machine allows",
        ),
        (
            Message::NotDeclared {
                name: "nothing".into(),
            },
            "no value `nothing` is declared",
            "no value `nothing` is declared",
        ),
        (
            Message::FunctionPath {
                name: "count".into(),
            },
            "`count()` is what a function gives, not a value a store holds",
            "`count()` is what a function gives, not a value a store holds",
        ),
        (
            Message::NotSet { global: strength },
            "`strength` is not set",
            "the value is not set",
        ),
        (
            Message::NoStep {
                ty: int,
                step: PathStep::Field("max".into()),
            },
            r#"Int has no field "max""#,
            r#"the value has no field "max""#,
        ),
        (
            Message::NoStep {
                ty: health,
                step: PathStep::Key(Key::string("brawl")),
            },
            r#"{boxes: Int, count: List<Int>} has no key "brawl""#,
            r#"the value has no key "brawl""#,
        ),
        (
            Message::NoStep {
                ty: int,
                step: PathStep::Index(-2),
            },
            "Int has no index -2",
            "the value has no index -2",
        ),
        (
            Message::NoneOnPath,
            "the optional value is none",
            "the optional value is none",
        ),
        (
            Message::KeyMistyped {
                expected: int,
                found: Found::String,
            },
            "expected an Int key, found String",
            "expected a key of another type, found String",
        ),
        (
            Message::OtherLayout {
                global: Some(strength),
            },
            "the store does not hold `strength` where the program reads it, of its type",
            "the store does not hold the program's values where it reads them",
        ),
        (
            Message::OtherLayout { global: None },
            "the store does not hold the program's values where it reads them",
            "the store does not hold the program's values where it reads them",
        ),
        (
            Message::PastAddressSpace {
                store: 1,
                machine: 1,
            },
            "the store's 1 word and the machine's 1 word are more than a run can address",
            "the store's 1 word and the machine's 1 word are more than a run can address",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::Bool,
                in_some: false,
            },
            "expected Int, found Bool",
            "expected another type, found Bool",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::String,
                in_some: true,
            },
            "expected Int, found some(String)",
            "expected another type, found some(String)",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::Int,
                in_some: false,
            },
            "expected Int, found Int",
            "expected another type, found Int",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::List,
                in_some: false,
            },
            "expected Int, found a list",
            "expected another type, found a list",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::Record,
                in_some: false,
            },
            "expected Int, found a record",
            "expected another type, found a record",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::None,
                in_some: false,
            },
            "expected Int, found none",
            "expected another type, found none",
        ),
        (
            Message::Mistyped {
                expected: int,
                found: Found::Some,
                in_some: false,
            },
            "expected Int, found some(..)",
            "expected another type, found some(..)",
        ),
        (
            Message::Mistyped {
                expected: health,
                found: Found::RecordWithout {
                    record: health,
                    field: 1,
                },
                in_some: false,
            },
            "expected {boxes: Int, count: List<Int>}, found a record without `count`",
            "expected another type, found a record without one of its fields",
        ),
        (
            Message::Mistyped {
                expected: health,
                found: Found::RecordWithExtra,
                in_some: false,
            },
            "expected {boxes: Int, count: List<Int>}, found a record with an extra field",
            "expected another type, found a record with an extra field",
        ),
        (
            Message::TooLarge { needed: 4, left: 1 },
            "the value needs 4 words of memory, 1 are left",
            "the value needs 4 words of memory, 1 are left",
        ),
        (
            Message::TooLong { len: 5, max: 4 },
            "the value holds a string, list or map 5 long, longer than the 4 a value may hold",
            "the value holds a string, list or map 5 long, longer than the 4 a value may hold",
        ),
        (
            Message::Unset { global: strength },
            "`strength` is not set, set it to an Int before running",
            "a value the program reads is not set",
        ),
        (
            Message::OutOfFuel { fuel: 3 },
            "the expression took too long: it used all of its 3 steps",
            "the expression took too long: it used all of its 3 steps",
        ),
        (
            Message::Overflow {
                op: Op::Add,
                a: i64::MAX,
                b: 1,
            },
            "`9223372036854775807 + 1` overflows an Int",
            "`9223372036854775807 + 1` overflows an Int",
        ),
        (
            Message::Overflow {
                op: Op::Neg,
                a: i64::MIN,
                b: 0,
            },
            "`-(-9223372036854775808)` overflows an Int",
            "`-(-9223372036854775808)` overflows an Int",
        ),
        (
            Message::Overflow {
                op: Op::Abs,
                a: i64::MIN,
                b: 0,
            },
            "`(-9223372036854775808).abs()` overflows an Int",
            "`(-9223372036854775808).abs()` overflows an Int",
        ),
        (
            Message::Overflow {
                op: Op::Sub,
                a: i64::MIN,
                b: 1,
            },
            "`-9223372036854775808 - 1` overflows an Int",
            "`-9223372036854775808 - 1` overflows an Int",
        ),
        (
            Message::Overflow {
                op: Op::Mul,
                a: i64::MAX,
                b: 2,
            },
            "`9223372036854775807 * 2` overflows an Int",
            "`9223372036854775807 * 2` overflows an Int",
        ),
        (
            Message::Overflow {
                op: Op::Div,
                a: i64::MIN,
                b: -1,
            },
            "`-9223372036854775808 / -1` overflows an Int",
            "`-9223372036854775808 / -1` overflows an Int",
        ),
        (
            Message::DivideByZero { op: Op::Rem, a: 7 },
            "`7 % 0` divides by zero",
            "`7 % 0` divides by zero",
        ),
        (
            Message::IndexOutOfRange { index: 7, len: 1 },
            "index 7 is out of range of a list of 1 entry",
            "index 7 is out of range of a list of 1 entry",
        ),
        (
            Message::IndexOutOfRange { index: -1, len: 3 },
            "index -1 is out of range of a list of 3 entries",
            "index -1 is out of range of a list of 3 entries",
        ),
        (
            Message::ClampBounds { low: 3, high: 1 },
            "`clamp` needs its low bound 3 to be at most its high bound 1",
            "`clamp` needs its low bound 3 to be at most its high bound 1",
        ),
        (
            Message::HostFailed,
            "the function failed",
            "the function failed",
        ),
        (
            Message::Fault(Fault::NotABool),
            "the program is faulty: a Bool other than 0 or 1",
            "the program is faulty: a Bool other than 0 or 1",
        ),
        (
            Message::LoadingProgram,
            "while loading the program",
            "while loading the program",
        ),
        (
            Message::Setting { global: strength },
            "while setting `strength`",
            "while setting a value",
        ),
        (
            Message::SettingAt {
                path: "health.count[7]".into(),
            },
            "while setting `health.count[7]`",
            "while setting `health.count[7]`",
        ),
        (
            Message::AtPath {
                root: Root::Global(g("health")),
                path: Box::new([
                    Seg::Field {
                        record: health,
                        field: 1,
                    },
                    Seg::Index(7),
                ]),
            },
            "at `health.count[7]`",
            "at `_._[7]`",
        ),
        (
            Message::AtPath {
                root: Root::Returned(0),
                path: Box::new([Seg::Index(2)]),
            },
            "at `count(..)[2]`",
            "at `_(..)[2]`",
        ),
        (
            Message::CallingHost { function: 0 },
            "while calling `count`",
            "while calling a function",
        ),
        (
            Message::ReadingHostResult { function: 0 },
            "while reading what `count` returned",
            "while reading what a function returned",
        ),
        (
            Message::Instruction { pc: 12 },
            "while running instruction 12",
            "while running instruction 12",
        ),
        (
            Message::Running,
            "while running the program",
            "while running the program",
        ),
        (
            Message::ReadingResult,
            "while reading the program's value",
            "while reading the program's value",
        ),
    ];

    for (message, named, unnamed) in &cases {
        assert_eq!(show(message, Some(&program)), *named, "{message:?}");
        assert_eq!(show(message, None), *unnamed, "{message:?}");
    }

    // Ids the program does not have show as if there were no program.
    assert_eq!(
        show(&Message::Setting { global: 99 }, Some(&program)),
        "while setting a value"
    );
    assert_eq!(
        show(
            &Message::Mistyped {
                expected: TypeId(999),
                found: Found::Int,
                in_some: false
            },
            Some(&program)
        ),
        "expected another type, found Int"
    );
}

#[test]
fn every_fault_has_its_own_text() {
    use std::collections::HashSet;

    let faults = [
        Fault::WideAddress,
        Fault::WideLength,
        Fault::NoneReference,
        Fault::NotABool,
        Fault::PastAddressSpace,
        Fault::StackLargerThanMachine,
        Fault::ConstantsLargerThanMachine,
        Fault::OutsideMemory,
        Fault::StackOutsideMemory,
        Fault::ConstantOutsideMemory,
        Fault::NotOneResult,
        Fault::NoSuchConstant,
        Fault::NoSuchFunction,
        Fault::NoSuchType,
        Fault::CodeTooLong,
        Fault::JumpPastEnd,
        Fault::FieldPastRecord,
        Fault::ListTooLarge,
        Fault::ListPastStack,
        Fault::ArgumentsPastStack,
        Fault::UndeclaredHeap,
        Fault::UndeclaredMemory,
        Fault::UndeclaredStack,
        Fault::ListTooLongToCount,
        Fault::EmptyStack,
        Fault::ValueChangedType,
        Fault::RecordLostField,
        Fault::RecordWrongSize,
        Fault::RecordNotRecordType,
        Fault::ListLostEntries,
        Fault::MapLostEntries,
        Fault::MapUnsorted,
        Fault::RecordLostFields,
        Fault::OptionLostValue,
        Fault::ReadAsSeveral,
        Fault::NotUtf8,
        Fault::StorePastLimit,
        Fault::WriteToStore,
        Fault::DynamicStep,
    ];

    let texts = faults
        .iter()
        .map(|f| show(&Message::Fault(*f), None))
        .collect::<HashSet<_>>();
    assert_eq!(texts.len(), faults.len());
    assert!(
        texts
            .iter()
            .all(|t| t.starts_with("the program is faulty: "))
    );
}

#[test]
fn mistyped_nested_set_gives_path_then_setting() {
    let env = env();
    let mut globals = Globals::new(&env);
    let set_messages = |error: &GlobalsError| {
        error
            .labels()
            .map(|l| l.message().to_string())
            .collect::<Vec<_>>()
    };

    let error = globals
        .set(
            "health",
            &Value::record([
                ("boxes", Value::int(1)),
                ("count", Value::list(vec![Value::int(1), Value::bool(true)])),
            ]),
        )
        .unwrap_err();

    assert_eq!(
        set_messages(&error),
        [
            "expected Int, found Bool",
            "at `health.count[1]`",
            "while setting `health`",
        ]
    );
    assert_eq!(
        format!("{error:#}"),
        "expected Int, found Bool: at `health.count[1]`: while setting `health`"
    );
    assert_eq!(error.to_string(), "expected Int, found Bool");
    assert_eq!(
        error.labels().map(|l| l.is_primary()).collect::<Vec<_>>(),
        [true, false, false]
    );
    assert!(error.labels().all(|l| l.span().is_none()));
    assert_eq!(error.labels().len(), 3);

    // A whole value of the wrong type has no path.
    let error = globals.set("strength", &Value::none()).unwrap_err();
    assert_eq!(
        set_messages(&error),
        ["expected Int, found none", "while setting `strength`"]
    );

    let error = globals
        .set("bonus", &Value::some(Value::bool(true)))
        .unwrap_err();
    assert_eq!(
        set_messages(&error),
        [
            "expected Option<Int>, found some(Bool)",
            "while setting `bonus`"
        ]
    );

    let error = globals.set("nothing", &Value::int(1)).unwrap_err();
    assert_eq!(set_messages(&error), ["no value `nothing` is declared"]);
}

#[test]
fn mistyped_host_result_gives_path_function_then_instruction() {
    let env = env();
    let globals = Globals::new(&env);
    let source = "1 + count(2).n";
    let (program, debug) = compile_debug(source, &env, &Options::new()).unwrap();
    let mut machine = Machine::default();
    let run = machine.load(&program, &globals).unwrap();

    let mut host = |_: &str, _: Vec<Value>| Ok(Value::record([("n", Value::bool(false))]));
    let error = run.run_with(&mut host).unwrap_err();

    assert_eq!(
        labels(&error, &program, Some(&debug), source),
        [
            (
                "expected Int, found Bool".to_owned(),
                Some("count(2)".to_owned())
            ),
            ("at `count(..).n`".to_owned(), None),
            ("while reading what `count` returned".to_owned(), None),
            (
                "while running instruction 2".to_owned(),
                Some("count(2)".to_owned())
            ),
            ("while running the program".to_owned(), None),
        ]
    );

    let run = machine.load(&program, &globals).unwrap();
    let mut host = |_: &str, _: Vec<Value>| Ok(Value::int(1));
    let error = run.run_with(&mut host).unwrap_err();
    assert_eq!(
        messages(&error, &program),
        [
            "expected {n: Int}, found Int",
            "while reading what `count` returned",
            "while running instruction 2",
            "while running the program",
        ]
    );
}

#[test]
fn host_failure_keeps_the_hosts_error_as_source() {
    use std::error::Error as _;

    let env = env();
    let program = compile("count(2).n", &env).unwrap();
    let globals = Globals::new(&env);
    let mut machine = Machine::default();
    let run = machine.load(&program, &globals).unwrap();

    let mut host = |_: &str, _: Vec<Value>| Err("no table".to_owned());
    let error = run.run_with(&mut host).unwrap_err();

    assert_eq!(
        messages(&error, &program),
        [
            "the function failed",
            "while calling `count`",
            "while running instruction 1",
            "while running the program",
        ]
    );
    assert_eq!(error.source().unwrap().to_string(), "no table");

    // Errors evalite makes have no source.
    let error = Machine::default()
        .load(&compile("1 / 0", &env).unwrap(), &globals)
        .unwrap()
        .run()
        .unwrap_err();
    assert!(error.source().is_none());
}

#[test]
fn runtime_errors_give_instruction_then_running() {
    let env = env();

    for (source, primary, span) in [
        (
            "1 + strength * 9223372036854775807",
            "`2 * 9223372036854775807` overflows an Int",
            "strength * 9223372036854775807",
        ),
        (
            "(strength - 2) / (strength - 2)",
            "`0 / 0` divides by zero",
            "(strength - 2) / (strength - 2)",
        ),
        (
            "health.count[strength + 5]",
            "index 7 is out of range of a list of 1 entry",
            "health.count[strength + 5]",
        ),
        (
            "strength.clamp(3, 1)",
            "`clamp` needs its low bound 3 to be at most its high bound 1",
            "strength.clamp(3, 1)",
        ),
        (
            "-(strength - 9223372036854775807 - 3)",
            "`-(-9223372036854775808)` overflows an Int",
            "-(strength - 9223372036854775807 - 3)",
        ),
    ] {
        let (program, debug) = compile_debug(source, &env, &Options::new()).unwrap();
        let mut machine = Machine::default();
        let mut globals = Globals::new(&env);

        for (name, _) in program.globals() {
            let value = match name {
                "strength" => Value::int(2),
                "health" => Value::record([
                    ("boxes", Value::int(1)),
                    ("count", Value::list(vec![Value::int(1)])),
                ]),
                _ => Value::none(),
            };

            globals.set(name, &value).unwrap();
        }

        let error = machine.load(&program, &globals).unwrap().run().unwrap_err();
        let got = labels(&error, &program, Some(&debug), source);

        assert_eq!(got.len(), 3, "{source}: {got:?}");
        assert_eq!(
            got[0],
            (primary.to_owned(), Some(span.to_owned())),
            "{source}"
        );
        assert!(
            got[1].0.starts_with("while running instruction "),
            "{source}"
        );
        assert_eq!(got[1].1.as_deref(), Some(span), "{source}");
        assert_eq!(
            got[2],
            ("while running the program".to_owned(), None),
            "{source}"
        );

        let primary_flags = error
            .labels(&program, None)
            .map(|l| l.is_primary())
            .collect::<Vec<_>>();
        assert_eq!(primary_flags, [true, false, false]);
        assert!(error.labels(&program, None).all(|l| l.span().is_none()));

        let instr = error
            .frames()
            .iter()
            .filter_map(|f| f.location)
            .collect::<Vec<_>>();
        assert!(matches!(instr.as_slice(), [Location::Instr(_)]), "{source}");
        assert_eq!(error.to_string(), primary);
        assert!(
            format!("{error:#}").starts_with(&format!("{primary}: while running instruction "))
        );
    }
}

#[test]
fn out_of_fuel_gives_instruction_then_running() {
    let env = env();
    let source = "1 + 2 + 3";
    let (program, debug) = compile_debug(source, &env, &Options::new()).unwrap();
    let mut machine = Machine::new(Limits::new().fuel(3));
    let globals = Globals::new(&env);
    let error = machine.load(&program, &globals).unwrap().run().unwrap_err();

    assert_eq!(
        labels(&error, &program, Some(&debug), source),
        [
            (
                "the expression took too long: it used all of its 3 steps".to_owned(),
                Some("3".to_owned())
            ),
            (
                "while running instruction 3".to_owned(),
                Some("3".to_owned())
            ),
            ("while running the program".to_owned(), None),
        ]
    );
    assert_eq!(error.message(), &Message::OutOfFuel { fuel: 3 });
}

#[test]
fn load_and_unset_errors_have_context() {
    let env = env();
    let program = compile("strength", &env).unwrap();
    let mut globals = Globals::new(&env);

    let error = Machine::new(Limits::new().memory(0))
        .load(&program, &globals)
        .unwrap_err();
    assert_eq!(
        messages(&error, &program),
        [
            "the program needs 1 word of memory, more than the 0 the machine allows",
            "while loading the program",
        ]
    );

    let mut machine = Machine::default();
    let error = machine.load(&program, &globals).unwrap().run().unwrap_err();
    assert_eq!(
        messages(&error, &program),
        [
            "`strength` is not set, set it to an Int before running",
            "while running instruction 0",
            "while running the program",
        ]
    );

    globals.set("strength", &Value::int(1)).unwrap();
    let outcome = machine.load(&program, &globals).unwrap().run().unwrap();
    assert_eq!(outcome.value(), &Value::int(1));
}

#[test]
fn structural_compile_spans_point_into_its_text() {
    let env = env();
    let structure = Structure::from_text("(strength + 1) * [1, 2][strength]").unwrap();
    let (program, debug) = structure.compile_debug(&env, &Options::new()).unwrap();
    let text = structure.to_text();

    let mut machine = Machine::default();
    let mut globals = Globals::new(&env);
    globals.set("strength", &Value::int(2)).unwrap();
    let error = machine.load(&program, &globals).unwrap().run().unwrap_err();

    let got = labels(&error, &program, Some(&debug), &text);
    assert_eq!(got[0].1.as_deref(), Some("[1, 2][strength]"));
    assert_eq!(
        compile_debug(&text, &env, &Options::new()).unwrap().1,
        debug
    );
}

#[test]
fn debug_info_has_a_span_per_instruction_and_round_trips() {
    let env = env();
    let source = "strength.max(health.boxes) + count(1).n + [1, 2].len()";
    let (program, debug) = compile_debug(source, &env, &Options::new()).unwrap();

    for pc in 0..program.code.len() as u32 {
        let span = debug.span(pc).unwrap();
        assert!(source.get(span.range()).is_some(), "{pc}");
    }

    assert_eq!(debug.span(program.code.len() as u32), None);

    let bytes = debug.to_bytes().unwrap();
    assert_eq!(DebugInfo::from_bytes(&bytes).unwrap(), debug);
    assert!(DebugInfo::from_bytes(&bytes[..bytes.len() - 1]).is_err());

    // The same program compiles with and without debug info.
    assert_eq!(
        compile(source, &env).unwrap().to_bytes().unwrap(),
        program.to_bytes().unwrap()
    );
}

#[test]
fn errors_are_one_word() {
    assert_eq!(size_of::<RunError>(), size_of::<usize>());
    assert_eq!(size_of::<Result<(), RunError>>(), size_of::<usize>());
}

#[test]
fn counts_of_one_are_singular() {
    use crate::message::plural;

    assert_eq!(plural(1u32, "word", "words").to_string(), "1 word");
    assert_eq!(plural(0u32, "word", "words").to_string(), "0 words");
    assert_eq!(plural(1u64, "step", "steps").to_string(), "1 step");
    assert_eq!(plural(2usize, "entry", "entries").to_string(), "2 entries");
}
