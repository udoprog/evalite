//! The evalite playground: the project's page, with an editor that runs
//! evalite in the browser.

mod eval;
mod examples;
mod vars;

/// The playground's own classes (`style/`), from sockel-build.
mod class {
    include!(concat!(env!("OUT_DIR"), "/classes.rs"));
}

/// sockel's CSS, then `style/`'s.
const STYLESHEET: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

const GITHUB: &str = "https://github.com/udoprog/evalite";
const DOCS: &str = "https://docs.rs/evalite";
const CRATES: &str = "https://crates.io/crates/evalite";
const INSTALL: &str = "cargo add evalite";

use std::rc::Rc;

use gloo_timers::callback::Timeout;
use yew::prelude::*;

use sockel::{
    Button, ButtonSize, ButtonVariant, Card, Cluster, Gap, OverlayHost, SectionHead, Select,
    SelectOption, Stack, TextInput, Textarea,
};

use crate::eval::{Binding, Bounds, Done, Mark, evaluate};
use crate::examples::{EXAMPLES, Example};

/// How long the outcome waits after an edit before it is recomputed.
const DEBOUNCE_MS: u32 = 150;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    System,
    Light,
    Dark,
}

fn apply_theme(theme: Theme) {
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    else {
        return;
    };

    let _ = match theme {
        Theme::System => root.remove_attribute("data-theme"),
        Theme::Light => root.set_attribute("data-theme", "light"),
        Theme::Dark => root.set_attribute("data-theme", "dark"),
    };
}

/// One row of the variables table, as typed.
#[derive(Clone, PartialEq)]
struct Row {
    id: u32,
    name: String,
    ty: String,
    value: String,
}

/// What is wrong with a row, by the column it is in.
#[derive(Clone, PartialEq)]
enum RowError {
    Name(String),
    Type(String),
    Value(String),
}

fn row_binding(row: &Row) -> Result<Binding, RowError> {
    let name = row.name.trim();

    if name.is_empty() {
        return Err(RowError::Name("the variable has no name".into()));
    }

    let ty = vars::parse_type(&row.ty).map_err(RowError::Type)?;
    let value = vars::parse_value(&ty, &row.value).map_err(RowError::Value)?;

    Ok(Binding {
        name: name.to_owned(),
        ty,
        value,
    })
}

/// The page's state: what the outcome is computed from.
#[derive(Clone, PartialEq)]
struct Input {
    source: String,
    rows: Vec<Row>,
    bounds: Bounds,
}

impl Input {
    fn example(example: &Example) -> Self {
        let rows = example
            .vars
            .iter()
            .zip(0u32..)
            .map(|(&(name, ty, value), id)| Row {
                id,
                name: name.into(),
                ty: ty.into(),
                value: value.into(),
            })
            .collect();

        let mut bounds = Bounds::default();

        if let Some(fuel) = example.fuel {
            bounds.fuel = fuel;
        }

        Self {
            source: example.source.into(),
            rows,
            bounds,
        }
    }

    fn next_id(&self) -> u32 {
        self.rows
            .iter()
            .map(|r| r.id)
            .max()
            .map_or(0, |id| id.saturating_add(1))
    }
}

type Outcome = Result<Done, Vec<Mark>>;

/// A change to the input from a control's text.
type Edit = dyn Fn(&mut Input, String);

fn compute(input: &Input) -> Outcome {
    let bindings = input
        .rows
        .iter()
        .filter_map(|row| row_binding(row).ok())
        .collect::<Vec<_>>();
    evaluate(&input.source, &bindings, input.bounds)
}

#[function_component]
fn App() -> Html {
    let theme = use_state(|| Theme::System);
    let on_theme = {
        let theme = theme.clone();
        Callback::from(move |next| {
            apply_theme(next);
            theme.set(next);
        })
    };

    let themes = vec![
        SelectOption::new(Theme::System, "System"),
        SelectOption::new(Theme::Light, "Light"),
        SelectOption::new(Theme::Dark, "Dark"),
    ];

    html! {
        <OverlayHost>
            <sockel::Styles css={STYLESHEET} />
            <Stack gap={Gap::Wide} class={class::PAGE}>
                <Hero />
                <Playground />
                <footer>
                    <Cluster gap={Gap::Loose}>
                        <span class={class::MUTED}>{ "evalite is MIT or Apache-2.0" }</span>
                        <a href={GITHUB}>{ "GitHub" }</a>
                        <a href={DOCS}>{ "Documentation" }</a>
                        <a href={CRATES}>{ "crates.io" }</a>
                        <Select<Theme> label="Theme" options={themes} value={Some(*theme)} on_change={on_theme} />
                    </Cluster>
                </footer>
            </Stack>
        </OverlayHost>
    }
}

#[function_component]
fn Hero() -> Html {
    let copied = use_state(|| false);
    let on_copy = {
        let copied = copied.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(window) = web_sys::window() {
                let _ = window.navigator().clipboard().write_text(INSTALL);
                copied.set(true);
            }
        })
    };

    let points: [(&str, &str); 6] = [
        (
            "Strictly typed.",
            "Every expression has one type, known when it compiles. Nothing converts behind your back.",
        ),
        (
            "Safe on untrusted input.",
            "No recursion, bounded nesting, fixed memory and a step limit. It never panics.",
        ),
        (
            "Compile once, store it.",
            "A program is bytes you keep next to your data, and runs without the environment.",
        ),
        (
            "Verified on load.",
            "Stored bytes are not trusted: a loaded program is type-checked again before it runs.",
        ),
        (
            "Errors that point.",
            "Every error says what it expected and what it found, with labels on the exact spans.",
        ),
        (
            "Built for forms.",
            "Completions offer what fits each hole, by type, so users build expressions by picking.",
        ),
    ];

    html! {
        <header>
            <Stack gap={Gap::Loose}>
                <h1 class={class::DISPLAY}>{ "evalite" }</h1>
                <p class={class::LEAD}>
                    { "Small, strictly typed expressions that are safe to run on input you don't trust. \
                       Let your users write " }
                    <code class={class::CODE}>{ "min(wits, dexterity) + athletics" }</code>
                    { " and run it a million times without worrying." }
                </p>
                <Cluster gap={Gap::Normal}>
                    <code class={classes!(class::CODE, class::CODE_BLOCK)}>{ INSTALL }</code>
                    <Button onclick={on_copy}>{ if *copied { "Copied" } else { "Copy" } }</Button>
                    <Button variant={ButtonVariant::Primary} href={DOCS}>{ "Read the docs" }</Button>
                    <Button variant={ButtonVariant::Ghost} href={GITHUB}>{ "GitHub" }</Button>
                    <Button variant={ButtonVariant::Ghost} href={CRATES}>{ "crates.io" }</Button>
                </Cluster>
                <ul class={class::POINTS}>
                    { for points.iter().map(|(head, body)| html! {
                        <li><strong>{ *head }</strong>{ " " }<span class={class::MUTED}>{ *body }</span></li>
                    }) }
                </ul>
            </Stack>
        </header>
    }
}

#[function_component]
fn Playground() -> Html {
    let input = use_state(|| {
        EXAMPLES
            .first()
            .map(Input::example)
            .unwrap_or_else(|| Input {
                source: String::new(),
                rows: Vec::new(),
                bounds: Bounds::default(),
            })
    });
    let outcome = use_state(|| Rc::new(compute(&input)));

    {
        let outcome = outcome.clone();
        use_effect_with((*input).clone(), move |input| {
            let input = input.clone();
            let timeout = Timeout::new(DEBOUNCE_MS, move || outcome.set(Rc::new(compute(&input))));
            move || drop(timeout)
        });
    }

    let edit = |f: Rc<Edit>| {
        let input = input.clone();
        Callback::from(move |text: String| {
            let mut next = (*input).clone();
            f(&mut next, text);
            input.set(next);
        })
    };

    let on_source = edit(Rc::new(|input, text| input.source = text));

    let examples = EXAMPLES
        .iter()
        .enumerate()
        .map(|(index, e)| SelectOption::new(index, e.title))
        .collect::<Vec<_>>();
    let on_example = {
        let input = input.clone();
        Callback::from(move |index: usize| {
            if let Some(example) = EXAMPLES.get(index) {
                input.set(Input::example(example));
            }
        })
    };

    let on_add = {
        let input = input.clone();
        Callback::from(move |_: MouseEvent| {
            let mut next = (*input).clone();
            let id = next.next_id();
            next.rows.push(Row {
                id,
                name: format!("v{id}"),
                ty: "Int".into(),
                value: "0".into(),
            });
            input.set(next);
        })
    };

    let primary = match &**outcome {
        Err(marks) => marks
            .iter()
            .find(|m| m.primary)
            .and_then(|m| m.span.clone()),
        Ok(_) => None,
    };

    let rows = input.rows.iter().map(|row| {
        let id = row.id;
        let set = |f: fn(&mut Row, String)| {
            edit(Rc::new(move |input: &mut Input, text: String| {
                if let Some(row) = input.rows.iter_mut().find(|r| r.id == id) {
                    f(row, text);
                }
            }))
        };
        let on_remove = {
            let input = input.clone();
            Callback::from(move |_: MouseEvent| {
                let mut next = (*input).clone();
                next.rows.retain(|r| r.id != id);
                input.set(next);
            })
        };
        let error = row_binding(row).err();
        let (name_bad, ty_bad, value_bad) = match &error {
            Some(RowError::Name(_)) => (true, false, false),
            Some(RowError::Type(_)) => (false, true, false),
            Some(RowError::Value(_)) => (false, false, true),
            None => (false, false, false),
        };
        let message = match &error {
            Some(RowError::Name(m) | RowError::Type(m) | RowError::Value(m)) => m.clone(),
            None => String::new(),
        };

        html! {
            <li key={id}>
                <Stack gap={Gap::Hair}>
                    <div class={class::VAR_ROW}>
                        <TextInput class={class::CODE} label="Name" value={row.name.clone()} invalid={name_bad}
                            on_input={set(|r, t| r.name = t)} />
                        <TextInput class={class::CODE} label="Type" value={row.ty.clone()} invalid={ty_bad}
                            on_input={set(|r, t| r.ty = t)} />
                        <TextInput class={class::CODE} label="Value" value={row.value.clone()} invalid={value_bad}
                            on_input={set(|r, t| r.value = t)} />
                        <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_remove}>{ "Remove" }</Button>
                    </div>
                    <span class={class::ROW_ERROR} role="status">{ message }</span>
                </Stack>
            </li>
        }
    });

    html! {
        <section aria-label="Playground">
            <Stack gap={Gap::Loose}>
                <SectionHead title={html! { "Try it" }} />
                <div class={class::COLUMNS}>
                    <Stack gap={Gap::Normal}>
                        <Cluster gap={Gap::Snug}>
                            <Select<usize> label="Example" placeholder="Examples" options={examples} on_change={on_example} />
                        </Cluster>
                        <div class={class::EDITOR}>
                            <Textarea class={classes!(class::CODE, class::EDITOR_INPUT)} label="Expression"
                                value={input.source.clone()} on_input={on_source} rows={4}
                                invalid={primary.is_some()} />
                            <div class={classes!(class::CODE, class::EDITOR_MARKS)} aria-hidden="true">
                                { marked(&input.source, primary) }
                            </div>
                        </div>
                        <Stack gap={Gap::Snug}>
                            <strong>{ "Variables" }</strong>
                            <Stack tag="ul" gap={Gap::Snug}>{ for rows }</Stack>
                            <Cluster>
                                <Button size={ButtonSize::Small} onclick={on_add}>{ "Add a variable" }</Button>
                            </Cluster>
                        </Stack>
                    </Stack>
                    <Card class={class::OUTCOME} gap={Gap::Normal}>
                        <OutcomeView outcome={(*outcome).clone()} bounds={input.bounds} />
                    </Card>
                </div>
            </Stack>
        </section>
    }
}

/// `source` with the characters of `span` wrapped in a mark. An empty span
/// (the end of the expression) marks a space after it.
fn marked(source: &str, span: Option<std::ops::Range<usize>>) -> Html {
    let Some(span) = span else {
        return html! { <>{ source.to_owned() }{ "\u{200b}" }</> };
    };

    let before = source.chars().take(span.start).collect::<String>();
    let inside = source
        .chars()
        .skip(span.start)
        .take(span.end.saturating_sub(span.start))
        .collect::<String>();
    let after = source.chars().skip(span.end).collect::<String>();
    let inside = if inside.is_empty() {
        " ".to_owned()
    } else {
        inside
    };

    html! {
        <>{ before }<span class={class::MARK}>{ inside }</span>{ after }{ "\u{200b}" }</>
    }
}

#[derive(Properties, PartialEq)]
struct OutcomeProps {
    outcome: Rc<Outcome>,
    bounds: Bounds,
}

#[function_component]
fn OutcomeView(props: &OutcomeProps) -> Html {
    match &*props.outcome {
        Ok(done) => html! {
            <>
                <span class={class::MUTED}>{ "Value" }</span>
                <p class={classes!(class::CODE, class::VALUE)}>
                    { &done.value }
                    <span class={class::MUTED}>{ format!(" : {}", done.ty) }</span>
                </p>
                <Cluster gap={Gap::Loose}>
                    <span class={sockel::class::NUMERIC}>{ format!("{} of {} steps", done.steps, props.bounds.fuel) }</span>
                    <span class={sockel::class::NUMERIC}>{ format!("{} of {} words", done.memory, props.bounds.memory) }</span>
                </Cluster>
            </>
        },
        Err(marks) => {
            let (primary, context): (Vec<_>, Vec<_>) = marks.iter().partition(|m| m.primary);

            html! {
                <>
                    <span class={class::MUTED}>{ "Error" }</span>
                    { for primary.iter().map(|m| html! { <p class={class::ERROR}>{ &m.message }</p> }) }
                    if !context.is_empty() {
                        <ul class={sockel::class::PLAIN_LIST}>
                            { for context.iter().map(|m| html! {
                                <li class={class::MUTED}>{ format!("= {}", m.message) }</li>
                            }) }
                        </ul>
                    }
                </>
            }
        }
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_runs_as_meant() {
        for example in EXAMPLES {
            let input = Input::example(example);

            for row in &input.rows {
                assert!(
                    row_binding(row).is_ok(),
                    "{}: row `{}`",
                    example.title,
                    row.name
                );
            }

            let outcome = compute(&input);
            let fails = matches!(example.title, "Option error" | "Out of fuel");
            assert_eq!(outcome.is_err(), fails, "{}: {:?}", example.title, outcome);
        }
    }
}
