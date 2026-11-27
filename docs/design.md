# evalite design

This document describes how evalite works, and the decisions behind it, for
review. Sections 1 to 9 describe `main` as of 5554ba4 (references landed).
Section 10 is the planned globals store, decided but not built yet.

Every card that changes the design updates this file in the same commit,
including each decision taken and the alternatives it was weighed against
(board rule, 2026-11-27). The crate docs (`crates/evalite/src/lib.rs`) are the
user-facing reference. This file covers the internals and the reasons for them.

Source files are named relative to `crates/evalite/src/`.

## Contents

1. [Goals and invariants](#1-goals-and-invariants)
2. [The language](#2-the-language)
3. [Pipeline](#3-pipeline)
4. [The type table](#4-the-type-table)
5. [Memory layout](#5-memory-layout)
6. [The machine](#6-the-machine)
7. [Binding today](#7-binding-today)
8. [Stored formats and verification](#8-stored-formats-and-verification)
9. [Structures, completions and references](#9-structures-completions-and-references)
10. [Planned: the globals store](#10-planned-the-globals-store)

---

## 1. Goals and invariants

evalite runs small, strictly typed expressions written by untrusted users,
such as character-sheet formulas (`Int::min(wits, dexterity) + athletics`).
A host declares the names and functions an expression may use, compiles it
once, stores the program, and runs it many times.

The invariants every change must keep:

- **Untrusted input.** Source text, stored program bytes, stored structure
  bytes, `Type::parse`/`Value::parse` input, and structure edits may all come
  from an attacker. A host's own `Environment` and `Value`s are trusted for
  validity, but their size is still bounded where cheap to do so.
- **Bounded on any input.** Every stage is bounded in time, memory and stack:
  - nothing recurses over input-sized structures (all walks use explicit
    stacks);
  - no stage does work that grows faster than its input;
  - costs that come from user text, such as name suggestions, are bounded by
    what the host declared.
- **No panics** in runtime code (`no_panics` policy). The workspace denies
  `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `string_slice`
  and `arithmetic_side_effects` in clippy. Runtime code uses `get`,
  `checked_*` and `saturating_*` and returns errors.
- **Every mistake is an error, before running where possible.** Type errors,
  unknown names and syntax errors are compile errors. Overflow, division by
  zero, out-of-range indexes, missing map keys, unwrapping `none`, running out
  of fuel and host failures are run errors. Nothing converts implicitly.
- **Small and strictly typed.** Each expression has one type. There are no
  loops, no user-defined functions and no closures, so every run terminates
  within its code length times a constant, plus host calls.
- **Small public surface** (crate docs, "What is public"):
  - types are opaque, with no public fields and no public enums of internal
    variants;
  - values are built through constructors and read through try-accessors;
  - errors are inspected through `Display` and `labels()`.

## 2. The language

An expression is one line. The grammar (crate docs, "The language") is,
loosest first:

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
         | name "::" name args
         | "(" expr ")" | "[" (expr ("," expr)*)? "]"
args     = "(" (expr ("," expr)*)? ")"
name     = [A-Za-z_][A-Za-z0-9_]*
integer  = [0-9]+ | ("0x" | "0X") [0-9A-Fa-f]+
string   = '"' ([^"\\] | '\"' | '\\')* '"'
```

Comparisons do not chain: `a < b < c` is an error.

**Types** (`types.rs`, `Type`/`Kind`):

| Type | Notes |
|---|---|
| `Int` | i64. `-12` is `-` applied to `12`, folded to a constant. Hex literals `0x1A` keep their spelling in structures. |
| `Bool` | `true`, `false`. |
| `String` | Literals with only `\"` and `\\` escapes. No string operators. Strings are used as arguments, map keys and record indexes. |
| `List<T>` | `[a, b]`: one or more entries of one type. `[]` is an error. |
| `Option<T>` | Comes from declared names and functions, `list.get(i)` and `map.get(k)`. It nests: `Option<Option<Int>>` is `none`, `some(none)` or `some(some(1))`. |
| records | `{boxes: Int, "Brawl (bar)": Int}`, with fields sorted by name. Read with `.boxes`, or `["Brawl (bar)"]` for a field that isn't a valid name. Expressions have no record literals. |
| `Map<K, V>` | `K` is `Int` or `String`, which is checked at declaration (`DeclareError`) and in stored tables. Read with `m[k]` (a run error when missing) or `m.get(k)`. |

**Operators:**

- `+ - * / %` take `Int`s.
- `< <= > >=` take `Int`s and give `Bool`.
- `== !=` take two `Int`s or two `Bool`s, and nothing else.
- `&& ||` take `Bool`s and short-circuit.
- Unary `-` takes an `Int` and `!` a `Bool`.
- Postfix `x!` unwraps an option, and fails the run when it is absent.

**Methods** come from one table in `methods.rs` (`METHODS`), with signatures
written once as `sig::Shape` with one type variable `T`:

| On | Methods |
|---|---|
| `Int` | `abs()`, `min(Int)`, `max(Int)`, `clamp(Int, Int)` |
| `Option<T>` | `unwrap_or(T) -> T`, `is_some()`, `is_none()` |
| `List<T>` | `get(Int) -> Option<T>`, `len()` |
| `Map<Int, T>`, `Map<String, T>` | `get(K) -> Option<T>`, `len()` (listed once per key type, since `T` is the value type) |

**Path functions:**

- `Int::min(Int, ...)` and `Int::max(Int, ...)` take one or more arguments
  (`FUNCTIONS`).
- Every method is also a path function taking the receiver first, such as
  `Option::unwrap_or(x, 0)` and `Map::get(m, k)`. `Associated::for_receiver`
  picks the per-key-type map method.

**Host functions:**

- Declared with `Environment::function(name, params, ret)`.
- A name may have several overloads, and a call takes the first whose
  parameter types equal the argument types exactly.
- Redeclaring the same parameters replaces the return type.

**Names:**

- Value and function names are lowercase (`is_value_name`): a lowercase
  letter or `_`, then lowercase letters, digits and `_`.
- The lexer accepts any ASCII name, so `Strength` lexes, but no environment
  can declare it.
- Field names may be any string. Fields that aren't ASCII identifiers are
  written `["..."]`.

**Types and values as text** (`literal.rs`):

- `Type::parse` and `Value::parse` read what `Display` prints, iteratively,
  refusing input over `DEFAULT_MAX_LEN` and types over `MAX_TYPE_DEPTH`.
- `Value::parse` follows the type, so its nesting is bounded by the type's.
- Map values are written `{1: "a"}`. `Value::parse` refuses a duplicate key.
  `Value::map` sorts its entries and keeps the last of a repeated key.
- `Value::parse` reads `Int`s in decimal only: `0x1A` is refused there.

**Map literals** (f3b53baf, landed; decided by the user):

- Syntax is `{k: v, ...}`.
- Keys are literals only: an integer (decimal or hex, optionally negated) or a
  string. All keys are the same kind, which gives `Map<Int, V>` or
  `Map<String, V>`.
- A duplicate key is a compile error that points at both keys. `0x10` and
  `16` count as duplicates.
- All values have one type. `{}` is an error.
- Braces nest against `max_depth`.
- The literal compiles to the map layout of section 5. The checker sorts the
  keys once (O(n log n), finding repeats in the same pass) into a constant
  `List<K>`, and `Instr::Map { keys, map }` pops one value per key. The
  verifier checks that the keys constant is a non-empty, strictly ascending
  `List<K>`, so no verified program can build an unsorted map.
- **Values are evaluated in key order, not source order.** This is observable
  only through which run error or host call comes first. The alternative,
  checking key order at run time, would let a tampered but verified program
  fault. The stored program `FORMAT` is 5.
- An all-constant map literal is not folded into one constant (lists are);
  it is built at run time.
- Structures get a map node (with a structure `FORMAT` bump), and completion
  offers `{_: _}`.

## 3. Pipeline

```text
source text ──lexer──> tokens ──parser──> syntree Tree (with Error nodes)
     │                                           │
     │                     Environment ──> checker/compiler (check.rs)
     │                                           │  one iterative walk
     │                                           ▼
     │                              Program parts ──> Program::verify ──> Program
     │                                                       ▲              │
Structure ──render (to_text)──> source text                  │        Machine::load
                                     stored bytes ──decode───┘              │
                                                                      Run (bind…)
                                                                            │
                                                                     run / run_with
                                                                            ▼
                                                                    Outcome | RunError
```

A `Structure` compiles by rendering its text and compiling that text, so the
two have one checker and identical errors (section 9).

### 3.1 Lexer (`lexer.rs`)

- A cursor over `&str` that gives one token at a time, whitespace included.
- Rejected text, such as an unknown character, a lone `=`/`&`/`|`, a bad
  escape, an unterminated string or `0x` without digits, becomes a `Rejected`
  token plus a diagnostic. The lexer skips past it (a whole string literal
  when one starts there), so lexing always makes progress.
- `parse_int` checks the range: an integer past `i64` is a compile error at
  the literal.
- Bounds: linear in the input, with no allocation per token.

### 3.2 Parser (`parser.rs`)

- Builds a lossless `syntree` tree (`Syntax` kinds in `syntax.rs`). Every byte
  is covered, whitespace included, and nodes never end in whitespace.
- The flavor `Flavor` is compact: `u32` pointers and widths, and no token
  index (`EmptyVec`), since evalite never queries one.
- It is not recursive. It is a two-state loop (`Operand`/`Operator`) with
  three explicit stacks:
  - `frames`: pending prefix operators, binary operators and groups;
  - `groups`: open `(`, `[`, args and index;
  - binding powers: `OR` < `AND` < `COMPARE` < `SUM` < `PRODUCT`.
- Error recovery:
  - every syntax error is reported and wrapped in an `Error` node, which the
    checker treats as any type and never reports again;
  - `skip` consumes tokens up to a `,` of the innermost group, a closer, a
    binary operator or the end;
  - a mismatched closer closes the groups inside the one it matches, and each
    of those is reported once;
  - each token is consumed once, so recovery is linear.
- Bounds:
  - input longer than `u32::MAX` is refused (`TooLong`);
  - input longer than `Options::max_len` (`DEFAULT_MAX_LEN` = 64 KiB) is
    refused before lexing;
  - nesting: each prefix operator, parenthesis, list, argument list and index
    is a level, up to `Options::max_depth` (`DEFAULT_MAX_DEPTH` = 32). Past
    it, the construct is reported once (`TooDeep`) and its contents are
    skipped as an `Error` node without nesting further;
  - binary chains don't nest: `1+1+…+1` is flat in the frames.
- Diagnostics are collected by `Reporter` (`error.rs`), which keeps the
  `MAX_DIAGNOSTICS` (100) earliest in the source plus one "and N more" entry.
  Once full, each new report costs a constant 100-entry scan.

### 3.3 Checker and compiler (`check.rs`)

- One iterative walk over the tree, using a `Work` stack of
  enter-node/after-children items. It types every node and emits code in the
  same pass. Values on the compiler's stack carry their `TypeId` and span.
- An **error type** (`TypeId(u32::MAX)`, never in the table) is given to
  anything in error. Every rule passes it through silently, so each mistake is
  reported exactly once (`rank + true + bonus` reports only `rank + true`).
- Names:
  - an unknown name or function gets a suggestion from `suggest`: a
    case-insensitive match, or the unique nearest within two edits;
  - candidates whose length differs by more than the limit are skipped before
    any Levenshtein work, so the cost is bounded by the declared names, not by
    the input.
- Globals:
  - today each program interns its globals on first use (`global_ids`);
  - each global gets the next word offset in the program's own globals area
    (`global_words`);
  - section 7 describes this, and section 10 replaces it.
- Host calls are deduplicated by (name, argument types) into the program's
  `functions` table.
- Constants:
  - strings are interned once per program;
  - a list whose entries are all literals (ints, bools, strings, constant
    lists, and negated ints, which are folded) becomes a `Constant::List`;
  - other lists emit `Instr::List { len, entry }`.
- `&&`/`||` emit `And(skip)`/`Or(skip)` with a forward skip that is patched
  after the right operand.
- `finish` hands the parts to `Program::verify`. The compiler never states the
  program's memory: the verifier computes it, so compiled and stored programs
  are checked the same way.
- References (section 9.3): when asked, the same walk records every reference
  through a `Recorder`. There is no second checker.
- Bounds:
  - linear in the tree;
  - types are interned (section 4);
  - a type nesting past `MAX_TYPE_DEPTH` is a compile error (`TypeTooDeep`);
  - counts past `u32` are `TooLarge`.

### 3.4 Program (`program.rs`)

A `Program` holds:

- `types`: its own `Types` table;
- `constants`;
- `globals`: a (name, type) list in first-use order;
- `functions`: (name, params, ret);
- `code`: a `Vec<Instr>`;
- `result`: the result type;
- `memory`: `Memory { stack, heap, constants, globals }` in words.

It also caches `global_types` (the globals' types, built once),
`global_at` (each global's word offset) and `constant_at`.

It carries no source text. `DebugInfo` (a span per instruction) is optional,
from `compile_debug`, and stored separately.

### 3.5 Verifier (`Program::verify`)

The verifier runs the code once over types instead of values (`Model`: a
stack of `TypeId`s and a running word count). It checks:

- every constant is well typed, and a constant list's `Scalar::Const` entries
  refer only to earlier constants;
- globals: valid, unique names and known types, with offsets assigned from
  their widths;
- host functions: valid names, a unique (name, params), known types;
- each instruction's operands match its type operands (`Field` at an existing
  offset with that type, `Index`/`Get` on that list type, `MapIndex`/`MapGet`
  on that map type, and so on);
- `==`/`!=` take only `Int`/`Int` or `Bool`/`Bool`;
- jumps only go forward and within the code, both landing paths agree on the
  stack, and no jump lands in nothing;
- exactly one value of the result type is left.

It computes the stack's peak in words, and heap as the sum of the words of
lists built. Because every word's kind is proven here, the machine never
inspects what a word is.

It also bounds the globals' types written out: the sum per global, and
everything they reach, are each at most `MAX_GLOBAL_TYPE_NODES` (2^21), since
`Program::globals()` hands out built `Type`s.

### 3.6 Machine (`vm.rs`)

Section 6.

### 3.7 Where the limits are

| Limit | Value | Where | Bounds |
|---|---|---|---|
| `Options::max_len` | 64 KiB default | parser | source bytes, before lexing |
| `u32` source | 4 GiB | parser | spans fit `u32` |
| `Options::max_depth` | 32 default | parser | nesting of prefix ops and groups |
| `MAX_DIAGNOSTICS` | 100 | `Reporter` | diagnostics kept |
| `MAX_TYPE_DEPTH` | 64 | `Environment`, `Types`, literals, stored tables | type nesting |
| `MAX_TYPE_NODES` | 65 536 | `Types::add`, stored tables | one type's nodes with shared parts written out |
| `MAX_GLOBAL_TYPE_NODES` | 2^21 | verifier | a program's globals' types written out, together |
| `Limits::memory` | 1024 words default | `Machine` | stack + heap + constants + globals + bound/returned parts |
| `Limits::fuel` | 1000 default | `Run` | steps per run |
| `MAX_STEPS` | 4096 | completions | type walks per value when searching paths |
| `MAX_KEY_SHOWN` | 64 chars | messages | a string key quoted in an error |
| `MAX_QUOTED` | 32 chars | literals | input quoted in an error |
| `Word::reference` | 32 + 32 bits | machine | addresses and lengths fit `u32` |

## 4. The type table

`Types` (`types.rs`) is a per-program, append-only table of `TypeNode`s:
`Int`, `Bool`, `String`, `List(id)`, `Option(id)`, `Record(Vec<FieldNode>)`
(sorted by name, each name once) and `Map { key, value }`. `Map` is appended
last, which keeps the stored encoding of the older variants.

- **Parts come first.** A node only refers to earlier ids, so the table is
  never recursive data, and every walk over it is a forward or backward scan.
- **Interning.** Each distinct node is in the table once, so type equality is
  `TypeId` equality. That is how the checker and verifier compare types.
  - The leaves are found without hashing (`leaves`).
  - Other nodes are found through `heads: HashMap<u64, TypeId>` keyed by the
    node's hash, with collisions chained through `next`. `Passthrough` makes
    the map use the precomputed hash as is.
  - `intern(&Type)` builds a `Type` tree bottom-up with an explicit work list.
    It finds an existing record without cloning field names.
- **Hashing.** A small Fx-style hasher (`FxHasher`: rotate, xor, multiply by
  `0x517cc1b727220a95`).
  - Stored tables are untrusted, so record names could be chosen to collide.
    Each table therefore starts from a random seed
    (`RandomState::new().hash_one(0u8)`), which the writer cannot know.
  - The seed makes hashes differ between tables and processes. They are never
    stored and never used for identity outside one table, and the planned
    layout fingerprint (section 10) must not reuse them.
- **Per-node facts**, computed once when a node is added:
  - `depths`: nesting, refused past `MAX_TYPE_DEPTH`;
  - `sizes`: nodes with shared parts written out, saturating, refused past
    `MAX_TYPE_NODES`;
  - `widths`: the words a value of the type takes where it is held
    (section 5).
- **Back to `Type`.** `to_type`/`to_types` build `Type` trees from the leaves
  up over the ids reached (`reach`), each reached id once. `reached_size`
  gives what such a build holds, which the verifier bounds for globals.
- **Stored tables** (`from_stored`):
  - every part comes before what holds it;
  - records are sorted with unique names;
  - map keys are `Int` or `String`;
  - nothing is too large or too deep;
  - no node repeats (each must be added at its own index).

## 5. Memory layout

Memory is a flat array of 64-bit `Word`s. `word.rs` is the only code that
knows the bit layout.

| Kind | Words where held | Encoding |
|---|---|---|
| `Int` | 1 | two's complement i64 |
| `Bool` | 1 | 0 or 1 (anything else is a fault when read as a Bool) |
| `String` | 1 | reference: (start, byte length); bytes packed 8 per word, little-endian |
| `List<T>` | 1 | reference: (start, entries); entries `width(T)` words each, contiguous |
| record | 1 | reference: (start, words); fields at fixed offsets in name order |
| `Map<K, V>` | 1 | reference: (start, entries); entries `1 + width(V)` words, sorted by key |
| `Option<T>`, `T` with a niche | 1 | `T`'s word, or `Word::NONE` (all ones) when absent |
| `Option<T>`, `T` without a niche | 1 + `width(T)` | tag word (0/1), then `T`'s words, zero when absent |

**References.** A reference packs the address in the low 32 bits and the
length in the high 32 bits. The one pair that would equal `Word::NONE`
(`u32::MAX` parts at `u32::MAX`) is refused, so no reference is all ones.

**Niches.** `Bool`, `String`, `List`, record and `Map` values are never
`Word::NONE`, so their options take one word. `Int` uses every bit pattern,
and an option's absence is itself a value of the outer option, so
`Option<Int>` and `Option<Option<_>>` are tagged:

```text
Option<Int>            [tag][int]                 2 words
Option<Option<Int>>    [tag][tag][int]            3 words
Option<Option<Bool>>   [tag][bool-or-NONE]        2 words
Option<List<Int>>      [ref-or-NONE]              1 word
```

**Records** are not inline: a record is one reference word to its fields'
block. So every value is one word where held, except tagged options.
`field_offsets` gives each field's offset in name order:

```text
{a: Int, b: Option<Int>, c: List<Int>}
  ref ──> [a][b.tag][b.int][c ref] ──> [c0][c1]...
           0   1      2      3
```

**Maps** are sorted entry blocks. `Int` keys sort by value and `String` keys
by their bytes (`key_order`, which matches `Value::map`'s sort). A lookup is a
binary search over the entries (`find_key`), comparing strings word by word:

```text
Map<Int, Option<Int>>   ref(start, 3) ──> [k0][tag][v][k1][tag][v][k2][tag][v]
                                           stride = 1 + width(V) = 3
```

Binding refuses unsorted entries (`Fault::MapUnsorted`). `Value::map`
always sorts, so a host can't produce them through the public API.

## 6. The machine

### 6.1 Memory regions

A `Machine` reserves `Limits::memory` words up front (`words: Box<[Word]>`)
and runs one program at a time. `Machine::load` refuses a program whose
`memory().total()` exceeds the machine, copies the constants in, and gives a
`Run`:

```text
0          globals      len ──>                limit       stack_base        end
| globals area | bound parts, built lists, host results | constants | stack ──> |
                 ^ grows up (alloc), up to limit         copied at   stack_cap words,
                   minus the heap still reserved         load        sp deep
```

- The **globals area** holds each read global's words at its `global_at`
  offset.
- The **heap reservation** (`heap`) is the verifier's count of list words the
  code may build. Bound and returned values may use only
  `limit - len - heap` (`left()`), so the program's own lists always fit.
- `Outcome::memory` is the peak of `len`, plus the peak of `sp`, plus the
  constants.

### 6.2 Instructions

The `Instr` enum in `program.rs`:

| Instr | Effect |
|---|---|
| `Int(i64)`, `Bool(bool)` | push a literal |
| `Const(c)` | push a reference to constant `c` |
| `Global { at, ty }` | push the `width(ty)` words at offset `at` of the globals area |
| `Neg`, `Not`, `Abs` | unary on the top word |
| `Add` `Sub` `Mul` `Div` `Rem` | checked arithmetic; overflow and division by zero are run errors |
| `Lt` `Le` `Gt` `Ge`, `Eq` `Ne` | comparisons (`Eq`/`Ne` compare one word) |
| `And(skip)`, `Or(skip)` | pop a Bool. If it decides the result, push it back and skip forward `skip` instructions |
| `Field { at, ty }` | pop a record reference, push its field at offset `at` |
| `Index(entry)`, `Get(entry)` | list indexing: an error, or an option, when out of range |
| `List { len, entry }` | move the top `len` entries from the stack into a new heap block |
| `Min(n)`, `Max(n)`, `Clamp` | `Int` folds. `Clamp` errors when low > high |
| `UnwrapOr(opt)`, `Unwrap(opt)`, `IsSome(opt)`, `IsNone(opt)` | options, tagged or niche, decided by `opt`'s type |
| `Len` | a list's or map's length from its reference |
| `Call(f)` | call host function `f` |
| `MapIndex(map)`, `MapGet(map)` | binary search. A missing key is an error naming the key, or an absent option |

Operands are pushed in source order, so a binary operator finds its right
operand on top. Instructions whose work depends on a type carry the `TypeId`,
and the verifier has checked it against the stack model. The machine still
bounds-checks every memory access (`get`, `put`, `inside`). A failed check is
a `Fault`, meaning "a program evalite did not make", which compiled or
verified programs never cause.

### 6.3 Fuel

`Limits::fuel` (default 1000) steps per run:

- one per instruction;
- plus `len` for `List`;
- plus `n` for `Min(n)`/`Max(n)`;
- plus the words of the parts of each value a host function returns.

Running out is a run error. There are no backward jumps, so even without
fuel a run is bounded by its code length. Fuel bounds the work per run that
the host pays for.

The crate docs' Limits table omits the `Min`/`Max` cost.

Binding costs no fuel. It is bounded by the value's size, which the host
supplies.

### 6.4 Host calls

- The host is the `Host` trait, which any `FnMut(&str, Vec<Value>) ->
  Result<Value, String>` implements.
- `Call(f)` pops the arguments, reads them out as `Value`s, and calls
  `host.call(name, args)`.
- The returned value is checked against the declared return type (`measure`),
  charged to fuel, checked against `left()`, and written onto the stack, with
  its parts in the heap area.
- A host error becomes `RunError` `HostFailed`, with the host's message as
  its `source()`.
- `run()` uses `NoHost`, whose every call fails.

### 6.5 Errors and labels

- **Compile errors** are `Diagnostics`: a list of `Diagnostic { span, kind }`
  in source order (`error.rs`). Their `labels()` give a primary label, plus
  notes such as where an unclosed group needed closing.
- **Run errors** are `RunError` (`message.rs`): a chain of `Frame`s, one
  `Message` each, with an optional `Location::Instr(pc)`.
  - The `.context(...)` pattern adds frames, such as "while binding `x`",
    "while running instruction 2" and "while running the program".
  - Messages hold plain ids (globals, functions, `TypeId`s). Names are
    resolved against the `Program` only when the error is shown.
  - `labels(&program, Some(&debug))` maps each instruction location through
    `DebugInfo` to a source span.
- **Store errors** are `StoreError`, with context frames such as "at
  instruction 0" and "while verifying the program".
- **Binding mismatches** name the path into the value (`Root` plus `Seg`s:
  field, index, map key or key-of-entry). The first walk keeps no path. Only
  on a mismatch is it re-run with a `Nodes` trail to name where.

## 7. Binding today

Today every program has its own globals area, laid out at compile time:

- The compiler gives each global the next word offset on first use
  (`global_ids`, `global_words`), so the same name has different offsets in
  different programs.
- `Program::globals` lists them in that order, and the verifier recomputes
  the offsets (`global_at`) from the widths.
- `Memory::globals` is the area's size in words. It counts only the slots,
  not the parts.

A run binds each global before it runs:

1. `run.bind(name, &value)` finds the global by name with a linear scan of
   the program's globals. It refuses an unknown name (`UnknownValue`) and a
   second binding (`AlreadyBound`).
2. `measure` walks the value against its type without recursion, giving the
   words its parts need, or a path-named mismatch.
3. It checks those words against `left()` (`TooLarge`).
4. `write` writes the slot words, and allocates and fills the parts from
   `len` upward.
5. `run_with` refuses to start while any global is unbound (`Unbound`).

The cost per run is proportional to the size of every value bound, even when
the program reads one field of one global. Every run of every formula on a
sheet re-binds the sheet's values. That cost is what the globals store
(section 10) removes.

## 8. Stored formats and verification

All stored forms are encoded with musli `storage` and start with a
`format: u32`.

**Programs:** `Program::to_bytes`/`from_bytes`, `FORMAT` = 4 (4 added `Map`).
The stored form is
`Stored { format, types, constants, globals, functions, code, result, memory }`.
`from_bytes`:

1. decodes the bytes (`NotDecoded` on failure);
2. requires `format == FORMAT` exactly (`WrongFormat`): older programs are
   recompiled, not migrated;
3. checks the type table (`Types::from_stored`, section 4);
4. runs the full verifier (section 3.5);
5. requires the declared `memory` to equal the computed memory
   (`MemoryMismatch`).

What a verified program guarantees:

- every instruction is well typed against a sound stack model;
- jumps are forward and consistent;
- one result of the declared type is left;
- the memory it declares is exactly what it uses;
- names are valid and unique;
- all types are in bounds.

So the machine may run it on bare words, and any remaining fault is a bug in
evalite, not in the bytes.

**Debug info:** `DebugInfo::to_bytes`/`from_bytes` is a list of
`(start, end)` spans, and refuses a span that ends before it starts. It is
not versioned and is kept apart from the program.

**Structures:** `Structure::to_bytes`/`from_bytes`, `FORMAT` = 2 (2 added
`Repr::Hex`). It reads formats 1 and 2. The stored form is
`Stored { format, nodes, root }`, with only the live nodes, renumbered in
post order. `from_bytes` checks:

- every part index exists;
- no node has two parents, and the root has none;
- the root exists;
- every node is reachable from the root, which together with the parent
  check rules out cycles.

These checks are linear.

**What is not stored:**

- source text;
- environments;
- bound values;
- hashes.

`Type`/`Value` text forms (section 2) round-trip through `Display`.

## 9. Structures, completions and references

### 9.1 Structures (`structure.rs`)

A `Structure` is an expression as a table of `Node`s for a point-and-click
editor:

- `nodes: Vec<Node>`;
- `parents: Vec<Option<NodeId>>`;
- `root`.

A `Node` wraps a private `Repr`, one variant per expression kind:

- `Hole`, `Int`, `Hex`, `Bool`, `String`, `Name`;
- `Unary`, `Binary`, `Call`, `Method`, `Field`, `Index`, `List`, `Unwrap`.

A method or field whose name is `None` is a *member hole*. The encoding is the
stored form's, so variants keep their order.

- **Tree invariant.** Every node is part of at most one other, and the root of
  none. Edits (`add`, `replace`, `set_root`, `graft`) check it in
  `check_free`:
  - each part exists;
  - each part is used once;
  - no part is the root or already attached;
  - no part is an ancestor of the node being replaced (walking up, bounded by
    the node count).

  Nodes orphaned by edits stay in the table until stored.
- **Text.**
  - `to_text`/`text_of` render iteratively (`render`), adding parentheses
    where binding power needs them (`Repr::power`; negative literals bind as
    prefix).
  - `from_text` parses and builds nodes, dropping parentheses and reading
    `_` as a hole.
- **Compiling.**
  1. `check_nodes` reports every hole and every name that can't be written
     (invalid or reserved: `true`, `false`, `_`), in text order.
  2. Otherwise the rendered text goes through `parse_with` and
     `compile_parsed`.
  3. Each diagnostic is mapped to the innermost node whose rendered span
     holds it (`locate`).

  So a structure compiles exactly as its text, with the same program and the
  same errors, plus `StructureError::node`.

### 9.2 Completions (`complete.rs`)

`Structure::complete(env, at, root_type)` lists what fits at node `at`:

1. **What the place wants** (`want`). Walk up from `at` to the nearest parent
   whose expected type is known:
   - an operator's operand, or `==`'s other side;
   - a method or call argument from its signature or the host overloads that
     still fit;
   - a field or method name of the receiver;
   - an index (Int for lists, the key type for maps, String for records);
   - a list entry, from a sibling's type.

   Passing through lists without a typed sibling, `[_]` receivers and `!`
   records `Up` steps (`Entry`, `ListOf`, `OptionOf`), which are applied back
   down. The result is `Want::Any` or `OneOf(patterns)`. The walk is bounded
   by the node count. Sibling and receiver types come from compiling those
   subtrees (`compile_at`).
2. **Candidates at a value place** (`values`):
   - *paths* from every env value: the value, `x!`, `x.method(_…)`, and the
     fields and `[_]` entries that can still reach a wanted type
     (`Want::reachable`), each walk capped at `MAX_STEPS`;
   - host functions whose return type fits;
   - `Int::min`/`Int::max` by shape;
   - literals `0`, `true`, `false`, `""`;
   - `[_]`;
   - `Int` operators and `-_`, `Bool` operators and `!_`;
   - `_!`.

   At a record's index place, the record's field names are offered as string
   literals.
3. **At a member hole** (`members`): the receiver is compiled for its type.
   Fields and methods whose result reaches the wanted type are offered.
4. Candidates are deduplicated by label. A candidate whose label would exceed
   `DEFAULT_MAX_LEN` is dropped, so every pick parses back.

**Candidate recipes.** A `Completion` holds an `Insert`: an `Expr` recipe, a
`Field(name)` or a `Method { name, args }`. `Expr` is one of:

- `Literal`, `List`, `Binary`, `Unary`, `Unwrap`;
- `Call { function, args }`;
- `Path { name, steps: [Field | Entry], end: Value | Unwrap | Method }`.

`apply(at, completion)` builds the recipe in place, adding hole nodes for
every argument, entry index and operand, then `replace`s `at`. Nodes are made
only for the completion applied. A field that isn't a valid name becomes
`["name"]`, and a method keeps its existing arguments when the arity matches.

**Highlighting** (`highlight.rs`, public module): the lexer's tokens are
classified (keyword, ident, type, member, number, string, operator,
punctuation, whitespace, error) over the whole source, with rejected text as
`Error`. It is linear.

### 9.3 References (`references.rs`, landed in 5554ba4, b128c319)

`evalite::references(source, &env, &options)` and
`Structure::references(&env, &options)` list every name an expression uses,
with the path it reads, in source order:

- `Reference`: `kind()` (`ReferenceKind::{Value, Function}`,
  `#[non_exhaustive]`), `name()`, `span()` of the name, `node()` (the
  `Name`/`Call` node, for structures), and `steps()`.
- `Step`: `kind()` (`StepKind::{Field, Key, Index, Dynamic}`,
  `#[non_exhaustive]`), `name()` (a field, however it is spelled), `key()`
  (an `Int`/`String` `Value`), `index()`, `span()` (exactly the bytes to
  patch: the name, `"x y"` with quotes, the literal with its `-`, or the whole
  computed expression), and `node()`.
- Paths continue through `!`, `unwrap_or` and list/map `get`, including the
  path forms `Option::unwrap_or(o, d)` and `Map::get(m, k)`, because what they
  give is still part of the value. `get` adds a step. Paths end at anything
  else (`len`, `is_some`, arithmetic). So `a.unwrap_or(b).c` reports `a.c`
  and `b` alone. A host call is a root, and its path continues from its
  result (`f(a).dice`).
- How it works:
  - The checker records the references itself. Each compiler stack value
    carries an optional reference index, and `Recorder` appends steps as
    names, fields, indexes, methods, path calls, unwraps and calls are typed.
  - The expression must compile, and the program is still verified.
  - Structure node ids come from the rendered spans through one `HashMap`
    (`locate_nodes`). It is one linear pass.
  - When nothing is recorded, compile carries only an extra `None` per value.

Decisions (user, 2026-11-27, on b128c319; shape by the lead on landing):

| Question | Picked | Alternatives |
|---|---|---|
| What counts as a reference | globals, fields, literal keys and indexes, and host function names, as paths; computed steps marked `Dynamic` | only globals and field paths; only literal map keys |
| Typed or untyped | typed: needs the `Environment`, and the expression must check | untyped (syntax shape only); both |
| How it is computed | recorded by the checker's own walk | a second, reference-only type walk |
| API shape | `Reference`/`Step` structs plus `#[non_exhaustive]` kind enums, like `Completion`/`CompletionKind` | a data-carrying `Step` enum, which would make spans and node ids public fields |
| Where paths continue | through `!`, `unwrap_or`, `get` | end at every method |

Not exposed: whether a key literal was written in hex. Phase 2 reads it from
the span text or the `Hex` node.

### 9.4 Rewrite (`rewrite.rs`, 2a7885af)

`evalite::rewrite(source, &env, &rewrite, &options) -> (String, usize)` and
`Structure::rewrite(&env, &rewrite, &options) -> usize` replace one path with
another wherever an expression reads it, or a path past it, and give how
many references were rewritten:

- `Path` is the host's owned path: `Path::value(name)` or
  `Path::function(name)`, then `.field(name)`, `.int_key(n)`,
  `.string_key(s)` and `.index(n)`. It has literal steps only, and
  `Display`s as `specialties[26].dice`. `Reference::starts_with(&path)` tests
  a reference against it. A dynamic step starts with no literal step.
- `Rewrite::new(from, to)`, optionally `.check(&new_env)`. `from` and `to`
  must start at the same kind of name and have the same kinds of steps. Each
  part that differs is replaced in place, which covers renaming a value, a
  host function or a field along a path, and re-keying. `to`'s root must be a
  valid name.
- References are found against `env`, the environment the expression
  compiles against now. With `check`, the result must compile against that
  environment (usually the one with the new names), else nothing changes.
  The check also runs when nothing matched, so it reports formulas the new
  environment breaks.
- Text: each change is a span edit of exactly its bytes, so spacing and
  parentheses are kept. A dotted field whose new name isn't a plain field
  name becomes `["x y"]`, replacing from its `.`. A quoted field stays quoted.
  An `Int` key or index written in hex stays hex, with its `0x`/`0X` and
  digit case.
- Structures: node edits on a clone, swapped in only on success. A `Name` or
  `Call` is renamed, a `Field` is renamed or becomes an `Index` by a new
  `String` node, a `String` node is replaced, and an `Int`/`Hex` literal is
  replaced with one of the same kind.
- Cost: one references walk, then the edits sorted by start, so linear in
  the expression plus the edits (and the sort).
- `RewriteError` is opaque. Its causes are a shape mismatch, an invalid new
  name, the source failing to compile, and the result failing the check.
  `{:#}` and `source()` give the compile errors.

Decisions (lead, 2026-11-27, recommended options):

| Question | Picked | Alternatives |
|---|---|---|
| Path type | an owned, public `Path` builder with literal steps (also for Globals 3) | `&[Step]`, which hosts cannot build because steps carry spans; a data-carrying public enum |
| Key steps | `int_key`/`string_key` builders, so a path never holds a non-key value | `key(Value)` refused later for other kinds |
| Which rewrites | same root kind and same step kinds, so every changed part is a token edit | any prefix to any prefix, which can't be expressed when the path runs through `unwrap_or(..)` or `get(..)` |
| Two environments | find against `env`; check against an optional `Rewrite::check` env | check against `env` itself, which can't hold both the old and the new names of a renamed field |
| API shape | a `Rewrite` value passed to `evalite::rewrite` / `Structure::rewrite`, like `references` | free functions with six arguments, `from`, `to` and an `Option<&Environment>` among them |
| Count | references that read `from`; 0 when `from == to` | edits made |
| `i64::MIN` keys | refused by the check, since the literal can't be written; unchecked, it is written and doesn't compile | a special form |

## 10. Planned: the globals store

Decided by the lead on 2026-11-27 with its recommended options. The user
approved the direction and reviews the decisions here. Nothing in this
section is built yet. Breaking changes are allowed (board rule, 2026-11-27).

### 10.1 Why

A host such as a character sheet has one set of values (the sheet) and many
formulas over it. Today each run of each formula re-binds every global it
reads, copying the whole value, including every list, map and record it
holds, into the run's memory (section 7). A sheet with a 100 000-entry map
pays for copying that map on every formula that reads one entry. The store
makes the sheet's values persistent:

- they are written once and updated in place by path;
- runs read them where they are.

### 10.2 The model

- **One store per sheet instance.** A `Globals` store is built from an
  `Environment` (`Globals::new(&env)`). It holds its own word memory: a slot
  area, plus a heap for the parts of strings, lists, records and maps. It
  records the environment's layout fingerprint.
- **Fixed slots.** The `Environment` gives each global a slot when it is
  declared, in declaration order: a word offset with width `width(ty)` (one
  word, or more for a tagged option). Slots never move. A record is one
  reference word, so its fields aren't flattened into slots.
- **The compiler maps names to slots.** `Compiler::global` looks up the
  environment's slot instead of allocating per program, so
  `Instr::Global { at: slot, ty }` means the same thing in every program
  compiled from one environment. Programs keep the names, slots and types they
  read, for messages, references and verification.
- **Layout fingerprint.** A stable 64-bit hash of the ordered (name, type)
  list. It uses a fixed algorithm, never the per-table random seed of
  section 4, so it is identical across processes. Programs carry it, and
  loading a program against a store of another layout is refused.
- **Runs read the store, read-only.** `machine.load(&program)` plus
  `run.run_with(&globals, host)` (or the simplest equivalent). A run reads
  globals in place and allocates temporaries (built lists, host results, the
  stack) only in the machine's own scratch memory. There is no bind per run,
  and `Run::bind` is removed. Reading an unset global is a run error naming
  it.
- **In-place updates by path.**
  - Setting: `set(name, &Value)`, `set_at(path, &Value)`, `get`, `get_at`.
  - Maps: `insert`/`remove`.
  - Lists: `push`/`insert`/`remove`.
  - Every operation is type-checked against the declared type, with
    path-named errors as binding has today.
- **Amortized spare capacity.**
  - Store lists and maps have a capacity, kept in a header word just before
    the entries. The reference word keeps (start, len), so runs see the
    run-time layout of section 5 unchanged.
  - Growth past capacity doubles (minimum 4).
  - Shrinking below a quarter of capacity halves, so there's no thrash.
- **Immediate freeing.** Values never share parts (every value is a tree), so
  replacing or removing a value frees its old parts at once, with no
  reference counts or tracing.
- **Store limits.** The store has a word limit (part of `Limits` or its own),
  which counts the heap including free blocks. `set` and growth fail cleanly
  past it.
- **Reuse and compaction.** Freed blocks go onto power-of-two size-class free
  lists. When free words exceed half the heap, the store compacts:
  - it walks the live values from the slots with an explicit work list
    (they're trees, so each block is reached once);
  - it copies them densely and rewrites the references;
  - this is O(live size), and memory stays proportional to the live data.
- **Persistence and migration.**
  - `to_bytes` writes each set slot's value (not raw words) in the musli
    storage format, versioned.
  - `from_bytes(&env, bytes)` re-checks every value against the environment
    with binding's bounds, then rebuilds densely.
  - `migrate(&old, &new_env)` carries over each value whose name still exists
    with an equal type. Record fields carry over by name when only fields
    were added or removed. A new field without a value leaves the global
    unset, and each dropped value is reported with a reason.

Store memory:

```text
| slot area: slot 0 | slot 1 (2 words) | ... | heap: parts, free blocks ...           |
              │                                  ┌─────┬──────┬──────┬───────┐
              └── ref(start, len) ──────────────>│ cap │ e0   │ e1   │ spare │
                                                 └─────┴──────┴──────┴───────┘
                                               header word ^start
```

### 10.3 Phases

| Card | Phase | Changes |
|---|---|---|
| da3c369c | Globals 1 | Environment-owned slots, layout fingerprint, stored `FORMAT` bump recording the fingerprint and read slots. Transitional: `Run::bind` remains and writes into the slot layout. The verifier checks every global read against the recorded (slot, type). |
| 5885897e | Globals 2 | `Globals` store with `set`/`get`, set flags, a word limit, runs reading the store read-only, `Run::bind` removed. Updates cli, compare, sockel, playground, benches (a one-store, many-formulas bench modelled on characterize) and docs. |
| 8823970e | Globals 3 | `set_at`/`get_at` by path, reusing the references path type. Literal steps only, existing keys and indexes only. O(path + new value), plus O(log n) per map step. |
| e5e312bd | Globals 4 | Capacity, growth and shrink. Map `insert`/`remove` (binary search plus a `copy_within` shift). List `push`/`insert`/`remove`. Bench: 1 000 single-entry updates on a 100 000-entry map versus a whole-map set. |
| ae875cb2 | Globals 5 | Size-class free lists, compaction at more than half free, `stats()` (live, free, capacity). |
| fbaa6f18 | Globals 6 | `to_bytes`/`from_bytes` of values, `migrate` between layouts. |

They wait on each other in order. Globals 1 also waits on this document,
map literals (f3b53baf) and references (b128c319, landed), which all touch
`check.rs`/`program.rs`.

### 10.4 Decisions (lead, 2026-11-27)

| # | Question | Picked | Alternatives and why not |
|---|---|---|---|
| 1 | Slot granularity | One slot per global, width `width(ty)` | Flattened per field. It would make record fields directly addressable, but it ties the layout to every record's shape, so adding a field moves slots. Records are already one reference word. |
| 2 | Who owns name → slot | The `Environment`, the single source | Per program, as today. Offsets then differ per program, so no shared store is possible. |
| 3 | How a program knows its store | A layout fingerprint stored in the `Program` and checked at load | Matching names at every load (slower, and every program repeats it), or no check (unsafe: a slot of another type would be read as this one). |
| 4 | Fingerprint algorithm | Fixed and stable across processes | The table's `RandomState` seed differs per process. |
| 5 | Runtime between phases | Globals 1 keeps `Run::bind`, writing into the new slots | A single big-bang change. |
| 6 | Values per run | Read in place, read-only. Temporaries in scratch | Copy on load (the cost the store exists to remove). |
| 7 | Unset globals | A set flag per slot. Reading an unset global is a run error naming it | Default values (hide mistakes), or refusing to run unless all are set (a program may read only set ones). |
| 8 | Store limit | A word limit, `set` failing cleanly | Unbounded (unsafe for hosts storing user data). |
| 9 | Update paths | `Path` from References 2 (9.4), literal steps only | A separate path type. A dynamic step has no single target. |
| 10 | Missing key or index in `set_at` | Error. Inserting is Globals 4 | Upsert (blurs set and insert). |
| 11 | Container capacity | A header word before the entries, with the reference keeping (start, len) | Capacity in the reference word (changes the run-time layout and halves the address bits), or exact-size reallocation per update (O(n) per insert). |
| 12 | Growth and shrink | Double past capacity (min 4). Halve below a quarter | A fixed increment (quadratic total cost). Halving at a half (thrash). |
| 13 | Freeing | Immediate, as values share no parts | Reference counting or tracing GC (unneeded without sharing). |
| 14 | Reuse | Size-class free lists plus compaction at more than half free | Bump-only with periodic full rebuild (spiky). Free lists alone (fragmentation grows without bound). |
| 15 | Persistence | Values, re-verified on load, rebuilt densely | Raw words (fast, but it ties bytes to the in-memory layout and needs a word-level verifier). |
| 16 | Migration | Carry over by name with equal type. Record fields by name. A new field without a value leaves the global unset, reported | Refuse the whole store on any change (hostile to evolving sheets). Implicit defaults. |

### 10.5 Questions raised while writing this

The lead decided each of these on 2026-11-27, taking the recommended option
(board rule). The phase cards carry the decisions.

- **Decided:** A, one address space split at a boundary (Globals 2). B,
  refuse redeclaring, with an ordered slot list in `Environment` (Globals 1).
  C, accept on a fingerprint match, or else when every read (name, slot,
  type) matches (Globals 1). D, append-only environments, with removal as a
  new layout plus `migrate` (Globals 1/6).

- **A. Two memories in one run (Globals 2).** Today `Instr::Global` and every
  reference address one `words` slice. With a separate read-only store and
  scratch memory, a reference word must say which memory it points into.
  - *Recommended:* one address space split at a boundary. Store words take
    addresses `[0, store_len)` and scratch words follow. Reads dispatch on
    the address, and writes only ever go to scratch. This needs no change to
    the word layout, and the verifier's guarantees carry over.
  - *Alternatives:* a tag bit in references (costs address bits), or copying
    the read globals' parts into scratch (the cost the store removes).
  - The 32-bit address field caps store plus scratch at 2^32 words.
- **B. Declaration order and redeclaring (Globals 1).** Globals 1 says
  "redeclaring a name is an error, as today", but today `Environment::value`
  *replaces* an existing name's type. Also, `Environment.values` is a
  `BTreeMap` (name order), so it has no declaration order to assign slots by.
  Globals 1 must add an ordered slot list and decide redeclaration.
  - *Recommended:* refuse redeclaring with a new `DeclareError` kind, since
    slots never move.
  - *Alternative:* allow replacing only with an equal type.
- **C. Fingerprint versus read slots (Globals 1).** A whole-layout
  fingerprint makes appending any global invalidate every stored program,
  even ones that don't read it. Programs also record their read
  (name, slot, type).
  - *Recommended:* accept a program when its fingerprint matches (the fast
    path), or else when each read (name, slot, type) matches the store. Slots
    never move, so appending stays compatible.
  - *Alternative:* the strict fingerprint as decided, simpler but forcing
    recompiles.
- **D. Removed globals (Globals 1/6).** "Slots never move" means removing a
  global from an environment leaves a hole, or builds a new environment
  (a new layout, migrated with `migrate`).
  - *Recommended:* environments are append-only. Removal is a new layout plus
    `migrate`.
