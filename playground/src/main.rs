//! The evalite playground: the project's page, with an editor that runs
//! evalite in the browser.

mod code_editor;
mod complete;
mod editor;
mod eval;
mod examples;
mod share;
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
    Align, Button, ButtonSize, ButtonVariant, Card, CardGrid, Cluster, Disclosure, ErrorBox, Field,
    Gap, GridSize, InputSize, Inset, OverlayHost, PropertyList, PropertyRow, SectionHead, Select,
    SelectOption, Stack, Stepper, TextInput, Toolbar, ToolbarSpacer,
};

use crate::editor::Editor;
use crate::eval::{Binding, Bounds, Done, Mark, evaluate};
use crate::examples::{EXAMPLES, Example};
use crate::share::Shared;

/// How long the outcome waits after an edit before it is recomputed.
const DEBOUNCE_MS: u32 = 150;

/// The types the type picker offers; anything else is written out.
const TYPES: &[&str] = &[
    "Int",
    "Bool",
    "String",
    "Option<Int>",
    "Option<Bool>",
    "Option<String>",
    "List<Int>",
    "List<Bool>",
    "List<String>",
];

/// The type picker's choice for a type written out.
const OTHER: &str = "";

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
    /// The type is written out rather than picked.
    custom: bool,
}

impl Row {
    fn new(id: u32, name: &str, ty: &str, value: &str) -> Self {
        Self {
            id,
            name: name.into(),
            ty: ty.into(),
            value: value.into(),
            custom: !TYPES.contains(&ty),
        }
    }
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

/// A value of type `ty` to start from: the first of a few simple ones that
/// is one.
fn starting_value(ty: &str) -> Option<&'static str> {
    let ty = vars::parse_type(ty).ok()?;

    ["0", "false", "\"\"", "none", "[]"]
        .into_iter()
        .find(|text| vars::parse_value(&ty, text).is_ok())
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
            .map(|(&(name, ty, value), id)| Row::new(id, name, ty, value))
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

    /// The first example, or nothing.
    fn initial() -> Self {
        EXAMPLES
            .first()
            .map(Input::example)
            .unwrap_or_else(|| Input {
                source: String::new(),
                rows: Vec::new(),
                bounds: Bounds::default(),
            })
    }

    fn shared(shared: Shared) -> Self {
        let defaults = Bounds::default();

        Self {
            source: shared.source,
            rows: shared
                .vars
                .iter()
                .zip(0u32..)
                .map(|((name, ty, value), id)| Row::new(id, name, ty, value))
                .collect(),
            bounds: Bounds {
                fuel: shared.fuel.unwrap_or(defaults.fuel),
                memory: shared.memory.unwrap_or(defaults.memory),
                depth: shared.depth.unwrap_or(defaults.depth),
            },
        }
    }

    fn to_shared(&self) -> Shared {
        let defaults = Bounds::default();

        Shared {
            source: self.source.clone(),
            vars: self
                .rows
                .iter()
                .map(|r| (r.name.clone(), r.ty.clone(), r.value.clone()))
                .collect(),
            fuel: (self.bounds.fuel != defaults.fuel).then_some(self.bounds.fuel),
            memory: (self.bounds.memory != defaults.memory).then_some(self.bounds.memory),
            depth: (self.bounds.depth != defaults.depth).then_some(self.bounds.depth),
        }
    }

    fn next_id(&self) -> u32 {
        self.rows
            .iter()
            .map(|r| r.id)
            .max()
            .map_or(0, |id| id.saturating_add(1))
    }

    fn bindings(&self) -> Vec<Binding> {
        self.rows
            .iter()
            .filter_map(|row| row_binding(row).ok())
            .collect()
    }
}

type Outcome = Result<Done, Vec<Mark>>;

/// A change to the input from a control's text.
type Edit = dyn Fn(&mut Input, String);

fn compute(input: &Input) -> Outcome {
    evaluate(&input.source, &input.bindings(), input.bounds)
}

/// The state in the page's fragment, if any and well formed.
fn from_location() -> Option<Input> {
    let hash = web_sys::window()?.location().hash().ok()?;
    share::decode(&hash).map(Input::shared)
}

/// Keep the page's fragment at `input`, without a history entry per edit.
fn to_location(input: &Input) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let fragment = format!("#{}", share::encode(&input.to_shared()));

    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&fragment));
    }
}

fn copy(text: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().clipboard().write_text(text);
    }
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
            <Stack gap={Gap::Wide} class={sockel::class::WORKSPACE_PAGE}>
                <Hero />
                <Playground />
                <footer>
                    <Cluster gap={Gap::Loose}>
                        <span>{ "evalite is MIT or Apache-2.0" }</span>
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
            copy(INSTALL);
            copied.set(true);
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
                <Cluster gap={Gap::Tight}>
                    <div>
                        <TextInput class={class::CODE} label="Install" value={INSTALL} readonly=true
                            on_input={Callback::noop()} />
                    </div>
                    <Button onclick={on_copy}>{ if *copied { "Copied" } else { "Copy" } }</Button>
                    <Button variant={ButtonVariant::Primary} href={DOCS}>{ "Read the docs" }</Button>
                    <Button variant={ButtonVariant::Ghost} href={GITHUB}>{ "GitHub" }</Button>
                    <Button variant={ButtonVariant::Ghost} href={CRATES}>{ "crates.io" }</Button>
                </Cluster>
                <CardGrid size={GridSize::Wide} tag="ul">
                    { for points.iter().map(|(head, body)| html! {
                        <Card tag="li" inset={Inset::Compact} gap={Gap::Snug}>
                            <strong>{ *head }</strong>
                            <span>{ *body }</span>
                        </Card>
                    }) }
                </CardGrid>
            </Stack>
        </header>
    }
}

#[function_component]
fn Playground() -> Html {
    let input = use_state(|| from_location().unwrap_or_else(Input::initial));
    let outcome = use_state(|| Rc::new(compute(&input)));
    let linked = use_state(|| false);
    let vars_open = use_state(|| !input.rows.is_empty());
    let limits_open = use_state(|| false);

    {
        let (outcome, linked) = (outcome.clone(), linked.clone());
        use_effect_with((*input).clone(), move |input| {
            let input = input.clone();
            let timeout = Timeout::new(DEBOUNCE_MS, move || {
                to_location(&input);
                linked.set(false);
                outcome.set(Rc::new(compute(&input)));
            });
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
        let (input, vars_open) = (input.clone(), vars_open.clone());
        Callback::from(move |index: usize| {
            if let Some(example) = EXAMPLES.get(index) {
                vars_open.set(!example.vars.is_empty());
                input.set(Input::example(example));
            }
        })
    };

    let on_share = {
        let (input, linked) = (input.clone(), linked.clone());
        Callback::from(move |_: MouseEvent| {
            to_location(&input);

            if let Some(href) = web_sys::window().and_then(|w| w.location().href().ok()) {
                copy(&href);
                linked.set(true);
            }
        })
    };

    let on_add = {
        let (input, vars_open) = (input.clone(), vars_open.clone());
        Callback::from(move |_: MouseEvent| {
            let mut next = (*input).clone();
            let id = next.next_id();
            next.rows.push(Row::new(id, &format!("v{id}"), "Int", "0"));
            input.set(next);
            vars_open.set(true);
        })
    };

    let toggle = |open: &UseStateHandle<bool>| {
        let open = open.clone();
        Callback::from(move |next: bool| open.set(next))
    };

    let primary = match &**outcome {
        Err(marks) => marks
            .iter()
            .find(|m| m.primary)
            .and_then(|m| m.span.clone()),
        Ok(_) => None,
    };

    let bindings = Rc::new(input.bindings());

    let rows = input.rows.iter().map(|row| {
        html! { <VarRow key={row.id} row={row.clone()} input={input.clone()} /> }
    });

    let vars_summary = if input.rows.is_empty() {
        String::from("None")
    } else {
        input
            .rows
            .iter()
            .map(|r| r.name.trim())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let limit = |label: &'static str,
                 id: &'static str,
                 value: i64,
                 max: i64,
                 step: i64,
                 used: Option<u64>,
                 set: fn(&mut Bounds, i64)| {
        let input = input.clone();
        let on_change = Callback::from(move |n: i64| {
            let mut next = (*input).clone();
            set(&mut next.bounds, n);
            input.set(next);
        });

        html! {
            <Field label={html! { label }} {id} hint={used.map(|n| html! { format!("{n} used") })} class={classes!(sockel::class::GROW, sockel::class::BASIS_SM)}>
                <Stepper {id} label={label} {value} min={1} {max} {step} {on_change} />
            </Field>
        }
    };

    let bounds = input.bounds;
    let used = (**outcome).as_ref().ok();
    let limits_summary = match &**outcome {
        Ok(done) => format!(
            "{} of {} · {} of {}",
            done.steps,
            plural(bounds.fuel, "step", "steps"),
            done.memory,
            plural(u64::from(bounds.memory), "word", "words"),
        ),
        Err(_) => format!(
            "{} · {} · depth {}",
            plural(bounds.fuel, "step", "steps"),
            plural(u64::from(bounds.memory), "word", "words"),
            bounds.depth,
        ),
    };

    html! {
        <section aria-label="Playground">
            <Stack gap={Gap::Loose}>
                <SectionHead title={html! { "Try it" }} />
                <Stack gap={Gap::Tight}>
                    <Cluster gap={Gap::Loose} align={Align::Top}>
                        <Stack gap={Gap::Tight} class={classes!(sockel::class::GROW, sockel::class::BASIS_LG)}>
                            <Toolbar label="Example and link">
                                <Select<usize> label="Example" placeholder="Examples" options={examples} on_change={on_example} />
                                <ToolbarSpacer />
                                <Button variant={ButtonVariant::Quiet} onclick={on_share}>
                                    { if *linked { "Link copied" } else { "Copy link" } }
                                </Button>
                            </Toolbar>
                            <Editor source={input.source.clone()} on_change={on_source} {primary} {bindings} />
                        </Stack>
                        <Card class={classes!(class::OUTCOME, sockel::class::GROW, sockel::class::BASIS_MD)} gap={Gap::Tight}>
                            <OutcomeView outcome={(*outcome).clone()} />
                        </Card>
                    </Cluster>
                    <Cluster gap={Gap::Loose} align={Align::Top}>
                        <Disclosure class={classes!(sockel::class::GROW, sockel::class::BASIS_LG)} title={html! { "Variables" }} summary={html! { vars_summary }}
                            id="playground-variables" open={*vars_open} on_toggle={toggle(&vars_open)}
                            actions={html! {
                                <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_add}>
                                    { "Add a variable" }
                                </Button>
                            }}>
                            <Stack tag="ul" gap={Gap::Hair} class={sockel::class::PLAIN_LIST}>{ for rows }</Stack>
                        </Disclosure>
                        <Disclosure class={classes!(sockel::class::GROW, sockel::class::BASIS_MD)} title={html! { "Limits" }} summary={html! { limits_summary }}
                            id="playground-limits" open={*limits_open} on_toggle={toggle(&limits_open)}>
                            <Cluster gap={Gap::Medium} align={Align::Top}>
                                { limit("Fuel (steps)", "limit-fuel", i64::try_from(bounds.fuel).unwrap_or(i64::MAX), 1_000_000, 10, used.map(|d| d.steps),
                                    |b, n| b.fuel = u64::try_from(n).unwrap_or(1)) }
                                { limit("Memory (words)", "limit-memory", i64::from(bounds.memory), 1 << 20, 64, used.map(|d| d.memory),
                                    |b, n| b.memory = u32::try_from(n).unwrap_or(1)) }
                                { limit("Depth", "limit-depth", i64::try_from(bounds.depth).unwrap_or(i64::MAX), 256, 1, None,
                                    |b, n| b.depth = usize::try_from(n).unwrap_or(1)) }
                            </Cluster>
                        </Disclosure>
                    </Cluster>
                </Stack>
            </Stack>
        </section>
    }
}

/// `n` and its noun, singular for one.
fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[derive(Properties, PartialEq)]
struct VarRowProps {
    row: Row,
    input: UseStateHandle<Input>,
}

/// One variable: its name, its type (picked, or written out), its value,
/// and its error in a line kept for it.
#[function_component]
fn VarRow(props: &VarRowProps) -> Html {
    let id = props.row.id;
    let row = &props.row;

    let update = {
        let input = props.input.clone();
        move |f: Rc<dyn Fn(&mut Row)>| {
            let mut next = (*input).clone();

            if let Some(row) = next.rows.iter_mut().find(|r| r.id == id) {
                f(row);
            }

            input.set(next);
        }
    };

    let set = |f: fn(&mut Row, String)| {
        let update = update.clone();
        Callback::from(move |text: String| {
            update(Rc::new(move |row: &mut Row| f(row, text.clone())))
        })
    };

    let on_pick = {
        let update = update.clone();
        Callback::from(move |ty: &'static str| {
            update(Rc::new(move |row: &mut Row| {
                row.custom = ty == OTHER;

                if ty == OTHER {
                    return;
                }

                row.ty = ty.into();

                let fits = vars::parse_type(ty)
                    .is_ok_and(|parsed| vars::parse_value(&parsed, &row.value).is_ok());

                if !fits && let Some(value) = starting_value(ty) {
                    row.value = value.into();
                }
            }))
        })
    };

    let on_remove = {
        let input = props.input.clone();
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

    let types = TYPES
        .iter()
        .map(|&ty| SelectOption::new(ty, ty))
        .chain(std::iter::once(SelectOption::new(OTHER, "Other…")))
        .collect::<Vec<_>>();
    let picked = if row.custom {
        OTHER
    } else {
        TYPES
            .iter()
            .copied()
            .find(|&t| t == row.ty)
            .unwrap_or(OTHER)
    };

    html! {
        <li>
            <Stack gap={Gap::Hair}>
                <Cluster gap={Gap::Tight}>
                    <TextInput class={classes!(class::CODE, sockel::class::GROW, sockel::class::BASIS_SM)}
                        label="Name" size={InputSize::Small} value={row.name.clone()} invalid={name_bad}
                        on_input={set(|r, t| r.name = t)} />
                    <Select<&'static str> label="Type" size={InputSize::Small} options={types} value={Some(picked)}
                        invalid={ty_bad && !row.custom} on_change={on_pick} />
                    if row.custom {
                        <TextInput class={classes!(class::CODE, sockel::class::GROW, sockel::class::BASIS_MD)}
                            label="Type" size={InputSize::Small} placeholder="{boxes: Int}" value={row.ty.clone()} invalid={ty_bad}
                            on_input={set(|r, t| r.ty = t)} />
                    }
                    <TextInput class={classes!(class::CODE, sockel::class::GROW, sockel::class::BASIS_SM)}
                        label="Value" size={InputSize::Small} value={row.value.clone()} invalid={value_bad}
                        on_input={set(|r, t| r.value = t)} />
                    <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_remove}>{ "Remove" }</Button>
                </Cluster>
                <p class={sockel::field_error(true)} role="status">{ message }</p>
            </Stack>
        </li>
    }
}

#[derive(Properties, PartialEq)]
struct OutcomeProps {
    outcome: Rc<Outcome>,
}

#[function_component]
fn OutcomeView(props: &OutcomeProps) -> Html {
    match &*props.outcome {
        Ok(done) => html! {
            <PropertyList label="Outcome">
                <PropertyRow label="Value">
                    <span class={classes!(class::CODE, class::VALUE)}>{ &done.value }</span>
                </PropertyRow>
                <PropertyRow label="Type">
                    <span class={class::CODE}>{ &done.ty }</span>
                </PropertyRow>
            </PropertyList>
        },
        Err(marks) => {
            let (primary, context): (Vec<_>, Vec<_>) = marks.iter().partition(|m| m.primary);
            let message = primary
                .iter()
                .map(|m| m.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let causes = context
                .iter()
                .map(|m| AttrValue::from(m.message.clone()))
                .collect::<Vec<_>>();

            html! {
                <ErrorBox title="Error" {causes}>{ message }</ErrorBox>
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

    #[test]
    fn every_example_round_trips_a_link() {
        for example in EXAMPLES {
            let input = Input::example(example);
            let fragment = share::encode(&input.to_shared());
            let back = share::decode(&fragment).map(Input::shared);
            assert!(back == Some(input), "{}", example.title);
        }
    }

    #[test]
    fn every_picked_type_has_a_starting_value() {
        for ty in TYPES {
            assert!(starting_value(ty).is_some(), "{ty}");
        }
    }
}
