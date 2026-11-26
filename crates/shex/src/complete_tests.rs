#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::*;

fn env() -> Environment {
    let mut env = Environment::with_builtins();
    env.value("wits", Type::Int).unwrap();
    env.value("dead", Type::Bool).unwrap();
    env.value("bonus", Type::option(Type::Int)).unwrap();
    env.value("dots", Type::list(Type::Int)).unwrap();
    env.value("health", Type::opaque("Track")).unwrap();
    env.value("table", Type::String).unwrap();
    env.value(
        "m",
        Type::record([
            ("athletics", Type::Int),
            ("Brawl (bar)", Type::Int),
            ("brawl", Type::option(Type::Int)),
            ("flags", Type::record([("frenzy", Type::Bool)])),
        ]),
    )
    .unwrap();
    env.function("count", [Type::opaque("Track")], Type::Int)
        .unwrap();
    env.function("count", [Type::opaque("Track"), Type::Int], Type::Int)
        .unwrap();
    env.function("lookup", [Type::Int, Type::String], Type::Int)
        .unwrap();
    env.function("broken", [Type::Int], Type::opaque("Track"))
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
    "bonus.unwrap_or(_)",
    "dots.len()",
    "dots[_]",
    r#"m["Brawl (bar)"]"#,
    "m.athletics",
    "m.brawl.unwrap_or(_)",
    "wits",
    "count(_)",
    "count(_, _)",
    "lookup(_, _)",
    "max(_, _)",
    "min(_, _)",
    "0",
    "_ + _",
    "_ - _",
    "_ * _",
    "_ / _",
    "_ % _",
    "-_",
];

/// What fits a `Bool`'s place in [`env`].
const BOOL: &[&str] = &[
    "bonus.is_none()",
    "bonus.is_some()",
    "dead",
    "m.brawl.is_none()",
    "m.brawl.is_some()",
    "m.flags.frenzy",
    "lookup(_, _)",
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
    "count(_)",
    "count(_, _)",
    "lookup(_, _)",
    "max(_, _)",
    "min(_, _)",
    "0",
    "true",
    "false",
    r#""""#,
    "[_]",
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
];

/// Text, which of its holes, the type the whole expression must have, and
/// what fits there.
fn cases() -> Vec<(&'static str, usize, Option<Type>, Vec<&'static str>)> {
    vec![
        // Value holes, by the type the expression must have.
        ("_", 0, None, ANY.to_vec()),
        ("_", 0, Some(Type::Int), INT.to_vec()),
        ("_", 0, Some(Type::Bool), BOOL.to_vec()),
        (
            "_",
            0,
            Some(Type::String),
            vec!["table", "lookup(_, _)", r#""""#],
        ),
        (
            "_",
            0,
            Some(Type::list(Type::Int)),
            vec!["dots", "lookup(_, _)", "[_]"],
        ),
        (
            "_",
            0,
            Some(Type::list(Type::Bool)),
            vec!["lookup(_, _)", "[_]"],
        ),
        (
            "_",
            0,
            Some(Type::opaque("Track")),
            vec!["health", "broken(_)", "lookup(_, _)"],
        ),
        ("_", 0, Some(Type::opaque("Nothing")), vec!["lookup(_, _)"]),
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
        (
            "count(_)",
            0,
            None,
            vec!["health", "broken(_)", "lookup(_, _)"],
        ),
        ("count(health, _)", 0, None, INT.to_vec()),
        (
            "count(_, 1)",
            0,
            None,
            vec!["health", "broken(_)", "lookup(_, _)"],
        ),
        ("count(_, _, _)", 0, None, vec![]),
        ("max(_)", 0, None, INT.to_vec()),
        ("min(1, _, 3)", 0, None, INT.to_vec()),
        ("min(dead, _)", 0, None, vec![]),
        ("lookup(_, _)", 0, None, INT.to_vec()),
        (
            "lookup(_, _)",
            1,
            None,
            vec!["dots", "table", "lookup(_, _)", r#""""#, "[_]"],
        ),
        (
            "lookup(1, _)",
            0,
            None,
            vec!["dots", "table", "lookup(_, _)", r#""""#, "[_]"],
        ),
        ("lookup(_, table)", 0, None, INT.to_vec()),
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
            vec!["bonus", "m.brawl", "lookup(_, _)"],
        ),
        ("_.len()", 0, None, vec!["dots", "lookup(_, _)"]),
        ("_.abs()", 0, None, INT.to_vec()),
        ("_.frenzy", 0, None, vec!["m.flags", "lookup(_, _)"]),
        ("_.nothing", 0, None, vec!["lookup(_, _)"]),
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
        ("_[1]", 0, None, vec!["dots", "lookup(_, _)", "[_]"]),
        ("_[_]", 0, None, vec!["dots", "m", "lookup(_, _)", "[_]"]),
        (r#"_["Brawl (bar)"]"#, 0, None, vec!["m", "lookup(_, _)"]),
        // List entries, by a sibling or the list's own type.
        ("[1, _]", 0, None, INT.to_vec()),
        ("[_, dead]", 0, None, BOOL.to_vec()),
        ("[_]", 0, Some(Type::list(Type::Int)), INT.to_vec()),
        (
            "[[_]]",
            0,
            Some(Type::list(Type::list(Type::Bool))),
            BOOL.to_vec(),
        ),
        ("[_]", 0, Some(Type::Int), vec![]),
        ("[_]", 0, None, ANY.to_vec()),
        ("lookup(1, [_])", 0, None, ANY.to_vec()),
        ("count(health, [_][0])", 0, None, INT.to_vec()),
        (
            "_[0]",
            0,
            Some(Type::Int),
            vec!["dots", "lookup(_, _)", "[_]"],
        ),
        ("_[0]", 0, Some(Type::Bool), vec!["lookup(_, _)", "[_]"]),
        ("[_][0]", 0, Some(Type::Bool), BOOL.to_vec()),
        // Member holes, by the receiver's type and what the place takes.
        (
            "bonus._",
            0,
            None,
            vec!["is_none()", "is_some()", "unwrap_or(_)"],
        ),
        ("bonus._", 0, Some(Type::Int), vec!["unwrap_or(_)"]),
        ("1 + bonus._", 0, None, vec!["unwrap_or(_)"]),
        (
            "bonus._()",
            0,
            Some(Type::Bool),
            vec!["is_none()", "is_some()"],
        ),
        (
            "wits._",
            0,
            None,
            vec!["abs()", "clamp(_, _)", "max(_)", "min(_)"],
        ),
        ("wits._", 0, Some(Type::Bool), vec![]),
        ("dots._", 0, Some(Type::Int), vec!["len()"]),
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
            Some(Type::Int),
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl"],
        ),
        ("m._", 0, Some(Type::Bool), vec!["brawl", "flags"]),
        ("m.flags._", 0, None, vec!["frenzy"]),
        (
            "1 + m._",
            0,
            None,
            vec![r#"["Brawl (bar)"]"#, "athletics", "brawl"],
        ),
        // Holes are listed outermost first: the inner `._` is the second.
        (
            "max(m._._, 1)",
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
            "bonus" => vec!["bonus.is_none()", "bonus.is_some()", "bonus.unwrap_or(_)"],
            "dots" => vec!["dots.len()", "dots[_]"],
            "m" => vec![
                r#"m["Brawl (bar)"]"#,
                "m.athletics",
                "m.brawl.is_none()",
                "m.brawl.is_some()",
                "m.brawl.unwrap_or(_)",
                "m.flags.frenzy",
            ],
            "health" | "table" | "broken(_)" | r#""""# | "[_]" => vec![],
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
    let all = s.complete(&env(), hole, Some(&Type::Int)).unwrap();
    let find = |label: &str| all.iter().find(|c| c.label == label).unwrap();

    assert_eq!(find("wits").kind, CompletionKind::Value);
    assert_eq!(find("wits").ty, Some(Type::Int));
    assert_eq!(find("m.athletics").kind, CompletionKind::Value);
    assert_eq!(find("bonus.unwrap_or(_)").kind, CompletionKind::Method);
    assert_eq!(find("bonus.unwrap_or(_)").ty, Some(Type::Int));
    assert_eq!(find("count(_)").kind, CompletionKind::Function);
    assert_eq!(find("count(_)").ty, Some(Type::Int));
    // The builtin `lookup` gives whatever its list holds.
    assert_eq!(find("lookup(_, _)").ty, None);
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
                && matches!(completion.ty, Some(Type::Option(_) | Type::Record(_)));

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
    assert_eq!(s.compile(&env).unwrap().result_type(), Type::Int);

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
    assert_eq!(s.apply(hole, &field("x")), Err(EditError::NotAMember(hole)));
    assert_eq!(
        s.apply(hole, &method("abs", 0)),
        Err(EditError::NotAMember(hole))
    );
    assert_eq!(s.to_text(), "_ + 1");

    // An expression may replace any node, hole or not.
    let mut s = self::s("wits + 1");
    let one = s.post_order()[1];
    let completion = Completion {
        label: "bonus.unwrap_or(_)".into(),
        kind: CompletionKind::Method,
        ty: Some(Type::Int),
        insert: Insert::Expr(self::s("bonus.unwrap_or(_)")),
    };
    s.apply(one, &completion).unwrap();
    assert_eq!(s.to_text(), "wits + bonus.unwrap_or(_)");
}

fn field(name: &str) -> Completion {
    Completion {
        label: name.into(),
        kind: CompletionKind::Field,
        ty: None,
        insert: Insert::Field(name.into()),
    }
}

fn method(name: &str, args: usize) -> Completion {
    Completion {
        label: name.into(),
        kind: CompletionKind::Method,
        ty: None,
        insert: Insert::Method {
            name: name.into(),
            args,
        },
    }
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

    let int = Some(&Type::Int);
    pick(&mut s, int, "_ + _");
    pick(&mut s, int, "m.brawl.unwrap_or(_)");
    pick(&mut s, int, "0");
    pick(&mut s, int, "count(_, _)");
    pick(&mut s, int, "health");
    pick(&mut s, int, "lookup(_, _)");
    pick(&mut s, int, "wits");
    pick(&mut s, int, "[_]");
    pick(&mut s, int, "-_");
    pick(&mut s, int, "0");

    assert_eq!(
        s.to_text(),
        "m.brawl.unwrap_or(0) + count(health, lookup(wits, [-(0)]))"
    );
    assert!(s.holes().is_empty());
    assert_eq!(s.compile(&env).unwrap().result_type(), Type::Int);
}

#[test]
fn completion_errors() {
    let env = env();
    let s = s("_._");
    let member = s.holes()[0];

    let Err(CompleteError::Receiver(error)) = s.complete(&env, member, None) else {
        panic!("a receiver with a hole has no known members");
    };
    assert_eq!(error.kind(), &StructureErrorKind::Hole);
    assert_eq!(error.node(), s.holes()[1]);

    let s = self::s("nothing._");
    let Err(CompleteError::Receiver(error)) = s.complete(&env, s.holes()[0], None) else {
        panic!("an unknown receiver has no known members");
    };
    assert_eq!(error.to_string(), "unknown name `nothing`");

    let unknown = crate::structure::test_id(99);
    assert_eq!(
        s.complete(&env, unknown, None),
        Err(CompleteError::UnknownNode(unknown))
    );
}

/// Each method of the shared table compiles on its receiver, with
/// arguments of its parameters' types, to its result's type.
#[test]
fn method_table_matches_the_checker() {
    let mut env = Environment::new();

    let receivers = [
        Type::Int,
        Type::option(Type::Int),
        Type::option(Type::Bool),
        Type::list(Type::Bool),
    ];

    for (n, ty) in receivers.iter().enumerate() {
        env.value(&format!("r{n}"), ty.clone()).unwrap();
    }

    env.value("i", Type::Int).unwrap();
    env.value("b", Type::Bool).unwrap();

    let mut checked = std::collections::HashSet::new();

    for (n, ty) in receivers.iter().enumerate() {
        let names = crate::methods::method_names(ty);
        let (kind, _) = crate::methods::Receiver::of(ty).unwrap();

        for sig in crate::methods::methods(kind) {
            assert!(names.contains(&sig.name));
            let (params, ret) = sig.signature(ty);

            let args = params
                .iter()
                .map(|p| match p {
                    Type::Int => "i",
                    Type::Bool => "b",
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
                matches!(error.kind(), ErrorKind::ArgumentCount { .. }),
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
    let first = s.add(Node::Hole).unwrap();
    let mut inner = first;

    for _ in 0..100_000 {
        inner = s
            .add(Node::List {
                entries: vec![inner],
            })
            .unwrap();
    }

    let index = s.add(Node::Int(0)).unwrap();
    let root = s.root();
    s.replace(
        root,
        Node::Index {
            receiver: inner,
            index,
        },
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
        .complete(&env, first, Some(&Type::Int))
        .unwrap()
        .into_iter()
        .map(|c| c.label)
        .collect::<Vec<_>>();
    assert!(labels.is_empty(), "{labels:?}");
}
