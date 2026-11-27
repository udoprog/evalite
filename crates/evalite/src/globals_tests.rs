//! Tests of a store's updates by path: every kind of step, each error with
//! its message, the words freed and taken, and runs reading what was set.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::*;

fn int(n: i64) -> Value {
    Value::int(n)
}

fn s(s: &str) -> Value {
    Value::string(s)
}

fn track_type() -> Type {
    Type::record([("boxes", Type::int()), ("marks", Type::list(Type::bool()))])
}

fn skill_type() -> Type {
    Type::record([
        ("dots", Type::int()),
        ("specialties", Type::list(Type::string())),
    ])
}

fn env() -> Environment {
    let mut env = Environment::new();
    env.value("n", Type::int()).unwrap();
    env.value(
        "sheet",
        Type::record([
            ("name", Type::string()),
            ("rank", Type::int()),
            ("bonus", Type::option(Type::int())),
            ("tracks", Type::list(track_type())),
            ("skills", Type::map(Type::string(), skill_type())),
            (
                "by id",
                Type::map(
                    Type::int(),
                    Type::option(Type::option(Type::record([("a", Type::int())]))),
                ),
            ),
        ]),
    )
    .unwrap();
    env.value("unset", Type::record([("a", Type::int())]))
        .unwrap();
    env.function("roll", [Type::int()], Type::record([("a", Type::int())]))
        .unwrap();
    env
}

fn track(boxes: i64, marks: &[bool]) -> Value {
    Value::record([
        ("boxes", int(boxes)),
        ("marks", Value::list(marks.iter().map(|m| Value::bool(*m)))),
    ])
}

fn skill(dots: i64, specialties: &[&str]) -> Value {
    Value::record([
        ("dots", int(dots)),
        ("specialties", Value::list(specialties.iter().map(|x| s(x)))),
    ])
}

fn a(n: i64) -> Value {
    Value::record([("a", int(n))])
}

fn sheet() -> Value {
    Value::record([
        ("name", s("Ada")),
        ("rank", int(3)),
        ("bonus", Value::none()),
        (
            "tracks",
            Value::list([track(5, &[true, false]), track(3, &[])]),
        ),
        (
            "skills",
            Value::map([
                (s("brawl"), skill(2, &[])),
                (s("occult"), skill(3, &["ghosts", "rituals"])),
            ]),
        ),
        (
            "by id",
            Value::map([
                (int(-4), Value::some(Value::some(a(1)))),
                (int(7), Value::some(Value::none())),
                (int(9), Value::none()),
            ]),
        ),
    ])
}

fn store() -> Globals {
    let mut globals = Globals::new(&env());
    globals.set("sheet", &sheet()).unwrap();
    globals.set("n", &int(1)).unwrap();
    globals
}

fn sheet_path() -> Path {
    Path::value("sheet")
}

/// The value `source` gives over `globals`.
fn run(globals: &Globals, source: &str) -> Value {
    let program = compile(source, &env()).unwrap();
    Machine::default()
        .load(&program, globals)
        .unwrap()
        .run()
        .unwrap()
        .value
}

/// Each label of the error of setting `path` to `value`.
fn error(globals: &mut Globals, path: &Path, value: &Value) -> Vec<String> {
    let before = globals.get("sheet");
    let heap = globals.heap_words();
    let error = globals.set_at(path, value).unwrap_err();
    assert_eq!(globals.get("sheet"), before, "a failed set changes nothing");
    assert_eq!(globals.heap_words(), heap, "a failed set takes nothing");
    error.labels().map(|l| l.message().to_string()).collect()
}

#[test]
fn every_step_kind() {
    let mut globals = store();

    // A field, in place.
    globals
        .set_at(&sheet_path().field("rank"), &int(4))
        .unwrap();
    assert_eq!(globals.get_at(&sheet_path().field("rank")), Some(int(4)));

    // A field of a tagged option, both its words.
    globals
        .set_at(&sheet_path().field("bonus"), &Value::some(int(2)))
        .unwrap();
    assert_eq!(
        globals.get_at(&sheet_path().field("bonus")),
        Some(Value::some(int(2)))
    );

    // A field spelled as it is not written in a dotted path.
    globals
        .set_at(
            &sheet_path().field("by id").int_key(9),
            &Value::some(Value::some(a(2))),
        )
        .unwrap();

    // An index, then a field, then an index.
    let marks = sheet_path().field("tracks").index(0).field("marks");
    globals
        .set_at(&marks.clone().index(1), &Value::bool(true))
        .unwrap();
    assert_eq!(
        globals.get_at(&marks),
        Some(Value::list([Value::bool(true), Value::bool(true)]))
    );

    // A whole entry of a list.
    globals
        .set_at(&sheet_path().field("tracks").index(1), &track(4, &[false]))
        .unwrap();

    // A String key, then a field, then an index into a list of strings.
    let occult = sheet_path().field("skills").string_key("occult");
    globals
        .set_at(&occult.clone().field("specialties").index(1), &s("wards"))
        .unwrap();
    globals
        .set_at(&occult.clone().field("dots"), &int(5))
        .unwrap();

    // An Int key, a negative one, through both options to the field.
    globals
        .set_at(&sheet_path().field("by id").int_key(-4).field("a"), &int(8))
        .unwrap();

    let mut want = sheet();
    let Value(crate::value::Repr::Record(fields)) = &mut want else {
        panic!()
    };
    fields.insert("rank".into(), int(4));
    fields.insert("bonus".into(), Value::some(int(2)));
    fields.insert(
        "tracks".into(),
        Value::list([track(5, &[true, true]), track(4, &[false])]),
    );
    fields.insert(
        "skills".into(),
        Value::map([
            (s("brawl"), skill(2, &[])),
            (s("occult"), skill(5, &["ghosts", "wards"])),
        ]),
    );
    fields.insert(
        "by id".into(),
        Value::map([
            (int(-4), Value::some(Value::some(a(8)))),
            (int(7), Value::some(Value::none())),
            (int(9), Value::some(Value::some(a(2)))),
        ]),
    );
    assert_eq!(globals.get("sheet"), Some(want));

    // A path without steps is the value itself, set or not.
    assert_eq!(globals.get_at(&Path::value("n")), Some(int(1)));
    globals.set_at(&Path::value("unset"), &a(3)).unwrap();
    assert_eq!(globals.get("unset"), Some(a(3)));
}

#[test]
fn runs_observe_updates() {
    let mut globals = store();
    let source = r#"sheet.rank + sheet.tracks[1].boxes + sheet.skills["occult"].dots
        + sheet.bonus.unwrap_or(0) + sheet["by id"][-4]!!.a
        + sheet.skills["occult"].specialties.len()"#;
    assert_eq!(run(&globals, source), int(3 + 3 + 3 + 1 + 2));

    let occult = sheet_path().field("skills").string_key("occult");
    globals
        .set_at(&sheet_path().field("rank"), &int(10))
        .unwrap();
    globals
        .set_at(
            &sheet_path().field("tracks").index(1).field("boxes"),
            &int(20),
        )
        .unwrap();
    globals
        .set_at(&occult.clone().field("dots"), &int(30))
        .unwrap();
    globals
        .set_at(&sheet_path().field("bonus"), &Value::some(int(40)))
        .unwrap();
    globals
        .set_at(
            &sheet_path().field("by id").int_key(-4).field("a"),
            &int(50),
        )
        .unwrap();
    globals
        .set_at(
            &occult.clone().field("specialties"),
            &Value::list([s("a"), s("b"), s("c")]),
        )
        .unwrap();

    assert_eq!(run(&globals, source), int(10 + 20 + 30 + 40 + 50 + 3));

    globals
        .set_at(&occult.field("specialties").index(2), &s("sixty sigils"))
        .unwrap();
    assert_eq!(
        run(&globals, r#"sheet.skills["occult"].specialties[2]"#),
        s("sixty sigils")
    );
}

#[test]
fn type_errors() {
    let mut globals = store();

    // The whole part, named by the setting.
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("rank"),
            &Value::bool(true)
        ),
        ["expected Int, found Bool", "while setting `sheet.rank`"]
    );

    // Inside the part, at the path through it.
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("tracks").index(1),
            &Value::record([
                ("boxes", int(1)),
                ("marks", Value::list([Value::bool(true), int(1)])),
            ])
        ),
        [
            "expected Bool, found Int",
            "at `sheet.tracks[1].marks[1]`",
            "while setting `sheet.tracks[1]`"
        ]
    );

    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("skills").string_key("brawl"),
            &Value::record([("dots", int(1))])
        ),
        [
            "expected {dots: Int, specialties: List<String>}, found a record without `specialties`",
            r#"while setting `sheet.skills["brawl"]`"#
        ]
    );

    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("bonus"),
            &Value::some(s("two"))
        ),
        [
            "expected Option<Int>, found some(String)",
            "while setting `sheet.bonus`"
        ]
    );

    // A key of the wrong type.
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("skills").int_key(1),
            &skill(1, &[])
        ),
        [
            "expected a String key, found Int",
            "at `sheet.skills`",
            "while setting `sheet.skills[1]`"
        ]
    );
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("by id").string_key("x"),
            &Value::none()
        ),
        [
            "expected an Int key, found String",
            "at `sheet[\"by id\"]`",
            r#"while setting `sheet["by id"]["x"]`"#
        ]
    );

    // Steps of the wrong kind.
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("rank").field("max"),
            &int(1)
        ),
        [
            r#"Int has no field "max""#,
            "at `sheet.rank`",
            "while setting `sheet.rank.max`"
        ]
    );
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("tracks").int_key(0),
            &int(1)
        ),
        [
            "List<{boxes: Int, marks: List<Bool>}> has no key 0",
            "at `sheet.tracks`",
            "while setting `sheet.tracks[0]`"
        ]
    );
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("skills").index(0),
            &int(1)
        ),
        [
            "Map<String, {dots: Int, specialties: List<String>}> has no index 0",
            "at `sheet.skills`",
            "while setting `sheet.skills[0]`"
        ]
    );
    assert_eq!(
        error(&mut globals, &Path::value("n").index(0), &int(1)),
        ["Int has no index 0", "at `n`", "while setting `n[0]`"]
    );

    // A field the record does not have.
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("tracks").index(0).field("count"),
            &int(1)
        ),
        [
            r#"{boxes: Int, marks: List<Bool>} has no field "count""#,
            "at `sheet.tracks[0]`",
            "while setting `sheet.tracks[0].count`"
        ]
    );
}

#[test]
fn missing_keys_and_indexes() {
    let mut globals = store();

    assert_eq!(
        error(
            &mut globals,
            &sheet_path()
                .field("skills")
                .string_key("stealth")
                .field("dots"),
            &int(1)
        ),
        [
            r#"the map has no key "stealth""#,
            "at `sheet.skills`",
            r#"while setting `sheet.skills["stealth"].dots`"#
        ]
    );
    assert_eq!(
        error(
            &mut globals,
            &sheet_path().field("by id").int_key(8),
            &Value::none()
        ),
        [
            "the map has no key 8",
            "at `sheet[\"by id\"]`",
            r#"while setting `sheet["by id"][8]`"#
        ]
    );

    // Keys before, between and after every key there is.
    for key in [-5, -3, 0, 8, 10, i64::MIN, i64::MAX] {
        let path = sheet_path().field("by id").int_key(key);
        assert!(globals.set_at(&path, &Value::none()).is_err(), "{key}");
        assert_eq!(globals.get_at(&path), None, "{key}");
    }

    for key in ["", "a", "brawm", "occul", "occultist", "z"] {
        let path = sheet_path().field("skills").string_key(key);
        assert!(globals.set_at(&path, &skill(0, &[])).is_err(), "{key}");
        assert_eq!(globals.get_at(&path), None, "{key}");
    }

    let tracks = sheet_path().field("tracks");

    for index in [2, 3, -1, i64::MIN, i64::MAX] {
        assert_eq!(
            error(&mut globals, &tracks.clone().index(index), &track(0, &[])),
            [
                format!("index {index} is out of range of a list of 2 entries"),
                "at `sheet.tracks`".into(),
                format!("while setting `sheet.tracks[{index}]`"),
            ]
        );
        assert_eq!(globals.get_at(&tracks.clone().index(index)), None);
    }

    assert_eq!(
        error(
            &mut globals,
            &tracks.clone().index(1).field("marks").index(0),
            &Value::bool(true)
        ),
        [
            "index 0 is out of range of a list of 0 entries",
            "at `sheet.tracks[1].marks`",
            "while setting `sheet.tracks[1].marks[0]`"
        ]
    );
}

#[test]
fn paths_through_options() {
    let mut globals = store();
    let by_id = sheet_path().field("by id");

    // `some(none)` and `none` have no field, at either option.
    for key in [7, 9] {
        assert_eq!(
            error(
                &mut globals,
                &by_id.clone().int_key(key).field("a"),
                &int(1)
            ),
            [
                "the optional value is none".to_owned(),
                format!("at `sheet[\"by id\"][{key}]`"),
                format!("while setting `sheet[\"by id\"][{key}].a`"),
            ]
        );
        assert_eq!(globals.get_at(&by_id.clone().int_key(key).field("a")), None);
    }

    // The option itself is set whole.
    globals
        .set_at(&by_id.clone().int_key(7), &Value::some(Value::some(a(6))))
        .unwrap();
    assert_eq!(
        globals.get_at(&by_id.clone().int_key(7).field("a")),
        Some(int(6))
    );

    // Through a value that is itself an option.
    let mut env = Environment::new();
    env.value("bonus", Type::option(Type::record([("dice", Type::int())])))
        .unwrap();
    let mut globals = Globals::new(&env);
    globals.set("bonus", &Value::none()).unwrap();

    let error = globals
        .set_at(&Path::value("bonus").field("dice"), &int(1))
        .unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "the optional value is none: at `bonus`: while setting `bonus.dice`"
    );

    globals
        .set("bonus", &Value::some(Value::record([("dice", int(1))])))
        .unwrap();
    globals
        .set_at(&Path::value("bonus").field("dice"), &int(3))
        .unwrap();
    assert_eq!(
        globals.get("bonus"),
        Some(Value::some(Value::record([("dice", int(3))])))
    );
}

#[test]
fn roots_that_cannot_be_set() {
    let mut globals = store();

    assert_eq!(
        error(&mut globals, &Path::value("unset").field("a"), &int(1)),
        ["`unset` is not set", "while setting `unset.a`"]
    );
    assert_eq!(globals.get_at(&Path::value("unset").field("a")), None);
    assert_eq!(globals.get_at(&Path::value("unset")), None);

    assert_eq!(
        error(&mut globals, &Path::value("nothing").field("a"), &int(1)),
        [
            "no value `nothing` is declared",
            "while setting `nothing.a`"
        ]
    );
    assert_eq!(
        error(&mut globals, &Path::value("nothing"), &int(1)),
        ["no value `nothing` is declared"]
    );
    assert_eq!(globals.get_at(&Path::value("nothing")), None);

    assert_eq!(
        error(&mut globals, &Path::function("roll").field("a"), &int(1)),
        [
            "`roll()` is what a function gives, not a value a store holds",
            "while setting `roll().a`"
        ]
    );
    assert_eq!(
        error(&mut globals, &Path::function("roll"), &a(1)),
        [
            "`roll()` is what a function gives, not a value a store holds",
            "while setting `roll()`"
        ]
    );
    assert_eq!(globals.get_at(&Path::function("roll")), None);
}

/// A part replaced by one of the same size takes the words of the one it
/// replaces, and a larger one frees its words for the next of that size.
#[test]
fn replacing_a_part_frees() {
    let mut globals = store();
    let heap = globals.heap_words();
    let occult = sheet_path().field("skills").string_key("occult");
    let name = sheet_path().field("name");

    for n in 0..20 {
        // Words in place take nothing.
        globals
            .set_at(&sheet_path().field("rank"), &int(n))
            .unwrap();
        // A string of the same words, a list of the same entries, a record
        // of the same parts.
        globals
            .set_at(&name, &Value::from(format!("Ada {n:02}")))
            .unwrap();
        globals
            .set_at(
                &occult.clone().field("specialties"),
                &Value::list([s("a"), s("b")]),
            )
            .unwrap();
        globals
            .set_at(
                &sheet_path().field("tracks").index(0),
                &track(n, &[true, false]),
            )
            .unwrap();
        globals
            .set_at(
                &sheet_path().field("by id").int_key(-4),
                &Value::some(Value::some(a(n))),
            )
            .unwrap();
        assert_eq!(globals.heap_words(), heap, "{n}");
    }

    // A longer name takes new words, and frees its old ones.
    globals
        .set_at(&name, &s("Ada, Countess of Lovelace"))
        .unwrap();
    let longer = globals.heap_words();
    assert_eq!(longer, heap + 4);

    // The freed word is taken by the next part of its size.
    globals
        .set_at(&occult.clone().field("specialties").index(0), &s("one"))
        .unwrap();
    globals.set_at(&name, &s("Ada")).unwrap();
    globals.set_at(&name, &s("Bea")).unwrap();
    assert_eq!(globals.heap_words(), longer);
    assert_eq!(globals.get_at(&name), Some(s("Bea")));

    // Absent parts free theirs.
    globals
        .set_at(&sheet_path().field("by id").int_key(-4), &Value::none())
        .unwrap();
    globals
        .set_at(
            &sheet_path().field("by id").int_key(9),
            &Value::some(Value::some(a(1))),
        )
        .unwrap();
    assert_eq!(globals.heap_words(), longer);
}

/// A part that does not fit leaves the store as it was.
#[test]
fn the_limit_is_kept() {
    let mut env = Environment::new();
    env.value("names", Type::list(Type::string())).unwrap();

    // Two entries, and two strings of a word.
    let mut globals = Globals::with_limit(&env, 4);
    globals
        .set("names", &Value::list([s("Ada"), s("Bea")]))
        .unwrap();

    let error = error_of(
        &mut globals,
        &Path::value("names").index(1),
        &s("Beatrice B"),
    );
    assert_eq!(
        error,
        [
            "the value needs 2 words of memory, 0 are left",
            "while setting `names[1]`"
        ]
    );
    assert_eq!(
        globals.get("names"),
        Some(Value::list([s("Ada"), s("Bea")]))
    );

    // The same size fits, in the words it replaces.
    globals
        .set_at(&Path::value("names").index(1), &s("Beatrice"))
        .unwrap();
    assert_eq!(
        globals.get("names"),
        Some(Value::list([s("Ada"), s("Beatrice")]))
    );
}

/// A part too long for a reference is refused before `set_at` frees what
/// it replaces: the store's every word, free list and flag are as they
/// were. The longest is lowered for the test, as 2^32 would take
/// gigabytes.
#[test]
fn too_long_parts_leave_the_store_untouched() {
    let mut globals = store();
    // Some free blocks, which a set would take.
    globals
        .set_at(&sheet_path().field("name"), &s("Ada Lovelace"))
        .unwrap();
    globals
        .set_at(&sheet_path().field("name"), &s("Ada"))
        .unwrap();

    let sheet = globals.get("sheet");
    crate::memory::MAX_LEN.with(|max| max.set(4));
    let before = format!("{globals:?}");
    let bools = |n: usize| Value::list(vec![Value::bool(true); n]);
    let occult = sheet_path().field("skills").string_key("occult");

    let refused = [
        (sheet_path().field("name"), s("12345")),
        (
            sheet_path().field("tracks").index(0).field("marks"),
            bools(5),
        ),
        (sheet_path().field("tracks").index(1), track(1, &[true; 5])),
        (
            occult.clone().field("specialties"),
            Value::list([s("a"), s("b"), s("c"), s("d"), s("e")]),
        ),
        (occult.clone(), skill(1, &["12345"])),
        (
            sheet_path().field("tracks"),
            Value::list(vec![track(1, &[]); 5]),
        ),
    ];

    for (path, value) in &refused {
        let error = error_of(&mut globals, path, value);
        assert_eq!(
            error.first().map(String::as_str),
            Some(
                "the value holds a string, list or map 5 long, longer than the 4 a value may hold"
            ),
            "{path} = {value}"
        );
        assert_eq!(format!("{globals:?}"), before, "{path} = {value}");
    }

    assert_eq!(globals.get("sheet"), sheet);

    // At the longest, a part still sets.
    globals
        .set_at(
            &sheet_path().field("tracks").index(0).field("marks"),
            &bools(4),
        )
        .unwrap();
    assert_eq!(run(&globals, "sheet.tracks[0].marks.len()"), int(4));
}

fn error_of(globals: &mut Globals, path: &Path, value: &Value) -> Vec<String> {
    let error = globals.set_at(path, value).unwrap_err();
    error.labels().map(|l| l.message().to_string()).collect()
}

/// Random updates of random entries of a map of lists, checked against
/// the same updates of the value.
#[test]
fn random_updates_match_the_value() {
    let mut env = Environment::new();
    env.value(
        "m",
        Type::map(Type::int(), Type::list(Type::option(Type::string()))),
    )
    .unwrap();

    let mut rng = crate::tests::Rng(0x5e7a7);
    let keys: Vec<i64> = (0..40).map(|n| n * 3 - 50).collect();
    let entry = |rng: &mut crate::tests::Rng| {
        let len = rng.below(4);
        (0..len)
            .map(|_| match rng.below(3) {
                0 => None,
                _ => Some("x".repeat(rng.below(20))),
            })
            .collect::<Vec<_>>()
    };

    let mut model: Vec<(i64, Vec<Option<String>>)> =
        keys.iter().map(|k| (*k, entry(&mut rng))).collect();
    let value = |model: &[(i64, Vec<Option<String>>)]| {
        Value::map(model.iter().map(|(k, v)| {
            let list = v.iter().map(|s| match s {
                Some(s) => Value::some(Value::string(s.as_str())),
                None => Value::none(),
            });
            (int(*k), Value::list(list))
        }))
    };

    let mut globals = Globals::new(&env);
    globals.set("m", &value(&model)).unwrap();

    for _ in 0..500 {
        let n = rng.below(model.len());
        let key = model[n].0;
        let path = Path::value("m").int_key(key);

        if rng.below(2) == 0 || model[n].1.is_empty() {
            let new = entry(&mut rng);
            model[n].1 = new;
            let list = model[n].1.iter().map(|s| match s {
                Some(s) => Value::some(Value::string(s.as_str())),
                None => Value::none(),
            });
            globals.set_at(&path, &Value::list(list)).unwrap();
        } else {
            let i = rng.below(model[n].1.len());
            let new = (rng.below(2) == 0).then(|| "y".repeat(rng.below(20)));
            let part = match &new {
                Some(s) => Value::some(Value::string(s.as_str())),
                None => Value::none(),
            };
            model[n].1[i] = new;
            globals
                .set_at(&path.index(i64::try_from(i).unwrap()), &part)
                .unwrap();
        }
    }

    assert_eq!(globals.get("m"), Some(value(&model)));
}
