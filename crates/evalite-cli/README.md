# evalite-cli

[<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
[<img alt="crates.io" src="https://img.shields.io/crates/v/evalite-cli.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite-cli)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite--cli-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite-cli)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/udoprog/evalite/ci.yml?branch=main&style=for-the-badge" height="20">](https://github.com/udoprog/evalite/actions?query=branch%3Amain)

`evalite`, the command line for [evalite]: small, strictly typed
expressions that are safe to run on anyone's input.

Variables are declared and set with `--var name:Type=value`, where a
type is `Int`, `Bool`, `String`, `Option<T>` or `List<T>`, and a value is
`3`, `true`, `"text"`, `none`, `some(2)` or `[1, 2]`.

```text
$ evalite eval 'wits.min(dexterity) + bonus.unwrap_or(0)' --var wits:Int=3 --var dexterity:Int=2 --var 'bonus:Option<Int>=some(1)'
3 : Int

$ evalite eval 'skills[3] * 2' --var 'skills:List<Int>=[1, 2]'
error: index 3 is out of range of a list of 2 entries
  | skills[3] * 2
  | ^^^^^^^^^
  = while running instruction 2
  = while running the program

$ evalite --var strength:Int=3          # one expression per line from stdin
> strength * 2
6 : Int
```

`--max-depth` (nesting), `--max-len` (bytes), `--memory` (64-bit words) and `--fuel` (steps)
set the limits, and `--explain` also prints the type, memory and steps.
An error prints the expression with a caret under what is wrong, then
what was being done, and exits non-zero.

[evalite]: https://docs.rs/evalite
