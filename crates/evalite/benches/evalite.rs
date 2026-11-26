//! Benchmarks of every stage a computed value goes through, on
//! expressions shaped like a character sheet's. See `benches/README.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::fmt::Write;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use evalite::{DEFAULT_FUEL, Environment, Limits, Machine, Program, Type, Value};

const ATTRIBUTES: [&str; 9] = [
    "intelligence",
    "wits",
    "resolve",
    "strength",
    "dexterity",
    "stamina",
    "presence",
    "manipulation",
    "composure",
];

const SKILLS: [&str; 24] = [
    "academics",
    "computer",
    "crafts",
    "investigation",
    "medicine",
    "occult",
    "politics",
    "science",
    "athletics",
    "brawl",
    "drive",
    "firearms",
    "larceny",
    "stealth",
    "survival",
    "weaponry",
    "animal_ken",
    "empathy",
    "expression",
    "intimidation",
    "persuasion",
    "socialize",
    "streetwise",
    "subterfuge",
];

/// How many `bonus_N: Option<Int>` globals the sheet has.
const BONUSES: usize = 16;

/// How many entries `merits: List<Option<Int>>` has.
const MERITS: usize = 12;

/// A character sheet's declarations.
fn env() -> Environment {
    let mut env = Environment::with_builtins();

    for name in ATTRIBUTES.iter().chain(&SKILLS) {
        env.value(name, Type::int()).unwrap();
    }

    for n in 0..BONUSES {
        env.value(&format!("bonus_{n}"), Type::option(Type::int()))
            .unwrap();
    }

    env.value(
        "specialties",
        Type::record(SKILLS.map(|s| (s, Type::option(Type::int())))),
    )
    .unwrap();

    let track = || Type::record([("boxes", Type::int()), ("count", Type::list(Type::int()))]);

    env.value("health", track()).unwrap();
    env.value("willpower", track()).unwrap();
    env.value("vitae", track()).unwrap();
    env.value("blood_potency", Type::int()).unwrap();
    env.value("humanity", Type::int()).unwrap();
    env.value("size", Type::int()).unwrap();
    env.value("armor", Type::option(Type::int())).unwrap();
    env.value("merits", Type::list(Type::option(Type::int())))
        .unwrap();
    env.value("feeding", Type::list(Type::int())).unwrap();
    env.value("table", Type::list(Type::int())).unwrap();
    env.function("rating", [Type::string()], Type::int())
        .unwrap();
    env
}

/// The sheet's values, by name.
fn values() -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();

    for (n, name) in ATTRIBUTES.iter().enumerate() {
        values.insert((*name).to_owned(), Value::int(1 + n as i64 % 5));
    }

    for (n, name) in SKILLS.iter().enumerate() {
        values.insert((*name).to_owned(), Value::int(n as i64 % 4));
    }

    let some_or_none = |n: usize| {
        if n.is_multiple_of(3) {
            Value::none()
        } else {
            Value::some(Value::int(n as i64 % 3))
        }
    };

    for n in 0..BONUSES {
        values.insert(format!("bonus_{n}"), some_or_none(n));
    }

    values.insert(
        "specialties".to_owned(),
        Value::record(
            SKILLS
                .iter()
                .enumerate()
                .map(|(n, s)| (*s, some_or_none(n))),
        ),
    );

    let track = |boxes: i64| {
        Value::record([
            ("boxes", Value::int(boxes)),
            (
                "count",
                Value::list(vec![Value::int(boxes - 2), Value::int(1), Value::int(1)]),
            ),
        ])
    };

    values.insert("health".to_owned(), track(7));
    values.insert("willpower".to_owned(), track(5));
    values.insert("vitae".to_owned(), track(10));
    values.insert("blood_potency".to_owned(), Value::int(2));
    values.insert("humanity".to_owned(), Value::int(7));
    values.insert("size".to_owned(), Value::int(5));
    values.insert("armor".to_owned(), Value::some(Value::int(1)));
    values.insert(
        "merits".to_owned(),
        Value::list((0..MERITS).map(some_or_none)),
    );
    values.insert("feeding".to_owned(), Value::list((0..11).map(Value::int)));
    values.insert(
        "table".to_owned(),
        Value::list((0..64).map(|n| Value::int(n * 3))),
    );
    values
}

/// The caller's `rating(String)`: a trait's rating, by name.
fn host(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match (name, args.as_slice()) {
        ("rating", [name]) => match name.as_str() {
            Some(_) => Ok(Value::int(3)),
            None => Err("wrong arguments to rating".to_owned()),
        },
        _ => Err(format!("no function {name}")),
    }
}

/// The sum of `terms` values, cycling through attributes, skills and
/// optional bonuses, as long as the default fuel allows.
fn long(terms: usize) -> String {
    let mut out = String::new();

    for n in 0..terms {
        if n > 0 {
            out.push_str(" + ");
        }

        match n % 3 {
            0 => out.push_str(ATTRIBUTES[n % ATTRIBUTES.len()]),
            1 => out.push_str(SKILLS[n % SKILLS.len()]),
            _ => write!(out, "bonus_{}.unwrap_or(0)", n % BONUSES).unwrap(),
        }
    }

    out
}

/// `depth` nested parentheses, each adding a value to what it holds, as
/// deep as the default `max_depth` allows.
fn deep(depth: usize) -> String {
    let mut out = String::new();

    for n in 0..depth {
        write!(out, "{} + (", ATTRIBUTES[n % ATTRIBUTES.len()]).unwrap();
    }

    out.push_str("blood_potency");

    for _ in 0..depth {
        out.push(')');
    }

    out
}

/// The single expressions, smallest first.
fn cases() -> Vec<(&'static str, String)> {
    let unwraps = (0..BONUSES)
        .map(|n| format!("bonus_{n}.unwrap_or(0)"))
        .collect::<Vec<_>>()
        .join(" + ");

    let tests = (0..BONUSES)
        .map(|n| {
            if n.is_multiple_of(2) {
                format!("bonus_{n}.is_none()")
            } else {
                format!("bonus_{n}.is_some()")
            }
        })
        .collect::<Vec<_>>()
        .join(" || ");

    let options = (0..8)
        .map(|n| format!("bonus_{n}"))
        .collect::<Vec<_>>()
        .join(", ");

    vec![
        ("pool", "dexterity + athletics + 2".to_owned()),
        (
            "pool_specialty",
            "strength + brawl + specialties.brawl.unwrap_or(0) + bonus_1.unwrap_or(0)".to_owned(),
        ),
        ("unwrap_or_16", unwraps),
        ("is_none_16", tests),
        (
            "get_literal",
            "[0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5].get(blood_potency).unwrap_or(0)".to_owned(),
        ),
        (
            "get_global",
            "feeding.get(blood_potency).unwrap_or(0) + table.get(humanity).unwrap_or(0)".to_owned(),
        ),
        ("host", r#"blood_potency + rating("Essence")"#.to_owned()),
        (
            "track",
            "health.boxes - health.count[1] - health.count[2] - willpower.count[1]".to_owned(),
        ),
        (
            "clamp_chain",
            "min(max(strength + brawl - armor.unwrap_or(0), 0), 5).clamp(0, 10).max(size).min(humanity) + max(wits, composure, 1)"
                .to_owned(),
        ),
        (
            "option_list",
            format!(
                "[{options}][blood_potency].unwrap_or(0) + merits[3].unwrap_or(0) + merits[4].unwrap_or(0)"
            ),
        ),
        ("long", long(370)),
        ("deep", deep(31)),
    ]
}

/// A whole sheet's computed values: pools for each attribute and skill
/// pair a sheet shows, derived traits, and the cases above.
fn sheet() -> Vec<String> {
    let mut out = Vec::new();

    for (n, skill) in SKILLS.iter().enumerate() {
        let attribute = ATTRIBUTES[n % ATTRIBUTES.len()];
        out.push(format!(
            "{attribute} + {skill} + specialties.{skill}.unwrap_or(0) + bonus_{}.unwrap_or(0)",
            n % BONUSES
        ));
        out.push(format!("{skill} == 0 && specialties.{skill}.is_none()"));
        out.push(format!("max({attribute} + {skill} - health.count[1], 0)"));
    }

    for (n, attribute) in ATTRIBUTES.iter().enumerate() {
        out.push(format!(
            "{attribute}.clamp(1, 5) + bonus_{}.unwrap_or(0)",
            n % BONUSES
        ));
    }

    out.extend(
        [
            "stamina + size",
            "health.boxes - health.count[1] - health.count[2]",
            "resolve + composure",
            "willpower.boxes - willpower.count[1]",
            "min(dexterity, wits) + athletics",
            "min(wits, dexterity) + armor.unwrap_or(0)",
            "size + strength + dexterity",
            "[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20].get(blood_potency).unwrap_or(10)",
            "feeding.get(blood_potency).unwrap_or(0)",
            "table.get(humanity).unwrap_or(0) + 1",
            "vitae.boxes - vitae.count[1]",
            "humanity >= 5 || bonus_3.is_some()",
            "merits[0].unwrap_or(0) + merits[1].unwrap_or(0) + merits[2].unwrap_or(0)",
            "blood_potency * 2 + 1",
            "(strength + stamina) / 2",
        ]
        .map(str::to_owned),
    );

    out.extend(cases().into_iter().map(|(_, e)| e));
    out
}

/// A program and what it reads.
struct Prepared {
    program: Program,
    bytes: Vec<u8>,
    bindings: Vec<(String, Value)>,
}

fn prepare(source: &str, env: &Environment, values: &BTreeMap<String, Value>) -> Prepared {
    let program = evalite::compile(source, env).unwrap_or_else(|e| panic!("{source}: {e}"));
    let bytes = program.to_bytes().unwrap();
    let bindings = program
        .globals()
        .into_iter()
        .map(|(name, _)| (name.to_owned(), values[name].clone()))
        .collect();

    let prepared = Prepared {
        program,
        bytes,
        bindings,
    };

    // Every benchmark must run within the default limits.
    let mut machine = Machine::new(Limits::new());
    let outcome = evaluate(&mut machine, &prepared);
    assert!(outcome.steps() <= DEFAULT_FUEL, "{source}");
    prepared
}

/// Load, bind and run one program, as a caller holding its values does.
fn evaluate(machine: &mut Machine, prepared: &Prepared) -> evalite::Outcome {
    let mut run = machine.load(&prepared.program).unwrap();

    for (name, value) in &prepared.bindings {
        run.bind(name, value.clone()).unwrap();
    }

    run.run_with(&mut host).unwrap()
}

fn stages(c: &mut Criterion) {
    let env = env();
    let values = values();

    for (name, source) in cases() {
        let prepared = prepare(&source, &env, &values);
        let mut group = c.benchmark_group(format!("stage/{name}"));

        group.bench_function("compile", |b| {
            b.iter(|| evalite::compile(black_box(&source), &env).unwrap())
        });

        group.bench_function("from_bytes", |b| {
            b.iter(|| Program::from_bytes(black_box(&prepared.bytes)).unwrap())
        });

        let mut machine = Machine::new(Limits::new());

        group.bench_function("load", |b| {
            b.iter(|| {
                machine.load(black_box(&prepared.program)).unwrap();
            })
        });

        group.bench_function("load_bind", |b| {
            b.iter(|| {
                let mut run = machine.load(black_box(&prepared.program)).unwrap();

                for (name, value) in &prepared.bindings {
                    run.bind(name, value.clone()).unwrap();
                }
            })
        });

        group.bench_function("load_bind_run", |b| {
            b.iter(|| evaluate(&mut machine, black_box(&prepared)))
        });

        group.bench_function("end_to_end", |b| {
            b.iter(|| {
                let program = evalite::compile(black_box(&source), &env).unwrap();
                let mut run = machine.load(&program).unwrap();

                for (name, _) in program.globals() {
                    run.bind(name, values[name].clone()).unwrap();
                }

                run.run_with(&mut host).unwrap()
            })
        });

        group.finish();
    }
}

/// What reading a result back into a [`Value`] costs: a value read whole
/// against the same value reduced to one `Int` by one more instruction.
fn read(c: &mut Criterion) {
    let env = env();
    let values = values();
    let mut group = c.benchmark_group("read");
    let mut machine = Machine::new(Limits::new());

    for source in [
        "table",
        "table.len()",
        "specialties",
        "specialties.brawl.unwrap_or(0)",
        "merits",
        "merits.len()",
    ] {
        let prepared = prepare(source, &env, &values);

        group.bench_with_input(BenchmarkId::from_parameter(source), &prepared, |b, p| {
            b.iter(|| evaluate(&mut machine, black_box(p)))
        });
    }

    group.finish();
}

/// A whole sheet: every computed value of one character, from source, from
/// stored bytes, and evaluated from compiled programs.
fn whole_sheet(c: &mut Criterion) {
    let env = env();
    let values = values();
    let sources = sheet();
    let prepared = sources
        .iter()
        .map(|s| prepare(s, &env, &values))
        .collect::<Vec<_>>();

    let mut group = c.benchmark_group("sheet");
    group.throughput(Throughput::Elements(sources.len() as u64));

    group.bench_function("compile", |b| {
        b.iter(|| {
            for source in &sources {
                black_box(evalite::compile(black_box(source), &env).unwrap());
            }
        })
    });

    group.bench_function("from_bytes", |b| {
        b.iter(|| {
            for p in &prepared {
                black_box(Program::from_bytes(black_box(&p.bytes)).unwrap());
            }
        })
    });

    let mut machine = Machine::new(Limits::new());

    group.bench_function("evaluate", |b| {
        b.iter(|| {
            for p in &prepared {
                black_box(evaluate(&mut machine, black_box(p)));
            }
        })
    });

    // The bindings looked up by name and cloned out of the sheet's values
    // for each program, as a caller would.
    group.bench_function("evaluate_lookup", |b| {
        b.iter(|| {
            for p in &prepared {
                let mut run = machine.load(&p.program).unwrap();

                for (name, _) in p.program.globals() {
                    run.bind(name, values[name].clone()).unwrap();
                }

                black_box(run.run_with(&mut host).unwrap());
            }
        })
    });

    group.finish();
}

criterion_group!(benches, stages, read, whole_sheet);
criterion_main!(benches);
