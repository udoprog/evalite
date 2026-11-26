//! shex, the sheet expression language: small, strictly typed expressions
//! over a character sheet's values, such as a computed widget's
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
//! let tree = shex::parse("min(wits, dexterity) + athletics")?;
//! assert_eq!(tree.span().range(), 0..32);
//!
//! let error = shex::parse("1 +").unwrap_err();
//! assert_eq!(error.to_string(), "expected an expression, found the end of the expression");
//! assert_eq!(error.span().range(), 3..3);
//! # Ok::<_, shex::Error>(())
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
//! - `List<T>`: `[10, 11, 12]`, entries of one type.
//! - `Option<T>`: a value that may be absent. It is unwrapped explicitly,
//!   `x.unwrap_or(0)`, never used as a `T`.
//! - Records: values with fields, such as `specialties.athletics`; their
//!   shape comes from the names the caller declares.
//!
//! The caller declares every name an expression may use, values and
//! functions alike, with its type. Today's sheet functions are `min`,
//! `max`, `lookup`, `count` and `boxes`.
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
//! chain: `some.field.unwrap_or(0).max(1)`. Which methods a value has
//! depends on its type, such as `unwrap_or` on an `Option<T>` and `min`,
//! `max`, `abs` and `clamp` on an `Int`. A name followed by `(` is a call
//! of a function; `.name` without `(` is a field.
//!
//! ## Nesting
//!
//! Each parenthesis, list, call's arguments, index and unary operator
//! nests one level inside what holds it. Going past `max_depth` (32 unless
//! set with `Options::max_depth`) is an error at the construct that went
//! too deep. Chains of operators, fields and methods do not nest.

mod error;
mod lexer;
mod parser;
mod syntax;

#[cfg(test)]
mod tests;

pub use self::error::{Delimiter, Error, ErrorKind, Found, Nesting};
pub use self::parser::{DEFAULT_MAX_DEPTH, Options, Tree, parse, parse_with};
pub use self::syntax::{Syntax, unescape};
pub use syntree::Span;
