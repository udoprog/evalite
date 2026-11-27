# evalite design

This document describes how evalite works, and the decisions behind it, for
review. Sections 1 to 9 describe `main` as of Globals 3 (8823970e, updates
by path). Section 10 is the globals store's plan: decided, and built up to
Globals 3.

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
7. [Slots and the store](#7-slots-and-the-store)
8. [Stored formats and verification](#8-stored-formats-and-verification)
9. [Structures, completions and references](#9-structures-completions-and-references)
10. [The globals store](#10-the-globals-store)

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

**Values:**

- Declared with `Environment::value(name, ty)`, each once: redeclaring a
  name is `DeclareKind::Redeclared`, since its slot never moves (section 7).

**Names:**

- Value and function names are lowercase (`is_value_name`): a lowercase
  letter or `_`, then lowercase letters, digits and `_`.
- The lexer accepts any ASCII name, so `Strength` lexes, but no environment
  can declare it.
- Field names may be any string. Fields that aren't ASCII identifiers are
  written `["..."]`.

**Types and values as text** (`literal.rs`):

- `Type::parse` and `Value::parse` read what `Display` prints, iteratively,
  refusing input over `DEFAULT_MAX_LEN` and types over `MAX_TYPE_DEPTH`. A
  type literal cannot reach `MAX_TYPE_NODES`: each node takes two bytes.
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
  fault. Map literals took the stored program `FORMAT` to 5 (now 6, section 8).
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
                                     stored bytes ──decode───┘              │ ◀── Globals
                                                                           Run  (set once)
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
  - a name is looked up in the environment (`Environment::slot`), which
    gives its slot and declared type;
  - its type is interned the first time the program uses it (cached by slot
    in `global_types`), and the program records it once as a `Global`
    (name, type, slot) in first-use order;
  - `Instr::Global { at }` takes the environment's slot, so a name has the
    same offset in every program compiled from one environment (section 7).
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
  - a type nesting past `MAX_TYPE_DEPTH` is a compile error (`TypeTooDeep`),
    and past `MAX_TYPE_NODES` written out `TypeTooLarge`;
  - counts past `u32` are `TooLarge`.

### 3.4 Program (`program.rs`)

A `Program` holds:

- `types`: its own `Types` table;
- `constants`;
- `globals`: a (name, type, slot) list in first-use order;
- `functions`: (name, params, ret);
- `code`: a `Vec<Instr>`;
- `result`: the result type;
- `memory`: `Memory { stack, heap, constants, globals }` in words;
- `layout`: the environment's fingerprint (section 7).

It also caches `global_types` (the globals' types, built once) and
`constant_at`.

It carries no source text. `DebugInfo` (a span per instruction) is optional,
from `compile_debug`, and stored separately.

### 3.5 Verifier (`Program::verify`)

The verifier runs the code once over types instead of values (`Model`: a
stack of `TypeId`s and a running word count). The model is a persistent
stack whose equal stacks share one id, hash-consed by (stack below, type on
top), so a jump records the stack it lands with, and the target compares
it, in constant time however deep the stack is. It checks:

- every constant is well typed, and a constant list's `Scalar::Const` entries
  refer only to earlier constants;
- globals: valid, unique names and known types, and slots that fit in
  `u32` words and share no word (sorted by slot, each must start at or
  after the previous one's end, `GlobalsOverlap`). `Instr::Global` must
  name a recorded (slot, type);
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

It also bounds the globals' and host functions' types written out: the sum
over every global, parameter and return type, and everything they reach,
are each at most `MAX_GLOBAL_TYPE_NODES` (2^21), since `Program::globals()`
and `Program::functions()` hand out built `Type`s. The compiler counts the
same totals as it adds each global and host function (`Held`, each type id
marked once), and reports the first to cross as `ErrorKind::TypesTooLarge`,
so a compiled program never fails this check in `Program::verify`. `functions()` builds
them in one `to_types` pass, so it walks the table once, not once per type.

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
| `MAX_TYPE_NODES` (public) | 65 536 | `Environment`, `Types::add`, stored tables | one type's nodes with shared parts written out; a declared type over it is `DeclareKind::TypeTooLarge`, a value's is `ErrorKind::TypeTooLarge` |
| `MAX_GLOBAL_TYPE_NODES` | 2^21 | compiler, verifier | a program's globals' and host functions' types written out, together; the compiler reports `ErrorKind::TypesTooLarge` at the global or call that crosses it, the verifier refuses stored bytes |
| `Limits::memory` | 1024 words default | `Machine` | stack + heap + constants + returned parts |
| `Globals::with_limit` | 2^20 words default (`DEFAULT_GLOBALS`) | `Globals` | the parts of the store's values, freed blocks included; the slots are not counted |
| store + machine | 2^32 words | `Machine::load` | one address space, so every address fits a reference (`PastAddressSpace`) |
| `Limits::fuel` | 1000 default | `Run` | steps per run |
| `MAX_STEPS` | 4096 | completions | type walks per value when searching paths |
| `MAX_KEY_SHOWN` | 64 chars | messages | a string key quoted in an error |
| `MAX_QUOTED` | 32 chars | literals | input quoted in an error |
| `Word::reference` | 32 + 32 bits | machine | addresses and lengths fit `u32`; `measure` refuses a set or returned string, list or map longer than `u32::MAX` (`TooLong`) before anything is written or freed |

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
    stored and never used for identity outside one table. The layout
    fingerprint (section 7) is a separate, fixed FNV-1a hash.
- **Per-node facts**, computed once when a node is added:
  - `depths`: nesting, refused past `MAX_TYPE_DEPTH`;
  - `sizes`: nodes with shared parts written out, saturating, refused past
    `MAX_TYPE_NODES`;
  - `widths`: the words a value of the type takes where it is held
    (section 5);
  - `offsets`: each record field's word offset, in name order, so
    `field_at` (the verifier's `Field` check) and `field_named` (the
    compiler's field access) are binary searches, not scans of the fields.
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

Setting a value refuses unsorted entries (`Fault::MapUnsorted`), and so
does a host function's result. `Value::map` always sorts, so a host can't
produce them through the public API.

**Shared value code** (`memory.rs`). Checking a `Value` against a type
(`measure`, with a path-naming rerun on a mismatch), writing it (`write`),
reading it back (`read`), the sizes of the blocks a value's parts take
(`parts`) and the blocks a held value's parts are in (`blocks`) are free
functions over a `Words`/`WordsMut` trait: a word getter, the end of
memory, a setter and an allocator. A run's memory and a `Globals` store
both implement it, so values have one layout and one checker wherever they
are held. Every walk uses an explicit work list.

## 6. The machine

### 6.1 Memory regions

A `Machine` reserves `Limits::memory` words up front (`words: Box<[Word]>`)
and runs one program at a time over a `Globals` store (section 7).
`Machine::load(&program, &globals)`:

1. refuses a program the store does not accept (`OtherLayout`, naming the
   first global it does not hold, section 7);
2. refuses one whose `memory().total()` exceeds the machine
   (`TooMuchMemory`);
3. refuses a store and machine whose words together are past 2^32
   (`PastAddressSpace`), so every address fits a reference;
4. copies the constants in, and gives a `Run`.

A run addresses **one space** (decision A, section 10.5): the store's words
from 0, then the machine's. Reads dispatch on the address (`Words::word`:
below `base` the store, else the machine); writes go only to the machine
(`WordsMut::set_word` refuses an address below `base`, and `copy`, the one
bulk move, refuses a destination there with `Fault::WriteToStore`). A
range that straddles the boundary is refused. The word layout is
unchanged, so references into the store and into scratch look alike:

```text
0         store_len = base       len ──>              limit       stack_base      end
| slots | store parts | built lists, host results | constants | stack ──> |
  the store, read-only  ^ grows up (alloc), up to     copied at   stack_cap words,
                          limit minus the heap still  load        sp deep
                          reserved
```

- `Instr::Global { at, ty }` reads the store's slot words at `at`, after
  checking the slot's set flag (section 7). The program's values are never
  copied: loading costs the constants, however large the values are.
- The **heap reservation** (`heap`) is the verifier's count of list words the
  code may build. Returned values may use only `limit - len - heap`
  (`left()`), so the program's own lists always fit.
- `Outcome::memory` is the peak of `len - base`, plus the peak of `sp`, plus
  the constants: the machine's words only.
- A read that may be in either memory pays one comparison for the
  dispatch. Pushes and pops skip it, as the stack is always the machine's,
  and so does a one-word `Instr::Global`, always the store's. Benchmarks
  against bind-per-run: `sheet/evaluate` 36.1 → 11.7 µs, a characterize
  sheet of 400 formulas over a 10 000-entry map 99 ms → 31 µs, and the
  370-read `stage/long` unchanged (`benches/README.md`).

### 6.2 Instructions

The `Instr` enum in `program.rs`:

| Instr | Effect |
|---|---|
| `Int(i64)`, `Bool(bool)` | push a literal |
| `Const(c)` | push a reference to constant `c` |
| `Global { at, ty }` | push the `width(ty)` words of the store's slot at word `at`; an unset slot is a run error |
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
- plus `len` for `List`, and the entries for `Map`;
- plus `n` for `Min(n)`/`Max(n)`;
- plus, for `MapIndex`/`MapGet`, one per key the binary search probes and
  one per word of a `String` key compared, so a lookup costs about
  log(entries) × key words, not a step however long its keys are;
- plus the words of the parts of each value a host function returns.

Running out is a run error. There are no backward jumps, so even without
fuel a run is bounded by its code length. Fuel bounds the work per run that
the host pays for.

Setting a store's values costs no fuel. It is bounded by the value's
size, which the host supplies, and by the store's limit.

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
  - The `.context(...)` pattern adds frames, such as "while running
    instruction 2" and "while running the program".
  - Messages hold plain ids (globals, functions, `TypeId`s). Names are
    resolved only when the error is shown, through the `Names` trait:
    a `Program` for run errors.
  - `labels(&program, Some(&debug))` maps each instruction location through
    `DebugInfo` to a source span.
- **Store errors** are `StoreError`, with context frames such as "at
  instruction 0" and "while verifying the program".
- **Store errors from setting** are `GlobalsError`: the same `Message`
  frames ("expected Int, found Bool", "at `health.count[1]`", "while
  setting `health`"), shown against the store's own layout (its `Names`:
  slot index → name, its type table) when the error is made, since the
  caller has no program to resolve them with. Its `labels()` have no spans.
- **Value mismatches** (setting, host results) name the path into the
  value (`Root` plus `Seg`s: field, index, map key or key-of-entry). The
  first walk keeps no path. Only on a mismatch is it re-run with a `Nodes`
  trail to name where.

## 7. Slots and the store

Globals 1 (da3c369c) gave the `Environment` the layout. Globals 2
(5885897e) holds the values in a `Globals` store that runs read in place
(section 10). Globals 3 (8823970e) updates a part of a value in place by
path.

**Slots.** `Environment` keeps `slots: Vec<Slot { name, ty, at }>` in
declaration order, plus a name-ordered index (`names`) for lookup and for
`values()`:

- `Environment::value` gives each value the next slot: word `at` is the sum
  of the widths of every value declared before it. The width is `width(ty)`,
  computed from the `Type` the same way `Types::width` computes it from the
  id: one word, or a word more than its value for a tagged option.
- Slots never move. Redeclaring a name is `DeclareKind::Redeclared`, and the
  first declaration stands. Environments are append-only (decision D,
  section 10.5). Slot words past `u32::MAX` are `DeclareKind::TooManySlots`.
- A record is one reference word, so its fields are not slots.

**Fingerprint.** `Environment::fingerprint()` is a 64-bit FNV-1a hash of the
layout, kept up to date as each value is declared (O(type size), bounded by
`MAX_TYPE_NODES`). It covers, for each value in declaration order, the name's
length (u64 LE) and bytes, then its type in pre-order, as one tag byte per node:

- `Int` 0, `Bool` 1, `String` 2;
- `List` 3 and `Option` 4, then the inner type;
- `Record` 5, then the field count, then each field's name and type in
  name order;
- `Map` 6, then key and value.

It never uses the table's random seed, so it is the same in every process
and on every platform. A test pins its values. Functions are not part of it.

**Programs.** The compiler emits `Instr::Global { at: slot, ty }` and
records each read global as (name, type, slot), with the fingerprint as
`Program::fingerprint()`. `Environment::accepts(&program)` and
`Globals::accepts(&program)` (decision C) share `env::refused`, which
accepts a program when each global it reads is declared with an equal type
at the same slot. Appending values keeps stored programs accepted. Checking
every read costs the program's global types, which are bounded by
`MAX_GLOBAL_TYPE_NODES`. `Machine::load` refuses a program its store does
not accept.

There is no fingerprint fast path (removed in the store hardening card,
e3868667). A matching fingerprint is no proof of the same layout: FNV-1a 64
collisions cost about 2^32 work, and in characterize environments the
layout comes from user-defined sheet templates, so it is untrusted. A
collision would let a program read a slot's words as another type. The
machine bounds-checks every access, so that would be a fault, not
unsafety, but faults are reserved for evalite's own bugs. Stored bytes
could claim any fingerprint anyway. So every program, compiled or decoded,
is checked read by read, which is linear in its reads. The fingerprint is
kept as stored metadata and a quick hint (`Program::fingerprint()` against
`Environment::fingerprint()`), never as grounds to accept.

**The store** (`globals.rs`). `Globals::new(&env)` (or `with_limit(&env,
words)`) copies the environment's slots (name, `Type`, slot word), interns
each type into a type table of its own, and records the fingerprint. It
holds the values declared when it is made; one declared later is not in
it, and a program reading it is refused at load. Its memory:

- `words`: the slot area (`env.slot_words()` words), then the heap of
  parts, grown by bump allocation;
- `set`: a flag per word of the slot area, true at the first word of each
  set slot. `Instr::Global { at, .. }` checks `set[at]`; unset, it is
  `Message::Unset`, naming the program's global with that slot, as an
  error of the instruction (decision 7: only a read fails, so `ready &&
  rank > 2` runs with `rank` unset when `ready` is false);
- `free`: freed blocks by exact size (`BTreeMap<usize, Vec<usize>>`).

`set(name, &value)`:

1. finds the slot by name (`NotDeclared` names the name, which the layout
   cannot);
2. `measure`s the value against the slot's type (path-named mismatch);
3. lists the block sizes the value's parts need (`parts`), and the blocks
   the old value's parts hold (`blocks`, when set);
4. works out the words that must be taken past the heap's end: each needed
   block is matched against free blocks and the old value's of its exact
   size, and the rest bump. If that is past the limit it is `TooLarge`,
   before anything changed, so a failed set leaves the old value;
5. frees the old blocks onto the free lists, then `write`s the new value at
   the slot, allocating from the free list of a block's size, else by bump;
6. sets the flag.

So replacing a value with one of the same shape reuses its blocks and the
store does not grow (`replacing_frees`). The cost is O(old value + new
value), plus a log per block for the free-list map. Freed blocks are only
reused at their exact size; size classes and compaction are Globals 5.

`get(name)` reads a set value back (`None` when unset or not declared).

**Updates by path** (Globals 3). `set_at(&path, &value)` replaces one part
of a set value: a record's field, a list's entry, or the value of a map's
key. The path is the `Path` of section 9.3, whose steps are literal fields,
keys and indexes, never `Dynamic`. A path without steps is `set` (so it may
set an unset value); `get_at(&path)` reads a part (`None` for anything
`set_at` would refuse; no steps is `get`).

1. `locate` walks the steps from the slot, each against the type it
   reached, keeping the `Seg`s of the path so far:
   - through each present option first (tagged: check the tag, step past
     it; niche: check the word is not `Word::NONE`), as `bonus!.dice`
     reads `bonus.dice` (section 9.3);
   - a field: `Types::field_index` (binary search by name), then the
     record's reference word, its size checked against the type, then the
     field's offset;
   - a key: the key's kind against the map's key type, then
     `memory::find_key`, a binary search of the sorted entries comparing
     `Int`s, or a string's words against the key's bytes. The part is the
     word after the key;
   - an index: `0 <= index < len`, then `start + index × width`.

   It never writes, so any error leaves the store as it was. A step error
   carries `AtPath`, the part it was taken from.
2. `replace` is `set`'s steps 2 to 5 at that word: `measure_at` checks the
   value against the part's type (a mismatch inside it is named by the
   whole path, `at sheet.tracks[1].marks[1]`), `parts` and `blocks` of the
   old part give the sizes, the limit is checked before anything changes,
   the old part's blocks are freed, and the new part is `write`n in place.
   The words around it (the record's other fields, the list's other
   entries, the map's keys) are untouched, so a map stays sorted.

So a fixed-size part (`Int`, `Bool`, a tag and its words) is written over
in place and a part with blocks takes blocks of its sizes, reusing the old
part's, so a same-shaped replace does not grow the store. The cost is
O(path) steps, a binary search per key step (a word per 8 bytes of a
`String` key per probe), plus O(old part + new part). The old part is
walked to free it; a part has no other record of its blocks.

Errors carry `SettingAt { path }`, the path as the caller wrote it
(`Path`'s `Display`), outermost. The new messages: `FunctionPath`
(a `Path::function`, which a store never holds), `NotSet` (steps into an
unset value), `NoStep { ty, step }` (a field the record lacks, or a step of
the wrong kind: `Int has no field "max"`, `List<..> has no key 0`),
`NoneOnPath`, `KeyMistyped` (`expected an Int key, found String`), and the
run's `NoSuchKey` and `IndexOutOfRange` for a missing key or index.
Inserting is Globals 4.

The limit counts the heap, freed blocks included, and not the slots. It
is kept inside 2^32 minus the slot area, so every store address fits a
reference.
## 8. Stored formats and verification

All stored forms are encoded with musli `storage` and start with a
`format: u32`.

**Programs:** `Program::to_bytes`/`from_bytes`, `FORMAT` = 7 (4 added `Map`,
5 map literals, 6 each global's slot and the layout fingerprint, 7 dropped
`Memory::globals`, as a program's values are in the store, not the
machine). The stored form is
`Stored { format, types, constants, globals, functions, code, result, memory, layout }`,
where each global is (name, type, slot). `layout` is last, so a length
prefix's offset doesn't depend on its varint width.
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
- names are valid and unique, and no two globals' slots share a word, so
  setting one never changes what another reads;
- all types are in bounds.

So the machine may run it on bare words, and any remaining fault is a bug in
evalite, not in the bytes.

Stored bytes have no size cap, so loading them is kept close to linear in
their size, whatever the bytes repeat or share:

- a jump records and compares a stack id, not a copy of the stack (section
  3.5);
- `Field` is a binary search of the record's offsets (section 4);
- a map-key constant is checked sorted once, however many `Map`
  instructions share it, since string keys compare byte by byte;
- the globals' and host functions' types written out are bounded together
  (section 3.5).

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
- environments (only the layout's fingerprint and the read slots), nor
  whether a program was compiled here, so a decoded program is checked read
  by read at load (section 7);
- a store's values (Globals 6 stores them);
- type table hashes.

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

  The parts, and the replaced node's old parts, are checked against sets,
  so an edit is linear in its parts plus the replaced node's depth.

  Nodes orphaned by edits stay in the table until stored.
- **Text.**
  - `to_text`/`text_of` render iteratively (`render`), adding parentheses
    where binding power needs them (`Repr::power`; negative literals bind as
    prefix).
  - `from_text` parses and builds nodes, dropping parentheses and reading
    `_` as a hole.
- **Compiling.**
  1. `check_nodes` reports every hole and every name that can't be written
     (invalid or reserved: `true`, `false`, `_`), in text order, sorted by
     each node's start from a map built once over the rendered spans.
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
  must start at the same kind of name and have the same kinds of steps (an
  Int key and a String key are different kinds). Each part that differs is replaced in place, which covers renaming a value, a
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

## 10. The globals store

Decided by the lead on 2026-11-27 with its recommended options. The user
approved the direction and reviews the decisions here. Globals 1 is built
(section 7): environment-owned slots, the fingerprint, and the read
compatibility check. Globals 2 is built (sections 6.1 and 7): the store,
`set`/`get`, set flags, the store limit, runs reading the store in place,
and `Run::bind` removed. Globals 3 is built (section 7): `set_at`/`get_at`
by path. The rest is not built yet. Breaking changes are
allowed (board rule, 2026-11-27).

### 10.1 Why

A host such as a character sheet has one set of values (the sheet) and many
formulas over it. Before Globals 2 each run of each formula re-bound every
global it read, copying the whole value, including every list, map and
record it held, into the run's memory. A sheet with a 100 000-entry map paid
for copying that map on every formula that read one entry. The store makes
the sheet's values persistent:

- they are written once and updated in place by path;
- runs read them where they are.

### 10.2 The model

- **One store per sheet instance.** A `Globals` store is built from an
  `Environment` (`Globals::new(&env)`). It holds its own word memory: a slot
  area, plus a heap for the parts of strings, lists, records and maps. It
  records the environment's layout fingerprint.
- **Fixed slots** (built). The `Environment` gives each global a slot when it is
  declared, in declaration order: a word offset with width `width(ty)` (one
  word, or more for a tagged option). Slots never move. A record is one
  reference word, so its fields aren't flattened into slots.
- **The compiler maps names to slots** (built). `Compiler::global` looks up the
  environment's slot instead of allocating per program, so
  `Instr::Global { at: slot, ty }` means the same thing in every program
  compiled from one environment. Programs keep the names, slots and types they
  read, for messages, references and verification.
- **Layout fingerprint** (built, section 7). A stable 64-bit hash of the
  ordered (name, type) list. It uses a fixed algorithm (FNV-1a), never the
  per-table random seed of section 4, so it is identical across processes.
  Programs carry it as metadata. Loading a program the store's environment
  doesn't `accepts` (every read slot matching; a matching fingerprint alone
  is not enough) is refused.
- **Runs read the store, read-only** (built). `machine.load(&program,
  &globals)` then `run.run()`/`run.run_with(host)`: the store is given at
  load, so load can refuse a program the store does not accept. A run reads
  globals in place and allocates temporaries (built lists, host results, the
  stack) only in the machine's own scratch memory. There is no bind per run,
  and `Run::bind` is removed. Reading an unset global is a run error naming
  it.
- **In-place updates by path** (`set`, `set_at`, `get`, `get_at` built).
  - Setting: `set(name, &Value)`, `set_at(path, &Value)`, `get`, `get_at`.
  - Maps: `insert`/`remove`.
  - Lists: `push`/`insert`/`remove`.
  - Every operation is type-checked against the declared type, with
    path-named errors as `set` gives (built for `set`).
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
    with `set`'s bounds, then rebuilds densely.
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
| da3c369c | Globals 1 (built) | Environment-owned slots, layout fingerprint, stored `FORMAT` bump recording the fingerprint and read slots. Transitional: `Run::bind` remains and writes into the slot layout. The verifier checks every global read against the recorded (slot, type). |
| 5885897e | Globals 2 (built) | `Globals` store with `set`/`get`, set flags, a word limit, runs reading the store read-only, `Run::bind` removed. Updates cli, compare, sockel, playground, benches (a one-store, many-formulas bench modelled on characterize) and docs. |
| 8823970e | Globals 3 (built) | `set_at`/`get_at` by path, reusing the references path type. Literal steps only, existing keys and indexes only. O(path + new value), plus O(log n) per map step. |
| e5e312bd | Globals 4 | Capacity, growth and shrink. Map `insert`/`remove` (binary search plus a `copy_within` shift). List `push`/`insert`/`remove`. Bench: 1 000 single-entry updates on a 100 000-entry map versus a whole-map set. |
| ae875cb2 | Globals 5 | Size-class free lists, compaction at more than half free, `stats()` (live, free, capacity). |
| fbaa6f18 | Globals 6 | `to_bytes`/`from_bytes` of values, `migrate` between layouts. |

They wait on each other in order. Globals 1 waited on this document, map
literals (f3b53baf) and references (b128c319), which all touch
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

**Taken while building Globals 2** (worker, 2026-11-27, within the card's
decisions):

| Question | Picked | Alternatives and why not |
|---|---|---|
| Where the store meets the run | `Machine::load(&program, &globals)`, the run borrowing the store | `run.run_with(&globals, host)` as the card sketched: load could not refuse a program of another layout, and every run entry point would take the store. |
| When an unset global fails | When `Instr::Global` reads it, as an error of the instruction | Checking every read global before the run: refuses a run that would not read it (`a && b`). |
| Set errors | `GlobalsError`, opaque, its frames shown against the store's names when made | `RunError`: its names resolve against a program, which a caller setting a value does not have, and another program's ids would name the wrong values. |
| "Simple free" | Exact-size free lists, reused by the next block of that size; the limit checked by matching the new value's block sizes against free and old blocks before changing anything | Bump only (a store replacing values grows until its limit); freeing only a block at the heap's end (order-dependent); checking the limit without reuse (a same-size replace fails at the limit). |
| What the store limit counts | The heap, freed blocks included; not the slots, which the environment bounds | Slots too: the limit would depend on how many values are declared. |
| `Memory::globals` | Removed, `FORMAT` 7 | Kept: it counted the slots as machine memory, which they no longer are. |
| The fingerprint fast path | Only for programs compiled here; a decoded program is checked read by read. **Superseded** (e3868667): no fast path at all, every program is checked read by read | For every program: stored bytes may keep the fingerprint and tamper the types, which reads a slot as another type (a fault). A compiled program's fingerprint can collide too, from an untrusted layout. |

**Taken while building Globals 3** (worker, 2026-11-27, within the card's
decisions):

| Question | Picked | Alternatives and why not |
|---|---|---|
| Options along a path | Stepped through when present, as references continue through `!` (9.3); an absent one is `NoneOnPath` | Refusing any path through an option: `Path` has no unwrap step, so a field of an `Option<record>` could not be set but by replacing the whole option. |
| A path without steps | `set`/`get` of the value, unset or not | An error: a host building paths would special-case the root. |
| Steps into an unset value | `NotSet`, an error | Treating it as a missing part: the cause is the value, not the step. |
| Cost | O(path + old part + new part), plus a binary search per key | O(path + new part) as the card sketched: freeing the old part's blocks needs its blocks, found by walking it. Replacing an `Int` in a 100 000-entry map is still O(log n). |
| How the error names the path | `while setting <path>` from `Path`'s `Display`, plus `at <prefix>` for a step error and `at <full path>` for a mismatch inside the value | Only `Seg`s: a failing step (a field the record lacks) has no `Seg`. |
| Key lookup | `memory::find_key` over `Words`, comparing a string's words with the key's bytes | Writing the key into the store to reuse the machine's `find_key`: it charges fuel and needs scratch memory. |

### 10.5 Questions raised while writing this

The lead decided each of these on 2026-11-27, taking the recommended option
(board rule). The phase cards carry the decisions.

- **Decided:** A, one address space split at a boundary (Globals 2). B,
  refuse redeclaring, with an ordered slot list in `Environment` (Globals 1).
  C, accept on a fingerprint match, or else when every read (name, slot,
  type) matches (Globals 1); the fingerprint match was later dropped as
  grounds to accept (e3868667, section 7). D, append-only environments, with removal as a
  new layout plus `migrate` (Globals 1/6). B and C are built (section 7):
  `DeclareKind::Redeclared`, and `Environment::accepts`.

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
