//! evalite is a small, strictly typed expression evaluator. It was built for
//! character-sheet expressions, such as a computed widget's
//! `min(wits, dexterity) + athletics` or
//! `specialties.brawl.unwrap_or(0) + 1`.
//!
//! - **Safe on untrusted input.** Parsing never recurses: nesting is tracked
//!   on an explicit stack bounded by a settable `max_depth`, so even 100 000
//!   parentheses give an error, not a stack overflow. Runtime code never
//!   panics.
//! - **Clear errors.** Every error has a byte span and says what was
//!   expected and what was found.
//! - **Lossless trees.** The parse is a [`syntree`](https://docs.rs/syntree)
//!   tree covering every byte of the input, whitespace included.
//!
//! ```
//! let tree = evalite::parse("min(wits, dexterity) + athletics")?;
//! assert_eq!(tree.span().range(), 0..32);
//!
//! let error = evalite::parse("1 +").unwrap_err();
//! assert_eq!(error.to_string(), "expected an expression, found the end of the expression");
//! assert_eq!(error.span().range(), 3..3);
//! # Ok::<_, evalite::Error>(())
//! ```
//!
//! ## Types
//!
//! Every expression has one type, known when it is compiled; nothing
//! converts implicitly.
//!
//! - `Int`: a 64-bit signed integer, such as `12` (`-12` is `-` applied to
//!   `12`).
//! - `Bool`: `true` or `false`, from comparisons and `!`, `&&`, `||`.
//! - `String`: a literal such as `"Essence (1e)"`, naming a library table.
//!   `\"` and `\\` are its only escapes.
//! - `List<T>`: `[10, 11, 12]`, one or more entries of one type. `[]` is
//!   an error, since it has no entry type.
//! - `Option<T>`: a value that may be absent. It is unwrapped explicitly,
//!   `x.unwrap_or(0)`, never used as a `T`. `Option<Option<T>>` is not a
//!   type.
//! - Records, written `{athletics: Int, "Brawl (bar)": Int}`: fields read
//!   with `.athletics` or `["Brawl (bar)"]`, the latter only with a string
//!   literal. A sheet's track is one, such as
//!   `health: {boxes: Int, count: List<Int>}`, read as `health.boxes`.
//!
//! The rules:
//!
//! - `+ - * / %` and unary `-` take `Int`s; `< <= > >=` take two `Int`s and
//!   give a `Bool`; `==` and `!=` take two `Int`s or two `Bool`s.
//! - `&&`, `||` and `!` take `Bool`s; `&&` and `||` skip their right operand
//!   when the left decides.
//! - `list[i]` takes a `List<T>` and an `Int` and gives a `T`.
//! - A call matches its function's declared parameters exactly.
//!
//! ## Environment
//!
//! An [`Environment`] declares every name an expression may use, with its
//! type, and every function. Values and functions are named apart, so a
//! value `min` and the function `min(...)` can both exist. Names are
//! lowercase, so `Strength` is unknown, with a hint when `strength` is
//! declared.
//!
//! The [`Builtin`] functions are `min(Int, ...)` and `max(Int, ...)`, of
//! one or more `Int`s, and `lookup(Int, List<T>)`, a list's entry at an
//! index clamped to its ends. The caller declares the rest, such as
//! `lookup(Int, String)` for a named table; a name declared several ways
//! calls the first whose parameters the arguments match.
//!
//! ```
//! use evalite::{Environment, Type};
//!
//! let mut env = Environment::with_builtins();
//! env.value("health", Type::record([("boxes", Type::Int)]))?;
//! env.value("specialties", Type::record([("brawl", Type::option(Type::Int))]))?;
//! env.function("lookup", [Type::Int, Type::String], Type::Int)?;
//!
//! let program = evalite::compile("health.boxes + specialties.brawl.unwrap_or(0)", &env)?;
//! assert_eq!(program.result_type(), Type::Int);
//!
//! let error = evalite::compile("lookup(1, health)", &env).unwrap_err();
//! assert_eq!(error.to_string(), "no `lookup` takes (Int, {boxes: Int}), it takes (Int, List<T>) or (Int, String)");
//! assert_eq!(error.span().range(), 0..17);
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Programs
//!
//! [`compile`] gives a [`Program`]: code for a stack machine, with the
//! values it reads and the caller's functions it calls, each with its
//! type, so it runs without the environment.
//!
//! Memory is 64-bit words, and each type's layout is fixed when the
//! program is compiled. An `Int` is a word, a `Bool` a word of 0 or 1, and
//! a string, list or record a word referring to its parts: a string's
//! bytes 8 to a word, a list's entries one after another, a record's
//! fields in name order. An `Option<Int>` is two words, whether it is
//! present and then the `Int`; every other option is one word, with a
//! value no `Bool` or reference has for absent. A program's [`Memory`] is
//! the deepest its stack gets, the entries of the lists it builds as it
//! runs, its constants and the values it reads, all in words. There are
//! no loops, so each instruction runs at most once.
//!
//! [`Program::to_bytes`] stores a program with `musli`, and
//! [`Program::from_bytes`] reads it back. Stored bytes are not trusted: a
//! loaded program is checked by running its code over types instead of
//! values, and is refused unless it is well typed and uses exactly the
//! memory it declares. That check is what lets the machine run on untyped
//! words: nothing it runs looks at what a word is, and every name and
//! field is an offset fixed when the program was compiled.
//!
//! ```
//! use evalite::{Environment, Program, Type};
//!
//! let mut env = Environment::with_builtins();
//! env.value("blood_potency", Type::Int)?;
//!
//! let program = evalite::compile("lookup(blood_potency, [10, 11, 12, 13])", &env)?;
//! assert_eq!(program.memory().stack, 2);
//! assert_eq!(program.memory().constants, 4);
//!
//! let loaded = Program::from_bytes(&program.to_bytes()?)?;
//! assert_eq!(loaded.globals(), [("blood_potency", Type::Int)]);
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Running
//!
//! A [`Machine`] runs programs in a fixed memory, [`DEFAULT_MEMORY`] words
//! unless [`Limits::memory`] says otherwise, reserved when it is made.
//! [`Machine::load`] refuses a program whose memory is more than that,
//! saying how much it needs, and copies the program's constants in. Each
//! value the program reads is then bound with [`Run::bind`], which checks
//! it against the value's declared type and that its parts fit in the
//! memory left; a missing or mistyped binding is an error, never a
//! default. The caller's functions are a [`Host`], taking and giving
//! [`Value`]s.
//!
//! A run takes at most [`DEFAULT_FUEL`] steps unless [`Limits::fuel`]
//! says otherwise: one per instruction, one more per entry of a list,
//! `min` or `max` it makes, and one per word of a value a function
//! returns. Faults are errors, never panics: `Int` arithmetic that
//! overflows, dividing by zero, `list[i]` past either end (`lookup`
//! clamps instead), `clamp` with its bounds the wrong way round, and a
//! function's failure or wrongly typed result.
//!
//! ```
//! use evalite::{Environment, Limits, Machine, Type, Value};
//!
//! let mut env = Environment::with_builtins();
//! env.value("health", Type::record([("boxes", Type::Int), ("count", Type::list(Type::Int))]))?;
//! env.value("brawl", Type::option(Type::Int))?;
//! env.function("lookup", [Type::Int, Type::String], Type::Int)?;
//!
//! let program = evalite::compile(r#"health.count[1] + brawl.unwrap_or(0) + lookup(1, "Essence")"#, &env)?;
//!
//! let mut machine = Machine::new(Limits::new().fuel(100));
//! let mut run = machine.load(&program)?;
//! run.bind("health", Value::record([
//!     ("boxes", Value::Int(7)),
//!     ("count", Value::List(vec![Value::Int(3), Value::Int(1)])),
//! ]))?;
//! run.bind("brawl", Value::None)?;
//!
//! let mut host = |name: &str, args: Vec<Value>| match (name, args.as_slice()) {
//!     ("lookup", [Value::Int(1), Value::String(table)]) if table == "Essence" => Ok(Value::Int(10)),
//!     _ => Err(format!("no function {name}")),
//! };
//!
//! let outcome = run.run_with(&mut host)?;
//! assert_eq!(outcome.value, Value::Int(11));
//! assert_eq!(outcome.steps, 12);
//!
//! let error = Machine::new(Limits::new().memory(2)).load(&program).unwrap_err();
//! assert_eq!(error.to_string(), "the program needs 8 words of memory, more than the 2 the machine allows");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Structures
//!
//! A [`Structure`] is an expression as data, the shape a form builds: a
//! table of [`Node`]s, which may have holes (`_` as text) while it is being
//! edited. It encodes with `musli`, and compiles exactly as its
//! [`to_text`](Structure::to_text) would, with the same errors, each also
//! naming the [`NodeId`] it is about; a hole is an error. Writing one as
//! text and parsing that back gives the same structure.
//!
//! [`Structure::complete`] lists what fits in a hole, for a form to offer:
//! the values, paths into records, methods, calls, literals and operators
//! whose type the hole's place takes, which comes from what holds it. At a
//! member hole, `bonus._` with `bonus` an `Option<Int>` in an `Int`'s
//! place, that is `unwrap_or(_)`. [`Structure::apply`] fills the hole with
//! one, leaving holes for its arguments.
//!
//! ## Grammar
//!
//! Lowest precedence first; whitespace may separate any two tokens.
//!
//! ```text
//! expr     = or
//! or       = and ("||" and)*
//! and      = compare ("&&" compare)*
//! compare  = sum (("==" | "!=" | "<" | "<=" | ">" | ">=") sum)?
//! sum      = product (("+" | "-") product)*
//! product  = unary (("*" | "/" | "%") unary)*
//! unary    = ("-" | "!") unary | postfix
//! postfix  = primary ("." name args? | "[" expr "]")*
//! primary  = integer | string | "true" | "false" | name args?
//!          | "(" expr ")" | "[" (expr ("," expr)*)? "]"
//! args     = "(" (expr ("," expr)*)? ")"
//! name     = [A-Za-z_][A-Za-z0-9_]*
//! integer  = [0-9]+
//! string   = '"' ([^"\\] | '\"' | '\\')* '"'
//! ```
//!
//! Binary operators are left-associative, except comparisons, which do not
//! chain (`a < b < c` is an error). Every expression a computed widget
//! stores today is in this grammar: a value key such as `health[2]` or
//! `m["Brawl (bar)"]` is a name with field and index postfixes.
//!
//! ## Methods
//!
//! Methods are called Rust-style on a value, `value.method(args)`, and
//! chain: `some.field.unwrap_or(0).max(1)`. A name followed by `(` is a
//! call of a function; `.name` without `(` is a field.
//!
//! - `Int`: `abs()`, `min(Int)`, `max(Int)`, `clamp(Int, Int)`.
//! - `Option<T>`: `unwrap_or(T)`, `is_some()`, `is_none()`.
//! - `List<T>`: `len()`.
//!
//! ## Nesting
//!
//! Each parenthesis, list, call's arguments, index and unary operator
//! nests one level inside what holds it. Going past `max_depth` (32 unless
//! set with `Options::max_depth`) is an error at the construct that went
//! too deep. Chains of operators, fields and methods do not nest.

mod check;
mod complete;
mod env;
mod error;
mod lexer;
mod methods;
mod parser;
mod program;
mod structure;
mod syntax;
mod types;
mod vm;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod check_tests;

#[cfg(test)]
mod vm_tests;

#[cfg(test)]
mod structure_tests;

#[cfg(test)]
mod complete_tests;

pub use self::check::{compile, compile_with};
pub use self::complete::{CompleteError, Completion, CompletionKind, Insert};
pub use self::env::{Builtin, DeclareError, Environment};
pub use self::error::{Arity, Callee, Delimiter, Error, ErrorKind, Found, Nesting};
pub use self::parser::{DEFAULT_MAX_DEPTH, Options, Tree, parse, parse_with};
pub use self::program::{Memory, Program, StoreError};
pub use self::structure::{
    BinaryOp, EditError, HOLE, LoadError, Node, NodeId, Structure, StructureError,
    StructureErrorKind, UnaryOp,
};
pub use self::syntax::{Syntax, unescape};
pub use self::types::{MAX_TYPE_DEPTH, Type};
pub use self::vm::{
    DEFAULT_FUEL, DEFAULT_MEMORY, Host, Limits, Machine, Outcome, Run, RunError, Value,
};
pub use syntree::Span;
