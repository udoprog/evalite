# evalite-sockel

[<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
[<img alt="crates.io" src="https://img.shields.io/crates/v/evalite-sockel.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite-sockel)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite--sockel-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite-sockel)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/udoprog/evalite/ci.yml?branch=main&style=for-the-badge" height="20">](https://github.com/udoprog/evalite/actions?query=branch%3Amain)

Let your users write [evalite](https://docs.rs/evalite) expressions in a
[Yew](https://yew.rs) app, with an editor that highlights them, underlines
every mistake as they type and completes the names you declare.

**[See it in the evalite playground](https://udoprog.github.io/evalite/)**,
which is built with it.

- **An expression editor.** [`ExpressionEditor`] is
  [sockel](https://docs.rs/sockel)'s code editor with evalite's
  highlighting, every diagnostic underlined and marked in the gutter, and
  completions from your environment (as you type a name, after a `.`, or on
  Ctrl+Space). It compiles as you type, after a short pause, and hands you
  the [`Program`](https://docs.rs/evalite/latest/evalite/struct.Program.html)
  or the diagnostics.
- **Errors people can read.** [`ErrorView`] lists each diagnostic with the
  source line it is about and the span marked, under the editor or on its
  own, such as for a stored formula that no longer compiles.
- **Small pieces.** [`TypeBadge`], [`ExpressionPreview`] and
  [`NamesReference`], for tables of formulas, their types and the names an
  expression may use.
- **A formula field.** [`ExpressionField`] is the editor in a sockel
  `Field`: a label, a hint and the first diagnostic in the error line,
  for a formula in an ordinary form.
- **Results that stay put.** [`ResultView`] shows a run's value read by its
  type, or its error, in room that does not change with the value.
- **Sample inputs.** [`VariablesEditor`] is rows of name, type and value,
  parsed by evalite, that say which variables the expression reads.
- **Styled by sockel.** Nothing to style: the components use sockel's
  components and tokens, so they look like the rest of a sockel app.
- **Translated.** Every word comes from a catalog, [`L10N`].

[`ExpressionEditor`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionEditor.html
[`ErrorView`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ErrorView.html
[`TypeBadge`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.TypeBadge.html
[`ExpressionPreview`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionPreview.html
[`NamesReference`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.NamesReference.html
[`ExpressionField`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionField.html
[`ResultView`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ResultView.html
[`VariablesEditor`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.VariablesEditor.html
[`L10N`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/static.L10N.html

<br>

## Getting started

Add evalite-sockel beside evalite, yew and sockel. It enables sockel's
`badge`, `button`, `code-editor`, `disclosure`, `field`, `input`, `layout` and
`notice` and `property-list` features; mount sockel's stylesheet as for any
sockel app.

```sh
cargo add evalite evalite-sockel
```

Keep the source in your state, the environment in an `Rc` that stays the
same while the names do, and the last compile's diagnostics to show:

```rust
use std::rc::Rc;

use evalite::{Diagnostics, Environment, Type};
use evalite_sockel::{Compiled, ErrorView, ExpressionEditor};
use yew::prelude::*;

#[function_component]
fn Formula() -> Html {
    let source = use_state(|| AttrValue::from("wits.min(3) + bonus"));
    let errors = use_state(|| None::<Rc<Diagnostics>>);

    let environment = use_memo((), |_| {
        let mut env = Environment::new();
        let _ = env.value("wits", Type::int());
        let _ = env.value("bonus", Type::int());
        env
    });

    let on_input = {
        let source = source.clone();
        Callback::from(move |text: String| source.set(text.into()))
    };

    let on_compile = {
        let errors = errors.clone();
        Callback::from(move |compiled: Compiled| errors.set(compiled.err()))
    };

    html! {
        <>
            <ExpressionEditor source={(*source).clone()} {on_input} {environment} {on_compile} />
            if let Some(diagnostics) = &*errors {
                <ErrorView diagnostics={diagnostics.clone()} source={(*source).clone()} />
            }
        </>
    }
}
```

<br>

## Smaller pieces

[`TypeBadge`] shows a [`Type`](https://docs.rs/evalite/latest/evalite/struct.Type.html)
such as `Option<Int>` as a sockel badge. [`ExpressionPreview`] shows an
expression highlighted like the editor's, read-only and inline, for a
table of stored formulas:

```rust
use evalite::Type;
use evalite_sockel::{ExpressionPreview, TypeBadge};
use yew::prelude::*;

html! {
    <tr>
        <td><ExpressionPreview source="wits.min(3) + bonus" /></td>
        <td><TypeBadge ty={Type::option(Type::int())} /></td>
    </tr>
}
```

[`NamesReference`] lists the values and functions of an environment, with
a filter; `on_pick` gets the name clicked, to put into the editor's source:

```rust
use std::rc::Rc;

use evalite::Environment;
use evalite_sockel::NamesReference;
use yew::prelude::*;

let on_pick = Callback::from(move |name: String| {
    source.set(format!("{}{name}", &**source).into());
});

html! { <NamesReference {environment} {on_pick} /> }
```

<br>

## A formula field and its result

A field in a form compiles once the text settles and hands over the
program; run it, then show what it gave:

```rust
use std::rc::Rc;

use evalite::{Environment, Globals, Limits, Machine, Type};
use evalite_sockel::{Compiled, ExpressionField, Ran, ResultView};
use yew::prelude::*;

#[function_component]
fn Damage() -> Html {
    let source = use_state(|| AttrValue::from("1 + 2"));
    let ran = use_state(|| None::<Rc<Ran>>);
    let environment = use_memo((), |_| Environment::new());

    let on_input = {
        let source = source.clone();
        Callback::from(move |text: String| source.set(text.into()))
    };

    let on_compile = {
        let ran = ran.clone();
        let environment = environment.clone();
        Callback::from(move |compiled: Compiled| {
            let shown = compiled.ok().and_then(|program| {
                let globals = Globals::new(&environment);
                let mut machine = Machine::new(Limits::new());
                let outcome = machine.load(&program, &globals).ok()?.run().ok()?;
                Some(Rc::new(Ran::of(&outcome, &program)))
            });

            ran.set(shown);
        })
    };

    html! {
        <>
            <ExpressionField id="damage" label={html! { "Damage" }} source={(*source).clone()}
                {on_input} {environment} {on_compile} />
            if let Some(ran) = &*ran {
                <ResultView ran={ran.clone()} />
            }
        </>
    }
}
```

<br>

## Styles and pruning

The components write sockel's classes only, so sockel's stylesheet covers
them. An app that prunes sockel's CSS with
[`Config::prune`](https://docs.rs/sockel-build/latest/sockel_build/struct.Config.html#method.prune)
does not see the sockel components this crate renders, so it keeps their
classes with [`CLASSES`], adding evalite-sockel as a build dependency too
(`cargo add --build evalite-sockel`):

```rust
sockel_build::Config::new()
    .prune(
        sockel_build::Usage::new()
            .source("src/")
            .foreign(evalite_sockel::CLASSES.iter().copied()),
    )
    .compile();
```

[`CLASSES`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/constant.CLASSES.html

<br>

## Translations

The words are messages in [`L10N`], with every id under the `evalite-`
prefix, read through the same `ContextProvider<Phrases>` as sockel's. An
app adds the catalog beside sockel's, after its own, so its own
`evalite-*` messages win:

```rust
let book = Phrasebook::builder()
    .language(Lang::EN_US, "English")
    .catalog(&APP)
    .catalog(&evalite_sockel::L10N)
    .catalog(&sockel::phrasebookcatalog01::L10N)
    .build();
```

Label props (`label`) override a message for one use.
