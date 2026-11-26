#![allow(
    clippy::arithmetic_side_effects,
    clippy::string_slice,
    clippy::indexing_slicing
)]

use syntree::{FlavorDefault, Node};

use crate::error::{Delimiter, ErrorKind, Found, Nesting};
use crate::lexer::span;
use crate::syntax::unescape;
use crate::*;

type N<'a> = Node<'a, Syntax, FlavorDefault>;

type Range = std::ops::Range<usize>;

/// `source` parsed with `options`, or its first diagnostic.
fn parse_with(source: &str, options: &Options) -> Result<Tree, Diagnostic> {
    crate::parse_with(source, options)
        .and_then(Parsed::complete)
        .map_err(|errors| errors.iter().next().unwrap().clone())
}

/// `source` parsed, or its first diagnostic.
fn parse(source: &str) -> Result<Tree, Diagnostic> {
    parse_with(source, &Options::new())
}

/// `source` parsed with recovery, and each diagnostic.
fn recover_with(source: &str, options: &Options) -> (Tree, Vec<Diagnostic>) {
    let Parsed { tree, reporter } = crate::parse_with(source, options).unwrap();
    let errors = reporter
        .finish()
        .map(|errors| errors.into_iter().collect())
        .unwrap_or_default();
    (tree, errors)
}

fn text<'s>(source: &'s str, node: N<'_>) -> &'s str {
    &source[node.range()]
}

/// The tree covers every byte, in order, and no node but the root starts
/// or ends with whitespace.
fn check_tree(source: &str, tree: &Tree) {
    let mut joined = String::new();
    let mut at = 0;

    for node in tree.walk() {
        if node.first().is_none() {
            assert_eq!(node.range().start, at, "{source:?}: gap before {node:?}");
            at = node.range().end;
            joined.push_str(text(source, node));
        } else if node.value() != Syntax::Root {
            for edge in [node.first(), node.last()].into_iter().flatten() {
                assert_ne!(
                    edge.value(),
                    Syntax::Whitespace,
                    "{source:?}: {:?} at {:?} has whitespace at an edge",
                    node.value(),
                    node.range()
                );
            }
        }
    }

    assert_eq!(joined, source);

    let roots = tree.children().collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].value(), Syntax::Root);
    assert_eq!(roots[0].range(), 0..source.len());
    assert_eq!(
        roots[0]
            .children()
            .filter(|n| n.value() != Syntax::Whitespace)
            .count(),
        1
    );
}

/// The levels of nesting the parser counts in the expression at `node`.
fn nesting(node: N<'_>) -> usize {
    let children = node.children().collect::<Vec<_>>();
    let max = |nodes: &[N<'_>]| nodes.iter().map(|n| nesting(*n)).max().unwrap_or(0);

    match node.value() {
        Syntax::Paren | Syntax::List | Syntax::Unary | Syntax::Args => 1 + max(&children),
        Syntax::Index => max(&children[..1]).max(1 + max(&children[1..])),
        _ => max(&children),
    }
}

fn render(source: &str, node: N<'_>) -> String {
    if node.value() == Syntax::Error {
        let parts = node
            .children()
            .filter(|n| n.value() != Syntax::Whitespace)
            .map(|n| render(source, n))
            .collect::<Vec<_>>();

        return if parts.is_empty() {
            "(error)".to_owned()
        } else {
            format!("(error {})", parts.join(" "))
        };
    }

    if node.first().is_none() {
        return text(source, node).to_owned();
    }

    let parts = node
        .children()
        .filter(|n| {
            !matches!(
                n.value(),
                Syntax::Whitespace
                    | Syntax::OpenParen
                    | Syntax::CloseParen
                    | Syntax::OpenBracket
                    | Syntax::CloseBracket
                    | Syntax::Comma
                    | Syntax::Dot
                    | Syntax::ColonColon
            )
        })
        .map(|n| render(source, n))
        .collect::<Vec<_>>();

    match node.value() {
        Syntax::Integer | Syntax::String | Syntax::Bool | Syntax::Name => {
            assert_eq!(parts.len(), 1);
            parts[0].clone()
        }
        Syntax::Binary => format!("({} {} {})", parts[1], parts[0], parts[2]),
        Syntax::Unary => format!("({} {})", parts[0], parts[1]),
        Syntax::Paren => format!("(paren {})", parts[0]),
        Syntax::List => format!("[{}]", parts.join(" ")),
        Syntax::Path => parts.join("::"),
        Syntax::Args => parts.join(" "),
        Syntax::Call => format!("(call {})", parts.join(" ").trim_end()),
        Syntax::Method => format!("(method {})", parts.join(" ").trim_end()),
        Syntax::Field => format!("(field {})", parts.join(" ")),
        Syntax::Index => format!("(index {})", parts.join(" ")),
        Syntax::Unwrap => format!("(unwrap {})", parts[0]),
        other => panic!("unexpected {other:?}"),
    }
}

/// `source` parsed and rendered as an s-expression.
fn sexp(source: &str) -> String {
    let tree = parse(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
    render_tree(source, &tree)
}

fn render_tree(source: &str, tree: &Tree) -> String {
    check_tree(source, tree);
    let root = tree.first().unwrap();
    let expr = root
        .children()
        .find(|n| n.value() != Syntax::Whitespace)
        .unwrap();
    render(source, expr)
}

fn table(cases: &[(&str, &str)]) {
    for (source, expected) in cases {
        assert_eq!(sexp(source), *expected, "{source:?}");
    }
}

#[test]
fn literals() {
    table(&[
        ("0", "0"),
        ("12", "12"),
        ("007", "007"),
        ("9223372036854775807", "9223372036854775807"),
        (r#""""#, r#""""#),
        (r#""Essence (1e)""#, r#""Essence (1e)""#),
        (r#""a \"b\" \\ é""#, r#""a \"b\" \\ é""#),
        ("true", "true"),
        ("false", "false"),
        ("a", "a"),
        ("_a1", "_a1"),
        ("Strength", "Strength"),
        ("true_ish", "true_ish"),
        ("falsey", "falsey"),
    ]);

    let tree = parse("true").unwrap();
    assert_eq!(tree.first().unwrap().first().unwrap().value(), Syntax::Bool);
    let tree = parse("truex").unwrap();
    assert_eq!(tree.first().unwrap().first().unwrap().value(), Syntax::Name);
}

#[test]
fn operators() {
    table(&[
        ("a + b", "(+ a b)"),
        ("a - b", "(- a b)"),
        ("a * b", "(* a b)"),
        ("a / b", "(/ a b)"),
        ("a % b", "(% a b)"),
        ("a == b", "(== a b)"),
        ("a != b", "(!= a b)"),
        ("a < b", "(< a b)"),
        ("a <= b", "(<= a b)"),
        ("a > b", "(> a b)"),
        ("a >= b", "(>= a b)"),
        ("a && b", "(&& a b)"),
        ("a || b", "(|| a b)"),
        ("-a", "(- a)"),
        ("!a", "(! a)"),
    ]);
}

#[test]
fn precedence_and_associativity() {
    table(&[
        ("1 + 2 * 3", "(+ 1 (* 2 3))"),
        ("1 * 2 + 3", "(+ (* 1 2) 3)"),
        ("a - b - c", "(- (- a b) c)"),
        ("a + b - c + d", "(+ (- (+ a b) c) d)"),
        ("a / b * c % d", "(% (* (/ a b) c) d)"),
        ("a || b || c", "(|| (|| a b) c)"),
        ("a && b && c", "(&& (&& a b) c)"),
        ("a || b && c", "(|| a (&& b c))"),
        ("a && b || c", "(|| (&& a b) c)"),
        ("a == b + 1", "(== a (+ b 1))"),
        ("a + 1 < b * 2", "(< (+ a 1) (* b 2))"),
        ("a < b || c >= d", "(|| (< a b) (>= c d))"),
        ("a < b && b < c", "(&& (< a b) (< b c))"),
        ("(a < b) == c", "(== (paren (< a b)) c)"),
        ("a == (b < c)", "(== a (paren (< b c)))"),
        ("-a * b", "(* (- a) b)"),
        ("-a + -b", "(+ (- a) (- b))"),
        ("a - -b", "(- a (- b))"),
        ("a--b", "(- a (- b))"),
        ("--a", "(- (- a))"),
        ("-!-a", "(- (! (- a)))"),
        ("!a && b", "(&& (! a) b)"),
        ("-a.b", "(- (field a b))"),
        ("-a.b(1)", "(- (method a b 1))"),
        ("-a[1]", "(- (index a 1))"),
        ("-(a + b)", "(- (paren (+ a b)))"),
        ("(a + b) * c", "(* (paren (+ a b)) c)"),
        ("a * (b + c)", "(* a (paren (+ b c)))"),
        ("1 + 2 * 3 - 4 / 5", "(- (+ 1 (* 2 3)) (/ 4 5))"),
        (
            "a || b && c == d + e * -f",
            "(|| a (&& b (== c (+ d (* e (- f))))))",
        ),
        (
            "a * b + c == d && e || f",
            "(|| (&& (== (+ (* a b) c) d) e) f)",
        ),
    ]);
}

#[test]
fn postfixes() {
    table(&[
        ("a.b", "(field a b)"),
        ("a.b.c", "(field (field a b) c)"),
        // `!` after a value unwraps it, binding like `.` and `[`.
        ("a!", "(unwrap a)"),
        ("a.b!", "(unwrap (field a b))"),
        ("a!.b", "(field (unwrap a) b)"),
        ("a![0]", "(index (unwrap a) 0)"),
        ("rating(1, l)!", "(unwrap (call rating 1 l))"),
        ("a!!", "(unwrap (unwrap a))"),
        ("-a!", "(- (unwrap a))"),
        ("!a!", "(! (unwrap a))"),
        ("(a + b)!", "(unwrap (paren (+ a b)))"),
        ("a! + 1", "(+ (unwrap a) 1)"),
        // `!=` is still one token.
        ("a!=1", "(!= a 1)"),
        ("a! == 1", "(== (unwrap a) 1)"),
        ("a.b()", "(method a b)"),
        ("a.b(1)", "(method a b 1)"),
        ("a.b(1, c)", "(method a b 1 c)"),
        ("a.b.c(1).d", "(field (method (field a b) c 1) d)"),
        (
            "x.unwrap_or(0).max(1)",
            "(method (method x unwrap_or 0) max 1)",
        ),
        (
            "some.field.unwrap_or(0)",
            "(method (field some field) unwrap_or 0)",
        ),
        ("a[1]", "(index a 1)"),
        ("a[1][2]", "(index (index a 1) 2)"),
        ("a[i + 1]", "(index a (+ i 1))"),
        (r#"m["k"]"#, r#"(index m "k")"#),
        ("a.b[1].c", "(field (index (field a b) 1) c)"),
        ("f(1).g", "(field (call f 1) g)"),
        ("f(1)[0]", "(index (call f 1) 0)"),
        ("(a).b", "(field (paren a) b)"),
        ("[1, 2][0]", "(index [1 2] 0)"),
        ("1.abs()", "(method 1 abs)"),
        (r#""x".len()"#, r#"(method "x" len)"#),
        ("true.not()", "(method true not)"),
        ("a.true_", "(field a true_)"),
        ("a.b + c.d", "(+ (field a b) (field c d))"),
        ("a[b[c]]", "(index a (index b c))"),
        ("a.b(c.d(e))", "(method a b (method c d e))"),
    ]);
}

#[test]
fn groups() {
    table(&[
        ("(a)", "(paren a)"),
        ("((a))", "(paren (paren a))"),
        ("[]", "[]"),
        ("[1]", "[1]"),
        ("[1, 2, 3]", "[1 2 3]"),
        ("[a + 1, -b]", "[(+ a 1) (- b)]"),
        ("[[1], []]", "[[1] []]"),
        ("f()", "(call f)"),
        ("f(a)", "(call f a)"),
        ("min(a, b)", "(call min a b)"),
        ("Int::min(a, b)", "(call Int::min a b)"),
        ("Int :: min(a)", "(call Int::min a)"),
        ("f(g(h(1)))", "(call f (call g (call h 1)))"),
        ("f([1, 2], \"t\")", "(call f [1 2] \"t\")"),
    ]);
}

#[test]
fn whitespace() {
    table(&[
        (" a ", "a"),
        ("\t\n a \r\n", "a"),
        ("a+b", "(+ a b)"),
        ("\t a \n + \r b", "(+ a b)"),
        ("a . b", "(field a b)"),
        ("a .b ( 1 )", "(method a b 1)"),
        ("f ( a , b )", "(call f a b)"),
        ("f\n(a)", "(call f a)"),
        ("a [ 1 ]", "(index a 1)"),
        ("- a", "(- a)"),
        ("( a )", "(paren a)"),
        ("[ ]", "[]"),
        ("f( )", "(call f)"),
        ("[ 1 ,2 , 3 ]", "[1 2 3]"),
        ("\u{a0}a\u{2003}+\u{3000}b", "(+ a b)"),
    ]);
}

/// Every form `crates/api/src/expr.rs` accepts.
#[test]
fn current_expressions() {
    table(&[
        ("12", "12"),
        ("  strength ", "strength"),
        ("dexterity + wits - 1", "(- (+ dexterity wits) 1)"),
        ("a - (b - c)", "(- a (paren (- b c)))"),
        ("-a", "(- a)"),
        (
            "min(wits, dexterity) + athletics",
            "(+ (call min wits dexterity) athletics)",
        ),
        ("max (a)", "(call max a)"),
        (
            "rating(blood_potency, [10, 11, 12 , 13])",
            "(call rating blood_potency [10 11 12 13])",
        ),
        (
            r#"rating(primal_urge, "Essence (1e)")"#,
            r#"(call rating primal_urge "Essence (1e)")"#,
        ),
        (
            r#"rating(blood_potency, "Vitae \"2e\"")"#,
            r#"(call rating blood_potency "Vitae \"2e\"")"#,
        ),
        (
            r#"specialties.athletics + health[2] + m["Brawl (bar)"]"#,
            r#"(+ (+ (field specialties athletics) (index health 2)) (index m "Brawl (bar)"))"#,
        ),
        (
            "count(health) + count( health , 1 + 1)",
            "(+ (call count health) (call count health (+ 1 1)))",
        ),
        (
            r#"boxes(health) - boxes(m["a b"])"#,
            r#"(- (call boxes health) (call boxes (index m "a b")))"#,
        ),
        ("min + 1", "(+ min 1)"),
        ("max(dexterity, wits, 7)", "(call max dexterity wits 7)"),
        ("-dexterity + 1", "(+ (- dexterity) 1)"),
        (
            "rating(blood_potency + 5, [10, 11, 12, 13])",
            "(call rating (+ blood_potency 5) [10 11 12 13])",
        ),
        ("rating(-wits, [10, 11])", "(call rating (- wits) [10 11])"),
        ("rating(0, [wits + 1])", "(call rating 0 [(+ wits 1)])"),
        ("9223372036854775807 + 1", "(+ 9223372036854775807 1)"),
        ("-9223372036854775807 - 2", "(- (- 9223372036854775807) 2)"),
        (
            "a + min(b, a) + rating(c, [a, d])",
            "(+ (+ a (call min b a)) (call rating c [a d]))",
        ),
        (
            "count(t, s) + boxes(u) + count(t)",
            "(+ (+ (call count t s) (call boxes u)) (call count t))",
        ),
        ("count(health, -4)", "(call count health (- 4))"),
        (
            "boxes(health) - count(health)",
            "(- (call boxes health) (call count health))",
        ),
        (
            &format!("{}1{}", "(".repeat(32), ")".repeat(32)),
            &format!("{}1{}", "(paren ".repeat(32), ")".repeat(32)),
        ),
    ]);
}

#[test]
fn node_spans() {
    let source = " min(a, b.c) + x[1] ";
    let tree = parse(source).unwrap();

    let spans = tree
        .walk()
        .filter(|n| n.first().is_some())
        .map(|n| (n.value(), text(source, n)))
        .collect::<Vec<_>>();

    use Syntax::*;

    assert_eq!(
        spans,
        [
            (Root, source),
            (Binary, "min(a, b.c) + x[1]"),
            (Call, "min(a, b.c)"),
            (Name, "min"),
            (Args, "(a, b.c)"),
            (Name, "a"),
            (Field, "b.c"),
            (Name, "b"),
            (Name, "c"),
            (Index, "x[1]"),
            (Name, "x"),
            (Integer, "1"),
        ]
    );
}

fn error(source: &str) -> (String, std::ops::Range<usize>) {
    error_with(source, &Options::new())
}

fn error_with(source: &str, options: &Options) -> (String, std::ops::Range<usize>) {
    let error = match parse_with(source, options) {
        Ok(..) => panic!("{source:?} parsed"),
        Err(error) => error,
    };

    (error.to_string(), error.span().range())
}

/// Each source has the one diagnostic given.
fn errors(cases: &[(&str, &str, Range)]) {
    for (source, message, range) in cases {
        let (_, errors) = recover_with(source, &Options::new());
        let errors = errors
            .iter()
            .map(|e| (e.to_string(), e.span().range()))
            .collect::<Vec<_>>();

        assert_eq!(
            errors,
            [((*message).to_owned(), range.clone())],
            "{source:?}"
        );
    }
}

#[test]
fn error_unexpected_character() {
    errors(&[
        ("a + é", "unexpected character 'é'", 4..6),
        ("é + a", "unexpected character 'é'", 0..2),
        ("#", "unexpected character '#'", 0..1),
        ("a ; b", "unexpected character ';'", 2..3),
        ("a @", "unexpected character '@'", 2..3),
        (
            "1.5",
            "expected a field or method name after `.`, found an integer",
            2..3,
        ),
    ]);

    assert_eq!(
        parse("é").unwrap_err().kind(),
        &ErrorKind::UnexpectedCharacter('é')
    );
}

#[test]
fn error_single_operator() {
    errors(&[
        ("a = b", "`=` is not an operator, did you mean `==`?", 2..3),
        ("a & b", "`&` is not an operator, did you mean `&&`?", 2..3),
        ("a | b", "`|` is not an operator, did you mean `||`?", 2..3),
    ]);
}

#[test]
fn error_strings() {
    errors(&[
        (
            "\"abc",
            "this string is never closed, expected a `\"` before the end",
            0..4,
        ),
        (
            "\"a\\",
            "this string is never closed, expected a `\"` before the end",
            0..3,
        ),
        (
            "\"a\\\"",
            "this string is never closed, expected a `\"` before the end",
            0..4,
        ),
        (
            r#""a\nb""#,
            "unknown escape `\\n` in a string, only `\\\"` and `\\\\` are allowed",
            2..4,
        ),
        (
            r#"x + "a\éb""#,
            "unknown escape `\\é` in a string, only `\\\"` and `\\\\` are allowed",
            6..9,
        ),
    ]);
}

#[test]
fn error_integer_too_large() {
    let message = "this integer is too large, the largest is 9223372036854775807";

    errors(&[
        ("99999999999999999999", message, 0..20),
        ("9223372036854775808", message, 0..19),
        ("-9223372036854775808", message, 1..20),
        ("a + 18446744073709551616", message, 4..24),
    ]);
}

#[test]
fn error_expected_expression() {
    errors(&[
        (
            "",
            "expected an expression, found the end of the expression",
            0..0,
        ),
        (
            "   ",
            "expected an expression, found the end of the expression",
            3..3,
        ),
        (
            "a +",
            "expected an expression, found the end of the expression",
            3..3,
        ),
        (
            "-",
            "expected an expression, found the end of the expression",
            1..1,
        ),
        ("+ a", "expected an expression, found `+`", 0..1),
        ("a * * b", "expected an expression, found `*`", 4..5),
        ("f(a,)", "expected an expression, found `)`", 4..5),
        ("[1,]", "expected an expression, found `]`", 3..4),
        ("()", "expected an expression, found `)`", 1..2),
        ("( )", "expected an expression, found `)`", 2..3),
        ("a[]", "expected an expression, found `]`", 2..3),
        (")", "expected an expression, found `)`", 0..1),
        ("]", "expected an expression, found `]`", 0..1),
        (",", "expected an expression, found `,`", 0..1),
        (".a", "expected an expression, found `.`", 0..1),
        ("a && || b", "expected an expression, found `||`", 5..7),
        ("(a + )", "expected an expression, found `)`", 5..6),
    ]);
}

#[test]
fn error_expected_operator() {
    let top = "expected an operator or the end of the expression, found";

    errors(&[
        ("a b", &format!("{top} a name"), 2..3),
        ("1 2", &format!("{top} an integer"), 2..3),
        ("1 (2)", &format!("{top} `(`"), 2..3),
        ("a true", &format!("{top} `true`"), 2..6),
        ("a false", &format!("{top} `false`"), 2..7),
        ("a \"s\"", &format!("{top} a string"), 2..5),
        // `a!` unwraps `a`, so the name after it is what is unexpected.
        ("a !b", &format!("{top} a name"), 3..4),
        ("a, b", &format!("{top} `,`"), 1..2),
        ("f(a) b", &format!("{top} a name"), 5..6),
        ("(a b)", "expected an operator or `)`, found a name", 3..4),
        (
            "f(a b)",
            "expected an operator, `,` or `)`, found a name",
            4..5,
        ),
        (
            "[1 2]",
            "expected an operator, `,` or `]`, found an integer",
            3..4,
        ),
        (
            "x[1 2]",
            "expected an operator or `]`, found an integer",
            4..5,
        ),
        (
            "f([1 2])",
            "expected an operator, `,` or `]`, found an integer",
            5..6,
        ),
        (
            "x.m(a b)",
            "expected an operator, `,` or `)`, found a name",
            6..7,
        ),
        ("(a, b)", "expected an operator or `)`, found `,`", 2..3),
        ("x[1, 2]", "expected an operator or `]`, found `,`", 3..4),
    ]);
}

#[test]
fn error_expected_name() {
    errors(&[
        (
            "a.",
            "expected a field or method name after `.`, found the end of the expression",
            2..2,
        ),
        (
            "a.1",
            "expected a field or method name after `.`, found an integer",
            2..3,
        ),
        (
            "a.(b)",
            "expected a field or method name after `.`, found `(`",
            2..3,
        ),
        (
            "a..b",
            "expected a field or method name after `.`, found `.`",
            2..3,
        ),
        (
            "a. true",
            "expected a field or method name after `.`, found `true`",
            3..7,
        ),
        (
            "a.\"b\"",
            "expected a field or method name after `.`, found a string",
            2..5,
        ),
    ]);
}

#[test]
fn error_paths() {
    errors(&[
        (
            "Int::",
            "expected a function name after `::`, found the end of the expression",
            5..5,
        ),
        (
            "Int::1 + a",
            "expected a function name after `::`, found an integer",
            5..6,
        ),
        (
            "Int::min + 1",
            "`Int::min` is a function, call it with `Int::min(...)`",
            0..8,
        ),
        (
            "Int :: min",
            "`Int::min` is a function, call it with `Int::min(...)`",
            0..10,
        ),
    ]);
}

/// Each source has one diagnostic, an unclosed group: its message, its
/// primary span at the opener and its secondary one where it had to close.
fn unclosed(cases: &[(&str, &str, Range, Range)]) {
    for (source, message, open, close) in cases {
        let (_, errors) = recover_with(source, &Options::new());
        let errors = errors
            .iter()
            .map(|e| {
                let labels = e
                    .labels()
                    .map(|l| (l.message().to_string(), l.span().map(|s| s.range())))
                    .collect::<Vec<_>>();
                (e.to_string(), e.span().range(), labels)
            })
            .collect::<Vec<_>>();

        let closer = if message.starts_with("expected `)`") {
            ')'
        } else {
            ']'
        };

        assert_eq!(
            errors,
            [(
                (*message).to_owned(),
                open.clone(),
                vec![
                    ((*message).to_owned(), Some(open.clone())),
                    (format!("expected `{closer}` here"), Some(close.clone())),
                ]
            )],
            "{source:?}"
        );
    }
}

#[test]
fn error_unclosed() {
    unclosed(&[
        (
            "(a",
            "expected `)` to close the `(` at byte 0, found the end of the expression",
            0..1,
            2..2,
        ),
        (
            "((a)",
            "expected `)` to close the `(` at byte 0, found the end of the expression",
            0..1,
            4..4,
        ),
        (
            "f(a",
            "expected `)` to close the `(` at byte 1, found the end of the expression",
            1..2,
            3..3,
        ),
        (
            "x.m(a, 1",
            "expected `)` to close the `(` at byte 3, found the end of the expression",
            3..4,
            8..8,
        ),
        (
            "[1, 2",
            "expected `]` to close the `[` at byte 0, found the end of the expression",
            0..1,
            5..5,
        ),
        (
            "x[1",
            "expected `]` to close the `[` at byte 1, found the end of the expression",
            1..2,
            3..3,
        ),
        (
            "(a]",
            "expected `)` to close the `(` at byte 0, found `]`",
            0..1,
            2..3,
        ),
        (
            "[1)",
            "expected `]` to close the `[` at byte 0, found `)`",
            0..1,
            2..3,
        ),
        (
            "f(a]",
            "expected `)` to close the `(` at byte 1, found `]`",
            1..2,
            3..4,
        ),
        (
            "x[1)",
            "expected `]` to close the `[` at byte 1, found `)`",
            1..2,
            3..4,
        ),
        (
            "f([1)",
            "expected `]` to close the `[` at byte 2, found `)`",
            2..3,
            4..5,
        ),
    ]);

    let (_, errors) = recover_with("f(a, (b]", &Options::new());
    let error = errors.iter().find(|e| e.span() == Span::new(5, 6)).unwrap();
    assert_eq!(
        error.kind(),
        &ErrorKind::Unclosed {
            open: Delimiter::Paren,
            close_span: Span::new(7, 8),
            found: Found::Token(Syntax::CloseBracket),
        }
    );
}

#[test]
fn error_unmatched() {
    errors(&[
        ("a)", "unmatched `)`, there is no group open to close", 1..2),
        ("a]", "unmatched `]`, there is no group open to close", 1..2),
        (
            "(a))",
            "unmatched `)`, there is no group open to close",
            3..4,
        ),
        (
            "f(a) + 1)",
            "unmatched `)`, there is no group open to close",
            8..9,
        ),
    ]);
}

#[test]
fn error_chained_comparison() {
    let message = "comparisons do not chain, combine them with `&&` or add parentheses";

    errors(&[
        ("a < b < c", message, 6..7),
        ("a == b != c", message, 7..9),
        ("a < b + 1 > c", message, 10..11),
        ("a <= b >= c", message, 7..9),
        ("x && a < b < c", message, 11..12),
        ("a < b == (c > d)", message, 6..8),
    ]);

    table(&[
        ("(a < b) < c", "(< (paren (< a b)) c)"),
        ("a < (b < c)", "(< a (paren (< b c)))"),
        ("a < b && (c > d)", "(&& (< a b) (paren (> c d)))"),
    ]);
}

#[test]
fn error_too_deep() {
    let deep =
        |open: &str, close: &str, n: usize| format!("{}1{}", open.repeat(n), close.repeat(n));

    let message = |what: &str, max: usize| {
        format!("this {what} nests too deeply, the limit is {max} levels of nesting")
    };

    assert_eq!(
        error(&deep("(", ")", 33)),
        (message("parenthesis", 32), 32..33)
    );
    assert_eq!(error(&deep("-", "", 33)), (message("`-`", 32), 32..33));
    assert_eq!(error(&deep("!", "", 33)), (message("`!`", 32), 32..33));
    assert_eq!(error(&deep("[", "]", 33)), (message("list", 32), 32..33));
    assert_eq!(error(&deep("f(", ")", 33)), (message("call", 32), 65..66));
    assert_eq!(error(&deep("a[", "]", 33)), (message("index", 32), 65..66));
    assert_eq!(
        error(&deep("a.m(", ")", 33)),
        (message("call", 32), 131..132)
    );
    assert!(parse(&deep("(", ")", 32)).is_ok());
    assert!(parse(&deep("-", "", 32)).is_ok());
    assert!(parse(&deep("f(", ")", 32)).is_ok());
    assert!(parse(&deep("a[", "]", 32)).is_ok());

    let two = Options::new().max_depth(2);
    assert_eq!(error_with("f(g(h(1)))", &two), (message("call", 2), 5..6));
    assert_eq!(error_with("[[[1]]]", &two), (message("list", 2), 2..3));
    assert_eq!(error_with("a[b[c[1]]]", &two), (message("index", 2), 5..6));
    assert_eq!(error_with("!!!a", &two), (message("`!`", 2), 2..3));
    assert_eq!(error_with("-(-a)", &two), (message("`-`", 2), 2..3));
    assert!(parse_with("f(g(1))", &two).is_ok());
    // Closed groups do not count towards later ones.
    assert!(parse_with("(1) + (2) + ((3))", &two).is_ok());
    assert!(parse_with("((1)).a((2))", &two).is_ok());

    let zero = Options::new().max_depth(0);
    assert!(parse_with("a + b * c.d.e(", &zero).is_err());
    assert!(parse_with("a + b * c.d.e", &zero).is_ok());
    assert!(parse_with("a || b && c == d", &zero).is_ok());
    assert_eq!(error_with("(a)", &zero), (message("parenthesis", 0), 0..1));
    assert_eq!(error_with("-a", &zero), (message("`-`", 0), 0..1));
    assert_eq!(error_with("f()", &zero), (message("call", 0), 1..2));
    assert_eq!(error_with("[]", &zero), (message("list", 0), 0..1));

    assert_eq!(
        parse(&deep("(", ")", 40)).unwrap_err().kind(),
        &ErrorKind::TooDeep {
            nesting: Nesting::Paren,
            max: 32
        }
    );
}

/// Very deep input is an error, or with the limit raised a tree, never a
/// stack overflow.
#[test]
fn very_deep() {
    let n = 100_000;
    let roomy = Options::new().max_len(usize::MAX);
    let parens = format!("{}1{}", "(".repeat(n), ")".repeat(n));

    assert_eq!(
        error_with(&parens, &roomy),
        (
            "this parenthesis nests too deeply, the limit is 32 levels of nesting".to_owned(),
            32..33
        )
    );
    // The groups left open are reported at their openers, before it, so
    // this finds the one `TooDeep` among them.
    let too_deep = |source: &str| {
        let (_, errors) = recover_with(source, &roomy);
        errors
            .iter()
            .filter(|e| matches!(e.kind(), ErrorKind::TooDeep { .. }))
            .map(|e| e.span().range())
            .collect::<Vec<_>>()
    };

    assert_eq!(too_deep(&"(".repeat(n)), vec![32..33]);
    assert_eq!(error_with(&"-".repeat(n), &roomy).1, 32..33);
    assert_eq!(too_deep(&"f(".repeat(n)), vec![65..66]);

    let unlimited = Options::new().max_depth(usize::MAX).max_len(usize::MAX);
    let tree = parse_with(&parens, &unlimited).unwrap();
    let depth = tree.walk().with_depths().map(|(d, _)| d).max().unwrap();
    assert_eq!(depth, n as isize + 2);

    assert!(parse_with(&format!("{}a", "-".repeat(n)), &unlimited).is_ok());
    assert!(parse_with(&format!("{}1{}", "[".repeat(n), "]".repeat(n)), &unlimited).is_ok());
    assert_eq!(error_with(&"(".repeat(n), &unlimited).1, 0..1);

    // Chains do not nest, so the length limit aside, any length.
    let fields = format!("a{}", ".b".repeat(n));
    assert!(parse_with(&fields, &unlimited).is_ok());
    let methods = format!("a{}", ".m(1)".repeat(n));
    assert!(parse_with(&methods, &unlimited).is_ok());
    let sum = format!("1{}", " + 1".repeat(n));
    assert!(parse_with(&sum, &unlimited).is_ok());
    let list = format!("[{}1]", "1, ".repeat(n));
    assert!(parse_with(&list, &unlimited).is_ok());
}

#[test]
fn error_kinds() {
    let error = parse("a b").unwrap_err();
    assert_eq!(
        error.kind(),
        &ErrorKind::ExpectedOperator {
            found: Found::Token(Syntax::NameToken),
            open: None
        }
    );

    assert_eq!(
        Diagnostic::new(span(0, 0), ErrorKind::TooLong(5_000_000_000)).to_string(),
        "the expression is 5000000000 bytes long, more than the 4294967295 that can be parsed"
    );
    assert_eq!(
        Diagnostic::new(span(0, 0), ErrorKind::OverLimit { len: 9, max: 8 }).to_string(),
        "the expression is 9 bytes long, the limit is 8"
    );
    assert_eq!(
        Diagnostic::new(span(0, 0), ErrorKind::Internal("x".to_owned())).to_string(),
        "internal error in evalite: x"
    );

    let error: Box<dyn std::error::Error> = Box::new(parse("").unwrap_err());
    assert!(!error.to_string().is_empty());
}

/// Each source parses with recovery to the s-expression given, with the
/// diagnostics given, in order, and nothing more.
/// A source, its s-expression, and each diagnostic's message and range.
type Recovery<'a> = (&'a str, &'a str, &'a [(&'a str, Range)]);

fn recovers(cases: &[Recovery<'_>]) {
    recovers_with(&Options::new(), cases);
}

fn recovers_with(options: &Options, cases: &[Recovery<'_>]) {
    for (source, expected, diagnostics) in cases {
        let (tree, errors) = recover_with(source, options);
        let errors = errors
            .iter()
            .map(|e| (e.to_string(), e.span().range()))
            .collect::<Vec<_>>();
        let diagnostics = diagnostics
            .iter()
            .map(|(m, r)| ((*m).to_owned(), r.clone()))
            .collect::<Vec<_>>();

        assert_eq!(render_tree(source, &tree), *expected, "{source:?}");
        assert_eq!(errors, diagnostics, "{source:?}");
    }
}

#[test]
fn recover_missing_operand() {
    recovers(&[
        (
            "min(1, , 2)",
            "(call min 1 (error) 2)",
            &[("expected an expression, found `,`", 7..8)],
        ),
        (
            "a + * b",
            "(+ a (* (error) b))",
            &[("expected an expression, found `*`", 4..5)],
        ),
        (
            "a +",
            "(+ a (error))",
            &[(
                "expected an expression, found the end of the expression",
                3..3,
            )],
        ),
        (
            "a +  ",
            "(+ a (error))",
            &[(
                "expected an expression, found the end of the expression",
                5..5,
            )],
        ),
        (
            "",
            "(error)",
            &[(
                "expected an expression, found the end of the expression",
                0..0,
            )],
        ),
        (
            "f(a,) + 1",
            "(+ (call f a (error)) 1)",
            &[("expected an expression, found `)`", 4..5)],
        ),
        (
            "() + x[]",
            "(+ (paren (error)) (index x (error)))",
            &[
                ("expected an expression, found `)`", 1..2),
                ("expected an expression, found `]`", 7..8),
            ],
        ),
        (
            ".a + b",
            "(+ (error . a) b)",
            &[("expected an expression, found `.`", 0..1)],
        ),
        (
            "f(,)",
            "(call f (error) (error))",
            &[
                ("expected an expression, found `,`", 2..3),
                ("expected an expression, found `)`", 3..4),
            ],
        ),
        (
            "-",
            "(- (error))",
            &[(
                "expected an expression, found the end of the expression",
                1..1,
            )],
        ),
    ]);
}

#[test]
fn recover_unexpected_operand() {
    let args = "expected an operator, `,` or `)`, found a name";

    recovers(&[
        (
            "f(a b, c d)",
            "(call f (error a b) (error c d))",
            &[(args, 4..5), (args, 9..10)],
        ),
        (
            "a !b",
            "(error (unwrap a) b)",
            &[(
                "expected an operator or the end of the expression, found a name",
                3..4,
            )],
        ),
        (
            "a b c + d",
            "(+ (error a b c) d)",
            &[(
                "expected an operator or the end of the expression, found a name",
                2..3,
            )],
        ),
        // A group in what is skipped is skipped whole, `,` and all.
        (
            "f(a 1 (b, c), d)",
            "(call f (error a 1 ( b , c )) d)",
            &[("expected an operator, `,` or `)`, found an integer", 4..5)],
        ),
        (
            "-a b * c",
            "(* (- (error a b)) c)",
            &[(
                "expected an operator or the end of the expression, found a name",
                3..4,
            )],
        ),
        (
            "a.1 + b",
            "(+ (error a . 1) b)",
            &[(
                "expected a field or method name after `.`, found an integer",
                2..3,
            )],
        ),
        (
            "a.(b).c",
            "(error a . ( b ) . c)",
            &[("expected a field or method name after `.`, found `(`", 2..3)],
        ),
        (
            "x.",
            "(error x .)",
            &[(
                "expected a field or method name after `.`, found the end of the expression",
                2..2,
            )],
        ),
    ]);
}

#[test]
fn recover_groups() {
    recovers(&[
        (
            "(a + b",
            "(paren (+ a b))",
            &[(
                "expected `)` to close the `(` at byte 0, found the end of the expression",
                0..1,
            )],
        ),
        (
            "[1, 2",
            "[1 2]",
            &[(
                "expected `]` to close the `[` at byte 0, found the end of the expression",
                0..1,
            )],
        ),
        (
            "a) + b",
            "(+ (error a )) b)",
            &[("unmatched `)`, there is no group open to close", 1..2)],
        ),
        (
            "(a)) + b])",
            "(error (error (+ (error (paren a) )) b) ]) ))",
            &[
                ("unmatched `)`, there is no group open to close", 3..4),
                ("unmatched `]`, there is no group open to close", 8..9),
                ("unmatched `)`, there is no group open to close", 9..10),
            ],
        ),
        // Each group still open at the end is reported once, at its
        // opener.
        (
            "f((a, [b",
            "(call f (paren (error a , [ b)))",
            &[
                (
                    "expected `)` to close the `(` at byte 1, found the end of the expression",
                    1..2,
                ),
                (
                    "expected `)` to close the `(` at byte 2, found the end of the expression",
                    2..3,
                ),
                ("expected an operator or `)`, found `,`", 4..5),
            ],
        ),
        // A comma in a group that takes none, and the group unclosed.
        (
            "(a, b",
            "(paren (error a , b))",
            &[
                (
                    "expected `)` to close the `(` at byte 0, found the end of the expression",
                    0..1,
                ),
                ("expected an operator or `)`, found `,`", 2..3),
            ],
        ),
        // A closer closes the group it matches, and those inside it.
        (
            "[f(a] + 1",
            "(+ [(call f a)] 1)",
            &[("expected `)` to close the `(` at byte 2, found `]`", 2..3)],
        ),
        // Matching none, it closes the innermost.
        (
            "(a] + 1",
            "(+ (paren a) 1)",
            &[("expected `)` to close the `(` at byte 0, found `]`", 0..1)],
        ),
        (
            "a, b",
            "(error a , b)",
            &[(
                "expected an operator or the end of the expression, found `,`",
                1..2,
            )],
        ),
        (
            "(a, b) + x[1, 2]",
            "(+ (paren (error a , b)) (index x (error 1 , 2)))",
            &[
                ("expected an operator or `)`, found `,`", 2..3),
                ("expected an operator or `]`, found `,`", 12..13),
            ],
        ),
    ]);
}

#[test]
fn recover_tokens() {
    recovers(&[
        (
            "a = b && c",
            "(&& (error a = b) c)",
            &[("`=` is not an operator, did you mean `==`?", 2..3)],
        ),
        (
            "= a",
            "(error (error) = a)",
            &[("`=` is not an operator, did you mean `==`?", 0..1)],
        ),
        (
            "1 + é + 2",
            "(+ (+ 1 (error é)) 2)",
            &[("unexpected character 'é'", 4..6)],
        ),
        (
            "a ; b + c",
            "(+ (error a ; b) c)",
            &[("unexpected character ';'", 2..3)],
        ),
        (
            "=",
            "(error (error) = (error))",
            &[
                ("`=` is not an operator, did you mean `==`?", 0..1),
                (
                    "expected an expression, found the end of the expression",
                    1..1,
                ),
            ],
        ),
        (
            "rating(a, \"x",
            "(call rating a (error \"x))",
            &[
                (
                    "expected `)` to close the `(` at byte 6, found the end of the expression",
                    6..7,
                ),
                (
                    "this string is never closed, expected a `\"` before the end",
                    10..12,
                ),
            ],
        ),
        ("a @", "(error a @)", &[("unexpected character '@'", 2..3)]),
        (
            "f(\"a\\qb\", 1)",
            "(call f (error \"a\\qb\") 1)",
            &[(
                "unknown escape `\\q` in a string, only `\\\"` and `\\\\` are allowed",
                4..6,
            )],
        ),
        (
            "x + \"abc",
            "(+ x (error \"abc))",
            &[(
                "this string is never closed, expected a `\"` before the end",
                4..8,
            )],
        ),
        (
            "99999999999999999999 + a",
            "(+ (error 99999999999999999999) a)",
            &[(
                "this integer is too large, the largest is 9223372036854775807",
                0..20,
            )],
        ),
        (
            "a < b < c && d",
            "(&& (error (< a b) < c) d)",
            &[(
                "comparisons do not chain, combine them with `&&` or add parentheses",
                6..7,
            )],
        ),
        (
            "a < b < c < d",
            "(< (error (< a b) < c) d)",
            &[(
                "comparisons do not chain, combine them with `&&` or add parentheses",
                6..7,
            )],
        ),
    ]);
}

#[test]
fn recover_too_deep() {
    let message =
        |what: &str| format!("this {what} nests too deeply, the limit is 2 levels of nesting");
    let paren = message("parenthesis");
    let call = message("call");

    recovers_with(
        &Options::new().max_depth(2),
        &[
            (
                "((((a, b)))) + 1",
                "(+ (paren (paren (paren (error ( a , b ))))) 1)",
                &[(&paren, 2..3)],
            ),
            // Reported once, however many groups are too deep.
            (
                "f(g(h(1)), g(h(2)))",
                "(call f (call g (call h (error 1))) (call g (call h (error 2))))",
                &[(&call, 5..6)],
            ),
            (
                "!!!!a.b && c",
                "(&& (! (! (! (error ! a . b)))) c)",
                &[(&message("`!`"), 2..3)],
            ),
            (
                "(((a",
                "(paren (paren (paren (error a))))",
                &[
                    (
                        "expected `)` to close the `(` at byte 0, found the end of the expression",
                        0..1,
                    ),
                    (
                        "expected `)` to close the `(` at byte 1, found the end of the expression",
                        1..2,
                    ),
                    (&paren, 2..3),
                ],
            ),
        ],
    );
}

/// A syntax error is checked around: what parses is still type checked, but
/// nothing about the error is reported again.
#[test]
fn recover_checks_around() {
    let mut env = Environment::new();
    env.value("rank", Type::int()).unwrap();

    let messages = |source: &str| {
        compile(source, &env)
            .unwrap_err()
            .iter()
            .map(|e| (e.to_string(), e.span().range()))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        messages("Int::min(rank, , true) + nothing"),
        [
            ("expected an expression, found `,`".to_owned(), 15..16),
            ("unknown name `nothing`".to_owned(), 25..32),
        ]
    );
    assert_eq!(
        messages("(rank + true) + (rank"),
        [
            (
                "`+` takes Int and Int, found Int and Bool".to_owned(),
                1..12
            ),
            (
                "expected `)` to close the `(` at byte 16, found the end of the expression"
                    .to_owned(),
                16..17
            ),
        ]
    );
    assert_eq!(
        messages("bonus b + rank = 1"),
        [
            ("unknown name `bonus`".to_owned(), 0..5),
            (
                "expected an operator or the end of the expression, found a name".to_owned(),
                6..7
            ),
            (
                "`=` is not an operator, did you mean `==`?".to_owned(),
                15..16
            ),
        ]
    );
    assert_eq!(
        messages("f(rank b)"),
        [
            ("unknown function `f`".to_owned(), 0..1),
            (
                "expected an operator, `,` or `)`, found a name".to_owned(),
                7..8
            ),
        ]
    );
}

/// Recovery consumes each token once, so even with no limits, input made of
/// nothing but errors parses in linear time, and its diagnostics are capped.
#[test]
fn recovery_is_linear() {
    let n = 50_000;
    let unlimited = Options::new().max_depth(usize::MAX).max_len(usize::MAX);
    let env = Environment::new();

    for source in [
        format!("{}{}", "(".repeat(n), "]".repeat(n)),
        format!("{}{}", "[(".repeat(n), "]".repeat(n)),
        format!("{}{}", "f(".repeat(n), "]".repeat(n)),
        "a b ".repeat(n),
        ") ".repeat(n),
        ", ".repeat(n),
        ". ".repeat(n),
        "a < ".repeat(n),
        "a.".repeat(n),
        "# ".repeat(n),
        "(".repeat(n),
    ] {
        let (tree, errors) = recover_with(&source, &unlimited);
        check_tree(&source, &tree);
        assert!(!errors.is_empty());
        assert!(errors.len() <= MAX_DIAGNOSTICS + 1);

        let parsed = crate::parse_with(&source, &unlimited).unwrap();
        let all = crate::check::compile_parsed(&source, parsed, &env).unwrap_err();
        assert!(all.len() <= MAX_DIAGNOSTICS + 1);
    }
}

#[test]
fn unescapes() {
    assert_eq!(unescape(r#""""#).as_deref(), Some(""));
    assert_eq!(unescape(r#""ab""#).as_deref(), Some("ab"));
    assert_eq!(
        unescape(r#""a \"b\" \\ é""#).as_deref(),
        Some("a \"b\" \\ é")
    );
    assert_eq!(unescape(r#""a\n""#), None);
    assert_eq!(unescape(r#""a"b""#), None);
    assert_eq!(unescape(r#""a\""#), None);
    assert_eq!(unescape("\"a"), None);
    assert_eq!(unescape("a"), None);
    assert_eq!(unescape("\""), None);

    let source = r#"rating(a, "Vitae \"2e\"")"#;
    let tree = parse(source).unwrap();
    let string = tree.walk().find(|n| n.value() == Syntax::String).unwrap();
    assert_eq!(
        unescape(text(source, string)).as_deref(),
        Some("Vitae \"2e\"")
    );
}

#[test]
fn options() {
    assert_eq!(Options::default(), Options::new());
    assert_eq!(Options::new(), Options::new().max_depth(DEFAULT_MAX_DEPTH));
    assert_eq!(DEFAULT_MAX_DEPTH, 32);
    assert_eq!(DEFAULT_MAX_LEN, 64 * 1024);
}

#[test]
fn max_len() {
    let options = Options::new().max_len(3);
    assert!(parse_with("1+2", &options).is_ok());
    let error = parse_with("1+22", &options).unwrap_err();
    assert_eq!(*error.kind(), ErrorKind::OverLimit { len: 4, max: 3 });
    assert!(parse_with(&" ".repeat(DEFAULT_MAX_LEN + 1), &Options::new()).is_err());
}

/// splitmix64.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub(crate) fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Whatever the parse gives, it holds together: the tree covers the input
/// within one level past the depth limit, each diagnostic points into the
/// input, the tree has an `Error` node only when there is a diagnostic, and
/// checking it never fails inside evalite.
fn check_any(source: &str, options: &Options, max_depth: usize) {
    let (tree, errors) = recover_with(source, options);
    check_tree(source, &tree);
    let root = tree.first().unwrap();
    assert!(nesting(root) <= max_depth.saturating_add(1), "{source:?}");

    // A group left open is closed as a group, but anything else in error
    // is an `Error` node.
    let has_error = tree.walk().any(|n| n.value() == Syntax::Error);
    assert!(!has_error || !errors.is_empty(), "{source:?}");
    assert!(errors.len() <= MAX_DIAGNOSTICS + 1, "{source:?}");

    let mut deep = 0;

    for error in &errors {
        let range = error.span().range();
        assert!(
            range.start <= range.end && range.end <= source.len(),
            "{source:?}"
        );
        assert!(source.is_char_boundary(range.start), "{source:?}");
        assert!(source.is_char_boundary(range.end), "{source:?}");
        assert!(
            !matches!(error.kind(), ErrorKind::Internal(..)),
            "{source:?}: {error}"
        );
        assert!(!error.to_string().is_empty());

        if let ErrorKind::TooDeep { max, .. } = error.kind() {
            assert_eq!(*max, max_depth);
            deep += 1;
        }
    }

    assert!(deep <= 1, "{source:?}: {errors:?}");

    let mut env = Environment::new();
    env.value("a", Type::int()).unwrap();
    env.value("b1", Type::list(Type::int())).unwrap();
    env.value("_x", Type::option(Type::bool())).unwrap();

    let parsed = crate::parse_with(source, options).unwrap();

    match crate::check::compile_parsed(source, parsed, &env) {
        Ok(..) => assert!(errors.is_empty(), "{source:?}: {errors:?}"),
        Err(all) => {
            for error in &all {
                assert!(
                    !matches!(error.kind(), ErrorKind::Internal(..)),
                    "{source:?}: {error}"
                );
            }

            // The syntax errors are all reported with the type errors, as
            // long as there is room for them.
            if all.len() <= MAX_DIAGNOSTICS {
                for error in &errors {
                    assert!(all.iter().any(|e| e == error), "{source:?}: {error}");
                }
            }
        }
    }
}

#[test]
fn random_token_soup() {
    const TOKENS: &[&str] = &[
        "a",
        "b1",
        "_x",
        "min",
        "f",
        "1",
        "0",
        "99999999999999999999",
        "\"s\"",
        "\"\\\"\"",
        "\"",
        "\\",
        "true",
        "false",
        "+",
        "-",
        "*",
        "/",
        "%",
        "==",
        "!=",
        "<",
        "<=",
        ">",
        ">=",
        "&&",
        "||",
        "!",
        "=",
        "&",
        "|",
        "(",
        ")",
        "[",
        "]",
        ",",
        ".",
        " ",
        " ",
        "\t",
        "\n",
        "é",
        "#",
        "(",
        "(",
        "[",
        ")",
        "]",
        "-",
        "!",
    ];

    let mut rng = Rng(0x5eed);

    for _ in 0..20_000 {
        let len = rng.below(40);
        let source = (0..len).map(|_| rng.pick(TOKENS)).collect::<String>();
        let max_depth = rng.below(6);
        check_any(&source, &Options::new().max_depth(max_depth), max_depth);
    }
}

/// A well-formed expression's text and the nesting the parser counts in
/// it, and whether it binds looser than a postfix (an operator
/// expression). Postfixes apply only to primaries and comparisons are
/// parenthesized, so the text means what was built.
fn generate(rng: &mut Rng, budget: usize) -> (String, usize, bool) {
    const ATOMS: &[&str] = &["a", "b", "x_1", "0", "12", "true", "\"t\"", "min"];
    const OPS: &[&str] = &["+", "-", "*", "/", "%", "&&", "||"];
    const COMPARE: &[&str] = &["==", "!=", "<", "<=", ">", ">="];
    const SPACE: &[&str] = &["", " ", "  ", "\n"];

    let primary = |(text, nest, loose): (String, usize, bool)| {
        if loose {
            (format!("({text})"), nest + 1)
        } else {
            (text, nest)
        }
    };

    if budget == 0 {
        return (rng.pick(ATOMS).to_owned(), 0, false);
    }

    let budget = budget - 1;
    let s = rng.pick(SPACE);

    match rng.below(11) {
        0 => (rng.pick(ATOMS).to_owned(), 0, false),
        1 => {
            let (l, ln, _) = generate(rng, budget);
            let (r, rn, _) = generate(rng, budget);
            (format!("{l}{s}{}{s}{r}", rng.pick(OPS)), ln.max(rn), true)
        }
        2 => {
            let (l, ln, _) = generate(rng, budget);
            let (r, rn, _) = generate(rng, budget);
            (
                format!("({l}{s}{}{s}{r})", rng.pick(COMPARE)),
                ln.max(rn) + 1,
                false,
            )
        }
        3 => {
            let (e, n) = primary(generate(rng, budget));
            (format!("{}{s}{e}", rng.pick(&["-", "!"])), n + 1, true)
        }
        4 => {
            let (e, n, _) = generate(rng, budget);
            (format!("({s}{e}{s})"), n + 1, false)
        }
        5 | 6 => {
            let count = rng.below(4);
            let mut nest = 0;
            let args = (0..count)
                .map(|_| {
                    let (e, n, _) = generate(rng, budget);
                    nest = nest.max(n);
                    e
                })
                .collect::<Vec<_>>()
                .join(&format!(",{s}"));

            if rng.below(2) == 0 {
                (format!("[{s}{args}{s}]"), nest + 1, false)
            } else {
                (
                    format!("{}{s}({args})", rng.pick(&["f", "min", "rating"])),
                    nest + 1,
                    false,
                )
            }
        }
        7 => {
            let (e, n) = primary(generate(rng, budget));
            (format!("{e}{s}.{s}{}", rng.pick(&["field", "b"])), n, false)
        }
        8 => {
            let (e, n) = primary(generate(rng, budget));
            let (arg, an, _) = generate(rng, budget);
            (format!("{e}.unwrap_or({arg})"), n.max(an + 1), false)
        }
        9 => {
            let (e, n) = primary(generate(rng, budget));
            let (index, i, _) = generate(rng, budget);
            (format!("{e}[{s}{index}{s}]"), n.max(i + 1), false)
        }
        _ => (format!("{s}{}{s}", rng.pick(ATOMS)), 0, false),
    }
}

#[test]
fn random_expressions() {
    let mut rng = Rng(0xc0ffee);
    let mut deep = 0;

    for _ in 0..5_000 {
        let budget = rng.below(7);
        let (source, nest, _) = generate(&mut rng, budget);
        let max_depth = rng.below(8);

        match parse_with(&source, &Options::new().max_depth(max_depth)) {
            Ok(tree) => {
                assert!(
                    nest <= max_depth,
                    "{source:?} nests {nest}, over {max_depth}"
                );
                check_tree(&source, &tree);
                assert_eq!(nesting(tree.first().unwrap()), nest, "{source:?}");
            }
            Err(error) => {
                assert!(nest > max_depth, "{source:?}: {error}");
                assert!(
                    matches!(error.kind(), ErrorKind::TooDeep { max, .. } if *max == max_depth),
                    "{source:?}: {error}"
                );

                // Too deep is reported once, and the over-deep groups are
                // skipped whole.
                let (tree, errors) = recover_with(&source, &Options::new().max_depth(max_depth));
                check_tree(&source, &tree);
                assert_eq!(errors, [error], "{source:?}");
                deep += 1;
            }
        }
    }

    // Both outcomes are exercised.
    assert!(deep > 100, "{deep}");
}
