//! Tests of a store's values as bytes and carried over to another layout:
//! round trips, tampered and hostile bytes refused with their messages, and
//! each rule of migration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::globals::persist::{FORMAT, Part, Stored, StoredValue};
use crate::*;

fn int(n: i64) -> Value {
    Value::int(n)
}

fn s(s: &str) -> Value {
    Value::string(s)
}

fn skill_type() -> Type {
    Type::record([
        ("dots", Type::int()),
        ("specialties", Type::list(Type::string())),
    ])
}

fn skill(dots: i64, specialties: &[&str]) -> Value {
    Value::record([
        ("dots", int(dots)),
        ("specialties", Value::list(specialties.iter().map(|x| s(x)))),
    ])
}

/// An environment of every kind of value.
fn env() -> Environment {
    let mut env = Environment::new();
    env.value("n", Type::int()).unwrap();
    env.value("flag", Type::bool()).unwrap();
    env.value("name", Type::string()).unwrap();
    env.value("bonus", Type::option(Type::option(Type::int())))
        .unwrap();
    env.value("marks", Type::list(Type::bool())).unwrap();
    env.value("skills", Type::map(Type::string(), skill_type()))
        .unwrap();
    env.value(
        "by_level",
        Type::map(Type::int(), Type::list(Type::string())),
    )
    .unwrap();
    env.value("empty", Type::record::<&str>([])).unwrap();
    env.value("unset", Type::int()).unwrap();
    env
}

fn values() -> Vec<(&'static str, Value)> {
    vec![
        ("n", int(-7)),
        ("flag", Value::bool(true)),
        ("name", s("Ada, Countess of Lovelace")),
        ("bonus", Value::some(Value::none())),
        (
            "marks",
            Value::list([Value::bool(false), Value::bool(true)]),
        ),
        (
            "skills",
            Value::map([
                (s("brawl"), skill(2, &[])),
                (s("occult"), skill(3, &["ghosts", "rituals"])),
            ]),
        ),
        (
            "by_level",
            Value::map([
                (int(-1), Value::list([])),
                (int(4), Value::list([s(""), s("x")])),
            ]),
        ),
        ("empty", Value::record::<&str>([])),
    ]
}

fn filled(env: &Environment) -> Globals {
    let mut globals = Globals::new(env);

    for (name, value) in values() {
        globals.set(name, &value).unwrap();
    }

    globals
}

fn stored(globals: &Globals) -> Stored {
    crate::framed::body(&globals.to_bytes().unwrap())
}

fn read(env: &Environment, stored: &Stored) -> Result<Globals, GlobalsError> {
    Globals::from_bytes(env, &crate::framed::to_bytes(FORMAT, stored).unwrap())
}

/// `{:#}` of what reading `stored` back gives.
fn refused(env: &Environment, stored: &Stored) -> String {
    format!("{:#}", read(env, stored).unwrap_err())
}

fn value_mut<'a>(stored: &'a mut Stored, name: &str) -> &'a mut Vec<Part> {
    &mut stored
        .values
        .iter_mut()
        .find(|v| v.name == name)
        .unwrap()
        .parts
}

#[test]
fn every_kind_of_value_round_trips() {
    let env = env();
    let globals = filled(&env);
    let read = Globals::from_bytes(&env, &globals.to_bytes().unwrap()).unwrap();

    for (name, value) in values() {
        assert_eq!(read.get(name), Some(value), "{name}");
    }

    assert_eq!(read.get("unset"), None);
    assert_eq!(read.limit(), DEFAULT_GLOBALS);

    // Bytes of the read store are the same bytes.
    assert_eq!(read.to_bytes().unwrap(), globals.to_bytes().unwrap());

    // Only set values are stored, by name and in slot order.
    let names: Vec<_> = stored(&globals)
        .values
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(
        names,
        [
            "n", "flag", "name", "bonus", "marks", "skills", "by_level", "empty"
        ]
    );

    let empty = Globals::new(&env);
    let read = Globals::from_bytes(&env, &empty.to_bytes().unwrap()).unwrap();
    assert!(env.values().all(|(name, _)| read.get(name).is_none()));
}

#[test]
fn parts_are_in_pre_order() {
    let env = env();
    let stored = stored(&filled(&env));
    let parts = |name: &str| {
        stored
            .values
            .iter()
            .find(|v| v.name == name)
            .unwrap()
            .parts
            .clone()
    };

    assert_eq!(parts("bonus"), [Part::Some, Part::None]);
    assert_eq!(
        parts("skills"),
        [
            Part::Map(2),
            Part::String("brawl".into()),
            Part::Record(vec!["dots".into(), "specialties".into()]),
            Part::Int(2),
            Part::List(0),
            Part::String("occult".into()),
            Part::Record(vec!["dots".into(), "specialties".into()]),
            Part::Int(3),
            Part::List(2),
            Part::String("ghosts".into()),
            Part::String("rituals".into()),
        ]
    );
}

/// A store read back is laid out anew: no words free and no room spare,
/// however the store it was written from was updated, and runs read it.
#[test]
fn a_read_store_is_dense_and_runs() {
    let env = env();
    let mut globals = filled(&env);
    let skills = Path::value("skills");

    for i in 0..50 {
        let key = format!("skill {i}");
        globals
            .insert(&skills.clone().string_key(key.as_str()), &skill(i, &["a"]))
            .unwrap();
        globals
            .push(&Path::value("marks"), &Value::bool(i % 2 == 0))
            .unwrap();
    }

    for i in (0..50).step_by(3) {
        let key = format!("skill {i}");
        globals
            .remove(&skills.clone().string_key(key.as_str()))
            .unwrap();
    }

    let before = globals.stats();
    assert!(before.spare() > 0 || before.free() > 0, "{before:?}");

    let read = Globals::from_bytes(&env, &globals.to_bytes().unwrap()).unwrap();
    let after = read.stats();
    assert_eq!((after.free(), after.spare()), (0, 0), "{after:?}");
    assert_eq!(after.heap(), after.live());
    assert_eq!(after.live(), before.live() - before.spare());

    for (name, _) in env.values() {
        assert_eq!(read.get(name), globals.get(name), "{name}");
    }

    let program = compile(
        r#"skills["skill 4"].dots + skills["occult"].dots + marks.len()"#,
        &env,
    )
    .unwrap();
    let mut machine = Machine::default();
    assert_eq!(
        machine
            .load(&program, &read)
            .unwrap()
            .run()
            .unwrap()
            .value(),
        &int(4 + 3 + 52)
    );
}

#[test]
fn a_read_store_keeps_to_its_limit() {
    let mut env = Environment::new();
    env.value("name", Type::string()).unwrap();
    let mut globals = Globals::new(&env);
    globals.set("name", &s(&"x".repeat(80))).unwrap();
    let bytes = globals.to_bytes().unwrap();

    let read = Globals::from_bytes_with_limit(&env, &bytes, 10).unwrap();
    assert_eq!(read.limit(), 10);
    assert_eq!(read.get("name"), Some(s(&"x".repeat(80))));

    let error = Globals::from_bytes_with_limit(&env, &bytes, 9).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "the value needs 10 words of memory, 9 are left: while setting `name`: while reading stored values"
    );
}

/// The deepest value a declared type holds is read back; one deeper is
/// refused before it is built.
#[test]
fn values_nest_as_deep_as_types() {
    let mut ty = Type::int();
    let mut value = int(1);

    for _ in 1..MAX_TYPE_DEPTH {
        ty = Type::option(ty);
        value = Value::some(value);
    }

    let mut env = Environment::new();
    env.value("deep", ty).unwrap();
    let mut globals = Globals::new(&env);
    globals.set("deep", &value).unwrap();
    let mut stored = stored(&globals);
    assert_eq!(stored.values[0].parts.len(), MAX_TYPE_DEPTH);
    assert_eq!(read(&env, &stored).unwrap().get("deep"), Some(value));

    // Past it, with as many parts as the store has words.
    let parts = value_mut(&mut stored, "deep");
    parts.clear();
    parts.extend((0..1 << 16).map(|_| Part::Some));
    parts.push(Part::Int(1));
    assert_eq!(
        refused(&env, &stored),
        "the value nests too deeply: while setting `deep`: while reading stored values"
    );

    parts_of(&mut stored, (0..1 << 16).map(|_| Part::List(1)).collect());
    assert_eq!(
        refused(&env, &stored),
        "the value nests too deeply: while setting `deep`: while reading stored values"
    );
}

fn parts_of(stored: &mut Stored, parts: Vec<Part>) {
    stored.values[0].parts = parts;
}

#[test]
fn tampered_bytes_are_refused() {
    let env = env();
    let good = stored(&filled(&env));
    let tamper = |f: &dyn Fn(&mut Stored)| {
        let mut stored: Stored =
            crate::framed::body(&crate::framed::to_bytes(FORMAT, &good).unwrap());
        f(&mut stored);
        refused(&env, &stored)
    };

    // A later format is refused by its number, before its body is read,
    // whatever that body's layout.
    let mut newer = musli::storage::to_vec(&(FORMAT + 1)).unwrap();
    newer.extend([0xff, 0x00, 0x13, 0x37]);
    assert_eq!(
        format!("{:#}", Globals::from_bytes(&env, &newer).unwrap_err()),
        "it was stored in format 3, this evalite reads format 2: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| s.values[0].name = "m".into()),
        "no value `m` is declared: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| {
            let again = StoredValue {
                name: "n".into(),
                parts: vec![Part::Int(1)],
            };
            s.values.push(again);
        }),
        "the value is stored twice: while setting `n`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| *value_mut(s, "name") = vec![Part::Int(1)]),
        "expected String, found Int: while setting `name`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "skills")[9] = Part::Bool(true)),
        "expected String, found Bool: at `skills[\"occult\"].specialties[0]`: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| {
            value_mut(s, "skills").pop();
        }),
        "the value's parts end before the value does: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| *value_mut(s, "n") = Vec::new()),
        "the value's parts end before the value does: while setting `n`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "n").push(Part::Int(2))),
        "the value has parts past its end: while setting `n`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "marks")[0] = Part::List(3)),
        "the value's parts end before the value does: while setting `marks`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "marks")[0] = Part::List(1)),
        "the value has parts past its end: while setting `marks`: while reading stored values"
    );
    // A field missing, one more, and one twice.
    assert_eq!(
        tamper(&|s| {
            let parts = value_mut(s, "skills");
            parts[2] = Part::Record(vec!["dots".into()]);
            parts.remove(4);
        }),
        "expected {dots: Int, specialties: List<String>}, found a record without `specialties`: at `skills[\"brawl\"]`: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| {
            let parts = value_mut(s, "skills");
            parts[2] = Part::Record(vec!["dots".into(), "more".into(), "specialties".into()]);
            parts.insert(4, Part::Int(0));
        }),
        "expected {dots: Int, specialties: List<String>}, found a record with an extra field: at `skills[\"brawl\"]`: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "skills")[2] = Part::Record(vec!["dots".into(), "dots".into()])),
        "the value has a record of fields not sorted and unique: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| {
            value_mut(s, "skills")[2] = Part::Record(vec!["specialties".into(), "dots".into()]);
        }),
        "the value has a record of fields not sorted and unique: while setting `skills`: while reading stored values"
    );
    // Keys out of order, and a key twice.
    assert_eq!(
        tamper(&|s| value_mut(s, "skills")[5] = Part::String("a".into())),
        "the value has a map of keys not sorted and unique: while setting `skills`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "by_level")[3] = Part::Int(-1)),
        "the value has a map of keys not sorted and unique: while setting `by_level`: while reading stored values"
    );
    assert_eq!(
        tamper(&|s| value_mut(s, "by_level")[3] = Part::String("a".into())),
        "expected Int, found String: at `by_level[key #1]`: while setting `by_level`: while reading stored values"
    );
}

/// A count far past the parts that follow it is read as far as they go,
/// reserving nothing for the rest.
#[test]
fn huge_counts_end_early() {
    let mut env = Environment::new();
    env.value("l", Type::list(Type::int())).unwrap();
    env.value("m", Type::map(Type::int(), Type::int())).unwrap();

    for (name, part) in [("l", Part::List(u64::MAX)), ("m", Part::Map(u64::MAX))] {
        let stored = Stored {
            values: vec![StoredValue {
                name: name.into(),
                parts: vec![part, Part::Int(1), Part::Int(2), Part::Int(3)],
            }],
        };

        assert_eq!(
            refused(&env, &stored),
            format!(
                "the value's parts end before the value does: while setting `{name}`: while reading stored values"
            )
        );
    }
}

/// Every cut of the bytes, and every byte changed, is refused or read as a
/// store of values of their declared types; none panics.
#[test]
fn cut_and_flipped_bytes_never_panic() {
    let env = env();
    let bytes = filled(&env).to_bytes().unwrap();

    for at in 0..bytes.len() {
        assert!(
            Globals::from_bytes(&env, &bytes[..at]).is_err(),
            "cut at {at}"
        );
    }

    let mut rng = crate::tests::Rng(0xb17e5);

    for at in 0..bytes.len() {
        for _ in 0..4 {
            let mut flipped = bytes.clone();
            flipped[at] = rng.next() as u8;

            if let Ok(read) = Globals::from_bytes(&env, &flipped) {
                // Whatever it holds is of its declared type.
                let again = Globals::from_bytes(&env, &read.to_bytes().unwrap()).unwrap();

                for (name, _) in env.values() {
                    assert_eq!(again.get(name), read.get(name));
                }
            }
        }
    }
}

/// A random mix of updates, written out and read back after each phase,
/// reads back what the store holds.
#[test]
fn random_stores_round_trip() {
    let env = env();
    let mut globals = filled(&env);
    let mut rng = crate::tests::Rng(0x5701e);
    let skills = Path::value("skills");

    for phase in 0..20 {
        for _ in 0..50 {
            let key = format!("s{}", rng.below(30));
            let path = skills.clone().string_key(key.as_str());

            match rng.below(4) {
                0 => {
                    let _ = globals.remove(&path);
                }
                1 => {
                    let dots = rng.below(5) as i64;
                    let _ = globals.insert(&path, &skill(dots, &["x"; 3][..rng.below(4)]));
                }
                2 => {
                    let n = rng.below(100) as i64;
                    let path = Path::value("by_level").int_key(n);
                    let _ = globals.insert(&path, &Value::list([s(&"y".repeat(rng.below(20)))]));
                }
                _ => {
                    let bonus = match rng.below(3) {
                        0 => Value::none(),
                        1 => Value::some(Value::none()),
                        _ => Value::some(Value::some(int(phase))),
                    };
                    globals.set("bonus", &bonus).unwrap();
                }
            }
        }

        let read = Globals::from_bytes(&env, &globals.to_bytes().unwrap()).unwrap();

        for (name, _) in env.values() {
            assert_eq!(
                read.get(name),
                globals.get(name),
                "{name} after phase {phase}"
            );
        }
    }
}

fn migrated(old: &Globals, env: &Environment) -> (Globals, Vec<String>) {
    let (globals, dropped) = Globals::migrate(old, env);
    (globals, dropped.iter().map(ToString::to_string).collect())
}

#[test]
fn migrating_to_the_same_layout_carries_everything() {
    let env = env();
    let old = filled(&env);
    let (new, dropped) = migrated(&old, &env);
    assert!(dropped.is_empty(), "{dropped:?}");

    for (name, _) in env.values() {
        assert_eq!(new.get(name), old.get(name), "{name}");
    }
}

#[test]
fn migrating_with_an_added_global() {
    let mut env = env();
    let old = filled(&env);
    env.value("added", Type::list(Type::int())).unwrap();

    let (mut new, dropped) = migrated(&old, &env);
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(new.get("added"), None);
    assert_eq!(new.get("n"), Some(int(-7)));

    // Programs reading the new value run once it is set.
    let program = compile("added.len() + n", &env).unwrap();
    let mut machine = Machine::default();
    assert!(machine.load(&program, &new).unwrap().run().is_err());
    new.set("added", &Value::list([int(1)])).unwrap();
    assert_eq!(
        machine.load(&program, &new).unwrap().run().unwrap().value(),
        &int(-6)
    );
}

/// Globals declared in another order take other slots; values carry over
/// by name.
#[test]
fn migrating_with_a_removed_global() {
    let old_env = env();
    let mut old = filled(&old_env);
    old.set("unset", &int(1)).unwrap();

    let mut env = Environment::new();
    env.value("skills", Type::map(Type::string(), skill_type()))
        .unwrap();
    env.value("n", Type::int()).unwrap();

    let (new, dropped) = Globals::migrate(&old, &env);
    assert_eq!(new.get("n"), Some(int(-7)));
    assert_eq!(new.get("skills"), old.get("skills"));

    let names: Vec<_> = dropped.iter().map(Dropped::name).collect();
    assert_eq!(
        names,
        [
            "flag", "name", "bonus", "marks", "by_level", "empty", "unset"
        ]
    );
    assert!(
        dropped
            .iter()
            .all(|d| *d.reason() == DropReason::NotDeclared)
    );
    assert_eq!(dropped[1].value(), &s("Ada, Countess of Lovelace"));
    assert_eq!(dropped[1].to_string(), "`name` is no longer declared");
}

#[test]
fn migrating_with_a_changed_type() {
    let old_env = env();
    let old = filled(&old_env);

    let mut env = Environment::new();
    env.value("n", Type::option(Type::int())).unwrap();
    env.value("name", Type::string()).unwrap();
    // The key type changes.
    env.value(
        "by_level",
        Type::map(Type::string(), Type::list(Type::string())),
    )
    .unwrap();
    // A field changes type and another is added: the change is reported.
    env.value(
        "skills",
        Type::map(
            Type::string(),
            Type::record([
                ("dots", Type::bool()),
                ("specialties", Type::list(Type::string())),
                ("xp", Type::int()),
            ]),
        ),
    )
    .unwrap();

    let (new, dropped) = Globals::migrate(&old, &env);
    assert_eq!(new.get("name"), old.get("name"));
    assert_eq!(new.get("n"), None);
    assert_eq!(new.get("skills"), None);

    let shown: Vec<_> = dropped.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "`n` was declared as Int, and is now Option<Int>",
            "`flag` is no longer declared",
            "`bonus` is no longer declared",
            "`marks` is no longer declared",
            "`skills` was declared as Map<String, {dots: Int, specialties: List<String>}>, and is now Map<String, {dots: Bool, specialties: List<String>, xp: Int}>",
            "`by_level` was declared as Map<Int, List<String>>, and is now Map<String, List<String>>",
            "`empty` is no longer declared",
        ]
    );
    assert_eq!(
        dropped[0].reason(),
        &DropReason::Retyped {
            old: Type::int(),
            new: Type::option(Type::int())
        }
    );
    assert_eq!(dropped[0].value(), &int(-7));
}

#[test]
fn migrating_records_by_field() {
    let mut old_env = Environment::new();
    let old_skill = Type::record([
        ("dots", Type::int()),
        ("notes", Type::string()),
        ("specialties", Type::list(Type::string())),
    ]);
    old_env
        .value("skills", Type::map(Type::string(), old_skill.clone()))
        .unwrap();
    old_env
        .value("track", Type::option(Type::list(old_skill.clone())))
        .unwrap();
    old_env.value("gains", old_skill.clone()).unwrap();
    old_env.value("none", Type::option(old_skill)).unwrap();

    let old_value = |dots, notes: &str| {
        Value::record([
            ("dots", int(dots)),
            ("notes", s(notes)),
            ("specialties", Value::list([s("a")])),
        ])
    };

    let mut old = Globals::new(&old_env);
    let skills = Value::map([
        (s("brawl"), old_value(1, "fists")),
        (s("occult"), old_value(3, "ghosts")),
    ]);
    old.set("skills", &skills).unwrap();
    let track = Value::some(Value::list([old_value(2, "x"), old_value(4, "y")]));
    old.set("track", &track).unwrap();
    old.set("gains", &old_value(5, "z")).unwrap();
    old.set("none", &Value::none()).unwrap();

    let mut env = Environment::new();
    let lost = Type::record([
        ("dots", Type::int()),
        ("specialties", Type::list(Type::string())),
    ]);
    let gained = Type::record([
        ("dots", Type::int()),
        ("notes", Type::string()),
        ("specialties", Type::list(Type::string())),
        ("xp", Type::int()),
    ]);
    env.value("skills", Type::map(Type::string(), lost.clone()))
        .unwrap();
    env.value("track", Type::option(Type::list(lost))).unwrap();
    env.value("gains", gained.clone()).unwrap();
    // A record gaining a field leaves the value unset, even with no record
    // in it to give the field a value for.
    env.value("none", Type::option(gained.clone())).unwrap();

    let (mut new, dropped) = Globals::migrate(&old, &env);

    // Removed fields are left out, wherever the records are.
    assert_eq!(
        new.get("skills"),
        Some(Value::map([
            (s("brawl"), skill(1, &["a"])),
            (s("occult"), skill(3, &["a"]))
        ]))
    );
    assert_eq!(
        new.get("track"),
        Some(Value::some(Value::list([
            skill(2, &["a"]),
            skill(4, &["a"])
        ])))
    );
    assert_eq!(new.get("gains"), None);
    assert_eq!(new.get("none"), None);

    let shown: Vec<_> = dropped.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "`gains` holds a record of type {dots: Int, notes: String, specialties: List<String>, xp: Int}, whose field `xp` it has no value for",
            "`none` holds a record of type {dots: Int, notes: String, specialties: List<String>, xp: Int}, whose field `xp` it has no value for",
        ]
    );
    assert_eq!(
        dropped[0].reason(),
        &DropReason::NewField {
            record: gained,
            field: "xp".into()
        }
    );

    // The value given back is what a caller makes the new one from.
    let mut fields: Vec<(String, Value)> = Vec::new();
    for name in ["dots", "notes", "specialties"] {
        fields.push((name.into(), dropped[0].value().field(name).unwrap().clone()));
    }
    fields.push(("xp".into(), int(0)));
    new.set(dropped[0].name(), &Value::record(fields)).unwrap();

    let program = compile(r#"gains.dots + gains.xp + skills["occult"].dots"#, &env).unwrap();
    assert_eq!(
        Machine::default()
            .load(&program, &new)
            .unwrap()
            .run()
            .unwrap()
            .value(),
        &int(8)
    );
}

#[test]
fn migrating_keeps_the_limit_and_packs_the_values() {
    let env = env();
    let mut old = Globals::with_limit(&env, 4000);

    for (name, value) in values() {
        old.set(name, &value).unwrap();
    }

    for i in 0..40 {
        old.push(&Path::value("marks"), &Value::bool(i % 3 == 0))
            .unwrap();
    }

    let (new, dropped) = Globals::migrate(&old, &env);
    assert!(dropped.is_empty());
    assert_eq!(new.limit(), 4000);
    let stats = new.stats();
    assert_eq!((stats.free(), stats.spare()), (0, 0), "{stats:?}");
}
