# shexel benchmarks

[`shexel.rs`](shexel.rs) measures each stage a computed value goes through, on
expressions shaped like a character sheet's (attributes, skills,
`specialties`, optional `bonus_N`s, tracks, lookup tables), with
[criterion](https://docs.rs/criterion).

```sh
CARGO_BUILD_JOBS=4 cargo bench -p shexel --bench shexel
# one group or case: the argument is a regex over the names
cargo bench -p shexel --bench shexel -- 'stage/long/'
cargo bench -p shexel --bench shexel -- '^sheet/'
# a quick check that every case still compiles and runs
cargo bench -p shexel --bench shexel -- --test
```

Results go to `target/criterion/`, with a report in
`target/criterion/report/index.html`.

## Comparing against a baseline

Save the numbers on the base, then compare a change against them:

```sh
git switch main
cargo bench -p shexel --bench shexel -- --save-baseline main
git switch my-change
cargo bench -p shexel --bench shexel -- --baseline main
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
- `load`: `Machine::load`, which copies the constants in.
- `load_bind`: `load` and `bind` of every global it reads, cloning each
  value; `bind` is the difference from `load`.
- `load_bind_run`: that and `run_with`, which includes reading the result
  back into a `Value`; `run` is the difference from `load_bind`.
- `end_to_end`: compile, load, bind and run.

The cases, smallest first: `pool` (a dice pool sum), `pool_specialty`
(attribute + skill + specialty + bonus), `unwrap_or_16` and `is_none_16`
(sixteen optional globals), `lookup_literal`, `lookup_global` and
`lookup_host` (tables), `track` (record and track field reads),
`clamp_chain`, `option_list` (lists of `Option<Int>`), `long` (a 370 term
sum, near the default fuel) and `deep` (31 nested parentheses, near the
default `max_depth`).

`read/<expr>` evaluates a value read whole against the same value reduced
to an `Int` by one more instruction, so the difference is roughly what
reading the result costs.

`sheet/*` is a whole character sheet, 108 computed values over one
environment: `compile` and `from_bytes` of all of them, and `evaluate` of
all of them from compiled programs, with the bindings prepared
(`evaluate`) or looked up by name and cloned from the sheet's values
(`evaluate_lookup`).

## Profiling

There is no `perf` on the usual machine; callgrind works. Build with debug
info and run one benchmark under it (cargo prints the executable's path):

```sh
CARGO_PROFILE_BENCH_DEBUG=true cargo bench -p shexel --bench shexel --no-run
valgrind --tool=callgrind --callgrind-out-file=/tmp/shexel.cg \
    target/release/build/shexel/<hash>/out/shexel-<hash> --bench '^sheet/evaluate$' --profile-time 2
callgrind_annotate --inclusive=yes /tmp/shexel.cg | head -60
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
