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
    assert_eq!(texts("[1, 2]")[0], (Punctuation, "["));
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
