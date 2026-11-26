# evalite

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

- **Strictly typed.** `Int`, `Bool`, `String`, lists, records and
  `Option`, checked when compiled, with no implicit conversions: `1 +
  true` and an unhandled `Option` are compile errors, not runtime
  surprises.
- **Safe on untrusted input.** The parser never recurses and nesting is
  bounded, so 100 000 parentheses give an error, not a stack overflow.
  A run has a fixed memory and a step limit, and nothing panics:
  overflow, division by zero and out-of-range indexes are errors.
- **Compile once, run later.** A [`Program`] is plain bytes you can
  store, and it runs without the environment it was checked against.
- **Verified on load.** Stored bytes are not trusted: a loaded program is
  type-checked again and refused unless it uses exactly the memory it
  declares.
- **Errors worth showing.** Each error says what was expected and what was
  found, with [labels] pointing at every place in the input it is about,
  ready to draw under the text.
- **Built for forms.** A [`Structure`] is an expression with holes, and
  [completions] list exactly what fits in each one, by type.

<br>

## Example

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
let error = evalite::compile("wits + bonus", &env).unwrap_err();
assert_eq!(
    error.to_string(),
    "`+` takes Int and Int, found Int and Option<Int>, unwrap the optional value first with `.unwrap_or(...)`",
);
assert_eq!(error.span().range(), 0..12);
```

<br>

## Types

Every expression has one type, known when it is compiled; nothing
converts implicitly.

- `Int`: a 64-bit signed integer, such as `12` (`-12` is `-` applied to
  `12`).
- `Bool`: `true` or `false`, from comparisons and `!`, `&&`, `||`.
- `String`: a literal such as `"Essence (1e)"`, naming a library table.
  `\"` and `\\` are its only escapes.
- `List<T>`: `[10, 11, 12]`, one or more entries of one type. `[]` is
  an error, since it has no entry type.
- `Option<T>`: a value that may be absent. It is unwrapped explicitly,
  `x.unwrap_or(0)`, never used as a `T`. `Option<Option<T>>` is not a
  type.
- Records, written `{athletics: Int, "Brawl (bar)": Int}`: fields read
  with `.athletics` or `["Brawl (bar)"]`, the latter only with a string
  literal. A sheet's track is one, such as
  `health: {boxes: Int, count: List<Int>}`, read as `health.boxes`.

The rules:

- `+ - * / %` and unary `-` take `Int`s; `< <= > >=` take two `Int`s and
  give a `Bool`; `==` and `!=` take two `Int`s or two `Bool`s.
- `&&`, `||` and `!` take `Bool`s; `&&` and `||` skip their right operand
  when the left decides.
- `list[i]` takes a `List<T>` and an `Int` and gives a `T`.
- A call matches its function's declared parameters exactly.

<br>

## Environment

An [`Environment`] declares every name an expression may use, with its
type, and every function. Values and functions are named apart, so a
value `min` and the function `min(...)` can both exist. Names are
lowercase, so `Strength` is unknown, with a hint when `strength` is
declared.

The [`Builtin`] functions are `min(Int, ...)` and `max(Int, ...)`, of
one or more `Int`s, and `lookup(Int, List<T>)`, a list's entry at an
index clamped to its ends. The caller declares the rest, such as
`lookup(Int, String)` for a named table; a name declared several ways
calls the first whose parameters the arguments match.

```rust
use evalite::{Environment, Type};

let mut env = Environment::with_builtins();
env.value("health", Type::record([("boxes", Type::int())]))?;
env.value("specialties", Type::record([("brawl", Type::option(Type::int()))]))?;
env.function("lookup", [Type::int(), Type::string()], Type::int())?;

let program = evalite::compile("health.boxes + specialties.brawl.unwrap_or(0)", &env)?;
assert_eq!(program.result_type(), Type::int());

let error = evalite::compile("lookup(1, health)", &env).unwrap_err();
assert_eq!(error.to_string(), "no `lookup` takes (Int, {boxes: Int}), it takes (Int, List<T>) or (Int, String)");
assert_eq!(error.span().range(), 0..17);
```

<br>

## Programs

[`compile`] gives a [`Program`]: code for a stack machine, with the
values it reads and the caller's functions it calls, each with its
type, so it runs without the environment.

Memory is 64-bit words, and each type's layout is fixed when the
program is compiled. An `Int` is a word, a `Bool` a word of 0 or 1, and
a string, list or record a word referring to its parts: a string's
bytes 8 to a word, a list's entries one after another, a record's
fields in name order. An `Option<Int>` is two words, whether it is
present and then the `Int`; every other option is one word, with a
value no `Bool` or reference has for absent. A program's [`Memory`] is
the deepest its stack gets, the entries of the lists it builds as it
runs, its constants and the values it reads, all in words. There are
no loops, so each instruction runs at most once.

[`Program::to_bytes`] stores a program with `musli`, and
[`Program::from_bytes`] reads it back. Stored bytes are not trusted: a
loaded program is checked by running its code over types instead of
values, and is refused unless it is well typed and uses exactly the
memory it declares. That check is what lets the machine run on untyped
words: nothing it runs looks at what a word is, and every name and
field is an offset fixed when the program was compiled.

```rust
use evalite::{Environment, Program, Type};

let mut env = Environment::with_builtins();
env.value("blood_potency", Type::int())?;

let program = evalite::compile("lookup(blood_potency, [10, 11, 12, 13])", &env)?;
assert_eq!(program.memory().stack(), 2);
assert_eq!(program.memory().constants(), 4);

let loaded = Program::from_bytes(&program.to_bytes()?)?;
assert_eq!(loaded.globals(), [("blood_potency", Type::int())]);
```

<br>

## Running

A [`Machine`] runs programs in a fixed memory, [`DEFAULT_MEMORY`] words
unless [`Limits::memory`] says otherwise, reserved when it is made.
[`Machine::load`] refuses a program whose memory is more than that,
saying how much it needs, and copies the program's constants in. Each
value the program reads is then bound with [`Run::bind`], which checks
it against the value's declared type and that its parts fit in the
memory left; a missing or mistyped binding is an error, never a
default. The caller's functions are a [`Host`], taking and giving
[`Value`]s.

A run takes at most [`DEFAULT_FUEL`] steps unless [`Limits::fuel`]
says otherwise: one per instruction, one more per entry of a list,
`min` or `max` it makes, and one per word of a value a function
returns. Faults are errors, never panics: `Int` arithmetic that
overflows, dividing by zero, `list[i]` past either end (`lookup`
clamps instead), `clamp` with its bounds the wrong way round, and a
function's failure or wrongly typed result.

A [`RunError`] is opaque. It shows its message, `{:#}` shows the whole
chain of what was being done, and [`RunError::labels`] gives each part
with the program's names. With the [`DebugInfo`] from
[`compile_debug`], each label also has the span of the expression that
failed.

```rust
use evalite::{Environment, Limits, Machine, Type, Value};

let mut env = Environment::with_builtins();
env.value("health", Type::record([("boxes", Type::int()), ("count", Type::list(Type::int()))]))?;
env.value("brawl", Type::option(Type::int()))?;
env.function("lookup", [Type::int(), Type::string()], Type::int())?;

let program = evalite::compile(r#"health.count[1] + brawl.unwrap_or(0) + lookup(1, "Essence")"#, &env)?;

let mut machine = Machine::new(Limits::new().fuel(100));
let mut run = machine.load(&program)?;
run.bind("health", Value::record([
    ("boxes", Value::int(7)),
    ("count", Value::list(vec![Value::int(3), Value::int(1)])),
]))?;
run.bind("brawl", Value::none())?;

let mut host = |name: &str, args: Vec<Value>| match (name, args.as_slice()) {
    ("lookup", [n, table]) if n.as_int() == Some(1) && table.as_str() == Some("Essence") => Ok(Value::int(10)),
    _ => Err(format!("no function {name}")),
};

let outcome = run.run_with(&mut host)?;
assert_eq!(outcome.value(), &Value::int(11));
assert_eq!(outcome.steps(), 12);

let error = Machine::new(Limits::new().memory(2)).load(&program).unwrap_err();
assert_eq!(error.to_string(), "the program needs 8 words of memory, more than the 2 the machine allows");
```

<br>

## Structures

A [`Structure`] is an expression as data, the shape a form builds: a
table of [`Node`]s, which may have holes (`_` as text) while it is being
edited. It encodes with `musli`, and compiles exactly as its
[`to_text`][Structure::to_text] would, with the same errors, each also
naming the [`NodeId`] it is about; a hole is an error. Writing one as
text and parsing that back gives the same structure.

[`Structure::complete`] lists what fits in a hole, for a form to offer:
the values, paths into records, methods, calls, literals and operators
whose type the hole's place takes, which comes from what holds it. At a
member hole, `bonus._` with `bonus` an `Option<Int>` in an `Int`'s
place, that is `unwrap_or(_)`. [`Structure::apply`] fills the hole with
one, leaving holes for its arguments.

<br>

## Grammar

Lowest precedence first; whitespace may separate any two tokens.

```text
expr     = or
or       = and ("||" and)*
and      = compare ("&&" compare)*
compare  = sum (("==" | "!=" | "<" | "<=" | ">" | ">=") sum)?
sum      = product (("+" | "-") product)*
product  = unary (("*" | "/" | "%") unary)*
unary    = ("-" | "!") unary | postfix
postfix  = primary ("." name args? | "[" expr "]")*
primary  = integer | string | "true" | "false" | name args?
         | "(" expr ")" | "[" (expr ("," expr)*)? "]"
args     = "(" (expr ("," expr)*)? ")"
name     = [A-Za-z_][A-Za-z0-9_]*
integer  = [0-9]+
string   = '"' ([^"\\] | '\"' | '\\')* '"'
```

Binary operators are left-associative, except comparisons, which do not
chain (`a < b < c` is an error). Every expression a computed widget
stores today is in this grammar: a value key such as `health[2]` or
`m["Brawl (bar)"]` is a name with field and index postfixes.

<br>

## Methods

Methods are called Rust-style on a value, `value.method(args)`, and
chain: `some.field.unwrap_or(0).max(1)`. A name followed by `(` is a
call of a function; `.name` without `(` is a field.

- `Int`: `abs()`, `min(Int)`, `max(Int)`, `clamp(Int, Int)`.
- `Option<T>`: `unwrap_or(T)`, `is_some()`, `is_none()`.
- `List<T>`: `len()`.

<br>

## Nesting

Each parenthesis, list, call's arguments, index and unary operator
nests one level inside what holds it. Going past `max_depth` (32 unless
set with `Options::max_depth`) is an error at the construct that went
too deep. Chains of operators, fields and methods do not nest.

<br>

## What is public

A type is public only if a caller needs to name it, and its insides are
private: no public fields, and no public enums of internal variants.
Callers build things through constructors, such as [`Value::int`], and
read them through methods that never panic, such as [`Value::as_int`].
Errors are opaque: they implement `Display` and `std::error::Error`, and
where they point into source text they are inspected through
`labels()`, as [`ParseError::labels`] is. A predicate is added only for a
named caller that must tell errors apart. Items only the crate's own
tests use are `pub(crate)`. Every public item is documented, with an
example covering its failure paths, which `#![deny(missing_docs)]` and
clippy's `missing_errors_doc` and `missing_panics_doc` enforce.

[`Builtin`]: https://docs.rs/evalite/latest/evalite/enum.Builtin.html
[`compile_debug`]: https://docs.rs/evalite/latest/evalite/fn.compile_debug.html
[`compile`]: https://docs.rs/evalite/latest/evalite/fn.compile.html
[`DebugInfo`]: https://docs.rs/evalite/latest/evalite/struct.DebugInfo.html
[`DEFAULT_FUEL`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_FUEL.html
[`DEFAULT_MEMORY`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MEMORY.html
[`Environment`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html
[`Host`]: https://docs.rs/evalite/latest/evalite/trait.Host.html
[`Limits::fuel`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.fuel
[`Limits::memory`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.memory
[`Machine::load`]: https://docs.rs/evalite/latest/evalite/struct.Machine.html#method.load
[`Machine`]: https://docs.rs/evalite/latest/evalite/struct.Machine.html
[`Memory`]: https://docs.rs/evalite/latest/evalite/struct.Memory.html
[`Node`]: https://docs.rs/evalite/latest/evalite/struct.Node.html
[`NodeId`]: https://docs.rs/evalite/latest/evalite/struct.NodeId.html
[`ParseError::labels`]: https://docs.rs/evalite/latest/evalite/struct.ParseError.html#method.labels
[`Program::from_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.from_bytes
[`Program::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.to_bytes
[`Program`]: https://docs.rs/evalite/latest/evalite/struct.Program.html
[`Run::bind`]: https://docs.rs/evalite/latest/evalite/struct.Run.html#method.bind
[`RunError::labels`]: https://docs.rs/evalite/latest/evalite/struct.RunError.html#method.labels
[`RunError`]: https://docs.rs/evalite/latest/evalite/struct.RunError.html
[`Structure::apply`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.apply
[`Structure::complete`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
[`Structure`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html
[`Value::as_int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.as_int
[`Value::int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.int
[`Value`]: https://docs.rs/evalite/latest/evalite/struct.Value.html
[completions]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
[labels]: https://docs.rs/evalite/latest/evalite/struct.ParseError.html#method.labels
[Structure::to_text]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.to_text
