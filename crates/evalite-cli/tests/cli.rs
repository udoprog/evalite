#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::Write;
use std::process::{Command, Stdio};

struct Output {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_evalite"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();

    Output {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

#[test]
fn prints_value_and_type() {
    let out = run(&["eval", "strength + 2", "--var", "strength:Int=3"], "");
    assert!(out.ok);
    assert_eq!(out.stdout, "5 : Int\n");
}

#[test]
fn unwraps_options() {
    let expr = "bonus.unwrap_or(10) + 1";
    let none = run(&["eval", expr, "--var", "bonus:Option<Int>=none"], "");
    assert_eq!(none.stdout, "11 : Int\n");
    let some = run(&["eval", expr, "--var", "bonus:Option<Int>=some(2)"], "");
    assert_eq!(some.stdout, "3 : Int\n");
}

#[test]
fn lists_and_strings() {
    let out = run(&["eval", "xs[1]", "--var", "xs:List<Int>=[4, 5]"], "");
    assert_eq!(out.stdout, "5 : Int\n");
    let out = run(&["eval", "name", "--var", "name:String=\"Ann\""], "");
    assert_eq!(out.stdout, "\"Ann\" : String\n");
}

#[test]
fn explain_prints_details() {
    let out = run(&["eval", "1 + 2", "--explain"], "");
    assert!(out.ok);
    assert!(
        out.stdout.starts_with("3 : Int\ntype: Int\nmemory: "),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("\nsteps: "), "{}", out.stdout);
}

#[test]
fn explain_counts_words() {
    // An `Option<Int>` is two words wherever it is held.
    let out = run(
        &[
            "eval",
            "x.unwrap_or(4) + 1",
            "--var",
            "x:Option<Int>=none",
            "--explain",
        ],
        "",
    );
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(
        out.stdout.trim_end(),
        "5 : Int\ntype: Int\nmemory: 5 words at most (stack 3, heap 0, constants 0, values read 2), 5 used\nsteps: 5"
    );
}

#[test]
fn type_error_shows_caret() {
    let out = run(&["eval", "1 + true"], "");
    assert!(!out.ok);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.starts_with("error: "), "{}", out.stderr);
    assert!(
        out.stderr.contains("  | 1 + true\n  | ^^^^^^^^\n"),
        "{}",
        out.stderr
    );
}

#[test]
fn unclosed_group_shows_its_opener() {
    let out = run(&["eval", "(1 + 2"], "");
    assert!(!out.ok);
    assert_eq!(
        out.stderr,
        "error: expected `)` to close the `(` at byte 0, found the end of the expression\n  \
         | (1 + 2\n  |       ^\n  = the `(` opened here\n  | (1 + 2\n  | ^\n"
    );
}

#[test]
fn depth_limit() {
    let out = run(&["eval", "((((1))))", "--max-depth", "2"], "");
    assert!(!out.ok);
    assert!(out.stderr.contains("nests too deeply"), "{}", out.stderr);
    assert!(run(&["eval", "((((1))))"], "").ok);
}

#[test]
fn memory_limit() {
    let out = run(&["eval", "[1, 2, 3]", "--memory", "1"], "");
    assert!(!out.ok);
    assert!(
        out.stderr.contains("more than the 1 the machine allows"),
        "{}",
        out.stderr
    );
}

#[test]
fn fuel_limit() {
    let out = run(&["eval", "1 + 2 + 3", "--fuel", "1"], "");
    assert!(!out.ok);
    assert!(out.stderr.contains("took too long"), "{}", out.stderr);
    assert!(run(&["eval", "1 + 2 + 3", "--fuel", "100"], "").ok);
}

#[test]
fn runtime_error_shows_caret_and_context() {
    let out = run(&["eval", "2 + 1 / 0"], "");
    assert!(!out.ok);
    assert_eq!(
        out.stderr,
        "error: `1 / 0` divides by zero\n  | 2 + 1 / 0\n  |     ^^^^^\n  = while running instruction 3\n  = while running the program\n"
    );
}

#[test]
fn overflow_and_load_errors_point_and_say_why() {
    let out = run(&["eval", "x + 1", "--var", "x:Int=9223372036854775807"], "");
    assert!(!out.ok);
    assert!(
        out.stderr.contains("  | x + 1\n  | ^^^^^\n"),
        "{}",
        out.stderr
    );

    let out = run(&["eval", "1", "--memory", "0"], "");
    assert!(!out.ok);
    assert!(
        out.stderr.contains("  = while loading the program\n"),
        "{}",
        out.stderr
    );
}

#[test]
fn bad_var() {
    for var in [
        "x",
        "x:Int",
        "x:Float=1",
        "x:Int=abc",
        "x:Option<Option<Int>>=some(1)",
    ] {
        let out = run(&["eval", "1", "--var", var], "");
        assert!(!out.ok, "{var}");
        assert_eq!(out.code, Some(2), "{var}");
        assert!(out.stderr.contains("--var"), "{var}: {}", out.stderr);
    }
}

#[test]
fn unused_var_is_fine() {
    let out = run(&["eval", "1", "--var", "x:Int=3"], "");
    assert_eq!(out.stdout, "1 : Int\n");
}

#[test]
fn stdin_loop() {
    let out = run(&["--var", "a:Int=4"], "a * 2\n\na + 1\n");
    assert!(out.ok);
    assert_eq!(out.stdout, "8 : Int\n5 : Int\n");
}

#[test]
fn stdin_loop_reports_errors_and_continues() {
    let out = run(&["--var", "a:Int=4"], "bad +\na + 1\n");
    assert!(!out.ok);
    assert_eq!(out.stdout, "5 : Int\n");
    assert!(out.stderr.contains("  | bad +\n"), "{}", out.stderr);
}

#[test]
fn eval_without_expression_reads_stdin() {
    let out = run(&["eval"], "1 + 1\n");
    assert_eq!(out.stdout, "2 : Int\n");
}
