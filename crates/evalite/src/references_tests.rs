//! Tests of references: every name an expression uses, with the typed path
//! it reads, and where each part is written.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::*;

fn env() -> Environment {
    let mut env = Environment::new();
    let dice = Type::record([("dice", Type::int())]);
    let c = Type::record([("c", Type::int())]);

    env.value("a", Type::int()).unwrap();
    env.value("b", Type::int()).unwrap();
    env.value("n", Type::int()).unwrap();
    env.value(
        "r",
        Type::record([
            ("x", Type::int()),
            ("x y", Type::record([("z", Type::int())])),
        ]),
    )
    .unwrap();
    env.value("m", Type::map(Type::int(), dice.clone()))
        .unwrap();
    env.value("s", Type::map(Type::string(), Type::int()))
        .unwrap();
    env.value("l", Type::list(Type::int())).unwrap();
    env.value("ls", Type::list(dice.clone())).unwrap();
    env.value("o", Type::option(c.clone())).unwrap();
    env.value("p", c).unwrap();
    env.function("roll", [Type::int()], Type::int()).unwrap();
    env.function("f", [Type::int()], dice).unwrap();
    env
}

/// Each reference of `source`, written as its name and steps, each part
/// followed by the text its span covers when that is not the part itself.
fn show(source: &str) -> Vec<String> {
    let refs = references(source, &env(), &Options::new()).unwrap();
    render(source, &refs)
}

fn render(source: &str, refs: &[Reference]) -> Vec<String> {
    let text = |span: Span<u32>| source.get(span.range()).unwrap();

    refs.iter()
        .map(|r| {
            assert_eq!(text(r.span()), r.name());

            let mut out = match r.kind() {
                ReferenceKind::Function => format!("{}()", r.name()),
                _ => r.name().to_owned(),
            };

            for step in r.steps() {
                let at = text(step.span());

                let (part, plain) = match step.kind() {
                    StepKind::Field => {
                        let name = step.name().unwrap();
                        (format!(".{name}"), name.to_owned())
                    }
                    StepKind::Key => {
                        let key = step.key().unwrap().to_string();
                        (format!("[key {key}]"), key)
                    }
                    StepKind::Index => {
                        let index = step.index().unwrap().to_string();
                        (format!("[{index}]"), index)
                    }
                    _ => ("[?]".to_owned(), String::new()),
                };

                out.push_str(&part);

                if at != plain {
                    out.push_str(&format!("<{at}>"));
                }
            }

            out
        })
        .collect()
}

#[test]
fn globals_in_the_order_written() {
    assert_eq!(show("a + b * a"), ["a", "b", "a"]);
    assert_eq!(show("-a < (b)"), ["a", "b"]);
    assert_eq!(show("1 + 2"), Vec::<String>::new());
    // The option literals are no names.
    assert_eq!(show("[None, Some(a)][0] == Some(b)"), ["a", "b"]);
}

#[test]
fn fields_by_either_spelling() {
    assert_eq!(show("r.x"), ["r.x"]);
    assert_eq!(show(r#"r["x y"].z"#), [r#"r.x y<"x y">.z"#]);
    assert_eq!(show(r#"r["x"] + (r).x"#), [r#"r.x<"x">"#, "r.x"]);
}

#[test]
fn map_keys_and_list_indexes() {
    assert_eq!(show("m[1].dice"), ["m[key 1].dice"]);
    assert_eq!(show("m[0x1A].dice"), ["m[key 26]<0x1A>.dice"]);
    assert_eq!(show("m[-2].dice"), ["m[key -2].dice"]);
    assert_eq!(show("m[(3)].dice"), ["m[key 3].dice"]);
    assert_eq!(show(r#"s["brawl"]"#), [r#"s[key "brawl"]"#]);
    assert_eq!(show("l[0] + l[(2)]"), ["l[0]", "l[2]"]);
    assert_eq!(show("ls[0x1].dice"), ["ls[1]<0x1>.dice"]);
}

#[test]
fn computed_steps_are_dynamic() {
    assert_eq!(show("l[n]"), ["l[?]<n>", "n"]);
    assert_eq!(show("m[n + 1].dice"), ["m[?]<n + 1>.dice", "n"]);
    assert_eq!(show("ls[(a)].dice"), ["ls[?]<a>.dice", "a"]);
    assert_eq!(show("l[-(1)]"), ["l[?]<-(1)>"]);
    assert_eq!(show("l[l[0]]"), ["l[?]<l[0]>", "l[0]"]);
}

#[test]
fn paths_continue_through_what_keeps_the_value() {
    assert_eq!(show("o.unwrap_or(p).c"), ["o.c", "p"]);
    assert_eq!(show("o!.c"), ["o.c"]);
    assert_eq!(show("m.get(0x1)!.dice"), ["m[key 1]<0x1>.dice"]);
    assert_eq!(show(r#"s.get("k").unwrap_or(0)"#), [r#"s[key "k"]"#]);
    assert_eq!(show("ls.get(n)!.dice"), ["ls[?]<n>.dice", "n"]);
    assert_eq!(show("Map::get(m, 7)!.dice"), ["m[key 7].dice"]);
    assert_eq!(show("Option::unwrap_or(o, p).c"), ["o.c", "p"]);
    assert_eq!(show("List::get(l, 1).unwrap_or(0)"), ["l[1]"]);
}

#[test]
fn paths_end_where_the_value_does() {
    assert_eq!(show("l.len() + m.len()"), ["l", "m"]);
    assert_eq!(show("r.x.abs().max(a)"), ["r.x", "a"]);
    assert_eq!(show("o.is_some()"), ["o"]);
    assert_eq!(show("Int::min(r.x, a)"), ["r.x", "a"]);
    assert_eq!(show("[p, o!][0].c"), ["p", "o"]);
}

#[test]
fn host_functions_are_references() {
    assert_eq!(show("roll(r.x)"), ["roll()", "r.x"]);
    assert_eq!(show("f(a).dice + roll(1)"), ["f().dice", "a", "roll()"]);

    let refs = references("roll(1)", &env(), &Options::new()).unwrap();
    assert_eq!(refs[0].kind(), ReferenceKind::Function);
    assert_eq!(refs[0].span().range(), 0..4);
    assert_eq!(refs[0].node(), None);
}

#[test]
fn only_what_compiles_has_references() {
    let errors = references("r.y + l[true]", &env(), &Options::new()).unwrap_err();
    assert_eq!(errors.len(), 2);

    let errors = references("m[true]", &env(), &Options::new()).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(errors.to_string().ends_with("found Bool"));

    assert!(references("l[", &env(), &Options::new()).is_err());
    assert!(references("(((a)))", &env(), &Options::new().max_depth(2)).is_err());
}

#[test]
fn structures_name_the_node_of_each_part() {
    let s = Structure::from_text(r#"m[-0x2].dice + r["x y"].z + roll(l[n])"#).unwrap();
    let text = s.to_text();
    let refs = s.references(&env(), &Options::new()).unwrap();

    assert_eq!(
        render(&text, &refs),
        [
            "m[key -2]<-0x2>.dice",
            r#"r.x y<"x y">.z"#,
            "roll()",
            "l[?]<n>",
            "n"
        ]
    );

    let node = |id: Option<NodeId>| s.get(id.unwrap()).unwrap();

    let [m, r, roll, l, n] = refs.as_slice() else {
        panic!()
    };

    assert_eq!(node(m.node()), &Node::name("m"));
    let [key, dice] = m.steps() else { panic!() };
    assert_eq!(node(key.node()).as_int(), Some(-2));
    let (receiver, name) = node(dice.node()).as_field().unwrap();
    assert_eq!(name, Some("dice"));
    assert_eq!(
        s.get(receiver).unwrap().as_index().map(|(_, k)| k),
        key.node()
    );

    let [quoted, z] = r.steps() else { panic!() };
    assert_eq!(node(quoted.node()).as_string(), Some("x y"));
    assert_eq!(node(z.node()).as_field().unwrap().1, Some("z"));

    assert_eq!(node(roll.node()).as_call().unwrap().0, "roll");
    assert_eq!(node(l.node()), &Node::name("l"));
    assert_eq!(l.steps()[0].node(), n.node());
    assert_eq!(node(n.node()), &Node::name("n"));
}

#[test]
fn structures_through_parentheses_and_methods() {
    let s = Structure::from_text("(o!).c + m.get((0x5))!.dice + (a + b).abs()").unwrap();
    let text = s.to_text();
    let refs = s.references(&env(), &Options::new()).unwrap();
    assert_eq!(
        render(&text, &refs),
        ["o.c", "m[key 5]<0x5>.dice", "a", "b"]
    );

    for r in &refs {
        assert!(r.node().is_some());
        assert!(r.steps().iter().all(|step| step.node().is_some()));
    }

    assert_eq!(
        s.get(refs[1].steps()[0].node().unwrap()).unwrap().as_int(),
        Some(5)
    );
}

#[test]
fn structures_must_compile() {
    let hole = Structure::from_text("m[_].dice").unwrap();
    let errors = hole.references(&env(), &Options::new()).unwrap_err();
    assert_eq!(errors.len(), 1);

    let wrong = Structure::from_text("r.y").unwrap();
    let errors = wrong.references(&env(), &Options::new()).unwrap_err();
    assert_eq!(errors.iter().next().unwrap().node(), wrong.root());
}

#[test]
fn long_expressions_do_not_recurse() {
    let source = vec!["r.x"; 10_000].join(" + ");
    let refs = references(&source, &env(), &Options::new().max_len(usize::MAX)).unwrap();
    assert_eq!(refs.len(), 10_000);
    assert!(refs.iter().all(|r| r.steps().len() == 1));

    let source = format!("{}0{}", "l[".repeat(30), "]".repeat(30));
    let refs = references(&source, &env(), &Options::new()).unwrap();
    assert_eq!(refs.len(), 30);
    assert_eq!(refs[29].steps()[0].kind(), StepKind::Index);
    assert_eq!(refs[0].steps()[0].kind(), StepKind::Dynamic);
}
