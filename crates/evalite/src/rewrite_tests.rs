//! Tests of rewrites: a path replaced by another in text and structures,
//! keeping everything else as it was written.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::*;

fn env() -> Environment {
    let mut env = Environment::new();
    let dice = Type::record([("dice", Type::int())]);

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
    env.value("o", Type::option(dice.clone())).unwrap();
    env.function("roll", [Type::int()], Type::int()).unwrap();
    env.function("f", [Type::int()], dice).unwrap();
    env
}

/// `source` rewritten from `from` to `to`, unchecked, with the count.
fn text(source: &str, from: Path, to: Path) -> (String, usize) {
    rewrite(source, &env(), &Rewrite::new(from, to), &Options::new()).unwrap()
}

fn v(name: &str) -> Path {
    Path::value(name)
}

#[test]
fn renames_values_keeping_the_formatting() {
    assert_eq!(
        text("a +  b*(a)  - Int::min( a ,1 )", v("a"), v("c")),
        ("c +  b*(c)  - Int::min( c ,1 )".to_owned(), 3)
    );
    assert_eq!(text("r . x", v("r"), v("q")), ("q . x".to_owned(), 1));
    assert_eq!(text("n+b", v("b"), v("bb")), ("n+bb".to_owned(), 1));
}

#[test]
fn renames_host_functions() {
    assert_eq!(
        text(
            "roll(roll(a)) + f(1).dice",
            Path::function("roll"),
            Path::function("dice")
        ),
        ("dice(dice(a)) + f(1).dice".to_owned(), 2)
    );
    assert_eq!(
        text(
            "f( 1 ).dice",
            Path::function("f").field("dice"),
            Path::function("f").field("d")
        ),
        ("f( 1 ).d".to_owned(), 1)
    );
}

#[test]
fn renames_fields_along_a_path() {
    let x = v("r").field("x");
    assert_eq!(
        text(
            r#"r.x + r["x"] + (r) . x + r.x.abs()"#,
            x.clone(),
            v("r").field("w")
        ),
        (r#"r.w + r["w"] + (r) . w + r.w.abs()"#.to_owned(), 4)
    );
    assert_eq!(
        text("r.x + a", x.clone(), v("p").field("w")),
        ("p.w + a".to_owned(), 1)
    );

    let z = v("r").field("x y").field("z");
    assert_eq!(
        text(r#"r["x y"].z"#, z, v("r").field("x y").field("zz")),
        (r#"r["x y"].zz"#.to_owned(), 1)
    );
}

#[test]
fn fields_switch_spelling_when_they_must() {
    let x = v("r").field("x");
    assert_eq!(
        text("r.x + r .  x", x.clone(), v("r").field("x \"y\"")),
        (r#"r["x \"y\""] + r ["x \"y\""]"#.to_owned(), 2)
    );
    assert_eq!(
        text("r.x", x.clone(), v("r").field("true")),
        (r#"r["true"]"#.to_owned(), 1)
    );
    assert_eq!(
        text("o!.dice", v("o").field("dice"), v("o").field("Brawl (bar)")),
        (r#"o!["Brawl (bar)"]"#.to_owned(), 1)
    );

    let mut new = Environment::new();
    new.value("r", Type::record([("a b", Type::int())]))
        .unwrap();
    let spaced = Rewrite::new(x.clone(), v("r").field("a b")).check(&new);
    let (out, _) = rewrite("r.x + r .  x", &env(), &spaced, &Options::new()).unwrap();
    assert_eq!(out, r#"r["a b"] + r ["a b"]"#);

    // A quoted field stays quoted.
    let y = v("r").field("x y");
    assert_eq!(
        text(r#"r["x y"].z"#, y, v("r").field("w")),
        (r#"r["w"].z"#.to_owned(), 1)
    );
}

#[test]
fn keys_and_indexes_keep_how_they_are_written() {
    let key = |k| v("m").int_key(k);
    assert_eq!(
        text(
            "m[0x1A].dice + m[26].dice + m.get( (0X1a) )!.dice",
            key(26),
            key(0xBEEF)
        ),
        (
            "m[0xBEEF].dice + m[48879].dice + m.get( (0Xbeef) )!.dice".to_owned(),
            3
        )
    );
    assert_eq!(
        text("m[-0x2].dice + m[2].dice", key(-2), key(5)),
        ("m[0x5].dice + m[2].dice".to_owned(), 1)
    );
    assert_eq!(
        text("Map::get(m, 7)!.dice", key(7), key(-7)),
        ("Map::get(m, -7)!.dice".to_owned(), 1)
    );
    assert_eq!(
        text(
            r#"s["brawl"] + s.get("brawl")!"#,
            v("s").string_key("brawl"),
            v("s").string_key("a \\ b")
        ),
        (r#"s["a \\ b"] + s.get("a \\ b")!"#.to_owned(), 2)
    );
    assert_eq!(
        text("l[0x0] + l[1] + l[n]", v("l").index(0), v("l").index(15)),
        ("l[0xF] + l[1] + l[n]".to_owned(), 1)
    );
}

#[test]
fn extreme_keys_are_checked() {
    let env = env();
    let rekey = |to: i64| Rewrite::new(v("m").int_key(1), v("m").int_key(to)).check(&env);

    let (out, _) = rewrite("m[0x1].dice", &env, &rekey(i64::MAX), &Options::new()).unwrap();
    assert_eq!(out, "m[0x7FFFFFFFFFFFFFFF].dice");

    // `i64::MIN` has no text as an expression, so the check refuses it.
    for source in ["m[1].dice", "m[0x1].dice"] {
        let error = rewrite(source, &env, &rekey(i64::MIN), &Options::new()).unwrap_err();
        assert!(format!("{error:#}").contains("too large"), "{error:#}");

        let mut s = Structure::from_text(source).unwrap();
        assert!(s.rewrite(&env, &rekey(i64::MIN), &Options::new()).is_err());
        assert_eq!(s.to_text(), source);
    }
}

#[test]
fn whole_paths_and_paths_past_them() {
    // Everything past `m[1]` is kept; `m[2]` and `m[n]` are not it.
    let (out, n) = text(
        "m[1].dice + m[2].dice + m[n].dice + m.len()",
        v("m").int_key(1).field("dice"),
        v("q").int_key(9).field("d"),
    );
    assert_eq!(out, "q[9].d + m[2].dice + m[n].dice + m.len()");
    assert_eq!(n, 1);

    // A path through a method keeps the method.
    assert_eq!(
        text(
            "o.unwrap_or(f(a)).dice",
            v("o").field("dice"),
            v("p").field("e")
        ),
        ("p.unwrap_or(f(a)).e".to_owned(), 1)
    );
}

#[test]
fn no_ops() {
    assert_eq!(text("a + b", v("c"), v("d")), ("a + b".to_owned(), 0));
    assert_eq!(text("a +  a", v("a"), v("a")), ("a +  a".to_owned(), 0));
    assert_eq!(
        text("l[n]", v("l").index(0), v("l").index(1)),
        ("l[n]".to_owned(), 0)
    );
    assert_eq!(
        text("roll(1)", v("roll"), v("dice")),
        ("roll(1)".to_owned(), 0)
    );
}

#[test]
fn checked_rewrites_must_compile() {
    let mut new = env();
    new.value("c", Type::int()).unwrap();

    let rename = Rewrite::new(v("a"), v("c"));
    let checked = rename.clone().check(&new);
    let (out, n) = rewrite("a + b", &env(), &checked, &Options::new()).unwrap();
    assert_eq!((out.as_str(), n), ("c + b", 1));

    let to_bool = Rewrite::new(v("a"), v("missing")).check(&new);
    let error = rewrite("a + b", &env(), &to_bool, &Options::new()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the rewritten expression does not compile"
    );
    assert!(format!("{error:#}").contains("missing"));
    assert!(std::error::Error::source(&error).is_some());

    // The check holds for no-ops too.
    let error = rewrite(
        "r.x",
        &env(),
        &Rewrite::new(v("a"), v("c")).check(&Environment::new()),
        &Options::new(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the rewritten expression does not compile"
    );
}

#[test]
fn refusals() {
    let refuse = |from: Path, to: Path| {
        rewrite("a", &env(), &Rewrite::new(from, to), &Options::new())
            .unwrap_err()
            .to_string()
    };

    assert_eq!(
        refuse(v("a"), v("a").field("x")),
        "cannot rewrite `a` to `a.x`, which starts at another kind of name or has other kinds of steps"
    );
    assert_eq!(
        refuse(v("m").int_key(1), v("m").index(1)),
        "cannot rewrite `m[1]` to `m[1]`, which starts at another kind of name or has other kinds of steps"
    );
    assert_eq!(
        refuse(v("m").int_key(1), v("m").string_key("x")),
        "cannot rewrite `m[1]` to `m[\"x\"]`, which starts at another kind of name or has other kinds of steps"
    );
    assert_eq!(
        refuse(Path::function("roll"), v("roll")),
        "cannot rewrite `roll()` to `roll`, which starts at another kind of name or has other kinds of steps"
    );
    assert!(refuse(v("a"), v("A")).starts_with("`A` is not a valid name"));
    assert!(refuse(v("a"), v("true")).starts_with("`true` is not a valid name"));
    assert!(refuse(v("a"), v("x y")).starts_with("`x y` is not a valid name"));

    let error = rewrite(
        "r.y",
        &env(),
        &Rewrite::new(v("a"), v("c")),
        &Options::new(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "the expression does not compile");
    assert!(format!("{error:#}").contains("no field \"y\""));
}

#[test]
fn starts_with() {
    let refs = references("m[1].dice + m[n].dice", &env(), &Options::new()).unwrap();
    assert!(refs[0].starts_with(&v("m")));
    assert!(refs[0].starts_with(&v("m").int_key(1).field("dice")));
    assert!(!refs[0].starts_with(&v("m").int_key(1).field("dice").field("e")));
    assert!(!refs[0].starts_with(&v("m").string_key("1")));
    assert!(!refs[1].starts_with(&v("m").int_key(1)));
    assert!(!refs[0].starts_with(&Path::function("m")));
}

#[test]
fn structures_rewrite_their_nodes() {
    let env = env();
    let options = Options::new();

    let edit = |source: &str, from: Path, to: Path| {
        let mut s = Structure::from_text(source).unwrap();
        let n = s.rewrite(&env, &Rewrite::new(from, to), &options).unwrap();
        // What it becomes is what its text would read back as.
        assert_eq!(Structure::from_text(&s.to_text()).unwrap(), s);
        (s.to_text(), n)
    };

    let source = "m[0x1A].dice + roll(r.x) + r[\"x y\"].z + l[3] + m[26].dice";
    assert_eq!(
        edit(
            source,
            v("m").int_key(26).field("dice"),
            v("m").int_key(-0xff).field("d")
        ),
        (
            "m[-0xFF].d + roll(r.x) + r[\"x y\"].z + l[3] + m[-255].d".to_owned(),
            2
        )
    );
    assert_eq!(
        edit(source, Path::function("roll"), Path::function("dice")),
        (
            "m[0x1A].dice + dice(r.x) + r[\"x y\"].z + l[3] + m[26].dice".to_owned(),
            1
        )
    );
    assert_eq!(
        edit(source, v("r").field("x"), v("r").field("x x")),
        (
            "m[0x1A].dice + roll(r[\"x x\"]) + r[\"x y\"].z + l[3] + m[26].dice".to_owned(),
            1
        )
    );
    assert_eq!(
        edit(source, v("r").field("x y"), v("rr").field("w")),
        (
            "m[0x1A].dice + roll(r.x) + rr[\"w\"].z + l[3] + m[26].dice".to_owned(),
            1
        )
    );
    assert_eq!(
        edit(source, v("l").index(3), v("l").index(-1)),
        (
            "m[0x1A].dice + roll(r.x) + r[\"x y\"].z + l[-1] + m[26].dice".to_owned(),
            1
        )
    );
    assert_eq!(
        edit(
            r#"s["k"] + o!.dice"#,
            v("s").string_key("k"),
            v("s").string_key("j")
        ),
        (r#"s["j"] + o!.dice"#.to_owned(), 1)
    );
}

#[test]
fn structures_are_unchanged_when_refused() {
    let env = env();
    let options = Options::new();

    let mut s = Structure::from_text("a + r.x").unwrap();
    let before = s.clone();

    let checked = Rewrite::new(v("a"), v("c")).check(&env);
    assert_eq!(
        s.rewrite(&env, &checked, &options).unwrap_err().to_string(),
        "the rewritten expression does not compile"
    );
    assert_eq!(s, before);

    let shape = Rewrite::new(v("a"), Path::function("a"));
    assert!(s.rewrite(&env, &shape, &options).is_err());
    assert_eq!(s, before);

    let mut hole = Structure::from_text("a + _").unwrap();
    let error = hole
        .rewrite(&env, &Rewrite::new(v("a"), v("c")), &options)
        .unwrap_err();
    assert_eq!(error.to_string(), "the expression does not compile");
    assert_eq!(hole.to_text(), "a + _");

    assert_eq!(
        s.rewrite(&env, &Rewrite::new(v("q"), v("c")), &options)
            .unwrap(),
        0
    );
    assert_eq!(s, before);
}

#[test]
fn long_expressions_do_not_recurse() {
    let source = vec!["r.x"; 10_000].join(" + ");
    let options = Options::new().max_len(usize::MAX);
    let rename = Rewrite::new(v("r").field("x"), v("r").field("y"));
    let (out, n) = rewrite(&source, &env(), &rename, &options).unwrap();
    assert_eq!(n, 10_000);
    assert_eq!(out, vec!["r.y"; 10_000].join(" + "));

    let mut s = Structure::from_text_with(&source, &options).unwrap();
    assert_eq!(s.rewrite(&env(), &rename, &options).unwrap(), 10_000);
    assert_eq!(s.to_text(), out);
}
