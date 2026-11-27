# evalite benchmarks

[`evalite.rs`](evalite.rs) measures each stage a computed value goes through, on
expressions shaped like a character sheet's (attributes, skills,
`specialties`, optional `bonus_N`s, tracks, tables), with
[criterion](https://docs.rs/criterion).

```sh
CARGO_BUILD_JOBS=4 cargo bench -p evalite --bench evalite
# one group or case: the argument is a regex over the names
cargo bench -p evalite --bench evalite -- 'stage/long/'
cargo bench -p evalite --bench evalite -- '^sheet/'
# a quick check that every case still compiles and runs
cargo bench -p evalite --bench evalite -- --test
```

Results go to `target/criterion/`, with a report in
`target/criterion/report/index.html`.

## Comparing against a baseline

Save the numbers on the base, then compare a change against them:

```sh
git switch main
cargo bench -p evalite --bench evalite -- --save-baseline main
git switch my-change
cargo bench -p evalite --bench evalite -- --baseline main
```

The machine is often busy building other things, so run the two back to
back, and run again any result that moves by less than ~15%: a single
comparison is not enough. `--warm-up-time 1 --measurement-time 3` makes a
full run take about five minutes.

## What is measured

`stage/<case>/<stage>`, for each single expression:

- `compile`: source to a verified `Program` (parse, check, codegen and the
  verifier, which `compile` runs too).
- `from_bytes`: a stored program decoded and verified.
- `load`: `Machine::load` over the sheet's `Globals` store, which checks
  the program's layout against the store's and copies the constants in.
- `load_run`: that and `run_with`, which reads the values in the store
  and includes reading the result back into a `Value`; `run` is the
  difference from `load`.
- `end_to_end`: compile, load and run.

The cases, smallest first: `pool` (a dice pool sum), `pool_specialty`
(attribute + skill + specialty + bonus), `unwrap_or_16` and `is_none_16`
(sixteen optional globals), `get_literal` and `get_global` (tables read
with `get`), `host` (a host function call), `track` (record and track
field reads),
`clamp_chain`, `option_list` (lists of `Option<Int>`), `long` (a 370 term
sum, near the default fuel) and `deep` (31 nested parentheses, near the
default `max_depth`).

`read/<expr>` evaluates a value read whole against the same value reduced
to an `Int` by one more instruction, so the difference is roughly what
reading the result costs.

`sheet/*` is a whole character sheet, 108 computed values over one
environment: `compile` and `from_bytes` of all of them, `evaluate` of all
of them from compiled programs over one store set once, and `set`, setting
every value of the sheet in a new store, which a sheet does once rather
than once a formula.

`characterize/evaluate` is a sheet as characterize computes one: the
sheet's values plus a `catalog` map of 10 000 entries, in one store set
once, and 400 small formulas over it, each reading two or three values, a
quarter of them one entry of the catalog.

`update/*` changes single entries of a 100 000-entry map in a store, 1 000
updates an iteration: `set_at` a field of an entry, `insert_existing` a
whole entry by a key the map has, and `insert_remove` a new key inserted
between two and removed again, which moves the entries after it. Against
them, `update/set_whole` is one update as a host without them makes it: the
map rebuilt with one entry changed and set whole, timed per update.

## Profiling

There is no `perf` on the usual machine; callgrind works. Build with debug
info and run one benchmark under it (cargo prints the executable's path):

```sh
CARGO_PROFILE_BENCH_DEBUG=true cargo bench -p evalite --bench evalite --no-run
valgrind --tool=callgrind --callgrind-out-file=/tmp/evalite.cg \
    target/release/build/evalite/<hash>/out/evalite-<hash> --bench '^sheet/evaluate$' --profile-time 2
callgrind_annotate --inclusive=yes /tmp/evalite.cg | head -60
```

## Where the time went (2026-11-26, `86ce38ce`)

On a Ryzen 9 9950X3D with rustc 1.100.0, `sheet/evaluate` took about
125 µs for 108 values, `sheet/compile` 460 µs and `sheet/from_bytes`
180 µs. Under callgrind, `Run::bind` was 78% of `sheet/evaluate`'s
instructions: `measure` 45% and `write` 23%, most of it comparing record
field names and allocating. Cloning and dropping the bound `Value`s was
most of the rest. `run_with` was 10%, reading the result 1% of that.
The run loop costs about 5 ns an instruction (`stage/long`). Calling
`Program::globals()` for every evaluation (`sheet/evaluate_lookup`)
nearly doubles the time, since it builds each `Type` again.

## The globals store (2026-11-27, Globals 2, 5885897e)

Against `main` at `2e27ac8`, back to back, `--warm-up-time 1
--measurement-time 3`. `characterize/evaluate` on `main` was the same
formulas with each one's values bound on every run, in a machine of 2^16
words to hold the catalog:

| Bench | `main` (bind per run) | store | Change |
|---|---|---|---|
| `sheet/evaluate` (108 values) | 36.1 µs | 11.7 µs | −68% |
| `characterize/evaluate` (400 formulas, 10 000-entry catalog) | 99.1 ms | 31.2 µs | −99.97% |
| `stage/long` (370 global reads) | 3.13 µs (`load_bind_run`) | 3.14 µs (`load_run`) | ≈ |

Binding was most of a run's cost, and all of it for a formula reading one
entry of a large map: `characterize` on `main` copied the whole catalog
into every run that read it. With the store, loading copies no values, and
`sheet/set` (12.6 µs) is what setting every value of the sheet costs, once.
The run loop pays a comparison per word read from memory, to tell the
store's words from the machine's; the stack and one-word globals skip it.
