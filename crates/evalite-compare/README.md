# evalite-compare

Benchmarks comparing evalite against its closest peers,
[evalexpr](https://docs.rs/evalexpr) and [fasteval](https://docs.rs/fasteval),
with compiling an expression and evaluating it measured apart. Not
published; the playground renders [`results.json`](results.json), a
committed snapshot of one run.

```sh
# from the workspace root
CARGO_BUILD_JOBS=4 cargo bench -p evalite-compare --bench compare -- --warm-up-time 1 --measurement-time 3
cargo run -p evalite-compare
# a quick check that every engine compiles and runs every expression
cargo bench -p evalite-compare -- --test
```

`cargo run -p evalite-compare` reads the medians criterion left in
`target/criterion/compare_*` and writes `results.json`, with the time it
was written, the CPU and the OS. Commit it with the change that motivated
the run.

The snapshot is from a shared development machine that is often building
other things, so compare engines within one snapshot and treat differences
under ~15% as noise.

## The expressions

In [`src/lib.rs`](src/lib.rs), each over the same 16 integer variables,
and checked to give the same value on every engine before it is measured:

- `arithmetic`: `(strength + brawl) * 2 - armor * 3 + 1`.
- `variables`: the sum of all 16 variables.
- `functions`: nested `min` and `max` calls.
- `logic`: comparisons joined with `&&` and `||`.
- `formula`: 64 terms of sums, differences and products.

All three engines read the same source for each. They avoid what one of
them lacks: fasteval has only `f64`, and neither peer has evalite's
options, records or lists.

## What each stage is

Benchmarks are `compare/<expression>/<engine>/{compile,evaluate}`.

| Engine   | `compile`                                              | `evaluate` |
|----------|--------------------------------------------------------|------------|
| evalite  | `evalite::compile`: parse, type check, generate and verify a `Program` | `Machine::load` over a `Globals` store set once, `run` |
| evalexpr | `build_operator_tree`: tokenize and build a `Node`     | `Node::eval_with_context` against a filled `HashMapContext` |
| fasteval | `Parser::parse` then `compile` into a reused, cleared `Slab`, which folds constants | `Instruction::eval` against a `BTreeMap<String, f64>` namespace |

Like the peers, which read their values from a context or namespace the
caller filled once, evalite reads its values in place from a `Globals`
store the caller set once, so `evaluate` copies none of them. evalite also
checks types when compiling, which the others leave to evaluation
(evalexpr) or do not have (fasteval).

Engine versions in `results.json` are read from `Cargo.lock`, which is not
committed, so run the bin from the workspace after building the bench.
