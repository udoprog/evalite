#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::error::ErrorKind;
use crate::message::Message;
use crate::structure::{EditKind, Repr, StructureKind};
use crate::*;

fn env() -> Environment {
    let mut env = Environment::with_builtins();
    env.value("wits", Type::int()).unwrap();
    env.value("dexterity", Type::int()).unwrap();
    env.value("dead", Type::bool()).unwrap();
    env.value("bonus", Type::option(Type::int())).unwrap();
    env.value("dots", Type::list(Type::int())).unwrap();
    env.value("health", crate::check_tests::track()).unwrap();
    env.value(
        "m",
        Type::record([
            ("athletics", Type::int()),
            ("Brawl (bar)", Type::int()),
            ("brawl", Type::option(Type::int())),
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
    env
}

fn s(text: &str) -> Structure {
    Structure::from_text(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

/// Every construct: text, and the text its structure gives back.
const CONSTRUCTS: &[(&str, &str)] = &[
    ("12", "12"),
    ("-12", "-12"),
    ("- 12", "-12"),
    ("-(12)", "-(12)"),
    ("--12", "--12"),
    ("-wits", "-wits"),
    ("!dead", "!dead"),
    ("!!dead", "!!dead"),
    ("true", "true"),
    ("false", "false"),
    (r#""Essence (1e)""#, r#""Essence (1e)""#),
    (r#""a \"b\" \\ c""#, r#""a \"b\" \\ c""#),
    ("wits", "wits"),
    ("_", "_"),
    ("bonus!", "bonus!"),
    ("bonus !", "bonus!"),
    ("_!", "_!"),
    ("m.brawl!", "m.brawl!"),
    ("dots.get(1)!", "dots.get(1)!"),
    ("(-wits)!", "(-wits)!"),
    ("-bonus!", "-bonus!"),
    ("bonus! + 1", "bonus! + 1"),
    ("wits+1", "wits + 1"),
    ("wits - 1", "wits - 1"),
    ("wits * 2", "wits * 2"),
    ("wits / 2", "wits / 2"),
    ("wits % 2", "wits % 2"),
    ("wits == 2", "wits == 2"),
    ("wits != 2", "wits != 2"),
    ("wits < 2", "wits < 2"),
    ("wits <= 2", "wits <= 2"),
    ("wits > 2", "wits > 2"),
    ("wits >= 2", "wits >= 2"),
    ("dead && true", "dead && true"),
    ("dead || true", "dead || true"),
    ("1 - 2 - 3", "1 - 2 - 3"),
    ("1 - (2 - 3)", "1 - (2 - 3)"),
    ("(1 + 2) * 3", "(1 + 2) * 3"),
    ("1 + 2 * 3", "1 + 2 * 3"),
    ("((wits))", "wits"),
    ("(1 < 2) == true", "(1 < 2) == true"),
    ("true == (1 < 2)", "true == (1 < 2)"),
    ("a || b && c", "a || b && c"),
    ("(a || b) && c", "(a || b) && c"),
    ("-(1 + 2)", "-(1 + 2)"),
    ("!(a && b)", "!(a && b)"),
    ("wits - -1", "wits - -1"),
    ("min(wits,3)", "min(wits, 3)"),
    ("f()", "f()"),
    ("count(health, _)", "count(health, _)"),
    ("bonus.unwrap_or(0)", "bonus.unwrap_or(0)"),
    ("wits.abs()", "wits.abs()"),
    ("(-5).abs()", "(-5).abs()"),
    ("(1 + 2).abs()", "(1 + 2).abs()"),
    ("(-wits).abs()", "(-wits).abs()"),
    ("5.abs()", "5.abs()"),
    ("bonus._(1)", "bonus._(1)"),
    ("m.athletics", "m.athletics"),
    ("m._", "m._"),
    ("m.brawl.unwrap_or(0).max(1)", "m.brawl.unwrap_or(0).max(1)"),
    (r#"m["Brawl (bar)"]"#, r#"m["Brawl (bar)"]"#),
    ("dots[2]", "dots[2]"),
    ("dots[wits + 1]", "dots[wits + 1]"),
    ("[1, 2, 3]", "[1, 2, 3]"),
    ("[]", "[]"),
    ("[-1, _]", "[-1, _]"),
    ("[[1], [2]][0][0]", "[[1], [2]][0][0]"),
    ("[10, 11].get(wits)", "[10, 11].get(wits)"),
];

#[test]
fn every_construct_round_trips_through_text() {
    for (text, canonical) in CONSTRUCTS {
        let structure = s(text);
        assert_eq!(structure.to_text(), *canonical, "{text:?}");
        assert_eq!(s(canonical), structure, "{text:?}");
        assert_eq!(s(&structure.to_text()).to_text(), *canonical, "{text:?}");
    }
}

#[test]
fn text_parses_to_the_nodes_it_names() {
    let structure = s("-12");
    assert_eq!(structure.get(structure.root()), Some(&Node(Repr::Int(-12))));

    let structure = s("-(12)");
    let Some(Node(Repr::Unary {
        op: UnaryOp::Neg,
        operand,
    })) = structure.get(structure.root())
    else {
        panic!("{structure:?}");
    };
    assert_eq!(structure.get(*operand), Some(&Node(Repr::Int(12))));

    let structure = s("m._");
    assert!(matches!(
        structure.get(structure.root()),
        Some(Node(Repr::Field { name: None, .. }))
    ));

    let structure = s("bonus._()");
    assert!(matches!(
        structure.get(structure.root()),
        Some(Node(Repr::Method { name: None, .. }))
    ));

    let structure = s("min(1, _)");
    assert_eq!(structure.holes().len(), 1);
    assert_eq!(s("_ + m._ + f(_)").holes().len(), 3);
}

#[test]
fn built_structures_render_and_parse_back() {
    let mut b = Structure::new();
    let one = b.add(Node(Repr::Int(1))).unwrap();
    let two = b.add(Node(Repr::Int(2))).unwrap();
    let sum = b
        .add(Node(Repr::Binary {
            op: BinaryOp::Add,
            left: one,
            right: two,
        }))
        .unwrap();
    let three = b.add(Node(Repr::Int(3))).unwrap();
    b.replace(
        b.root(),
        Node(Repr::Binary {
            op: BinaryOp::Mul,
            left: sum,
            right: three,
        }),
    )
    .unwrap();

    assert_eq!(b.to_text(), "(1 + 2) * 3");
    assert_eq!(s("(1 + 2) * 3"), b);
    assert_ne!(s("1 + 2 * 3"), b);

    let mut odd = Structure::new();
    let name = odd.add(Node(Repr::String("a\"b\\".into()))).unwrap();
    odd.replace(
        odd.root(),
        Node(Repr::List {
            entries: vec![name],
        }),
    )
    .unwrap();
    assert_eq!(odd.to_text(), r#"["a\"b\\"]"#);
    assert_eq!(s(&odd.to_text()), odd);
}

#[test]
fn negative_integer_receivers_and_operands() {
    // A structure holding `Int(-5)` where a postfix or `-` would bind it
    // differently gets parentheses.
    let mut b = Structure::new();
    let n = b.add(Node(Repr::Int(-5))).unwrap();
    b.replace(
        b.root(),
        Node(Repr::Method {
            receiver: n,
            name: Some("abs".into()),
            args: vec![],
        }),
    )
    .unwrap();
    assert_eq!(b.to_text(), "(-5).abs()");
    assert_eq!(s("(-5).abs()"), b);
    assert_eq!(
        b.compile(&env()).unwrap().to_bytes().unwrap(),
        compile("(-5).abs()", &env()).unwrap().to_bytes().unwrap()
    );

    // `i64::MIN` has no text, as in a text expression.
    let mut min = Structure::new();
    min.replace(min.root(), Node(Repr::Int(i64::MIN))).unwrap();
    let error = min.compile(&env()).unwrap_err();
    assert_eq!(error.first().unwrap().node(), min.root());
    assert!(matches!(
        error.first().unwrap().kind(),
        StructureKind::Compile(e) if e.first().map(Diagnostic::kind) == Some(&ErrorKind::IntegerTooLarge)
    ));
}

/// The same program, or the same error at the same node, as its text.
fn same_as_text(structure: &Structure, env: &Environment) {
    let text = structure.to_text();
    let from_text = compile(&text, env);
    let from_structure = structure.compile(env);

    match (from_text, from_structure) {
        (Ok(a), Ok(b)) => assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap(), "{text}"),
        (Err(a), Err(b)) => match b.first().unwrap().kind() {
            StructureKind::Compile(_) => {
                let b: Vec<_> = b
                    .iter()
                    .flat_map(|e| match e.kind() {
                        StructureKind::Compile(d) => d.iter().cloned().collect(),
                        _ => Vec::new(),
                    })
                    .collect();
                let a: Vec<_> = a.iter().cloned().collect();
                assert_eq!(a, b, "{text}");
            }
            // Checked before the text is.
            StructureKind::Hole | StructureKind::InvalidName(_) => {}
        },
        (a, b) => panic!("{text}: text gives {a:?}, structure {b:?}"),
    }
}

#[test]
fn compiling_a_structure_matches_compiling_its_text() {
    let env = env();

    for text in [
        "min(wits, 3) + bonus.unwrap_or(0)",
        "count(health) + count(health, 2)",
        "[10, 11, 12].get(wits)",
        r#"m["Brawl (bar)"] + m.athletics + m.brawl.unwrap_or(1).clamp(0, 5)"#,
        "dots[wits] * -2 > 3 && !dead || bonus.is_none()",
        "dots.len() % 2 == 0",
        // Errors, which must be the same.
        "wits + bonus",
        "count(1)",
        "unknown",
        "min()",
        "[]",
        "[1, true]",
        "dots[true]",
        "m.nope",
        "(1 < 2) == 1",
        "bonus.unwrap_or(true)",
        "wits.nope()",
    ] {
        same_as_text(&s(text), &env);
    }
}

#[test]
fn errors_name_the_node_they_are_about() {
    let env = env();

    let structure = s("wits + dots.get(true)");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(
        error.to_string(),
        "argument 1 of `get` on List<Int> must be Int, found Bool"
    );
    assert_eq!(structure.text_of(error.first().unwrap().node()), "true");

    let structure = s("min(wits, 1) + m.nope");
    let error = structure.compile(&env).unwrap_err();
    // `nope` is part of the field node.
    assert_eq!(structure.text_of(error.first().unwrap().node()), "m.nope");

    let structure = s("(wits + bonus) * 2");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(
        structure.text_of(error.first().unwrap().node()),
        "wits + bonus"
    );
}

#[test]
fn every_error_names_its_node() {
    let env = env();

    let structure = s("wits + foo * (m.nope + bar)");
    let error = structure.compile(&env).unwrap_err();
    let nodes: Vec<_> = error.iter().map(|e| structure.text_of(e.node())).collect();
    assert_eq!(error.len(), nodes.len());
    assert_eq!(nodes, ["foo", "m.nope", "bar"]);
    assert_eq!(structure.text_of(error.first().unwrap().node()), nodes[0]);

    // Every hole, as written.
    let structure = s("_ + min(_, _)");
    let error = structure.compile(&env).unwrap_err();
    let found: Vec<_> = error.iter().map(|e| e.node()).collect();
    assert_eq!(found, structure.holes());
    assert_eq!(
        error.to_string().lines().count(),
        3,
        "one line for each hole"
    );

    // Every name that cannot be written.
    let mut b = Structure::new();
    let x = b.add(Node(Repr::Name("Strength".into()))).unwrap();
    let y = b.add(Node(Repr::Name("true".into()))).unwrap();
    b.replace(
        b.root(),
        Node(Repr::Binary {
            op: BinaryOp::Add,
            left: x,
            right: y,
        }),
    )
    .unwrap();
    let error = b.compile(&env).unwrap_err();
    let found: Vec<_> = error.iter().map(|e| e.node()).collect();
    assert_eq!(found, [x, y]);
}

#[test]
fn holes_are_refused_when_compiling() {
    let env = env();

    for text in ["_", "wits + _", "min(_, 1)", "m._", "bonus._(0)", "[1, _]"] {
        let structure = s(text);
        let error = structure.compile(&env).unwrap_err();
        assert_eq!(
            error.first().unwrap().kind(),
            &StructureKind::Hole,
            "{text}"
        );
        assert_eq!(
            error.to_string(),
            "the expression is unfinished, fill in the `_`"
        );
        assert!(
            structure.holes().contains(&error.first().unwrap().node()),
            "{text}"
        );
    }

    // The first hole as written.
    let structure = s("_ + min(_)");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(error.first().unwrap().node(), structure.holes()[0]);
}

#[test]
fn names_that_are_no_names_are_refused() {
    let env = env();

    for (node, name) in [
        (Node(Repr::Name("Strength".into())), "Strength"),
        (Node(Repr::Name("true".into())), "true"),
        (Node(Repr::Name("_".into())), "_"),
        (Node(Repr::Name("a b".into())), "a b"),
        (Node(Repr::Name(String::new())), ""),
        (
            Node(Repr::Call {
                function: "x-y".into(),
                args: vec![],
            }),
            "x-y",
        ),
    ] {
        let mut b = Structure::new();
        b.replace(b.root(), node).unwrap();
        let error = b.compile(&env).unwrap_err();
        assert_eq!(
            error.first().unwrap().kind(),
            &StructureKind::InvalidName(name.into()),
            "{name}"
        );
    }

    let mut b = Structure::new();
    let m = b.add(Node(Repr::Name("m".into()))).unwrap();
    b.replace(
        b.root(),
        Node(Repr::Field {
            receiver: m,
            name: Some("Brawl (bar)".into()),
        }),
    )
    .unwrap();
    let error = b.compile(&env).unwrap_err();
    assert_eq!(error.first().unwrap().node(), b.root());
    assert_eq!(
        error.to_string(),
        "`Brawl (bar)` cannot be written as a name, names are letters, digits and `_`, starting with a letter or `_`"
    );
}

/// Text with syntax errors is refused, with all of them.
#[test]
fn from_text_reports_every_syntax_error() {
    let errors = Structure::from_text("f(a b, c d) + (1").unwrap_err();
    let errors = errors
        .iter()
        .map(|e| (e.to_string(), e.span().range()))
        .collect::<Vec<_>>();

    assert_eq!(
        errors,
        [
            (
                "expected an operator, `,` or `)`, found a name".to_owned(),
                4..5
            ),
            (
                "expected an operator, `,` or `)`, found a name".to_owned(),
                9..10
            ),
            (
                "expected `)` to close the `(` at byte 14, found the end of the expression"
                    .to_owned(),
                14..15
            ),
        ]
    );
}

#[test]
fn parse_depth_limits_apply() {
    let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    assert!(Structure::from_text(&deep).is_err());
    let ok = Structure::from_text_with(&deep, &Options::new().max_depth(64)).unwrap();
    assert_eq!(ok.to_text(), "1");

    // A structure nesting past the limit is refused when compiled, at the
    // node too deep.
    let mut b = Structure::new();
    let mut inner = b.add(Node(Repr::Int(1))).unwrap();

    for _ in 0..40 {
        inner = b
            .add(Node(Repr::List {
                entries: vec![inner],
            }))
            .unwrap();
    }

    b.set_root(inner).unwrap();
    let error = b.compile(&env()).unwrap_err();
    assert!(matches!(
        error.first().unwrap().kind(),
        StructureKind::Compile(e) if matches!(e.first().map(Diagnostic::kind), Some(ErrorKind::TooDeep { .. }))
    ));
    assert!(matches!(
        b.get(error.first().unwrap().node()),
        Some(Node(Repr::List { .. }))
    ));
    assert!(
        b.compile_with(&env(), &Options::new().max_depth(64))
            .is_ok()
    );
}

#[test]
fn long_chains_do_not_recurse() {
    let roomy = Options::new().max_len(usize::MAX);
    // 100 000 nodes deep through `+`, which does not nest.
    let text = (0..100_000).map(|_| "1").collect::<Vec<_>>().join(" + ");
    let structure = Structure::from_text_with(&text, &roomy).unwrap();
    assert_eq!(structure.to_text(), text);
    let bytes = structure.to_bytes().unwrap();
    assert_eq!(Structure::from_bytes(&bytes).unwrap(), structure);
    assert!(structure.compile_with(&env(), &roomy).is_ok());

    // 100 000 deep through `-x`, which nests: an error, not an overflow.
    let mut b = Structure::new();
    let mut inner = b.add(Node(Repr::Name("wits".into()))).unwrap();

    for _ in 0..100_000 {
        inner = b
            .add(Node(Repr::Unary {
                op: UnaryOp::Neg,
                operand: inner,
            }))
            .unwrap();
    }

    b.set_root(inner).unwrap();
    assert!(b.compile_with(&env(), &roomy).is_err());
    let copy = Structure::from_bytes(&b.to_bytes().unwrap()).unwrap();
    assert_eq!(copy, b);
    drop(copy);
}

#[test]
fn structures_round_trip_through_musli() {
    for (text, _) in CONSTRUCTS {
        let structure = s(text);
        let bytes = structure.to_bytes().unwrap();
        let loaded = Structure::from_bytes(&bytes).unwrap();
        assert_eq!(loaded, structure, "{text}");
        assert_eq!(loaded.to_text(), structure.to_text(), "{text}");
    }
}

#[test]
fn storing_leaves_out_unused_nodes() {
    let mut b = s("wits + 1");
    let two = b.add(Node(Repr::Int(2))).unwrap();
    let Some(Node(Repr::Binary { left, .. })) = b.get(b.root()).cloned() else {
        panic!();
    };
    b.replace(
        b.root(),
        Node(Repr::Binary {
            op: BinaryOp::Add,
            left,
            right: two,
        }),
    )
    .unwrap();
    assert_eq!(b.to_text(), "wits + 2");

    let loaded = Structure::from_bytes(&b.to_bytes().unwrap()).unwrap();
    assert_eq!(loaded.post_order().len(), 3);
    assert_eq!(loaded, b);
}

/// Node ids are opaque outside the crate, but tests are inside it.
fn id(n: u32) -> NodeId {
    crate::structure::test_id(n)
}

#[test]
fn tampered_structures_are_refused() {
    let invalid = |bytes: Vec<u8>| match Structure::from_bytes(&bytes) {
        Err(e) if matches!(e.message(), Message::Invalid(..)) => format!("{e:#}"),
        other => panic!("{other:?}"),
    };

    let leaf = Node(Repr::Int(1));
    let neg = |n| {
        Node(Repr::Unary {
            op: UnaryOp::Neg,
            operand: id(n),
        })
    };

    // Valid as a baseline.
    let bytes = stored_with(vec![leaf.clone(), neg(0)], 1);
    assert_eq!(Structure::from_bytes(&bytes).unwrap().to_text(), "-(1)");

    // A part past the end.
    let e = invalid(stored_with(vec![neg(5)], 0));
    assert_eq!(
        e,
        "the node has a part past the end: at node #0: while reading a stored structure"
    );

    // A part used twice.
    let both = Node(Repr::Binary {
        op: BinaryOp::Add,
        left: id(0),
        right: id(0),
    });
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), both], 1)),
        "the node is a part twice: at node #0: while reading a stored structure"
    );

    // A cycle away from the root, which is then unreachable.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), neg(2), neg(1)], 0)),
        "some nodes are not part of the expression: while reading a stored structure"
    );

    // The root as a part of itself.
    assert_eq!(
        invalid(stored_with(vec![neg(0)], 0)),
        "the node is a part twice: at node #0: while reading a stored structure"
    );

    // An unused node.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), leaf.clone()], 0)),
        "some nodes are not part of the expression: while reading a stored structure"
    );

    // A root past the end.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone()], 3)),
        "the root is past the end: while reading a stored structure"
    );

    // No nodes at all.
    assert_eq!(
        invalid(stored_with(vec![], 0)),
        "the root is past the end: while reading a stored structure"
    );

    // Another format, and bytes that are not a structure.
    let mut bytes = stored_with(vec![leaf], 0);
    bytes[0] = 7;
    assert!(matches!(
        Structure::from_bytes(&bytes).map_err(|e| e.message().clone()),
        Err(Message::WrongFormat { .. } | Message::NotDecoded)
    ));
    let error = Structure::from_bytes(&[1, 2, 3]).unwrap_err();
    assert_eq!(*error.message(), Message::NotDecoded);
    assert_eq!(
        format!("{error:#}"),
        "the bytes do not decode: while reading a stored structure"
    );
    assert!(std::error::Error::source(&error).is_some());
}

fn stored_with(nodes: Vec<Node>, root: u32) -> Vec<u8> {
    crate::structure::test_stored(nodes, id(root))
}

#[test]
fn edits_keep_the_structure_a_tree() {
    let mut b = Structure::new();
    let one = b.add(Node(Repr::Int(1))).unwrap();
    let neg = b
        .add(Node(Repr::Unary {
            op: UnaryOp::Neg,
            operand: one,
        }))
        .unwrap();

    // `one` is part of `neg` now.
    assert_eq!(
        b.add(Node(Repr::List { entries: vec![one] })),
        Err(EditKind::Attached(one).into())
    );
    // A node twice in one node.
    let two = b.add(Node(Repr::Int(2))).unwrap();
    assert_eq!(
        b.add(Node(Repr::List {
            entries: vec![two, two]
        })),
        Err(EditKind::Attached(two).into())
    );
    // The root.
    assert_eq!(
        b.add(Node(Repr::List {
            entries: vec![b.root()]
        })),
        Err(EditKind::Attached(b.root()).into())
    );
    // Unknown nodes.
    let unknown = id(99);
    assert_eq!(
        b.add(Node(Repr::List {
            entries: vec![unknown]
        })),
        Err(EditKind::UnknownNode(unknown).into())
    );
    assert_eq!(
        b.replace(unknown, Node(Repr::Hole)),
        Err(EditKind::UnknownNode(unknown).into())
    );

    b.replace(
        b.root(),
        Node(Repr::List {
            entries: vec![neg, two],
        }),
    )
    .unwrap();
    assert_eq!(b.to_text(), "[-(1), 2]");
    assert_eq!(b.parent(neg), Some(b.root()));

    // Making `neg` part of what it holds.
    assert_eq!(
        b.replace(one, Node(Repr::List { entries: vec![neg] })),
        Err(EditKind::Attached(neg).into())
    );

    // A free subtree holding the node it would replace.
    let mut c = Structure::new();
    let x = c.add(Node(Repr::Int(1))).unwrap();
    let free = c
        .add(Node(Repr::Unary {
            op: UnaryOp::Neg,
            operand: x,
        }))
        .unwrap();
    assert_eq!(
        c.replace(
            x,
            Node(Repr::List {
                entries: vec![free]
            })
        ),
        Err(EditKind::Cycle(free).into())
    );

    // Replacing a node frees its parts for reuse.
    b.replace(b.root(), Node(Repr::List { entries: vec![two] }))
        .unwrap();
    assert_eq!(b.parent(neg), None);
    b.replace(
        b.root(),
        Node(Repr::List {
            entries: vec![two, neg],
        }),
    )
    .unwrap();
    assert_eq!(b.to_text(), "[2, -(1)]");

    // A new root, whose old root is then free.
    let old = b.root();
    let top = b.add(Node(Repr::Name("wits".into()))).unwrap();
    b.set_root(top).unwrap();
    assert_eq!(b.to_text(), "wits");
    assert_eq!(b.set_root(two), Err(EditKind::Attached(two).into()));
    b.set_root(old).unwrap();
    assert_eq!(b.to_text(), "[2, -(1)]");
}

#[test]
fn grafting_copies_an_expression_into_a_hole() {
    let mut b = s("min(_, 1)");
    let hole = b.holes()[0];
    b.graft(hole, &s("bonus.unwrap_or(_)")).unwrap();
    assert_eq!(b.to_text(), "min(bonus.unwrap_or(_), 1)");

    let hole = b.holes()[0];
    b.graft(hole, &s("2")).unwrap();
    assert_eq!(b.to_text(), "min(bonus.unwrap_or(2), 1)");
    assert!(b.compile(&env()).is_ok());

    let mut root = Structure::new();
    root.graft(root.root(), &s("wits + 1")).unwrap();
    assert_eq!(root, s("wits + 1"));
}

/// A small seeded generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// A random structure, at most `depth` deep, with names from the test
/// environment and some that are not in it.
fn random(rng: &mut Rng, depth: u32) -> Structure {
    let mut b = Structure::new();
    let root = random_node(rng, &mut b, depth);
    b.set_root(root).unwrap();
    b
}

fn random_node(rng: &mut Rng, b: &mut Structure, depth: u32) -> NodeId {
    const NAMES: &[&str] = &[
        "wits",
        "dexterity",
        "dead",
        "bonus",
        "dots",
        "m",
        "health",
        "nope",
    ];
    const FUNCTIONS: &[&str] = &["min", "max", "chart", "count", "f"];
    const MEMBERS: &[&str] = &[
        "abs",
        "min",
        "max",
        "clamp",
        "unwrap_or",
        "is_some",
        "len",
        "athletics",
        "brawl",
        "Brawl",
    ];

    let leaf = depth == 0 || rng.below(3) == 0;
    let kids = |rng: &mut Rng, b: &mut Structure, n: u64| {
        (0..n)
            .map(|_| random_node(rng, b, depth.saturating_sub(1)))
            .collect::<Vec<_>>()
    };

    let node = if leaf {
        match rng.below(6) {
            0 => Node(Repr::Hole),
            1 => Node(Repr::Int(rng.below(200) as i64 - 100)),
            2 => Node(Repr::Bool(rng.below(2) == 0)),
            3 => Node(Repr::String(
                rng.pick(&["", "Brawl (bar)", "a\"b", "\\", "é ünï"])
                    .to_string(),
            )),
            _ => Node(Repr::Name(rng.pick(NAMES).to_string())),
        }
    } else {
        match rng.below(8) {
            0 => Node(Repr::Unary {
                op: *rng.pick(&[UnaryOp::Neg, UnaryOp::Not]),
                operand: kids(rng, b, 1)[0],
            }),
            1 | 2 => {
                let k = kids(rng, b, 2);
                Node(Repr::Binary {
                    op: *rng.pick(&BinaryOp::ALL),
                    left: k[0],
                    right: k[1],
                })
            }
            3 => {
                let n = rng.below(3);
                Node(Repr::Call {
                    function: rng.pick(FUNCTIONS).to_string(),
                    args: kids(rng, b, n),
                })
            }
            4 => {
                let n = rng.below(3) + 1;
                let k = kids(rng, b, n);
                Node(Repr::Method {
                    receiver: k[0],
                    name: (rng.below(5) != 0).then(|| rng.pick(MEMBERS).to_string()),
                    args: k[1..].to_vec(),
                })
            }
            5 => Node(Repr::Field {
                receiver: kids(rng, b, 1)[0],
                name: (rng.below(5) != 0).then(|| rng.pick(MEMBERS).to_string()),
            }),
            6 => {
                let k = kids(rng, b, 2);
                Node(Repr::Index {
                    receiver: k[0],
                    index: k[1],
                })
            }
            _ => {
                let n = rng.below(4);
                Node(Repr::List {
                    entries: kids(rng, b, n),
                })
            }
        }
    };

    b.add(node).unwrap()
}

#[test]
fn random_structures_round_trip() {
    let env = env();
    let mut rng = Rng(0x5eed_cafe_f00d_0001);
    let mut compiled = 0;

    for round in 0..3000 {
        let structure = random(&mut rng, 5);
        let text = structure.to_text();

        let parsed = Structure::from_text(&text)
            .unwrap_or_else(|e| panic!("round {round}: {text:?} does not parse: {e}"));
        assert_eq!(parsed, structure, "round {round}: {text:?}");
        assert_eq!(parsed.to_text(), text, "round {round}");

        let loaded = Structure::from_bytes(&structure.to_bytes().unwrap()).unwrap();
        assert_eq!(loaded, structure, "round {round}: {text:?}");

        same_as_text(&structure, &env);
        compiled += usize::from(structure.compile(&env).is_ok());
    }

    // Enough of them are well typed to test compiling too.
    assert!(compiled > 100, "{compiled}");
}

/// Every kind of node, stored. The bytes are pinned so the stored form
/// stays readable as `Node`'s API changes.
const GOLDEN: &str = r#"-wits + count(health, _) * m.athletics - [1, 2][0] + bonus.unwrap_or(0) + m._ + bonus._(1) + chart("s", [1]) * (!dead && true)"#;

#[test]
fn stored_form_is_stable() {
    let structure = s(GOLDEN);
    let bytes = structure.to_bytes().unwrap();
    let golden: &[u8] = &[
        3, 0, 1, 1, 36, 4, 1, 0, 4, 119, 105, 116, 115, 5, 2, 0, 0, 1, 0, 4, 1, 0, 6, 104, 101, 97,
        108, 116, 104, 0, 0, 7, 2, 0, 5, 99, 111, 117, 110, 116, 1, 2, 2, 3, 4, 1, 0, 1, 109, 9, 2,
        0, 5, 1, 1, 9, 97, 116, 104, 108, 101, 116, 105, 99, 115, 6, 3, 0, 2, 1, 4, 2, 6, 6, 3, 0,
        0, 1, 1, 2, 7, 1, 1, 0, 2, 1, 1, 0, 4, 11, 1, 0, 2, 9, 10, 1, 1, 0, 0, 10, 2, 0, 11, 1, 12,
        6, 3, 0, 1, 1, 8, 2, 13, 4, 1, 0, 5, 98, 111, 110, 117, 115, 1, 1, 0, 0, 8, 3, 0, 15, 1, 1,
        9, 117, 110, 119, 114, 97, 112, 95, 111, 114, 2, 1, 16, 6, 3, 0, 0, 1, 14, 2, 17, 4, 1, 0,
        1, 109, 9, 2, 0, 19, 1, 0, 6, 3, 0, 0, 1, 18, 2, 20, 4, 1, 0, 5, 98, 111, 110, 117, 115, 1,
        1, 0, 2, 8, 3, 0, 22, 1, 0, 2, 1, 23, 6, 3, 0, 0, 1, 21, 2, 24, 3, 1, 0, 1, 115, 1, 1, 0,
        2, 11, 1, 0, 1, 27, 7, 2, 0, 5, 99, 104, 97, 114, 116, 1, 2, 26, 28, 4, 1, 0, 4, 100, 101,
        97, 100, 5, 2, 0, 1, 1, 30, 2, 1, 0, 1, 6, 3, 0, 11, 1, 31, 2, 32, 6, 3, 0, 2, 1, 29, 2,
        33, 6, 3, 0, 0, 1, 25, 2, 34, 2, 35,
    ];
    assert_eq!(bytes, golden);
    assert_eq!(Structure::from_bytes(golden).unwrap(), structure);
}
