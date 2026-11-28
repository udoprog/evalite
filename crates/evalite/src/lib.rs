//! <img alt="evalite" src="https://raw.githubusercontent.com/udoprog/evalite/main/assets/wordmark.svg" height="64">
//!
//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/evalite.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite)
//!
//! Small, strictly typed expressions that are safe to run on anyone's input.
//!
//! evalite is "eval lite": let users write formulas such as
//! `wits.min(dexterity) + athletics` or `bonus.unwrap_or(0) + 1`, check
//! them once against the names you declare, store the compiled program, and
//! run it later in a fixed budget of memory and steps. Every mistake is
//! caught before it runs, and no input can crash, hang or exhaust the host.
//!
//! **[Try it in your browser in the playground](https://udoprog.github.io/evalite/)**.
//!
//! - **Strictly typed.** Every expression has one type, checked before it
//!   runs, and nothing converts on its own. With `rank: Int` and
//!   `bonus: Option<Int>`, `rank + true` does not compile, and neither does
//!   `rank + bonus`, whose error says to unwrap the optional value first with
//!   `.unwrap_or(...)`. `rank + bonus.unwrap_or(0)` compiles to an `Int`.
//! - **Safe on untrusted input.** Whatever a user types, the worst outcome is
//!   an error. 40 nested parentheses hit the nesting limit instead of
//!   overflowing the stack, `rank * 9223372036854775807` reports that it
//!   overflows an `Int`, `dice[5]` on a list of 3 reports that the index is out
//!   of range, and an expression that needs more steps than it is allowed stops
//!   with "the expression took too long". See [Limits and
//!   safety](#limits-and-safety).
//! - **Compile once, run many times.** Check an expression when the user saves
//!   it and store the compiled [`Program`] as bytes next to your data. Running
//!   it later reads your values in place from a [`Globals`] store without
//!   parsing, checking or copying them again — 3–12× faster to evaluate than
//!   `evalexpr`, and level with or ahead of `fasteval` on all but the smallest
//!   expression, while keeping static typing and sandboxing. See [Storing
//!   programs](#storing-programs),
//!   [Values](#values) and [Performance](#performance).
//! - **Stored bytes are checked.** Bytes read back from a database or a file
//!   may be corrupt or tampered with, so [`Program::from_bytes`] checks them
//!   again and refuses anything that is not a valid, well-typed program.
//!   Cutting the last byte off gives "the bytes do not decode".
//! - **Errors that point.** Every mistake is reported, not only the first,
//!   and each says what it expected and what it found, while its [labels]
//!   mark where, so you can underline it in your own UI. `Int::min(1, (2 + 3)` gives "expected `)` to close the `(` at byte 8, found the
//!   end of the expression", pointing at the `(` and noting where the `)` was
//!   expected. See [Errors](#errors).
//! - **Autocomplete that only offers what fits.** Leave a hole in a
//!   [`Structure`] and [completions] list only what would type-check there. In
//!   `rank + _` you are offered `rank`, `bonus.unwrap_or(_)` and `Int::min(_, _)`,
//!   but never `name`, a `String`, or a bare `bonus`, an `Option<Int>`. That is
//!   enough to build an editor where users pick from lists and every pick is
//!   valid. See [Autocomplete](#autocomplete).
//!
//! <br>
//!
//! ## Quick start
//!
//! Declare what an expression may use, compile it, then set the values in a
//! store and run it:
//!
//! ```
//! use evalite::{Environment, Globals, Limits, Machine, Type, Value};
//!
//! let mut env = Environment::new();
//! env.value("wits", Type::int())?;
//! env.value("dexterity", Type::int())?;
//! env.value("bonus", Type::option(Type::int()))?;
//!
//! let program = evalite::compile("wits.min(dexterity) + bonus.unwrap_or(0)", &env)?;
//! assert_eq!(program.result_type(), Type::int());
//!
//! let mut globals = Globals::new(&env);
//! globals.set("wits", &Value::int(3))?;
//! globals.set("dexterity", &Value::int(2))?;
//! globals.set("bonus", &Value::some(Value::int(1)))?;
//!
//! let mut machine = Machine::new(Limits::new());
//! let outcome = machine.load(&program, &globals)?.run()?;
//! assert_eq!(outcome.value().as_int(), Some(3));
//!
//! // Mistakes are caught when compiling, and point at the input.
//! let errors = evalite::compile("wits + bonus", &env).unwrap_err();
//! let error = errors.iter().next().unwrap();
//! assert_eq!(
//!     error.to_string(),
//!     "`+` takes Int and Int, found Int and Option<Int>, unwrap the optional value first with `.unwrap_or(...)`",
//! );
//! assert_eq!(error.span().range(), 0..12);
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## The language
//!
//! An expression is one line, such as `health.boxes - damage.len()` or
//! `rank >= 3 && !flagged`. Every expression has one type, known when it is
//! compiled, and nothing converts implicitly:
//!
//! - `Int`: a 64-bit signed integer, such as `12` (`-12` is `-` applied to
//!   `12`), or `0x1A2B` in hexadecimal: `0x` or `0X` and one or more digits
//!   in either case, kept as hexadecimal when a [`Structure`] is edited.
//! - `Bool`: `true` or `false`.
//! - `String`: a literal such as `"Essence (1e)"`. `\"` and `\\` are its only
//!   escapes.
//! - `List<T>`: `[10, 11, 12]`, entries of one type. An empty `[]` takes its
//!   type from where it is written, as `None` does (below): `[[], [1]]`,
//!   `total([])`, `opt.unwrap_or([])`. With nothing to give it a type it is
//!   an error.
//! - Records, such as `{boxes: Int, count: List<Int>}`: a field is read with
//!   `.boxes`, or with `["Brawl (bar)"]` when its name is not a plain name.
//!   An expression writes one with names as its keys, `{boxes: 7, count: [1]}`,
//!   each field once; its values are evaluated in name order. Where a record
//!   type is expected, such as an argument, its fields' values are expected
//!   to be that record's fields (`{brawl: None, athletics: 1}`), and a field
//!   it lacks or has besides is an error.
//! - `Map<K, V>`: values by key, the key an `Int` or a `String`, which come
//!   from a name or a function declared with one, or from a literal such as
//!   `{1: "a", 0x2A: "b"}` or `{"x": wits}`. `m[k]` is the value of key `k`,
//!   an error when it is missing, and `m.get(k)` an `Option<V>`, `None` when
//!   it is missing. A lookup is a binary search of the keys, which are held
//!   sorted. A literal's keys are integers, decimal or hexadecimal and
//!   optionally negated, or strings, all of one type and each given once
//!   (`0x10` and `16` are the same key); its values are of one type, and
//!   evaluated in key order. Like `[]`, an empty `{}` takes its type from
//!   where it is written (`[{}, {1: 2}]`), and is an error without one.
//! - `Option<T>`: a value that may be absent, which comes from a name or a
//!   function declared with it, or from `list.get(i)`. It is never used as a `T`:
//!   unwrap it with `x.unwrap_or(0)`, which gives `0` when it is absent, or
//!   with `x!`, which stops the run with an error when it is absent. Test it
//!   with `x.is_some()` and `x.is_none()`. Options nest: an
//!   `Option<Option<Int>>` is `None`, `Some(None)` or `Some(Some(1))`, and
//!   `x!!` unwraps both.
//!
//!   An expression writes one as `Some(x)`, or `None` where the option type
//!   is known from where it is written: beside a typed entry of a list or
//!   map (`[None, Some(1)]`), as an argument (`bonus_or(None)`), or as
//!   `unwrap_or`'s default of an option of options, or beside `==`
//!   (`bonus == None`). Elsewhere, write its type after it: `None::<Int>`.
//!   A bare `None` with nothing to give it a type is an error.
//!
//! An expression that must be a value of a known type, such as the value of
//! a setting declared `Option<Int>`, is compiled with [`compile_as`], which
//! gives that type to the whole expression: `None`, `[]`, `{}` and record
//! literals take it (`{boxes: None}` as a `{boxes: Option<Int>}`), and any
//! other type is an error, `expected Option<Int>, found Int`.
//!
//! Types and values are written the same way outside an expression, such as
//! in a form or on a command line: [`Type::parse`] reads `Option<List<Int>>`,
//! `Map<Int, String>` or `{boxes: Int}`, and [`Value::parse`] reads a value
//! of a type, such as `Some([1, -2])`, `{1: "a", 42: "b"}` or `{boxes: 7}`.
//! Both read back what `Display` prints.
//!
//! Operators, tightest first. Binary operators are left-associative, except
//! comparisons, which do not chain (`a < b < c` is an error):
//!
//! | Operators                    | Takes                        | Gives  |
//! |------------------------------|------------------------------|--------|
//! | `x.field`, `x.method(..)`, `list[i]`, `map[k]` | a record, any value, a `List<T>` and an `Int`, a `Map<K, V>` and a `K` | its type |
//! | postfix `x!`                 | an `Option<T>`               | `T`, or an error when absent |
//! | unary `-`, `!`               | `Int`, `Bool`                | the same |
//! | `*` `/` `%`                  | `Int`s                       | `Int`  |
//! | `+` `-`                      | `Int`s                       | `Int`  |
//! | `<` `<=` `>` `>=`            | `Int`s                       | `Bool` |
//! | `==` `!=`                    | two `Int`s, two `Bool`s, or two options of them, as `bonus == None` | `Bool` |
//! | `&&`                         | `Bool`s, the right skipped when the left is `false` | `Bool` |
//! | <code>&#124;&#124;</code>    | `Bool`s, the right skipped when the left is `true`  | `Bool` |
//!
//! Methods, which chain, as in `bonus.unwrap_or(0).max(1)`. A `T` is whatever
//! the receiver holds, so `bonus.unwrap_or(0)` of an `Option<Int>` takes and
//! gives an `Int`:
//!
//! | On          | Methods                                                      |
//! |-------------|--------------------------------------------------------------|
//! | `Int`       | `abs() -> Int`, `min(Int) -> Int`, `max(Int) -> Int`, `clamp(Int, Int) -> Int` |
//! | `Option<T>` | `unwrap_or(T) -> T`, `is_some() -> Bool`, `is_none() -> Bool` |
//! | `List<T>`   | `get(Int) -> Option<T>`, `len() -> Int`                      |
//! | `Map<K, V>` | `get(K) -> Option<V>`, `len() -> Int`                        |
//!
//! `list.get(i)` is the entry at an index from 0, or `None` when the index
//! is out of range, so `[10, 20].get(1)` is `Some(20)`, an `Option<Int>`,
//! and `[10, 20].get(2)` is `None`. Unwrap it like any option, as in
//! `[10, 20, 30].get(rank).unwrap_or(0)`. On a list of options it gives an
//! option of an option, so an index out of range (`None`) is told apart from
//! an absent entry (`Some(None)`). `list[i]` is the entry itself, and an
//! error out of range.
//!
//! A map is read the same way, by its keys, as in
//! `specialties[1001].dice` or `tags.get("brawl").unwrap_or(0)`:
//!
//! ```
//! use evalite::{Environment, Globals, Machine, Type, Value};
//!
//! let mut env = Environment::new();
//! env.value("specialties", Type::map(Type::int(), Type::record([("dice", Type::int())])))?;
//!
//! let program = evalite::compile("specialties[1001].dice + specialties.len()", &env)?;
//! let specialties = Value::map([
//!     (Value::int(1001), Value::record([("dice", Value::int(2))])),
//!     (Value::int(7), Value::record([("dice", Value::int(1))])),
//! ]);
//!
//! let mut globals = Globals::new(&env);
//! globals.set("specialties", &specialties)?;
//!
//! let mut machine = Machine::default();
//! assert_eq!(machine.load(&program, &globals)?.run()?.value(), &Value::int(4));
//!
//! let program = evalite::compile("specialties[3].dice", &env)?;
//! let error = machine.load(&program, &globals)?.run().unwrap_err();
//! assert_eq!(error.to_string(), "the map has no key 3");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! A type's functions are called by path, as in `Int::min(a, b, c)`:
//!
//! | Function                    | Gives                              |
//! |-----------------------------|------------------------------------|
//! | `Int::min(Int, ...) -> Int` | the least of one or more `Int`s    |
//! | `Int::max(Int, ...) -> Int` | the greatest of one or more `Int`s |
//!
//! Every method is a function of its type too, taking the receiver first, so
//! `Int::abs(x)` is `x.abs()`, `Option::unwrap_or(bonus, 0)` is
//! `bonus.unwrap_or(0)` and `Map::get(m, k)` is `m.get(k)`.
//!
//! Every other name, value or function, is one the caller declares, with its
//! type, and the caller runs its functions when a program calls them. A
//! function name may be declared several ways, and a call takes the first
//! whose parameters match. Here `skill` is a character's rating in a skill,
//! by name, and with a specialty it may add a die for:
//!
//! ```
//! use evalite::{Environment, Type};
//!
//! let mut env = Environment::new();
//! env.value("wits", Type::int())?;
//! env.function("skill", [Type::string()], Type::int())?;
//! env.function("skill", [Type::string(), Type::string()], Type::int())?;
//!
//! let program = evalite::compile(r#"wits + skill("Brawl", "Grappling")"#, &env)?;
//! assert_eq!(program.result_type(), Type::int());
//!
//! let error = evalite::compile("skill(wits)", &env).unwrap_err();
//! assert_eq!(error.to_string(), "no `skill` takes (Int), it takes (String) or (String, String)");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! A function's last parameter may repeat, given at least a count of times
//! the caller chooses, with
//! [`Environment::function_repeated`]:
//!
//! ```
//! use evalite::{Environment, Type};
//!
//! let mut env = Environment::new();
//! env.function_repeated("lowest", [], Type::int(), 1, Type::int())?;
//!
//! assert!(evalite::compile("lowest(3, 1, 2)", &env).is_ok());
//!
//! let error = evalite::compile("lowest()", &env).unwrap_err();
//! assert_eq!(error.to_string(), "`lowest` takes one or more Int, found 0");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! The whole grammar, loosest first; whitespace may separate any two tokens:
//!
//! ```text
//! expr     = or
//! or       = and ("||" and)*
//! and      = compare ("&&" compare)*
//! compare  = sum (("==" | "!=" | "<" | "<=" | ">" | ">=") sum)?
//! sum      = product (("+" | "-") product)*
//! product  = unary (("*" | "/" | "%") unary)*
//! unary    = ("-" | "!") unary | postfix
//! postfix  = primary ("." name args? | "[" expr "]" | "!")*
//! primary  = integer | string | "true" | "false" | name args?
//!          | name "::" name args
//!          | "(" expr ")" | "[" (expr ("," expr)*)? "]"
//! args     = "(" (expr ("," expr)*)? ")"
//! name     = [A-Za-z_][A-Za-z0-9_]*
//! integer  = [0-9]+ | ("0x" | "0X") [0-9A-Fa-f]+
//! string   = '"' ([^"\\] | '\"' | '\\')* '"'
//! ```
//!
//! <br>
//!
//! ## Limits and safety
//!
//! Four limits bound everything an expression can do, so it is safe to run
//! whatever a user typed, and a store of values has a limit of its own:
//!
//! | Limit   | Bounds                                                              | Default                | Set with             |
//! |---------|---------------------------------------------------------------------|------------------------|----------------------|
//! | Nesting | each parenthesis, list, call, index and unary operator nests a level | [`DEFAULT_MAX_DEPTH`] (32) | [`Options::max_depth`] |
//! | Length  | bytes of the expression's text, checked before it is lexed | [`DEFAULT_MAX_LEN`] (65536) | [`Options::max_len`] |
//! | Memory  | 64-bit words for the stack, built lists, constants and values functions return | [`DEFAULT_MEMORY`] (1024)  | [`Limits::memory`]     |
//! | Fuel    | steps: one per instruction, plus one per list or map entry built, `Int` that `min`/`max` compares, word of the parts of a value a function is passed or returns or the expression gives, charged before it is read, and key or string-key word a map lookup compares | [`DEFAULT_FUEL`] (1000) | [`Limits::fuel`] |
//! | Store   | 64-bit words for the parts of a store's values: strings, list and map entries, record fields, freed ones included | [`DEFAULT_GLOBALS`] (2^20) | [`Globals::with_limit`] |
//!
//! A type has bounds of its own: `MAX_TYPE_DEPTH` (64) levels and
//! `MAX_TYPE_NODES` (65536) nodes written out, which `Environment::value`
//! and `Environment::function` refuse, and a repeated parameter's minimum
//! is at most `MAX_REPEATED_MIN` (32768), which
//! `Environment::function_repeated` refuses past. The types of the globals
//! and host functions an expression uses may have 2^21 nodes together, and
//! an expression that passes that is a compile error.
//!
//! Going too deep is a compile error at the construct that did; the parser
//! never recurses, so no input overflows the stack. A program's memory is
//! known when it is compiled, and [`Machine::load`] refuses one that needs
//! more than the machine has. Running out of fuel stops the run. There are no
//! loops, so most expressions use a few dozen steps. Faults are errors, never
//! panics: `Int` overflow, division by zero, `list[i]` past either end
//! (`list.get(i)` gives `None` instead), `map[k]` of a missing key
//! (`map.get(k)` gives `None` instead), `x!` of an absent option, `clamp` with its
//! bounds the wrong way round, and a host function's failure or wrongly typed
//! result.
//!
//! ```
//! use evalite::{Environment, Globals, Limits, Machine, Options};
//!
//! let env = Environment::new();
//! let globals = Globals::new(&env);
//!
//! let error = evalite::compile_with("(((1)))", &env, &Options::new().max_depth(2)).unwrap_err();
//! assert_eq!(error.to_string(), "this parenthesis nests too deeply, the limit is 2 levels of nesting");
//!
//! let program = evalite::compile("Int::min(1, 2, 3, 4, 5) + 9223372036854775807", &env)?;
//!
//! let mut machine = Machine::new(Limits::new().fuel(3));
//! let error = machine.load(&program, &globals)?.run().unwrap_err();
//! assert_eq!(error.to_string(), "the expression took too long: it used all of its 3 steps");
//!
//! let mut machine = Machine::new(Limits::new());
//! let error = machine.load(&program, &globals)?.run().unwrap_err();
//! assert_eq!(error.to_string(), "`1 + 9223372036854775807` overflows an Int");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## Storing programs
//!
//! A [`Program`] is everything needed to run an expression: its code, and the
//! names and types of the values it reads and functions it calls. Compile
//! when the user saves, store [`Program::to_bytes`], and run it on every
//! change without parsing or checking again.
//!
//! Stored bytes are not trusted. They start with a format version, and
//! [`Program::from_bytes`] refuses another version, then checks the program
//! again by running its code over types instead of values: it must be well
//! typed and use exactly the memory it declares. That check is what lets the
//! machine run on bare words without looking at them.
//!
//! An [`Environment`] holds each value in a slot of its own, given in
//! declaration order and never moved, so a name is declared once. Every
//! program compiled from it reads a value at the same slot, and records the
//! environment's [`Environment::fingerprint`], a stable hash of the names and
//! types in order. [`Environment::accepts`] tells whether a
//! stored program reads its values where an environment holds them: one
//! compiled against the same fingerprint does, and so does one whose every
//! value is declared with its type in its slot, as in an environment that
//! only declares more values after the ones it reads. A program read from
//! bytes is checked value by value, as the bytes may claim any fingerprint.
//! [`Machine::load`] refuses a program its store does not accept.
//!
//! A program carries no source text. To point run errors at the expression
//! that failed, [`compile_debug`] also gives a [`DebugInfo`], which can be
//! stored next to the program with [`DebugInfo::to_bytes`].
//!
//! ```
//! use evalite::{DebugInfo, Environment, Globals, Machine, Options, Program, Type, Value};
//!
//! let mut env = Environment::new();
//! env.value("rank", Type::int())?;
//!
//! let (program, debug) = evalite::compile_debug("10 / rank", &env, &Options::new())?;
//! let (stored, stored_debug) = (program.to_bytes()?, debug.to_bytes()?);
//!
//! let program = Program::from_bytes(&stored)?;
//! let debug = DebugInfo::from_bytes(&stored_debug)?;
//! assert_eq!(program.globals().collect::<Vec<_>>(), [("rank", &Type::int())]);
//!
//! let mut globals = Globals::new(&env);
//! globals.set("rank", &Value::int(0))?;
//!
//! let mut machine = Machine::default();
//! let error = machine.load(&program, &globals)?.run().unwrap_err();
//! let primary = error.labels(&program, Some(&debug)).next().and_then(|l| l.span());
//! assert_eq!(primary.map(|s| s.range()), Some(0..9));
//!
//! assert!(Program::from_bytes(&stored[..stored.len() - 1]).is_err());
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## Values
//!
//! The values an expression reads live in a [`Globals`] store, one per set of
//! values, such as one per character sheet. [`Globals::set`] checks a value
//! against its declared type and writes it once, and every run of every
//! program over the store reads it where it is: loading a program copies no
//! values, however large they are, so a sheet of a thousand formulas over one
//! large map pays for the map once, not once a formula. Setting a value again
//! replaces it, freeing the words of the old one for the values set after it,
//! and a store moves its values together when most of its words are free, so
//! it takes at most twice the words its values do.
//! [`Globals::set_at`] replaces one part of a value in place, a field, a list
//! entry or a map key's value, found by a [`Path`], so updating one entry of a
//! large map costs the entry, not the map. [`Globals::insert`],
//! [`Globals::push`] and [`Globals::remove`] add and take out a list's entries
//! or a map's keys, into lists and maps that keep room to grow, so adding an
//! entry takes amortized O(1) words. A run reads the store and never
//! writes to it, and reading a value that is not set is a run error naming it.
//! A store has a word limit of its own ([`Globals::with_limit`]), apart from
//! the machine's [`Limits`].
//!
//! ```
//! use evalite::{Environment, Globals, Machine, Path, Type, Value};
//!
//! let mut env = Environment::new();
//! env.value("health", Type::record([("boxes", Type::int()), ("damage", Type::list(Type::int()))]))?;
//! env.value("stamina", Type::int())?;
//!
//! let mut globals = Globals::new(&env);
//! globals.set("health", &Value::record([
//!     ("boxes", Value::int(7)),
//!     ("damage", Value::list([Value::int(1), Value::int(2)])),
//! ]))?;
//!
//! let formulas = ["health.boxes - health.damage.len()", "health.boxes + stamina"];
//! let programs = formulas
//!     .iter()
//!     .map(|f| evalite::compile(f, &env))
//!     .collect::<Result<Vec<_>, _>>()?;
//!
//! let mut machine = Machine::default();
//! assert_eq!(machine.load(&programs[0], &globals)?.run()?.value(), &Value::int(5));
//!
//! let error = machine.load(&programs[1], &globals)?.run().unwrap_err();
//! assert_eq!(error.to_string(), "a value the program reads is not set");
//!
//! globals.set("stamina", &Value::int(3))?;
//! assert_eq!(machine.load(&programs[1], &globals)?.run()?.value(), &Value::int(10));
//!
//! globals.set_at(&Path::value("health").field("boxes"), &Value::int(9))?;
//! assert_eq!(machine.load(&programs[0], &globals)?.run()?.value(), &Value::int(7));
//!
//! globals.push(&Path::value("health").field("damage"), &Value::int(3))?;
//! assert_eq!(machine.load(&programs[0], &globals)?.run()?.value(), &Value::int(6));
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! A store is saved as its values, not its words: [`Globals::to_bytes`]
//! writes each set value by name, and [`Globals::from_bytes`] reads them back
//! into a store of an environment, setting each as [`Globals::set`] does, so
//! bytes from anyone are checked against the declared types and the store's
//! limit. When a sheet's environment changes, [`Globals::migrate`] carries
//! the values of an old store over to a store of the new one: each whose
//! name is still declared with its type, or with fields only removed from its
//! records, and gives back every other value with why it was dropped.
//!
//! ```
//! use evalite::{Environment, Globals, Type, Value};
//!
//! let mut env = Environment::new();
//! env.value("health", Type::record([("boxes", Type::int()), ("bashing", Type::int())]))?;
//! env.value("stamina", Type::int())?;
//!
//! let mut globals = Globals::new(&env);
//! globals.set("health", &Value::record([("boxes", Value::int(7)), ("bashing", Value::int(1))]))?;
//! globals.set("stamina", &Value::int(3))?;
//! let saved = globals.to_bytes()?;
//!
//! // The next version of the sheet drops `bashing`, and `stamina` with it.
//! let mut next = Environment::new();
//! next.value("health", Type::record([("boxes", Type::int())]))?;
//!
//! let (globals, dropped) = Globals::migrate(&Globals::from_bytes(&env, &saved)?, &next);
//! assert_eq!(globals.get("health"), Some(Value::record([("boxes", Value::int(7))])));
//! assert_eq!(dropped.len(), 1);
//! assert_eq!(dropped[0].to_string(), "`stamina` is no longer declared");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## Errors
//!
//! Every error is opaque and implements `Display` and `std::error::Error`.
//! Compiling reports every mistake it finds as [`Diagnostics`], in source
//! order, at most [`MAX_DIAGNOSTICS`] of them. Each mistake is reported once:
//! an expression in error is not reported again by what uses it, so
//! `rank + true + bonus` reports `rank + true`, not `+ bonus` as well.
//! A syntax error is recovered from: the parser skips to where the
//! expression can go on, and what it could parse is still checked, so
//! `Int::min(rank, , true) + bonus` reports the missing argument and the
//! unknown `bonus`.
//!
//! Those that point into the input, each [`Diagnostic`], [`StructureError`]
//! and [`RunError`], give [`Label`]s: a message and the span it is about, the
//! primary label first and then what explains it, such as where an unclosed
//! bracket had to close or what was being done when a run failed. This is
//! enough to draw them under the input, as the CLI does:
//!
//! ```
//! use evalite::{Environment, Labels, Type};
//!
//! fn render(source: &str, labels: Labels<'_>) -> String {
//!     let mut out = String::new();
//!
//!     for label in labels {
//!         let prefix = if label.is_primary() { "error" } else { "note" };
//!         out.push_str(&format!("{prefix}: {}\n", label.message()));
//!
//!         if let Some(span) = label.span() {
//!             let column = source.get(..span.start as usize).map_or(0, |s| s.chars().count());
//!             let width = source.get(span.range()).map_or(1, |s| s.chars().count().max(1));
//!             out.push_str(&format!("  {source}\n  {}{}\n", " ".repeat(column), "^".repeat(width)));
//!         }
//!     }
//!
//!     out
//! }
//!
//! let mut env = Environment::new();
//! env.value("rank", Type::int())?;
//!
//! let source = "Int::min(rank, true) + bonus";
//! let errors = evalite::compile(source, &env).unwrap_err();
//! let rendered: String = errors.iter().map(|error| render(source, error.labels())).collect();
//!
//! assert_eq!(rendered, "\
//! error: argument 2 of `Int::min` must be Int, found Bool
//!   Int::min(rank, true) + bonus
//!                  ^^^^
//! error: unknown name `bonus`
//!   Int::min(rank, true) + bonus
//!                          ^^^^^
//! ");
//!
//! let source = "Int::min(1, (2 + 3)";
//! let errors = evalite::compile(source, &env).unwrap_err();
//! let error = errors.iter().next().unwrap();
//!
//! assert_eq!(render(source, error.labels()), "\
//! error: expected `)` to close the `(` at byte 8, found the end of the expression
//!   Int::min(1, (2 + 3)
//!           ^
//! note: expected `)` here
//!   Int::min(1, (2 + 3)
//!                      ^
//! ");
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## Autocomplete
//!
//! A [`Structure`] is an expression as a tree of [`Node`]s instead of text,
//! and it may have holes, written `_`, where something is still missing. This
//! is what a point-and-click editor builds. [`Structure::complete`] lists
//! what can go in a hole: only what gives the type that place needs, so every
//! pick compiles. [`Structure::apply`] puts a pick in, leaving new holes for
//! any arguments it takes. A structure converts to and from text, stores
//! with [`Structure::to_bytes`], and compiles with the same errors as its
//! text, each also naming the [`NodeId`] it is about.
//!
//! ```
//! use evalite::{Environment, Structure, Type};
//!
//! let mut env = Environment::new();
//! env.value("wits", Type::int())?;
//! env.value("bonus", Type::option(Type::int()))?;
//! env.value("name", Type::string())?;
//!
//! let mut form = Structure::from_text("wits + bonus._")?;
//! let hole = form.holes()[0];
//!
//! let fits = form.complete(&env, hole, None)?;
//! assert_eq!(fits.iter().map(|c| c.label()).collect::<Vec<_>>(), ["unwrap_or(_)"]);
//!
//! form.apply(hole, &fits[0])?;
//! assert_eq!(form.to_text(), "wits + bonus.unwrap_or(_)");
//! assert!(form.compile(&env).is_err());
//!
//! let hole = form.holes()[0];
//! let fits = form.complete(&env, hole, None)?;
//! assert!(fits.iter().any(|c| c.label() == "wits"));
//! assert!(!fits.iter().any(|c| c.label() == "name"));
//!
//! form.apply(hole, &fits.iter().find(|c| c.label() == "wits").unwrap())?;
//! assert_eq!(form.compile(&env)?.result_type(), Type::int());
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## References
//!
//! To see what changing a declared name would affect, such as renaming a
//! field or re-keying an item, [`references`] lists every name an
//! expression uses, each as a [`Reference`]: the value or function it
//! starts at, then the [`Step`]s of the path it reads, a field, a map's key
//! or a list's index. A key or an index that is computed, as in `list[n]`,
//! is a dynamic step, since the path past it is only partly known. The
//! expression must compile, since only its types tell a map's key from a
//! list's index. Each part has the span it is written at, and for a
//! [`Structure`], from [`Structure::references`], the [`NodeId`] it is part
//! of too.
//!
//! ```
//! use evalite::{Environment, Options, StepKind, Type, Value};
//!
//! let mut env = Environment::new();
//! let specialty = Type::record([("dice", Type::int())]);
//! env.value("specialties", Type::map(Type::int(), specialty))?;
//! env.value("rank", Type::int())?;
//!
//! let source = "specialties[0x1A2B].dice + specialties[rank].dice";
//! let refs = evalite::references(source, &env, &Options::new())?;
//! assert_eq!(refs.iter().map(|r| r.name()).collect::<Vec<_>>(), ["specialties", "specialties", "rank"]);
//!
//! let [key, dice] = refs[0].steps() else { unreachable!() };
//! assert_eq!(key.key(), Some(&Value::int(0x1A2B)));
//! assert_eq!(&source[key.span().range()], "0x1A2B");
//! assert_eq!(dice.name(), Some("dice"));
//! assert_eq!(refs[1].steps()[0].kind(), StepKind::Dynamic);
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! To make the change, [`rewrite`] replaces one [`Path`] with another
//! wherever an expression reads it: a value or a function renamed, a field
//! renamed along a path, or a key re-keyed. Only the bytes of the parts that
//! change are replaced, so the rest of the text stays as the user wrote it,
//! and [`Structure::rewrite`] edits a structure's nodes the same way. Given
//! the environment with the new names, [`Rewrite::check`] refuses a rewrite
//! whose result would not compile.
//!
//! ```
//! use evalite::{Environment, Options, Path, Rewrite, Type};
//!
//! let specialty = Type::record([("dice", Type::int())]);
//! let mut env = Environment::new();
//! env.value("specialties", Type::map(Type::int(), specialty.clone()))?;
//!
//! let mut renamed = Environment::new();
//! renamed.value("skills", Type::map(Type::int(), specialty))?;
//!
//! let source = "specialties[0x1A2B].dice * 2 + specialties[7].dice";
//!
//! let rekey = Rewrite::new(
//!     Path::value("specialties").int_key(0x1A2B),
//!     Path::value("specialties").int_key(0x1A2C),
//! )
//! .check(&env);
//! let (text, n) = evalite::rewrite(source, &env, &rekey, &Options::new())?;
//! assert_eq!((text.as_str(), n), ("specialties[0x1A2C].dice * 2 + specialties[7].dice", 1));
//!
//! let rename = Rewrite::new(Path::value("specialties"), Path::value("skills")).check(&renamed);
//! let (text, n) = evalite::rewrite(source, &env, &rename, &Options::new())?;
//! assert_eq!((text.as_str(), n), ("skills[0x1A2B].dice * 2 + skills[7].dice", 2));
//!
//! // Renaming only one of them leaves `specialties[7]`, which `renamed` does
//! // not declare.
//! let one = Rewrite::new(
//!     Path::value("specialties").int_key(7),
//!     Path::value("skills").int_key(7),
//! )
//! .check(&renamed);
//! assert!(evalite::rewrite(source, &env, &one, &Options::new()).is_err());
//! # Ok::<_, Box<dyn std::error::Error>>(())
//! ```
//!
//! <br>
//!
//! ## Performance
//!
//! evalite is built for compile-once, evaluate-many workloads over a shared
//! [`Globals`] store. In
//! [evalite-compare](https://github.com/udoprog/evalite/tree/main/crates/evalite-compare),
//! measured against `evalexpr` and `fasteval` on the same 16 integer variables
//! with compile and evaluate timed apart:
//!
//! - **Evaluation:** 3–12× faster than `evalexpr` across every expression
//!   measured, and faster than `fasteval` on the 16-variable sum (104 ns vs
//!   216 ns) and the 64-term formula (381 ns vs 1.12 µs) — while providing
//!   static typing, options, records, lists, maps and deterministic fuel and
//!   memory limits. `fasteval` (`f64`-only, without type checking or
//!   sandboxing) is level on `functions` and `logic` and stays ahead by
//!   ~25 ns only on the smallest expression, where fixed per-call overhead
//!   dominates.
//! - **Compilation:** 2–22 µs per expression — slower than both peers on the
//!   smaller expressions, and level with `evalexpr` on the 64-term formula, because
//!   `evalite::compile` parses, statically type-checks, generates bytecode and
//!   runs the verifier before anything executes (work `evalexpr` defers to
//!   evaluation and `fasteval` does not do).
//! - **Shared store:** values live in [`Globals`] and are read in place, so
//!   evaluating 400 formulas over a 10,000-entry map takes microseconds instead
//!   of copying the map per run.
//!
//! See the [benchmark charts in the
//! playground](https://udoprog.github.io/evalite/) or
//! [crates/evalite-compare/README.md](https://github.com/udoprog/evalite/blob/main/crates/evalite-compare/README.md)
//! for methodology and full numbers.
//!
//! <br>
//!
//! ## The command line
//!
//! The `evalite` command runs expressions from the terminal, which is a quick
//! way to try the language. Install it with `cargo install evalite-cli`.
//! Variables are declared and set with `--var name:Type=value`:
//!
//! ```text
//! $ evalite eval 'wits.min(dexterity) + bonus.unwrap_or(0)' --var wits:Int=3 --var dexterity:Int=2 --var 'bonus:Option<Int>=Some(1)'
//! 3 : Int
//!
//! $ evalite eval 'skills[3] * 2' --var 'skills:List<Int>=[1, 2]'
//! error: index 3 is out of range of a list of 2 entries
//!   | skills[3] * 2
//!   | ^^^^^^^^^
//!   = while running instruction 2
//!   = while running the program
//!
//! $ evalite eval '[10, 20, 30].get(rank)' --var rank:Int=7
//! none : Option<Int>
//!
//! $ evalite eval '[10, 20, 30].get(rank).unwrap_or(0) * 2' --var rank:Int=1 --explain
//! 40 : Int
//! type: Int
//! memory: 6 words at most (stack 3, heap 0, constants 3), 6 used
//! steps: 7
//! ```
//!
//! Without an expression it reads one per line from stdin. `--max-depth`, `--max-len`,
//! `--memory` and `--fuel` set the limits, and `evalite --help` lists the
//! rest.
//!
//! <br>
//!
//! ## What is public
//!
//! A type is public only if a caller needs to name it, and its insides are
//! private: no public fields, and no public enums of internal variants.
//! Callers build things through constructors, such as [`Value::int`], and
//! read them through methods that never panic, such as [`Value::as_int`].
//! Errors are opaque: they implement `Display` and `std::error::Error`, and
//! where they point into source text they are inspected through
//! `labels()`, as [`Diagnostic::labels`] is. A predicate is added only for a
//! named caller that must tell errors apart. Items only the crate's own
//! tests use are `pub(crate)`. Every public item is documented, with an
//! example covering its failure paths, which `#![deny(missing_docs)]` and
//! clippy's `missing_errors_doc` and `missing_panics_doc` enforce.
//!
//! [`compile_as`]: https://docs.rs/evalite/latest/evalite/fn.compile_as.html
//! [`compile_debug`]: https://docs.rs/evalite/latest/evalite/fn.compile_debug.html
//! [`DebugInfo::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.DebugInfo.html#method.to_bytes
//! [`DebugInfo`]: https://docs.rs/evalite/latest/evalite/struct.DebugInfo.html
//! [`DEFAULT_FUEL`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_FUEL.html
//! [`DEFAULT_GLOBALS`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_GLOBALS.html
//! [`DEFAULT_MAX_DEPTH`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MAX_DEPTH.html
//! [`DEFAULT_MAX_LEN`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MAX_LEN.html
//! [`DEFAULT_MEMORY`]: https://docs.rs/evalite/latest/evalite/constant.DEFAULT_MEMORY.html
//! [`Diagnostic::labels`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html#method.labels
//! [`Diagnostic`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html
//! [`Diagnostics`]: https://docs.rs/evalite/latest/evalite/struct.Diagnostics.html
//! [`Environment::accepts`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html#method.accepts
//! [`Environment::fingerprint`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html#method.fingerprint
//! [`Environment::function_repeated`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html#method.function_repeated
//! [`Environment`]: https://docs.rs/evalite/latest/evalite/struct.Environment.html
//! [`Label`]: https://docs.rs/evalite/latest/evalite/struct.Label.html
//! [`Limits::fuel`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.fuel
//! [`Limits::memory`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html#method.memory
//! [`Machine::load`]: https://docs.rs/evalite/latest/evalite/struct.Machine.html#method.load
//! [`Globals`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html
//! [`Globals::from_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.from_bytes
//! [`Globals::insert`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.insert
//! [`Globals::migrate`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.migrate
//! [`Globals::push`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.push
//! [`Globals::remove`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.remove
//! [`Globals::set`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.set
//! [`Globals::set_at`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.set_at
//! [`Globals::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.to_bytes
//! [`Globals::with_limit`]: https://docs.rs/evalite/latest/evalite/struct.Globals.html#method.with_limit
//! [`Limits`]: https://docs.rs/evalite/latest/evalite/struct.Limits.html
//! [`MAX_DIAGNOSTICS`]: https://docs.rs/evalite/latest/evalite/constant.MAX_DIAGNOSTICS.html
//! [`Node`]: https://docs.rs/evalite/latest/evalite/struct.Node.html
//! [`NodeId`]: https://docs.rs/evalite/latest/evalite/struct.NodeId.html
//! [`Options::max_depth`]: https://docs.rs/evalite/latest/evalite/struct.Options.html#method.max_depth
//! [`Options::max_len`]: https://docs.rs/evalite/latest/evalite/struct.Options.html#method.max_len
//! [`Path`]: https://docs.rs/evalite/latest/evalite/struct.Path.html
//! [`Program::from_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.from_bytes
//! [`Program::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Program.html#method.to_bytes
//! [`Program`]: https://docs.rs/evalite/latest/evalite/struct.Program.html
//! [`Reference`]: https://docs.rs/evalite/latest/evalite/struct.Reference.html
//! [`references`]: https://docs.rs/evalite/latest/evalite/fn.references.html
//! [`Rewrite::check`]: https://docs.rs/evalite/latest/evalite/struct.Rewrite.html#method.check
//! [`rewrite`]: https://docs.rs/evalite/latest/evalite/fn.rewrite.html
//! [`RunError`]: https://docs.rs/evalite/latest/evalite/struct.RunError.html
//! [`Step`]: https://docs.rs/evalite/latest/evalite/struct.Step.html
//! [`Structure::apply`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.apply
//! [`Structure::complete`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
//! [`Structure::references`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.references
//! [`Structure::rewrite`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.rewrite
//! [`Structure::to_bytes`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.to_bytes
//! [`Structure`]: https://docs.rs/evalite/latest/evalite/struct.Structure.html
//! [`StructureError`]: https://docs.rs/evalite/latest/evalite/struct.StructureError.html
//! [`Type::parse`]: https://docs.rs/evalite/latest/evalite/struct.Type.html#method.parse
//! [`Value::as_int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.as_int
//! [`Value::int`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.int
//! [`Value::parse`]: https://docs.rs/evalite/latest/evalite/struct.Value.html#method.parse
//! [completions]: https://docs.rs/evalite/latest/evalite/struct.Structure.html#method.complete
//! [labels]: https://docs.rs/evalite/latest/evalite/struct.Diagnostic.html#method.labels

#![deny(missing_docs)]
#![deny(clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod check;
mod complete;
mod debug;
mod env;
mod error;
mod framed;
mod globals;
pub mod highlight;
mod lexer;
mod literal;
mod memory;
mod message;
mod methods;
mod parser;
mod program;
mod references;
mod rewrite;
mod sig;
mod structure;
mod syntax;
mod types;
mod value;
mod vm;
mod word;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod check_tests;

#[cfg(test)]
mod vm_tests;

#[cfg(test)]
mod globals_tests;

#[cfg(test)]
mod persist_tests;

#[cfg(test)]
mod structure_tests;

#[cfg(test)]
mod complete_tests;

#[cfg(test)]
mod message_tests;

#[cfg(test)]
mod highlight_tests;

#[cfg(test)]
mod literal_tests;

#[cfg(test)]
mod types_tests;

#[cfg(test)]
mod references_tests;

#[cfg(test)]
mod rewrite_tests;

pub use self::check::{compile, compile_as, compile_debug, compile_with};
pub use self::complete::{CompleteError, Completion, CompletionKind};
pub use self::debug::DebugInfo;
pub use self::env::{DeclareError, Environment, MAX_REPEATED_MIN};
pub use self::error::{Diagnostic, Diagnostics, MAX_DIAGNOSTICS};
pub use self::globals::{
    DEFAULT_GLOBALS, DropReason, Dropped, Globals, GlobalsError, GlobalsStats,
};
pub use self::message::{Label, Labels, RunError, StoreError};
pub use self::parser::{DEFAULT_MAX_DEPTH, DEFAULT_MAX_LEN, Options};
#[cfg(test)]
pub(crate) use self::parser::{Parsed, Tree, parse_with};
pub use self::program::{Memory, Program};
pub use self::references::{Path, Reference, ReferenceKind, Step, StepKind, references};
pub use self::rewrite::{Rewrite, RewriteError, rewrite};
pub use self::structure::{
    BinaryOp, EditError, HOLE, Node, NodeId, Structure, StructureError, StructureErrors, UnaryOp,
};
pub(crate) use self::syntax::Syntax;
pub use self::types::{FieldTypes, MAX_TYPE_DEPTH, MAX_TYPE_NODES, Type};
pub use self::value::{Fields, Value};
pub use self::vm::{DEFAULT_FUEL, DEFAULT_MEMORY, Host, Limits, Machine, Outcome, Run};
pub use syntree::Span;
