#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::error::ErrorKind;
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
    ("lookup(wits, [10, 11])", "lookup(wits, [10, 11])"),
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
    assert_eq!(structure.get(structure.root()), Some(&Node::Int(-12)));

    let structure = s("-(12)");
    let Some(Node::Unary {
        op: UnaryOp::Neg,
        operand,
    }) = structure.get(structure.root())
    else {
        panic!("{structure:?}");
    };
    assert_eq!(structure.get(*operand), Some(&Node::Int(12)));

    let structure = s("m._");
    assert!(matches!(
        structure.get(structure.root()),
        Some(Node::Field { name: None, .. })
    ));

    let structure = s("bonus._()");
    assert!(matches!(
        structure.get(structure.root()),
        Some(Node::Method { name: None, .. })
    ));

    let structure = s("min(1, _)");
    assert_eq!(structure.holes().len(), 1);
    assert_eq!(s("_ + m._ + f(_)").holes().len(), 3);
}

#[test]
fn built_structures_render_and_parse_back() {
    let mut b = Structure::new();
    let one = b.add(Node::Int(1)).unwrap();
    let two = b.add(Node::Int(2)).unwrap();
    let sum = b
        .add(Node::Binary {
            op: BinaryOp::Add,
            left: one,
            right: two,
        })
        .unwrap();
    let three = b.add(Node::Int(3)).unwrap();
    b.replace(
        b.root(),
        Node::Binary {
            op: BinaryOp::Mul,
            left: sum,
            right: three,
        },
    )
    .unwrap();

    assert_eq!(b.to_text(), "(1 + 2) * 3");
    assert_eq!(s("(1 + 2) * 3"), b);
    assert_ne!(s("1 + 2 * 3"), b);

    let mut odd = Structure::new();
    let name = odd.add(Node::String("a\"b\\".into())).unwrap();
    odd.replace(
        odd.root(),
        Node::List {
            entries: vec![name],
        },
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
    let n = b.add(Node::Int(-5)).unwrap();
    b.replace(
        b.root(),
        Node::Method {
            receiver: n,
            name: Some("abs".into()),
            args: vec![],
        },
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
    min.replace(min.root(), Node::Int(i64::MIN)).unwrap();
    let error = min.compile(&env()).unwrap_err();
    assert_eq!(error.node(), min.root());
    assert!(matches!(
        error.kind(),
        StructureErrorKind::Compile(e) if *e.kind() == ErrorKind::IntegerTooLarge
    ));
}

/// The same program, or the same error at the same node, as its text.
fn same_as_text(structure: &Structure, env: &Environment) {
    let text = structure.to_text();
    let from_text = compile(&text, env);
    let from_structure = structure.compile(env);

    match (from_text, from_structure) {
        (Ok(a), Ok(b)) => assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap(), "{text}"),
        (Err(a), Err(b)) => match b.kind() {
            StructureErrorKind::Compile(b) => assert_eq!(&a, b, "{text}"),
            // Checked before the text is.
            StructureErrorKind::Hole | StructureErrorKind::InvalidName(_) => {}
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
        "lookup(wits, [10, 11, 12])",
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

    let structure = s("wits + lookup(true, dots)");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(
        error.to_string(),
        "argument 1 of `lookup` must be Int, found Bool"
    );
    assert_eq!(structure.text_of(error.node()), "true");

    let structure = s("min(wits, 1) + m.nope");
    let error = structure.compile(&env).unwrap_err();
    // `nope` is part of the field node.
    assert_eq!(structure.text_of(error.node()), "m.nope");

    let structure = s("(wits + bonus) * 2");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(structure.text_of(error.node()), "wits + bonus");
}

#[test]
fn holes_are_refused_when_compiling() {
    let env = env();

    for text in ["_", "wits + _", "min(_, 1)", "m._", "bonus._(0)", "[1, _]"] {
        let structure = s(text);
        let error = structure.compile(&env).unwrap_err();
        assert_eq!(error.kind(), &StructureErrorKind::Hole, "{text}");
        assert_eq!(
            error.to_string(),
            "the expression is unfinished, fill in the `_`"
        );
        assert!(structure.holes().contains(&error.node()), "{text}");
    }

    // The first hole as written.
    let structure = s("_ + min(_)");
    let error = structure.compile(&env).unwrap_err();
    assert_eq!(error.node(), structure.holes()[0]);
}

#[test]
fn names_that_are_no_names_are_refused() {
    let env = env();

    for (node, name) in [
        (Node::Name("Strength".into()), "Strength"),
        (Node::Name("true".into()), "true"),
        (Node::Name("_".into()), "_"),
        (Node::Name("a b".into()), "a b"),
        (Node::Name(String::new()), ""),
        (
            Node::Call {
                function: "x-y".into(),
                args: vec![],
            },
            "x-y",
        ),
    ] {
        let mut b = Structure::new();
        b.replace(b.root(), node).unwrap();
        let error = b.compile(&env).unwrap_err();
        assert_eq!(
            error.kind(),
            &StructureErrorKind::InvalidName(name.into()),
            "{name}"
        );
    }

    let mut b = Structure::new();
    let m = b.add(Node::Name("m".into())).unwrap();
    b.replace(
        b.root(),
        Node::Field {
            receiver: m,
            name: Some("Brawl (bar)".into()),
        },
    )
    .unwrap();
    let error = b.compile(&env).unwrap_err();
    assert_eq!(error.node(), b.root());
    assert_eq!(
        error.to_string(),
        "`Brawl (bar)` cannot be written as a name, names are letters, digits and `_`, starting with a letter or `_`"
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
    let mut inner = b.add(Node::Int(1)).unwrap();

    for _ in 0..40 {
        inner = b
            .add(Node::List {
                entries: vec![inner],
            })
            .unwrap();
    }

    b.set_root(inner).unwrap();
    let error = b.compile(&env()).unwrap_err();
    assert!(matches!(
        error.kind(),
        StructureErrorKind::Compile(e) if matches!(e.kind(), ErrorKind::TooDeep { .. })
    ));
    assert!(matches!(b.get(error.node()), Some(Node::List { .. })));
    assert!(
        b.compile_with(&env(), &Options::new().max_depth(64))
            .is_ok()
    );
}

#[test]
fn long_chains_do_not_recurse() {
    // 100 000 nodes deep through `+`, which does not nest.
    let text = (0..100_000).map(|_| "1").collect::<Vec<_>>().join(" + ");
    let structure = s(&text);
    assert_eq!(structure.to_text(), text);
    let bytes = structure.to_bytes().unwrap();
    assert_eq!(Structure::from_bytes(&bytes).unwrap(), structure);
    assert!(structure.compile(&env()).is_ok());

    // 100 000 deep through `-x`, which nests: an error, not an overflow.
    let mut b = Structure::new();
    let mut inner = b.add(Node::Name("wits".into())).unwrap();

    for _ in 0..100_000 {
        inner = b
            .add(Node::Unary {
                op: UnaryOp::Neg,
                operand: inner,
            })
            .unwrap();
    }

    b.set_root(inner).unwrap();
    assert!(b.compile(&env()).is_err());
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
    let two = b.add(Node::Int(2)).unwrap();
    let Some(Node::Binary { left, .. }) = b.get(b.root()).cloned() else {
        panic!();
    };
    b.replace(
        b.root(),
        Node::Binary {
            op: BinaryOp::Add,
            left,
            right: two,
        },
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
        Err(LoadError::Invalid(e)) => e,
        other => panic!("{other:?}"),
    };

    let leaf = Node::Int(1);
    let neg = |n| Node::Unary {
        op: UnaryOp::Neg,
        operand: id(n),
    };

    // Valid as a baseline.
    let bytes = stored_with(vec![leaf.clone(), neg(0)], 1);
    assert_eq!(Structure::from_bytes(&bytes).unwrap().to_text(), "-(1)");

    // A part past the end.
    let e = invalid(stored_with(vec![neg(5)], 0));
    assert_eq!(e, "node #0 has a part #5 past the end");

    // A part used twice.
    let both = Node::Binary {
        op: BinaryOp::Add,
        left: id(0),
        right: id(0),
    };
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), both], 1)),
        "node #0 is a part twice"
    );

    // A cycle away from the root, which is then unreachable.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), neg(2), neg(1)], 0)),
        "some nodes are not part of the expression"
    );

    // The root as a part of itself.
    assert_eq!(
        invalid(stored_with(vec![neg(0)], 0)),
        "node #0 is a part twice"
    );

    // An unused node.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone(), leaf.clone()], 0)),
        "some nodes are not part of the expression"
    );

    // A root past the end.
    assert_eq!(
        invalid(stored_with(vec![leaf.clone()], 3)),
        "the root is past the end"
    );

    // No nodes at all.
    assert_eq!(invalid(stored_with(vec![], 0)), "the root is past the end");

    // Another format, and bytes that are not a structure.
    let mut bytes = stored_with(vec![leaf], 0);
    bytes[0] = 7;
    assert!(matches!(
        Structure::from_bytes(&bytes),
        Err(LoadError::Format(_) | LoadError::Decode(_))
    ));
    assert!(matches!(
        Structure::from_bytes(&[1, 2, 3]),
        Err(LoadError::Decode(_))
    ));
}

fn stored_with(nodes: Vec<Node>, root: u32) -> Vec<u8> {
    crate::structure::test_stored(nodes, id(root))
}

#[test]
fn edits_keep_the_structure_a_tree() {
    let mut b = Structure::new();
    let one = b.add(Node::Int(1)).unwrap();
    let neg = b
        .add(Node::Unary {
            op: UnaryOp::Neg,
            operand: one,
        })
        .unwrap();

    // `one` is part of `neg` now.
    assert_eq!(
        b.add(Node::List { entries: vec![one] }),
        Err(EditError::Attached(one))
    );
    // A node twice in one node.
    let two = b.add(Node::Int(2)).unwrap();
    assert_eq!(
        b.add(Node::List {
            entries: vec![two, two]
        }),
        Err(EditError::Attached(two))
    );
    // The root.
    assert_eq!(
        b.add(Node::List {
            entries: vec![b.root()]
        }),
        Err(EditError::Attached(b.root()))
    );
    // Unknown nodes.
    let unknown = id(99);
    assert_eq!(
        b.add(Node::List {
            entries: vec![unknown]
        }),
        Err(EditError::UnknownNode(unknown))
    );
    assert_eq!(
        b.replace(unknown, Node::Hole),
        Err(EditError::UnknownNode(unknown))
    );

    b.replace(
        b.root(),
        Node::List {
            entries: vec![neg, two],
        },
    )
    .unwrap();
    assert_eq!(b.to_text(), "[-(1), 2]");
    assert_eq!(b.parent(neg), Some(b.root()));

    // Making `neg` part of what it holds.
    assert_eq!(
        b.replace(one, Node::List { entries: vec![neg] }),
        Err(EditError::Attached(neg))
    );

    // A free subtree holding the node it would replace.
    let mut c = Structure::new();
    let x = c.add(Node::Int(1)).unwrap();
    let free = c
        .add(Node::Unary {
            op: UnaryOp::Neg,
            operand: x,
        })
        .unwrap();
    assert_eq!(
        c.replace(
            x,
            Node::List {
                entries: vec![free]
            }
        ),
        Err(EditError::Cycle(free))
    );

    // Replacing a node frees its parts for reuse.
    b.replace(b.root(), Node::List { entries: vec![two] })
        .unwrap();
    assert_eq!(b.parent(neg), None);
    b.replace(
        b.root(),
        Node::List {
            entries: vec![two, neg],
        },
    )
    .unwrap();
    assert_eq!(b.to_text(), "[2, -(1)]");

    // A new root, whose old root is then free.
    let old = b.root();
    let top = b.add(Node::Name("wits".into())).unwrap();
    b.set_root(top).unwrap();
    assert_eq!(b.to_text(), "wits");
    assert_eq!(b.set_root(two), Err(EditError::Attached(two)));
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
    const FUNCTIONS: &[&str] = &["min", "max", "lookup", "count", "f"];
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
            0 => Node::Hole,
            1 => Node::Int(rng.below(200) as i64 - 100),
            2 => Node::Bool(rng.below(2) == 0),
            3 => Node::String(
                rng.pick(&["", "Brawl (bar)", "a\"b", "\\", "é ünï"])
                    .to_string(),
            ),
            _ => Node::Name(rng.pick(NAMES).to_string()),
        }
    } else {
        match rng.below(8) {
            0 => Node::Unary {
                op: *rng.pick(&[UnaryOp::Neg, UnaryOp::Not]),
                operand: kids(rng, b, 1)[0],
            },
            1 | 2 => {
                let k = kids(rng, b, 2);
                Node::Binary {
                    op: *rng.pick(&BinaryOp::ALL),
                    left: k[0],
                    right: k[1],
                }
            }
            3 => {
                let n = rng.below(3);
                Node::Call {
                    function: rng.pick(FUNCTIONS).to_string(),
                    args: kids(rng, b, n),
                }
            }
            4 => {
                let n = rng.below(3) + 1;
                let k = kids(rng, b, n);
                Node::Method {
                    receiver: k[0],
                    name: (rng.below(5) != 0).then(|| rng.pick(MEMBERS).to_string()),
                    args: k[1..].to_vec(),
                }
            }
            5 => Node::Field {
                receiver: kids(rng, b, 1)[0],
                name: (rng.below(5) != 0).then(|| rng.pick(MEMBERS).to_string()),
            },
            6 => {
                let k = kids(rng, b, 2);
                Node::Index {
                    receiver: k[0],
                    index: k[1],
                }
            }
            _ => {
                let n = rng.below(4);
                Node::List {
                    entries: kids(rng, b, n),
                }
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
