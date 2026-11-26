# evalite

<img alt="evalite" src="https://raw.githubusercontent.com/udoprog/evalite/main/assets/wordmark.svg" height="64">

[<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
[<img alt="crates.io" src="https://img.shields.io/crates/v/evalite.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/udoprog/evalite/ci.yml?branch=main&style=for-the-badge" height="20">](https://github.com/udoprog/evalite/actions?query=branch%3Amain)

Small, strictly typed expressions that are safe to run on anyone's input.

evalite is "eval lite": let users write formulas such as
`min(wits, dexterity) + athletics` or `bonus.unwrap_or(0) + 1`, check
them once against the names you declare, store the compiled program, and
run it later in a fixed budget of memory and steps. Every mistake is
caught before it runs, and no input can crash, hang or exhaust the host.

**[Try it in your browser in the playground](https://udoprog.github.io/evalite/)**.

- **Strictly typed.** Every expression has one type, checked before it
  runs, and nothing converts on its own. With `rank: Int` and
  `bonus: Option<Int>`, `rank + true` does not compile, and neither does
  `rank + bonus`, whose error says to unwrap the optional value first with
  `.unwrap_or(...)`. `rank + bonus.unwrap_or(0)` compiles to an `Int`.
- **Safe on untrusted input.** Whatever a user types, the worst outcome is
  an error. 40 nested parentheses hit the nesting limit instead of
  overflowing the stack, `rank * 9223372036854775807` reports that it
  overflows an `Int`, `dice[5]` on a list of 3 reports that the index is out
  of range, and an expression that needs more steps than it is allowed stops
  with "the expression took too long". See [Limits and
  safety](#limits-and-safety).
- **Compile once, run many times.** Check an expression when the user saves
  it and store the compiled [`Program`] as bytes next to your data. Running
  it later only binds new values, without parsing or checking again. See
  [Storing programs](#storing-programs).
- **Stored bytes are checked.** Bytes read back from a database or a file
  may be corrupt or tampered with, so [`Program::from_bytes`] checks them
  again and refuses anything that is not a valid, well-typed program.
  Cutting the last byte off gives "the bytes do not decode".
- **Errors that point.** Every mistake is reported, not only the first,
  and each says what it expected and what it found, while its [labels]
  mark where, so you can underline it in your own UI. `min(1, (2 + 3)` gives "expected `)` to close the `(` at byte 3, found the
  end of the expression", pointing at the `(` and noting where the `)` was
  expected. See [Errors](#errors).
- **Autocomplete that only offers what fits.** Leave a hole in a
  [`Structure`] and [completions] list only what would type-check there. In
  `rank + _` you are offered `rank`, `bonus.unwrap_or(_)` and `min(_, _)`,
  but never `name`, a `String`, or a bare `bonus`, an `Option<Int>`. That is
  enough to build an editor where users pick from lists and every pick is
  valid. See [Autocomplete](#autocomplete).

<br>

## Quick start

Declare what an expression may use, compile it, then bind values and run
it:

```rust
use evalite::{Environment, Limits, Machine, Type, Value};

let mut env = Environment::with_builtins();
env.value("wits", Type::int())?;
env.value("dexterity", Type::int())?;
env.value("bonus", Type::option(Type::int()))?;

let program = evalite::compile("min(wits, dexterity) + bonus.unwrap_or(0)", &env)?;
assert_eq!(program.result_type(), Type::int());

let mut machine = Machine::new(Limits::new());
let mut run = machine.load(&program)?;
run.bind("wits", Value::int(3))?;
run.bind("dexterity", Value::int(2))?;
run.bind("bonus", Value::some(Value::int(1)))?;

let outcome = run.run()?;
assert_eq!(outcome.value().as_int(), Some(3));

// Mistakes are caught when compiling, and point at the input.
let errors = evalite::compile("wits + bonus", &env).unwrap_err();
let error = errors.iter().next().unwrap();
assert_eq!(
    error.to_string(),
    "`+` takes Int and Int, found Int and Option<Int>, unwrap the optional value first with `.unwrap_or(...)`",
);
assert_eq!(error.span().range(), 0..12);
```

<br>

## The language

An expression is one line, such as `health.boxes - damage.len()` or
`rank >= 3 && !flagged`. Every expression has one type, known when it is
compiled, and nothing converts implicitly:

- `Int`: a 64-bit signed integer, such as `12` (`-12` is `-` applied to
  `12`).
- `Bool`: `true` or `false`.
- `String`: a literal such as `"Essence (1e)"`. `\"` and `\\` are its only
  escapes.
- `List<T>`: `[10, 11, 12]`, one or more entries of one type, so `[]` is an
  error.
- Records, such as `{boxes: Int, count: List<Int>}`: a field is read with
  `.boxes`, or with `["Brawl (bar)"]` when its name is not a plain name.
- `Option<T>`: a value that may be absent, which comes from a name or a
  function declared with it, or from `list.get(i)`. It is never used as a `T`:
  unwrap it with `x.unwrap_or(0)`, which gives `0` when it is absent, or
  with `x!`, which stops the run with an error when it is absent. Test it
  with `x.is_some()` and `x.is_none()`. Options nest: an
  `Option<Option<Int>>` is `none`, `some(none)` or `some(some(1))`, and
  `x!!` unwraps both.

Types and values are written the same way outside an expression, such as
in a form or on a command line: [`Type::parse`] reads `Option<List<Int>>`
or `{boxes: Int}`, and [`Value::parse`] reads a value of a type, such as
`some([1, -2])` or `{boxes: 7}`. Both read back what `Display` prints.

Operators, tightest first. Binary operators are left-associative, except
comparisons, which do not chain (`a < b < c` is an error):

| Operators                    | Takes                        | Gives  |
|------------------------------|------------------------------|--------|
| `x.field`, `x.method(..)`, `list[i]` | a record, any value, a `List<T>` and an `Int` | its type |
| postfix `x!`                 | an `Option<T>`               | `T`, or an error when absent |
| unary `-`, `!`               | `Int`, `Bool`                | the same |
| `*` `/` `%`                  | `Int`s                       | `Int`  |
| `+` `-`                      | `Int`s                       | `Int`  |
| `<` `<=` `>` `>=`            | `Int`s                       | `Bool` |
| `==` `!=`                    | two `Int`s or two `Bool`s    | `Bool` |
| `&&`                         | `Bool`s, the right skipped when the left is `false` | `Bool` |
| <code>&#124;&#124;</code>    | `Bool`s, the right skipped when the left is `true`  | `Bool` |

Methods, which chain, as in `bonus.unwrap_or(0).max(1)`. A `T` is whatever
the receiver holds, so `bonus.unwrap_or(0)` of an `Option<Int>` takes and
gives an `Int`:

| On          | Methods                                                      |
|-------------|--------------------------------------------------------------|
| `Int`       | `abs() -> Int`, `min(Int) -> Int`, `max(Int) -> Int`, `clamp(Int, Int) -> Int` |
| `Option<T>` | `unwrap_or(T) -> T`, `is_some() -> Bool`, `is_none() -> Bool` |
| `List<T>`   | `get(Int) -> Option<T>`, `len() -> Int`                      |

`list.get(i)` is the entry at an index from 0, or `none` when the index
is out of range, so `[10, 20].get(1)` is `some(20)`, an `Option<Int>`,
and `[10, 20].get(2)` is `none`. Unwrap it like any option, as in
`[10, 20, 30].get(rank).unwrap_or(0)`. On a list of options it gives an
option of an option, so an index out of range (`none`) is told apart from
an absent entry (`some(none)`). `list[i]` is the entry itself, and an
error out of range.

Functions are called by name, `min(a, b)`. The builtins, from
[`Environment::with_builtins`], are:

| Function               | Gives                              |
|------------------------|------------------------------------|
| `min(Int, ...) -> Int` | the least of one or more `Int`s    |
| `max(Int, ...) -> Int` | the greatest of one or more `Int`s |

Every other name, value or function, is one the caller declares, with its
type, and the caller runs its functions when a program calls them. A
function name may be declared several ways, and a call takes the first
whose parameters match. Here `skill` is a character's rating in a skill,
by name, and with a specialty it may add a die for:

```rust
use evalite::{Environment, Type};

let mut env = Environment::with_builtins();
env.value("wits", Type::int())?;
env.function("skill", [Type::string()], Type::int())?;
env.function("skill", [Type::string(), Type::string()], Type::int())?;

let program = evalite::compile(r#"wits + skill("Brawl", "Grappling")"#, &env)?;
assert_eq!(program.result_type(), Type::int());

let error = evalite::compile("skill(wits)", &env).unwrap_err();
assert_eq!(error.to_string(), "no `skill` takes (Int), it takes (String) or (String, String)");
```

The whole grammar, loosest first; whitespace may separate any two tokens:

```text
expr     = or
or       = and ("||" and)*
and      = compare ("&&" compare)*
compare  = sum (("==" | "!=" | "<" | "<=" | ">" | ">=") sum)?
sum      = product (("+" | "-") product)*
product  = unary (("*" | "/" | "%") unary)*
unary    = ("-" | "!") unary | postfix
postfix  = primary ("." name args? | "[" expr "]" | "!")*
primary  = integer | string | "true" | "false" | name args?
         | "(" expr ")" | "[" (expr ("," expr)*)? "]"
args     = "(" (expr ("," expr)*)? ")"
name     = [A-Za-z_][A-Za-z0-9_]*
integer  = [0-9]+
string   = '"' ([^"\\] | '\"' | '\\')* '"'
```

<br>

## Limits and safety

Four limits bound everything an expression can do, so it is safe to run
whatever a user typed:

| Limit   | Bounds                                                              | Default                | Set with             |
|---------|---------------------------------------------------------------------|------------------------|----------------------|
| Nesting | each parenthesis, list, call, index and unary operator nests a level | [`DEFAULT_MAX_DEPTH`] (32) | [`Options::max_depth`] |
| Length  | bytes of the expression's text, checked before it is lexed | [`DEFAULT_MAX_LEN`] (65536) | [`Options::max_len`] |
| Memory  | 64-bit words for the stack, built lists, constants and bound values  | [`DEFAULT_MEMORY`] (1024)  | [`Limits::memory`]     |
| Fuel    | steps: one per instruction, plus one per list entry built or word a function returns | [`DEFAULT_FUEL`] (1000) | [`Limits::fuel`] |

Going too deep is a compile error at the construct that did; the parser
never recurses, so no input overflows the stack. A program's memory is
known when it is compiled, and [`Machine::load`] refuses one that needs
more than the machine has. Running out of fuel stops the run. There are no
loops, so most expressions use a few dozen steps. Faults are errors, never
panics: `Int` overflow, division by zero, `list[i]` past either end
(`list.get(i)` gives `none` instead), `x!` of an absent option, `clamp` with its
bounds the wrong way round, and a host function's failure or wrongly typed
result.

```rust
use evalite::{Environment, Limits, Machine, Options};

let env = Environment::with_builtins();

let error = evalite::compile_with("(((1)))", &env, &Options::new().max_depth(2)).unwrap_err();
assert_eq!(error.to_string(), "this parenthesis nests too deeply, the limit is 2 levels of nesting");

let program = evalite::compile("min(1, 2, 3, 4, 5) + 9223372036854775807", &env)?;

let mut machine = Machine::new(Limits::new().fuel(3));
let error = machine.load(&program)?.run().unwrap_err();
assert_eq!(error.to_string(), "the expression took too long: it used all of its 3 steps");

let mut machine = Machine::new(Limits::new());
let error = machine.load(&program)?.run().unwrap_err();
assert_eq!(error.to_string(), "`1 + 9223372036854775807` overflows an Int");
```

<br>

## Storing programs

A [`Program`] is everything needed to run an expression: its code, and the
names and types of the values it reads and functions it calls. Compile
when the user saves, store [`Program::to_bytes`], and run it on every
change without parsing or checking again.

Stored bytes are not trusted. They start with a format version, and
[`Program::from_bytes`] refuses another version, then checks the program
again by running its code over types instead of values: it must be well
typed and use exactly the memory it declares. That check is what lets the
machine run on bare words without looking at them.

A program carries no source text. To point run errors at the expression
that failed, [`compile_debug`] also gives a [`DebugInfo`], which can be
stored next to the program with [`DebugInfo::to_bytes`].

```rust
use evalite::{DebugInfo, Environment, Machine, Options, Program, Type, Value};

let mut env = Environment::with_builtins();
env.value("rank", Type::int())?;

let (program, debug) = evalite::compile_debug("10 / rank", &env, &Options::new())?;
let (stored, stored_debug) = (program.to_bytes()?, debug.to_bytes()?);

let program = Program::from_bytes(&stored)?;
let debug = DebugInfo::from_bytes(&stored_debug)?;
assert_eq!(program.globals(), [("rank", Type::int())]);

let mut machine = Machine::default();
let mut run = machine.load(&program)?;
run.bind("rank", Value::int(0))?;
let error = run.run().unwrap_err();
let primary = error.labels(&program, Some(&debug)).next().and_then(|l| l.span());
assert_eq!(primary.map(|s| s.range()), Some(0..9));

assert!(Program::from_bytes(&stored[..stored.len() - 1]).is_err());
```

<br>

## Errors

Every error is opaque and implements `Display` and `std::error::Error`.
Compiling reports every mistake it finds as [`Diagnostics`], in source
order, at most [`MAX_DIAGNOSTICS`] of them. Each mistake is reported once:
an expression in error is not reported again by what uses it, so
`rank + true + bonus` reports `rank + true`, not `+ bonus` as well.
A syntax error is recovered from: the parser skips to where the
expression can go on, and what it could parse is still checked, so
`min(rank, , true) + bonus` reports the missing argument and the
unknown `bonus`.

Those that point into the input, each [`Diagnostic`], [`StructureError`]
and [`RunError`], give [`Label`]s: a message and the span it is about, the
primary label first and then what explains it, such as where an unclosed
bracket had to close or what was being done when a run failed. This is
enough to draw them under the input, as the CLI does:

```rust
use evalite::{Environment, Labels, Type};

fn render(source: &str, labels: Labels<'_>) -> String {
    let mut out = String::new();

    for label in labels {
        let prefix = if label.is_primary() { "error" } else { "note" };
        out.push_str(&format!("{prefix}: {}\n", label.message()));

        if let Some(span) = label.span() {
            let column = source.get(..span.start as usize).map_or(0, |s| s.chars().count());
            let width = source.get(span.range()).map_or(1, |s| s.chars().count().max(1));
            out.push_str(&format!("  {source}\n  {}{}\n", " ".repeat(column), "^".repeat(width)));
        }
    }

    out
}

let mut env = Environment::with_builtins();
env.value("rank", Type::int())?;

let source = "min(rank, true) + bonus";
let errors = evalite::compile(source, &env).unwrap_err();
let rendered: String = errors.iter().map(|error| render(source, error.labels())).collect();

assert_eq!(rendered, "\
error: argument 2 of `min` must be Int, found Bool
  min(rank, true) + bonus
            ^^^^
error: unknown name `bonus`
  min(rank, true) + bonus
                    ^^^^^
");

let source = "min(1, (2 + 3)";
let errors = evalite::compile(source, &env).unwrap_err();
let error = errors.iter().next().unwrap();

assert_eq!(render(source, error.labels()), "\
error: expected `)` to close the `(` at byte 3, found the end of the expression
  min(1, (2 + 3)
     ^
note: expected `)` here
  min(1, (2 + 3)
                ^
");
```

<br>

## Autocomplete

A [`Structure`] is an expression as a tree of [`Node`]s instead of text,
and it may have holes, written `_`, where something is still missing. This
is what a point-and-click editor builds. [`Structure::complete`] lists
what can go in a hole: only what gives the type that place needs, so every
pick compiles. [`Structure::apply`] puts a pick in, leaving new holes for
any arguments it takes. A structure converts to and from text, stores
with [`Structure::to_bytes`], and compiles with the same errors as its
text, each also naming the [`NodeId`] it is about.

```rust
use evalite::{Environment, Structure, Type};

let mut env = Environment::with_builtins();
env.value("wits", Type::int())?;
env.value("bonus", Type::option(Type::int()))?;
env.value("name", Type::string())?;

let mut form = Structure::from_text("wits + bonus._")?;
let hole = form.holes()[0];

let fits = form.complete(&env, hole, None)?;
assert_eq!(fits.iter().map(|c| c.label()).collect::<Vec<_>>(), ["unwrap_or(_)"]);

form.apply(hole, &fits[0])?;
assert_eq!(form.to_text(), "wits + bonus.unwrap_or(_)");
assert!(form.compile(&env).is_err());

let hole = form.holes()[0];
let fits = form.complete(&env, hole, None)?;
assert!(fits.iter().any(|c| c.label() == "wits"));
assert!(!fits.iter().any(|c| c.label() == "name"));

form.apply(hole, &fits.iter().find(|c| c.label() == "wits").unwrap())?;
assert_eq!(form.compile(&env)?.result_type(), Type::int());
```

<br>

## The command line

The `evalite` command runs expressions from the terminal, which is a quick
way to try the language. Install it with `cargo install evalite-cli`.
Variables are declared and bound with `--var name:Type=value`:

```text
$ evalite eval 'min(wits, dexterity) + bonus.unwrap_or(0)' --var wits:Int=3 --var dexterity:Int=2 --var 'bonus:Option<Int>=some(1)'
3 : Int

$ evalite eval 'skills[3] * 2' --var 'skills:List<Int>=[1, 2]'
error: index 3 is out of range of a list of 2 entries
  | skills[3] * 2
  | ^^^^^^^^^
  = while running instruction 2
  = while running the program

$ evalite eval '[10, 20, 30].get(rank)' --var rank:Int=7
none : Option<Int>

$ evalite eval '[10, 20, 30].get(rank).unwrap_or(0) * 2' --var rank:Int=1 --explain
40 : Int
type: Int
memory: 7 words at most (stack 3, heap 0, constants 3, values read 1), 7 used
steps: 7
```

Without an expression it reads one per line from stdin. `--max-depth`, `--max-len`,
`--memory` and `--fuel` set the limits, and `evalite --help` lists the
rest.

<br>

## What is public

A type is public only if a caller needs to name it, and its insides are
private: no public fields, and no public enums of internal variants.
Callers build things through constructors, such as [`Value::int`], and
read them through methods that never panic, such as [`Value::as_int`].
Errors are opaque: they implement `Display` and `std::error::Error`, and
where they point into source text they are inspected through
`labels()`, as [`Diagnostic::labels`] is. A predicate is added only for a
named caller that must tell errors apart. Items only the crate's own
tests use are `pub(crate)`. Every public item is documented, with an
example covering its failure paths, which `#![deny(missing_docs)]` and
clippy's `missing_errors_doc` and `missing_panics_doc` enforce.

[`compile_debug`]: https://docs.rs/evalite/latest/evalite/fn.compile_debug.html
[`DebugInfo::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.DebugInfo.html#method.to_bytes
[`DebugInfo`]: https://docs.rs/evalite/latest/evalite/struct.DebugInfo.html
[`DEFAULT_FUEL`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_FUEL.html
[`DEFAULT_MAX_DEPTH`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MAX_DEPTH.html
[`DEFAULT_MAX_LEN`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MAX_LEN.html
[`DEFAULT_MEMORY`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MEMORY.html
[`Diagnostic::labels`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html#method.labels
[`Diagnostic`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html
[`Diagnostics`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostics.html
[`Environment::with_builtins`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html#method.with_builtins
[`Label`]: https://docs.rs/evalite/latest/evalite/struct.Label.html
[`Limits::fuel`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.fuel
[`Limits::memory`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.memory
[`Machine::load`]: https://docs.rs/evalite/latest/evalite/struct.Machine.html#method.load
[`MAX_DIAGNOSTICS`]: https://docs.rs/evalite/latest/evalite/constant.MAX_DIAGNOSTICS.html
[`Node`]: https://docs.rs/evalite/latest/evalite/struct.Node.html
[`NodeId`]: https://docs.rs/evalite/latest/evalite/struct.NodeId.html
[`Options::max_depth`]: https://docs.rs/evalite/latest/evalite/struct.Options.html#method.max_depth
[`Options::max_len`]: https://docs.rs/evalite/latest/evalite/struct.Options.html#method.max_len
[`Program::from_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.from_bytes
[`Program::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.to_bytes
[`Program`]: https://docs.rs/evalite/latest/evalite/struct.Program.html
[`RunError`]: https://docs.rs/evalite/latest/evalite/struct.RunError.html
[`Structure::apply`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.apply
[`Structure::complete`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
[`Structure::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.to_bytes
[`Structure`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html
[`StructureError`]: https://docs.rs/evalite/latest/evalite/struct.StructureError.html
[`Type::parse`]: https://docs.rs/evalite/latest/evalite/struct.Type.html#method.parse
[`Value::as_int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.as_int
[`Value::int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.int
[`Value::parse`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.parse
[completions]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
[labels]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html#method.labels
