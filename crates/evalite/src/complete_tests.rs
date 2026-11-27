#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::complete::{CompleteKind, Insert};
use crate::error::ErrorKind;
use crate::structure::{EditKind, Repr, StructureKind};
use crate::types::Kind;
use crate::*;

fn env() -> Environment {
    let mut env = Environment::new();
    env.value("wits", Type::int()).unwrap();
    env.value("dead", Type::bool()).unwrap();
    env.value("bonus", Type::option(Type::int())).unwrap();
    env.value("dots", Type::list(Type::int())).unwrap();
    env.value("health", crate::check_tests::track()).unwrap();
    env.value("table", Type::string()).unwrap();
    env.value(
        "m",
        Type::record([
            ("athletics", Type::int()),
            ("Brawl (bar)", Type::int()),
            ("brawl", Type::option(Type::int())),
            ("flags", Type::record([("frenzy", Type::bool())])),
        ]),
    )
    .unwrap();
    env.function("count", [crate::check_tests::track()], Type::int())
        .unwrap();
    env.function(
        "count",
        [crate::check_tests::track(), Type::int()],
        Type::int(),
    )
    .unwrap();
    env.function("chart", [Type::int(), Type::string()], Type::int())
        .unwrap();
    env.function("broken", [Type::int()], crate::check_tests::track())
        .unwrap();
    env
}

fn s(text: &str) -> Structure {
    Structure::from_text(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

/// The labels of the completions at hole `n` of `text`.
fn labels(text: &str, n: usize, root: Option<&Type>) -> Vec<String> {
    let s = s(text);
    let hole = s.holes()[n];
    s.complete(&env(), hole, root)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"))
        .into_iter()
        .map(|c| c.label)
        .collect()
}

/// What fits an `Int`'s place in [`env`].
const INT: &[&str] = &[
    "bonus!",
    "bonus.unwrap_or(_)",
    "dots.len()",
    "dots[_]",
    "health.boxes",
    "health.count.len()",
    "health.count[_]",
    r#"m["Brawl (bar)"]"#,
    "m.athletics",
    "m.brawl!",
    "m.brawl.unwrap_or(_)",
    "wits",
    "chart(_, _)",
    "count(_)",
    "count(_, _)",
    "Int::max(_, _)",
    "Int::min(_, _)",
    "0",
    "_ + _",
    "_ - _",
    "_ * _",
    "_ / _",
    "_ % _",
    "-_",
    "_!",
];

/// What fits a `Bool`'s place in [`env`].
const BOOL: &[&str] = &[
    "bonus.is_none()",
    "bonus.is_some()",
    "dead",
    "m.brawl.is_none()",
    "m.brawl.is_some()",
    "m.flags.frenzy",
    "true",
    "false",
    "_ == _",
    "_ != _",
    "_ < _",
    "_ <= _",
    "_ > _",
    "_ >= _",
    "_ && _",
    "_ || _",
    "!_",
    "_!",
];

/// What fits anywhere in [`env`].
const ANY: &[&str] = &[
    "bonus",
    "dead",
    "dots",
    "health",
    "m",
    "table",
    "wits",
    "broken(_)",
    "chart(_, _)",
    "count(_)",
    "count(_, _)",
    "Int::max(_, _)",
    "Int::min(_, _)",
    "0",
    "true",
    "false",
    r#""""#,
    "[_]",
    "{_: _}",
    "_ + _",
    "_ - _",
    "_ * _",
    "_ / _",
    "_ % _",
    "-_",
    "_ == _",
    "_ != _",
    "_ < _",
    "_ <= _",
    "_ > _",
    "_ >= _",
    "_ && _",
    "_ || _",
    "!_",
    "_!",
];

/// Text, which of its holes, the type the whole expression must have, and
/// what fits there.
fn cases() -> Vec<(&'static str, usize, Option<Type>, Vec<&'static str>)> {
    vec![
        // Value holes, by the type the expression must have.
        ("_", 0, None, ANY.to_vec()),
        ("_", 0, Some(Type::int()), INT.to_vec()),
        ("_", 0, Some(Type::bool()), BOOL.to_vec()),
        // What `!` unwraps is an option, of the type its place takes.
        (
            "1 + _!",
            0,
            None,
            vec![
                "bonus",
                "dots.get(_)",
                "health.count.get(_)",
                "m.brawl",
                "_!",
            ],
        ),
        (
            "_!",
            0,
            None,
            vec![
                "bonus",
                "dots.get(_)",
                "health.count.get(_)",
                "m.brawl",
                "_!",
            ],
        ),
        ("_!", 0, Some(Type::bool()), vec!["_!"]),
        // An option fits where one is wanted.
        (
            "_",
            0,
            Some(Type::option(Type::int())),
            vec![
                "bonus",
                "dots.get(_)",
                "health.count.get(_)",
                "m.brawl",
                "_!",
            ],
        ),
        ("_", 0, Some(Type::string()), vec!["table", r#""""#, "_!"]),
        (
            "_",
            0,
            Some(Type::list(Type::int())),
            vec!["dots", "health.count", "[_]", "_!"],
        ),
        ("_", 0, Some(Type::list(Type::bool())), vec!["[_]", "_!"]),
        (
            "_",
            0,
            Some(crate::check_tests::track()),
            vec!["health", "broken(_)", "_!"],
        ),
        (
            "_",
            0,
            Some(Type::record([("nothing", Type::bool())])),
            vec!["_!"],
        ),
        // Operands.
        ("1 + _", 0, None, INT.to_vec()),
        ("_ < 1", 0, None, INT.to_vec()),
        ("-_", 0, None, INT.to_vec()),
        ("!_", 0, None, BOOL.to_vec()),
        ("_ && dead", 0, None, BOOL.to_vec()),
        ("_ == dead", 0, None, BOOL.to_vec()),
        ("wits != _", 0, None, INT.to_vec()),
        ("_ == _", 0, None, int_or_bool_sorted()),
        // Call arguments, by the overloads the other arguments leave.
        ("count(_)", 0, None, vec!["health", "broken(_)", "_!"]),
        ("count(health, _)", 0, None, INT.to_vec()),
        ("count(_, 1)", 0, None, vec!["health", "broken(_)", "_!"]),
        ("count(_, _, _)", 0, None, vec![]),
        ("Int::max(_)", 0, None, INT.to_vec()),
        ("Int::min(1, _, 3)", 0, None, INT.to_vec()),
        ("Int::min(dead, _)", 0, None, vec![]),
        ("chart(_, _)", 0, None, INT.to_vec()),
        ("chart(_, _)", 1, None, vec!["table", r#""""#, "_!"]),
        ("chart(1, _)", 0, None, vec!["table", r#""""#, "_!"]),
        ("chart(_, table)", 0, None, INT.to_vec()),
        ("unknown(_)", 0, None, ANY.to_vec()),
        // Method arguments, by the receiver's type.
        ("bonus.unwrap_or(_)", 0, None, INT.to_vec()),
        ("m.flags.frenzy.unwrap_or(_)", 0, None, ANY.to_vec()),
        ("wits.clamp(_, 3)", 0, None, INT.to_vec()),
        ("wits.clamp(1, _)", 0, None, INT.to_vec()),
        ("wits.abs(_)", 0, None, vec![]),
        // Receivers, by the member they have.
        (
            "_.unwrap_or(1)",
            0,
            None,
            vec![
                "bonus",
                "dots.get(_)",
                "health.count.get(_)",
                "m.brawl",
                "_!",
            ],
        ),
        ("_.len()", 0, None, vec!["dots", "health.count", "_!"]),
        ("_.abs()", 0, None, INT.to_vec()),
        ("_.frenzy", 0, None, vec!["m.flags", "_!"]),
        ("_.nothing", 0, None, vec!["_!"]),
        // Indexes.
        ("dots[_]", 0, None, INT.to_vec()),
        (
            "m[_]",
            0,
            None,
            vec![
                r#""Brawl (bar)""#,
                r#""athletics""#,
                r#""brawl""#,
                r#""flags""#,
            ],
        ),
        (
            "_[1]",
            0,
            None,
            vec!["dots", "health.count", "[_]", "{_: _}", "_!"],
        ),
        (
            "_[_]",
            0,
            None,
            vec!["dots", "health", "m", "broken(_)", "[_]", "{_: _}", "_!"],
        ),
        (r#"_["Brawl (bar)"]"#, 0, None, vec!["m", "{_: _}", "_!"]),
        // List entries, by a sibling or the list's own type.
        ("[1, _]", 0, None, INT.to_vec()),
        ("[_, dead]", 0, None, BOOL.to_vec()),
        ("[_]", 0, Some(Type::list(Type::int())), INT.to_vec()),
        (
            "[[_]]",
            0,
            Some(Type::list(Type::list(Type::bool()))),
            BOOL.to_vec(),
        ),
        ("[_]", 0, Some(Type::int()), vec![]),
        ("[_]", 0, None, ANY.to_vec()),
        ("[_].get(1)", 0, None, ANY.to_vec()),
        ("count(health, [_][0])", 0, None, INT.to_vec()),
        (
            "_[0]",
            0,
            Some(Type::int()),
            vec!["dots", "health.count", "[_]", "{_: _}", "_!"],
        ),
        ("_[0]", 0, Some(Type::bool()), vec!["[_]", "{_: _}", "_!"]),
        ("[_][0]", 0, Some(Type::bool()), BOOL.to_vec()),
        // Map values, by a sibling or the map's own type; keys are literals
        // of a sibling key's type.
        ("{1: _}", 0, None, ANY.to_vec()),
        ("{1: dead, 2: _}", 0, None, BOOL.to_vec()),
        (
            "{1: _}",
            0,
            Some(Type::map(Type::int(), Type::int())),
            INT.to_vec(),
        ),
        (
            "[{1: _}]",
            0,
            Some(Type::list(Type::map(Type::string(), Type::bool()))),
            BOOL.to_vec(),
        ),
        ("{1: _}.len()", 0, None, ANY.to_vec()),
        ("{1: _}", 0, Some(Type::int()), vec![]),
        ("{_: 1}", 0, None, vec!["0", r#""""#]),
        ("{1: 2, _: 3}", 0, None, vec!["0"]),
        (r#"{"a": 2, _: 3}"#, 0, None, vec![r#""""#]),
        (
            "{_: dead}",
            0,
            Some(Type::map(Type::string(), Type::bool())),
            vec![r#""""#],
        ),
        (
            "_",
            0,
            Some(Type::map(Type::int(), Type::bool())),
            vec!["{_: _}", "_!"],
        ),
        // A record literal's values, by its field of the record its place
        // takes, not by a sibling; its keys are names, not literals.
        ("{a: 1, b: _}", 0, None, ANY.to_vec()),
        (
            "{b: 1, a: _}",
            0,
            Some(Type::record([("a", Type::bool()), ("b", Type::int())])),
            BOOL.to_vec(),
        ),
        ("count({boxes: _, count: [1]})", 0, None, INT.to_vec()),
        ("{a: 1, b: _}", 0, Some(Type::int()), vec![]),
        // Member holes, by the receiver's type and what the place takes.
        (
            "bonus._",
            0,
            None,
            vec!["is_none()", "is_some()", "unwrap_or(_)"],
        ),
        ("bonus._", 0, Some(Type::int()), vec!["unwrap_or(_)"]),
        ("1 + bonus._", 0, None, vec!["unwrap_or(_)"]),
        (
            "bonus._()",
            0,
            Some(Type::bool()),
            vec!["is_none()", "is_some()"],
        ),
        (
            "wits._",
            0,
            None,
            vec!["abs()", "clamp(_, _)", "max(_)", "min(_)"],
        ),
        ("wits._", 0, Some(Type::bool()), vec![]),
        ("dots._", 0, Some(Type::int()), vec!["get(_)", "len()"]),
        ("dead._", 0, None, vec![]),
        (
            "m._",
            0,
            None,
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl", "flags"],
        ),
        (
            "m._",
            0,
            Some(Type::int()),
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl"],
        ),
        ("m._", 0, Some(Type::bool()), vec!["brawl", "flags"]),
        ("m.flags._", 0, None, vec!["frenzy"]),
        (
            "1 + m._",
            0,
            None,
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl"],
        ),
        // Holes are listed outermost first: the inner `._` is the second.
        (
            "Int::max(m._._, 1)",
            1,
            None,
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl", "flags"],
        ),
    ]
}

/// [`INT`] and [`BOOL`] together, in [`ANY`]'s order.
fn int_or_bool_sorted() -> Vec<&'static str> {
    ANY.iter()
        .copied()
        .flat_map(|a| match a {
            // Paths and methods that are not in `ANY`, which lists only
            // values, go where their value is.
            "bonus" => vec![
                "bonus!",
                "bonus.is_none()",
                "bonus.is_some()",
                "bonus.unwrap_or(_)",
            ],
            "dots" => vec!["dots.len()", "dots[_]"],
            "health" => vec!["health.boxes", "health.count.len()", "health.count[_]"],
            "m" => vec![
                r#"m["Brawl (bar)"]"#,
                "m.athletics",
                "m.brawl!",
                "m.brawl.is_none()",
                "m.brawl.is_some()",
                "m.brawl.unwrap_or(_)",
                "m.flags.frenzy",
            ],
            "table" | "broken(_)" | r#""""# | "[_]" | "{_: _}" => vec![],
            a => vec![a],
        })
        .collect()
}

#[test]
fn completions_by_place_and_type() {
    for (text, n, root, expected) in cases() {
        assert_eq!(
            labels(text, n, root.as_ref()),
            expected,
            "{text:?} hole {n}, root {root:?}"
        );
    }
}

#[test]
fn completion_kinds_and_types() {
    let s = s("_");
    let hole = s.root();
    let all = s.complete(&env(), hole, Some(&Type::int())).unwrap();
    let find = |label: &str| all.iter().find(|c| c.label == label).unwrap();

    assert_eq!(find("wits").kind, CompletionKind::Value);
    assert_eq!(find("wits").ty, Some(Type::int()));
    assert_eq!(find("m.athletics").kind, CompletionKind::Value);
    assert_eq!(find("bonus.unwrap_or(_)").kind, CompletionKind::Method);
    assert_eq!(find("bonus.unwrap_or(_)").ty, Some(Type::int()));
    assert_eq!(find("count(_)").kind, CompletionKind::Function);
    assert_eq!(find("count(_)").ty, Some(Type::int()));
    assert_eq!(find("chart(_, _)").ty, Some(Type::int()));
    assert_eq!(
        find("chart(_, _)").signature(),
        Some("chart(Int, String) -> Int")
    );
    assert_eq!(find("0").kind, CompletionKind::Literal);
    assert_eq!(find("_ + _").kind, CompletionKind::Operator);

    let s = self::s("m._");
    let hole = s.holes()[0];
    let all = s.complete(&env(), hole, None).unwrap();
    let brawl = all
        .iter()
        .find(|c| c.label == r#"["Brawl (bar)"]"#)
        .unwrap();
    assert_eq!(brawl.kind, CompletionKind::Field);
    assert_eq!(brawl.insert, Insert::Field("Brawl (bar)".into()));

    let s = self::s("wits._");
    let all = s.complete(&env(), s.holes()[0], None).unwrap();
    let clamp = all.iter().find(|c| c.label == "clamp(_, _)").unwrap();
    assert_eq!(clamp.kind, CompletionKind::Method);
    assert_eq!(
        clamp.insert,
        Insert::Method {
            name: "clamp".into(),
            args: 2
        }
    );
}

/// Cases nothing in their hole makes compile, since what holds it is
/// wrong already.
const NO_PROGRAM: &[&str] = &["unknown(_)", "m.flags.frenzy.unwrap_or(_)"];

/// Applying each completion gives the text it shows, and one that
/// leaves no hole compiles, to the type it said.
#[test]
fn applying_completions() {
    let env = env();
    let mut applied = 0;
    let mut compiled = 0;

    for (text, n, root, _) in cases() {
        let base = s(text);
        let hole = base.holes()[n];

        for completion in base.complete(&env, hole, root.as_ref()).unwrap() {
            let mut s = base.clone();
            s.apply(hole, &completion).unwrap();
            applied += 1;

            let label = match &completion.insert {
                Insert::Expr(_) => completion.label.clone(),
                _ => format!(".{}", completion.label).replace(".[", "["),
            };

            let before = base.text_of(hole);
            let after = s.text_of(hole);
            assert!(
                after.ends_with(&label) || after == label,
                "{text:?}: {before:?} with {label:?} gave {after:?}"
            );

            assert_eq!(Structure::from_text(&s.to_text()).unwrap(), s);

            // A field that only leads to what fits, such as an option to
            // unwrap, takes another step before it compiles.
            let leads_on = matches!(completion.insert, Insert::Field(_))
                && matches!(
                    completion.ty,
                    Some(Type(Kind::Option(_)) | Type(Kind::Record(_)))
                );

            if !s.holes().is_empty() || leads_on || NO_PROGRAM.contains(&text) {
                continue;
            }

            let program = s.compile(&env).unwrap_or_else(|e| {
                panic!(
                    "{text:?} with {:?}: {} does not compile: {e}",
                    completion.label, s
                )
            });
            compiled += 1;

            if let Some(root) = &root
                && completion.ty.as_ref() == Some(root)
                && s.root() == hole
            {
                assert_eq!(&program.result_type(), root, "{s}");
            }

            if let (Some(ty), Insert::Expr(_)) = (&completion.ty, &completion.insert) {
                assert_eq!(&s.compile_at(hole, &env).unwrap().result_type(), ty, "{s}");
            }
        }
    }

    assert!(applied > 500, "{applied}");
    assert!(compiled > 50, "{compiled}");
}

#[test]
fn applying_members() {
    let env = env();

    let mut s = s("m._ + 1");
    let hole = s.holes()[0];
    s.apply(hole, &field("Brawl (bar)")).unwrap();
    assert_eq!(s.to_text(), r#"m["Brawl (bar)"] + 1"#);
    assert_eq!(s.compile(&env).unwrap().result_type(), Type::int());

    let mut s = self::s("m._ + 1");
    let hole = s.holes()[0];
    s.apply(hole, &field("athletics")).unwrap();
    assert_eq!(s.to_text(), "m.athletics + 1");

    // A method keeps the hole's arguments when it takes as many.
    let mut s = self::s("bonus._(0)");
    let hole = s.holes()[0];
    s.apply(hole, &method("unwrap_or", 1)).unwrap();
    assert_eq!(s.to_text(), "bonus.unwrap_or(0)");

    let mut s = self::s("bonus._(0)");
    let hole = s.holes()[0];
    s.apply(hole, &method("is_some", 0)).unwrap();
    assert_eq!(s.to_text(), "bonus.is_some()");

    let mut s = self::s("wits._");
    let hole = s.holes()[0];
    s.apply(hole, &method("clamp", 2)).unwrap();
    assert_eq!(s.to_text(), "wits.clamp(_, _)");
    assert_eq!(s.holes().len(), 2);

    // A method's name in place of a field's turns it into a method, and
    // back.
    let mut s = self::s("wits._()");
    let hole = s.holes()[0];
    s.apply(hole, &field("x")).unwrap();
    assert_eq!(s.to_text(), "wits.x");

    // Members only name members.
    let mut s = self::s("_ + 1");
    let hole = s.holes()[0];
    assert_eq!(
        s.apply(hole, &field("x")),
        Err(EditKind::NotAMember(hole).into())
    );
    assert_eq!(
        s.apply(hole, &method("abs", 0)),
        Err(EditKind::NotAMember(hole).into())
    );
    assert_eq!(s.to_text(), "_ + 1");

    // An expression may replace any node, hole or not.
    let mut s = self::s("wits + 1");
    let one = s.post_order()[1];
    let completion = s
        .complete(&env, one, None)
        .unwrap()
        .into_iter()
        .find(|c| c.label == "bonus.unwrap_or(_)")
        .unwrap();
    s.apply(one, &completion).unwrap();
    assert_eq!(s.to_text(), "wits + bonus.unwrap_or(_)");
}

fn field(name: &str) -> Completion {
    Completion {
        label: name.into(),
        kind: CompletionKind::Field,
        ty: None,
        signature: None,
        insert: Insert::Field(name.into()),
    }
}

fn method(name: &str, args: usize) -> Completion {
    Completion {
        label: name.into(),
        kind: CompletionKind::Method,
        ty: None,
        signature: None,
        insert: Insert::Method {
            name: name.into(),
            args,
        },
    }
}

/// Each expression offered builds what its label parses to.
fn assert_parsed(env: &Environment, text: &str, n: usize, root: Option<&Type>) -> usize {
    let s = s(text);
    let hole = s.holes()[n];
    let mut checked = 0;

    for c in s.complete(env, hole, root).unwrap() {
        let Insert::Expr(_) = &c.insert else {
            continue;
        };

        let mut built = Structure::new();
        built.apply(built.root(), &c).unwrap();

        let parsed = Structure::from_text(&c.label)
            .unwrap_or_else(|e| panic!("{text:?}: {:?} does not parse: {e}", c.label));
        assert_eq!(built, parsed, "{text:?}: {:?}", c.label);
        assert_eq!(built.to_text(), c.label, "{text:?}");
        checked += 1;
    }

    checked
}

#[test]
fn completions_are_their_labels() {
    let env = env();
    let mut checked = 0;

    for (text, n, root, _) in cases() {
        checked += assert_parsed(&env, text, n, root.as_ref());
    }

    assert!(checked > 500, "{checked}");

    // Names read as something else, and fields not written as names.
    let mut env = Environment::new();

    for name in ["_", "true", "false"] {
        env.value(name, Type::option(Type::list(Type::int())))
            .unwrap();
        env.function(name, [Type::int()], Type::int()).unwrap();
    }

    let fields = Type::record([
        ("Foo", Type::int()),
        ("x y", Type::int()),
        ("true", Type::int()),
        ("_", Type::list(Type::int())),
        ("q\"\\", Type::option(Type::int())),
    ]);
    env.value("r", Type::record([("inner", fields)])).unwrap();

    let mut checked = 0;

    for root in [None, Some(Type::int()), Some(Type::bool())] {
        checked += assert_parsed(&env, "_", 0, root.as_ref());
    }

    checked += assert_parsed(&env, r#"r.inner[_]"#, 0, None);
    assert!(checked > 50, "{checked}");

    let all = s("_").complete(&env, s("_").root(), None).unwrap();
    let labels = all.iter().map(|c| c.label.as_str()).collect::<Vec<_>>();
    assert!(labels.contains(&"_(_)"), "{labels:?}");
    assert!(!labels.contains(&"true(_)"), "{labels:?}");
}

/// What is too long to parse back is not offered.
#[test]
fn completions_parse_back() {
    let fits = "x".repeat(DEFAULT_MAX_LEN - "long.".len());
    let over = "y".repeat(DEFAULT_MAX_LEN - "long.".len() + 1);

    let mut env = Environment::new();
    let ty = Type::record([(fits.as_str(), Type::int()), (over.as_str(), Type::int())]);
    env.value("long", ty).unwrap();

    let s = s("_");
    let all = s.complete(&env, s.root(), Some(&Type::int())).unwrap();
    let paths = all
        .iter()
        .filter(|c| c.kind == CompletionKind::Value)
        .map(|c| c.label.len())
        .collect::<Vec<_>>();
    assert_eq!(paths, [DEFAULT_MAX_LEN]);
}

/// A form fills a structure one hole at a time, each with a completion.
#[test]
fn filling_a_form() {
    let env = env();
    let mut s = Structure::new();

    let pick = |s: &mut Structure, root: Option<&Type>, label: &str| {
        let hole = s.holes()[0];
        let all = s.complete(&env, hole, root).unwrap();
        let c = all
            .iter()
            .find(|c| c.label == label)
            .unwrap_or_else(|| panic!("{label:?} is not offered at {s}: {all:?}"));
        s.apply(hole, c).unwrap();
    };

    let int = Some(&Type::int());
    pick(&mut s, int, "_ + _");
    pick(&mut s, int, "m.brawl.unwrap_or(_)");
    pick(&mut s, int, "0");
    pick(&mut s, int, "count(_, _)");
    pick(&mut s, int, "health");
    pick(&mut s, int, "chart(_, _)");
    pick(&mut s, int, "wits");
    pick(&mut s, int, r#""""#);

    assert_eq!(
        s.to_text(),
        r#"m.brawl.unwrap_or(0) + count(health, chart(wits, ""))"#
    );
    assert!(s.holes().is_empty());
    assert_eq!(s.compile(&env).unwrap().result_type(), Type::int());
}

#[test]
fn completion_errors() {
    let env = env();
    let s = s("_._");
    let member = s.holes()[0];

    let Err(error) = s.complete(&env, member, None) else {
        panic!("a receiver with a hole has no known members");
    };
    let CompleteKind::Receiver(error) = error.kind() else {
        panic!("{error:?}");
    };
    assert_eq!(error.first().unwrap().kind(), &StructureKind::Hole);
    assert_eq!(error.first().unwrap().node(), s.holes()[1]);

    let s = self::s("nothing._");
    let Err(error) = s.complete(&env, s.holes()[0], None) else {
        panic!("an unknown receiver has no known members");
    };
    assert_eq!(error.to_string(), "the receiver's type is not known");
    assert_eq!(
        format!("{error:#}"),
        "the receiver's type is not known: unknown name `nothing`"
    );
    let CompleteKind::Receiver(error) = error.kind() else {
        panic!("{error:?}");
    };
    assert_eq!(error.to_string(), "unknown name `nothing`");
    let labels: Vec<_> = error
        .first()
        .unwrap()
        .labels()
        .map(|l| l.message().to_string())
        .collect();
    assert_eq!(labels, ["unknown name `nothing`"]);

    let unknown = crate::structure::test_id(99);
    assert_eq!(
        s.complete(&env, unknown, None),
        Err(CompleteKind::UnknownNode(unknown).into())
    );
}

/// Each method of the shared table compiles on its receiver, with
/// arguments of its parameters' types, to its result's type.
#[test]
fn method_table_matches_the_checker() {
    let mut env = Environment::new();

    let receivers = [
        Type::int(),
        Type::option(Type::int()),
        Type::option(Type::bool()),
        Type::list(Type::bool()),
    ];

    for (n, ty) in receivers.iter().enumerate() {
        env.value(&format!("r{n}"), ty.clone()).unwrap();
    }

    env.value("i", Type::int()).unwrap();
    env.value("b", Type::bool()).unwrap();

    let mut checked = std::collections::HashSet::new();

    for (n, ty) in receivers.iter().enumerate() {
        let names = crate::methods::method_names(ty);

        for sig in crate::methods::methods_of(ty) {
            assert!(names.contains(&sig.name));
            let (params, ret) = sig.signature(ty);

            let args = params
                .iter()
                .map(|p| match p {
                    Type(Kind::Int) => "i",
                    Type(Kind::Bool) => "b",
                    other => panic!("no argument of type {other}"),
                })
                .collect::<Vec<_>>()
                .join(", ");

            let text = format!("r{n}.{}({args})", sig.name);
            let program = compile(&text, &env).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(program.result_type(), ret, "{text}");
            checked.insert(sig.name);

            // One argument too many is refused, as the table says.
            let extra = if args.is_empty() {
                "i".to_owned()
            } else {
                format!("{args}, i")
            };
            let error = compile(&format!("r{n}.{}({extra})", sig.name), &env).unwrap_err();
            assert!(
                error
                    .iter()
                    .any(|e| matches!(e.kind(), ErrorKind::ArgumentCount { .. })),
                "{text}: {error}"
            );
        }
    }

    for sig in crate::methods::METHODS {
        assert!(checked.contains(sig.name), "{} is not checked", sig.name);
    }
}

/// Finding a hole's type walks up without recursing, however deep.
#[test]
fn deep_structures() {
    let env = env();
    let mut s = Structure::new();
    let first = s.add(Node(Repr::Hole)).unwrap();
    let mut inner = first;

    for _ in 0..100_000 {
        inner = s
            .add(Node(Repr::List {
                entries: vec![inner],
            }))
            .unwrap();
    }

    let index = s.add(Node(Repr::Int(0))).unwrap();
    let root = s.root();
    s.replace(
        root,
        Node(Repr::Index {
            receiver: inner,
            index,
        }),
    )
    .unwrap();

    let labels = s
        .complete(&env, first, None)
        .unwrap()
        .into_iter()
        .map(|c| c.label)
        .collect::<Vec<_>>();
    assert_eq!(labels, ANY);

    let labels = s
        .complete(&env, first, Some(&Type::int()))
        .unwrap()
        .into_iter()
        .map(|c| c.label)
        .collect::<Vec<_>>();
    assert!(labels.is_empty(), "{labels:?}");
}

#[test]
fn hexadecimal_integers_add_no_candidates() {
    // A hexadecimal integer fills a place as a decimal one does, and is
    // offered nowhere.
    for (hex, decimal) in [
        ("0x1F + _", "31 + _"),
        ("_ + 0xff", "_ + 255"),
        ("0x10._()", "16._()"),
        ("[0x1, _]", "[1, _]"),
    ] {
        assert_eq!(labels(hex, 0, None), labels(decimal, 0, None), "{hex}");
    }

    let all = labels("_", 0, Some(&Type::int()));
    assert!(all.iter().all(|label| !label.starts_with("0x")), "{all:?}");
}

/// A map's members are `get` and `len`, its values are reached by `[_]` as
/// a list's entries are, and its index takes its key type.
#[test]
fn map_completions() {
    let mut env = Environment::new();
    let dice = Type::record([("dice", Type::int())]);
    env.value("ids", Type::map(Type::int(), dice.clone()))
        .unwrap();
    env.value("tags", Type::map(Type::string(), Type::bool()))
        .unwrap();
    env.value("wits", Type::int()).unwrap();
    env.value("name", Type::string()).unwrap();

    let labels = |text: &str, root: Option<&Type>| -> Vec<String> {
        let s = s(text);
        let hole = s.holes()[0];
        s.complete(&env, hole, root)
            .unwrap_or_else(|e| panic!("{text:?}: {e}"))
            .into_iter()
            .map(|c| c.label)
            .collect()
    };

    let members = labels("ids._", None);
    assert_eq!(members, ["get(_)", "len()"], "ids._");
    assert_eq!(labels("tags._", None), ["get(_)", "len()"], "tags._");

    let int = labels("_", Some(&Type::int()));
    for label in ["ids.len()", "ids[_].dice", "tags.len()", "wits"] {
        assert!(int.contains(&label.to_owned()), "{label} in {int:?}");
    }

    let bool = labels("_", Some(&Type::bool()));
    assert!(bool.contains(&"tags[_]".to_owned()), "{bool:?}");

    let option = labels("_", Some(&Type::option(dice.clone())));
    assert!(option.contains(&"ids.get(_)".to_owned()), "{option:?}");

    // A map is offered as the receiver of an index already written, by
    // the key's type: an `Int` takes lists and `Int` maps, a `String` only
    // `String` maps (and records with the field).
    let receivers = labels("_[1]", None);
    assert!(receivers.contains(&"ids".to_owned()), "{receivers:?}");
    assert!(!receivers.contains(&"tags".to_owned()), "{receivers:?}");
    let receivers = labels(r#"_["x"]"#, None);
    assert!(receivers.contains(&"tags".to_owned()), "{receivers:?}");
    assert!(!receivers.contains(&"ids".to_owned()), "{receivers:?}");
    let receivers = labels("_[wits]", None);
    assert!(receivers.contains(&"ids".to_owned()), "{receivers:?}");
    let receivers = labels("_[name]", None);
    assert!(receivers.contains(&"tags".to_owned()), "{receivers:?}");
    assert!(!receivers.contains(&"ids".to_owned()), "{receivers:?}");

    // The index of a map takes its key type.
    let key = labels("ids[_]", None);
    assert!(
        key.contains(&"wits".to_owned()) && !key.contains(&"name".to_owned()),
        "{key:?}"
    );
    let key = labels("tags[_]", None);
    assert!(
        key.contains(&"name".to_owned()) && !key.contains(&"wits".to_owned()),
        "{key:?}"
    );
    let key = labels("ids.get(_)", None);
    assert!(
        key.contains(&"wits".to_owned()) && !key.contains(&"name".to_owned()),
        "{key:?}"
    );

    // A map is indexed as a list is.
    let receivers = labels("_[_]", None);
    assert!(
        receivers.contains(&"ids".to_owned()) && receivers.contains(&"tags".to_owned()),
        "{receivers:?}"
    );
    let receivers = labels("_.len()", None);
    assert!(
        receivers.contains(&"ids".to_owned()) && receivers.contains(&"tags".to_owned()),
        "{receivers:?}"
    );
}

/// The environment of the map index tests.
fn keyed_env() -> Environment {
    let mut env = Environment::new();
    let dice = Type::record([("dice", Type::int())]);
    env.value("ids", Type::map(Type::int(), dice)).unwrap();
    env.value("tags", Type::map(Type::string(), Type::bool()))
        .unwrap();
    env.value("lists", Type::map(Type::int(), Type::list(Type::int())))
        .unwrap();
    env.value("slists", Type::map(Type::string(), Type::list(Type::int())))
        .unwrap();
    env.value("wits", Type::int()).unwrap();
    env.value("name", Type::string()).unwrap();
    env
}

/// The labels at hole `n` of `text` in `env`.
fn labels_in(env: &Environment, text: &str, n: usize) -> Vec<String> {
    let s = s(text);
    let hole = s.holes()[n];
    s.complete(env, hole, None)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"))
        .into_iter()
        .map(|c| c.label)
        .collect()
}

/// A receiver whose index is of a known type is offered only when it is
/// keyed by that type, whatever it is wanted for after.
#[test]
fn map_index_receivers_take_the_key_type() {
    let env = keyed_env();
    let has = |text: &str, label: &str| labels_in(&env, text, 0).contains(&label.to_owned());

    for (text, yes, no) in [
        ("_[wits].dice", "ids", "tags"),
        ("_[wits].dice", "ids", "slists"),
        ("_[name].len()", "tags", "ids"),
        ("_[name].len()", "slists", "lists"),
        ("_[wits].len()", "lists", "tags"),
        ("_[wits].len()", "lists", "slists"),
    ] {
        assert!(has(text, yes), "{text:?} offers {yes}");
        assert!(!has(text, no), "{text:?} does not offer {no}");
    }

    // A `String` index reaches a record's field too, so it is `Int`s only
    // that keep lists out.
    assert!(!has("_[name].len()", "dots"));
}

/// A map literal's value is of any type however the map is indexed, and
/// its key is of the index's type.
#[test]
fn indexed_map_literals_have_candidates() {
    let env = keyed_env();

    for text in ["{1: _}[wits]", r#"{"x": _}["a"]"#] {
        let labels = labels_in(&env, text, 0);

        for label in ["wits", "name", "0", "\"\"", "true", "[_]", "{_: _}"] {
            assert!(
                labels.contains(&label.to_owned()),
                "{text:?} offers {label}: {labels:?}"
            );
        }
    }

    assert_eq!(labels_in(&env, r#"{_: 1}["a"]"#, 0), ["\"\""]);
    assert_eq!(labels_in(&env, "{_: 1}[wits]", 0), ["0"]);
    assert_eq!(labels_in(&env, "{_: 1}[name]", 0), ["\"\""]);
    assert_eq!(labels_in(&env, "{_: 1}", 0), ["0", "\"\""]);

    // Lists are indexed by an `Int` as maps keyed by one are.
    assert_eq!(labels_in(&env, "{_: 1}[wits]", 0), ["0"]);
    let labels = labels_in(&env, "[_][wits]", 0);
    assert!(labels.contains(&"wits".to_owned()), "{labels:?}");
}

/// A key is a literal: nothing else is offered in it, as nothing else is
/// a key.
#[test]
fn keys_hold_literals_only() {
    let env = keyed_env();

    // A `-` in a key is not a literal, so nothing under one is.
    assert_eq!(labels_in(&env, "{-_: 1}", 0), Vec::<String>::new());
    assert_eq!(labels_in(&env, "{1: 2, -_: 3}", 0), Vec::<String>::new());
    assert_eq!(labels_in(&env, "{--_: 1}", 0), Vec::<String>::new());

    // A value under a `-` is not a key.
    let labels = labels_in(&env, "{1: -_}", 0);
    assert!(labels.contains(&"wits".to_owned()), "{labels:?}");

    // Each of the keys offered is one.
    for text in ["{_: 1}", "{1: 2, _: 3}"] {
        let s = s(text);
        let hole = s.holes()[0];

        for completion in s.complete(&env, hole, None).unwrap() {
            let mut applied = s.clone();
            applied.apply(hole, &completion).unwrap();
            let text = applied.to_text();
            assert!(crate::compile(&text, &env).is_ok(), "{text:?}");
        }
    }
}

/// Completion in a large list or map of holes is linear: a sibling that is
/// a hole has no type, and is not compiled to find that out.
#[test]
fn completion_in_large_collections_is_linear() {
    use std::time::Instant;

    let env = Environment::new();
    let options = Options::new().max_len(usize::MAX).max_depth(1_000_000);

    let shapes: [fn(usize) -> String; 3] = [
        |n| format!("{{{}}}", vec!["1: _"; n].join(", ")),
        |n| format!("{{{}}}", vec!["_: 1"; n].join(", ")),
        |n| format!("[{}]", vec!["_"; n].join(", ")),
    ];

    for shape in shapes {
        let mut times = Vec::new();

        for n in [5_000usize, 20_000] {
            let s = Structure::from_text_with(&shape(n), &options).unwrap();
            let holes = s.holes();
            let hole = holes[holes.len() / 2];

            let start = Instant::now();
            assert!(!s.complete(&env, hole, None).unwrap().is_empty());
            times.push(start.elapsed().as_secs_f64());
        }

        // Four times the input takes four times as long, not sixteen.
        assert!(times[1] < times[0] * 10.0 + 0.05, "{times:?}");
    }
}

/// Only the first siblings that are not holes are tried for a type.
#[test]
fn siblings_tried_for_a_type_are_capped() {
    let env = keyed_env();
    let failing = ["_ + 1"; 8];

    // An entry that compiles is found past holes, however many.
    let text = format!("[{}, 1, _]", vec!["_"; 100].join(", "));
    let labels = labels_in(&env, &text, 100);
    assert!(
        labels.contains(&"wits".to_owned()) && !labels.contains(&"name".to_owned()),
        "{labels:?}"
    );

    // Past the cap of those that do not, it is not looked for.
    let text = format!("[{}, 1, _]", failing.join(", "));
    let labels = labels_in(&env, &text, 8);
    assert!(labels.contains(&"name".to_owned()), "{labels:?}");

    let text = format!("[{}, 1, _]", failing[..7].join(", "));
    let labels = labels_in(&env, &text, 7);
    assert!(!labels.contains(&"name".to_owned()), "{labels:?}");
}
