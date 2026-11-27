#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice
)]

use crate::highlight::{Class, Token, highlight};

fn texts(source: &str) -> Vec<(Class, &str)> {
    highlight(source)
        .map(|t| (t.class, &source[t.span]))
        .collect()
}

fn covers(source: &str) {
    let mut at = 0;

    for Token { span, .. } in highlight(source) {
        assert_eq!(span.start, at, "{source:?}");
        assert!(span.end > span.start, "{source:?}");
        at = span.end;
    }

    assert_eq!(at, source.len(), "{source:?}");
}

#[test]
fn each_class() {
    use Class::*;

    assert_eq!(
        texts("a.b(1) == \"s\" && true"),
        [
            (Ident, "a"),
            (Punctuation, "."),
            (Member, "b"),
            (Punctuation, "("),
            (Number, "1"),
            (Punctuation, ")"),
            (Whitespace, " "),
            (Operator, "=="),
            (Whitespace, " "),
            (String, "\"s\""),
            (Whitespace, " "),
            (Operator, "&&"),
            (Whitespace, " "),
            (Keyword, "true"),
        ]
    );

    assert_eq!(texts("false")[0].0, Keyword);
    assert_eq!(texts("None")[0], (Keyword, "None"));
    assert_eq!(texts("Some(1)")[0], (Keyword, "Some"));
    assert_eq!(texts("[Some(None)]")[3], (Keyword, "None"));
    // A field or method of that name is still a member.
    assert_eq!(texts("a.None")[2], (Member, "None"));
    assert_eq!(texts("Somewhere")[0], (Ident, "Somewhere"));
    assert_eq!(texts("[1, 2]")[0], (Punctuation, "["));
}

#[test]
fn path() {
    use Class::*;

    assert_eq!(
        texts("Int :: min(a)"),
        [
            (Type, "Int"),
            (Whitespace, " "),
            (Punctuation, "::"),
            (Whitespace, " "),
            (Ident, "min"),
            (Punctuation, "("),
            (Ident, "a"),
            (Punctuation, ")"),
        ]
    );

    // Before one `:` it is a record literal's field.
    assert_eq!(texts("Int:")[0], (Member, "Int"));
}

#[test]
fn member_after_whitespace() {
    assert_eq!(texts("a. b")[3], (Class::Member, "b"));
    assert_eq!(texts("a . b")[4], (Class::Member, "b"));
    assert_eq!(texts("a.(b")[3], (Class::Ident, "b"));
}

#[test]
fn errors_recover() {
    use Class::*;

    assert_eq!(
        texts("1 = 2 # é 3"),
        [
            (Number, "1"),
            (Whitespace, " "),
            (Error, "="),
            (Whitespace, " "),
            (Number, "2"),
            (Whitespace, " "),
            (Error, "#"),
            (Whitespace, " "),
            (Error, "é"),
            (Whitespace, " "),
            (Number, "3"),
        ]
    );

    assert_eq!(texts("\"abc"), [(Error, "\"abc")]);
    assert_eq!(
        texts("\"a\\qb\" 1"),
        [(Error, "\"a\\qb\""), (Whitespace, " "), (Number, "1")]
    );
    assert_eq!(texts("\"a\\"), [(Error, "\"a\\")]);
}

#[test]
fn covers_source() {
    for source in [
        "",
        " ",
        "wits + 1",
        "ünï + \"çode\" ✓",
        "\"unterminated",
        "\"bad \\x escape\" + 1",
        "a & b | c = d",
        "\"é\\é",
        "(((",
    ] {
        covers(source);
    }
}

#[test]
fn empty() {
    assert_eq!(highlight("").count(), 0);
}

/// What the parser recovers from as text that is no token is what the
/// highlighter classes as an error, and the rest is classified as usual.
#[test]
fn recovered_errors() {
    use crate::{Options, Parsed, Syntax};

    for source in [
        "f(\"a\\qb\", 1 # 2)",
        "a = b && c | d",
        "x + \"abc",
        "1 + é + 2",
        "min(1, , 2) ; [1, 2",
    ] {
        covers(source);

        let errors = highlight(source)
            .filter(|t| t.class == Class::Error)
            .map(|t| t.span)
            .collect::<Vec<_>>();

        let Parsed { tree, .. } = crate::parse_with(source, &Options::new()).unwrap();
        let rejected = tree
            .walk()
            .filter(|n| n.first().is_none())
            .filter(|n| {
                // `=`, `&` and `|` alone stand for the operator doubling them.
                n.value() == Syntax::Invalid || matches!(&source[n.range()], "=" | "&" | "|")
            })
            .map(|n| n.range())
            .collect::<Vec<_>>();

        assert!(!errors.is_empty(), "{source:?}");
        assert_eq!(errors, rejected, "{source:?}");
    }
}

#[test]
fn hexadecimal_integers() {
    use Class::*;

    assert_eq!(
        texts("0x1F + 0Xab"),
        [
            (Number, "0x1F"),
            (Whitespace, " "),
            (Operator, "+"),
            (Whitespace, " "),
            (Number, "0Xab"),
        ]
    );

    // Without digits, `0x` is one rejected token, not a number and a name.
    assert_eq!(texts("0x")[0].1, "0x");
    assert_ne!(texts("0x")[0].0, Number);
    covers("0x");
    covers("0xg + 0x1F");
    covers("-0x10");
}

#[test]
fn maps() {
    use Class::*;

    assert_eq!(
        texts(r#"{0x1: a, "b": 2}"#),
        [
            (Punctuation, "{"),
            (Number, "0x1"),
            (Punctuation, ":"),
            (Whitespace, " "),
            (Ident, "a"),
            (Punctuation, ","),
            (Whitespace, " "),
            (String, r#""b""#),
            (Punctuation, ":"),
            (Whitespace, " "),
            (Number, "2"),
            (Punctuation, "}"),
        ]
    );

    // `:` and `::` are different tokens.
    assert_eq!(
        texts("Int::min"),
        [(Type, "Int"), (Punctuation, "::"), (Ident, "min")]
    );
    covers("{1: {2: a}}[1][2]");
    covers("{: } :: : {");
}

#[test]
fn records() {
    use Class::*;

    // A record literal's field is a member, its value a name.
    assert_eq!(
        texts("{boxes : a}"),
        [
            (Punctuation, "{"),
            (Member, "boxes"),
            (Whitespace, " "),
            (Punctuation, ":"),
            (Whitespace, " "),
            (Ident, "a"),
            (Punctuation, "}"),
        ]
    );
    covers("{a: {b: c}}.a.b");
}

#[test]
fn annotation() {
    use Class::*;

    assert_eq!(
        texts("None::<Map<String, {a: Int}>> < 1"),
        [
            (Keyword, "None"),
            (Punctuation, "::"),
            (Punctuation, "<"),
            (Type, "Map"),
            (Punctuation, "<"),
            (Type, "String"),
            (Punctuation, ","),
            (Whitespace, " "),
            (Punctuation, "{"),
            (Member, "a"),
            (Punctuation, ":"),
            (Whitespace, " "),
            (Type, "Int"),
            (Punctuation, "}"),
            (Punctuation, ">"),
            (Punctuation, ">"),
            (Whitespace, " "),
            (Operator, "<"),
            (Whitespace, " "),
            (Number, "1"),
        ]
    );

    covers("None::<List<");
}
