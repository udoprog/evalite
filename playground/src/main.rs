//! The evalite playground: the project's page, with an editor that runs
//! evalite in the browser.

mod claims;
mod eval;
mod examples;
mod share;

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

use evalite::{Environment, Type, Value};
use evalite_sockel::{ErrorView, ExpressionEditor, NamesReference, Ran, ResultView};
use gloo_timers::callback::Timeout;
use yew::prelude::*;

use sockel::{
    Align, Button, ButtonSize, ButtonVariant, Card, Cluster, Disclosure, Field, Gap, InputSize,
    OverlayHost, SectionHead, Stack, Stepper, TextInput,
};

use crate::claims::{CLAIMS, Gives};
use crate::eval::{Binding, Bounds, Done, Failed, evaluate};
use crate::examples::{EXAMPLES, Example};
use crate::share::Shared;

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

impl Row {
    fn new(id: u32, name: &str, ty: &str, value: &str) -> Self {
        Self {
            id,
            name: name.into(),
            ty: ty.into(),
            value: value.into(),
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

    let ty = Type::parse(&row.ty).map_err(|e| RowError::Type(e.to_string()))?;
    let value = Value::parse(&row.value, &ty).map_err(|e| RowError::Value(e.to_string()))?;

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
                len: shared.len.unwrap_or(defaults.len),
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
            len: (self.bounds.len != defaults.len).then_some(self.bounds.len),
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

type Outcome = Result<Done, Failed>;

/// An outcome, with the source it is of.
#[derive(PartialEq)]
struct Computed {
    source: AttrValue,
    outcome: Outcome,
}

impl Computed {
    fn of(input: &Input) -> Rc<Self> {
        Rc::new(Self {
            source: input.source.clone().into(),
            outcome: compute(input),
        })
    }
}

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

impl Theme {
    /// The theme after this one, as the footer's button cycles them.
    fn next(self) -> Self {
        match self {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::System,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }
}

/// The playground's section, which loading an example scrolls to.
const PLAYGROUND_ID: &str = "playground";

fn scroll_to(id: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        element.scroll_into_view();
    }
}

#[function_component]
fn App() -> Html {
    let theme = use_state(|| Theme::System);
    let on_theme = {
        let theme = theme.clone();
        Callback::from(move |_: MouseEvent| {
            let next = theme.next();
            apply_theme(next);
            theme.set(next);
        })
    };

    let input = use_state(|| from_location().unwrap_or_else(Input::initial));
    let vars_open = use_state(|| !input.rows.is_empty());

    let on_load = {
        let (input, vars_open) = (input.clone(), vars_open.clone());
        Callback::from(move |(claim, case): (usize, usize)| {
            if let Some(case) = CLAIMS.get(claim).and_then(|c| c.cases.get(case)) {
                vars_open.set(!case.example.vars.is_empty());
                input.set(Input::example(&case.example));
                scroll_to(PLAYGROUND_ID);
            }
        })
    };

    html! {
        <OverlayHost>
            <sockel::Styles css={STYLESHEET} />
            <Stack gap={Gap::Wide} class={sockel::class::WORKSPACE_PAGE}>
                <Hero />
                <Playground input={input.clone()} vars_open={vars_open.clone()} />
                <Claims {on_load} />
                <footer>
                    <Cluster gap={Gap::Loose}>
                        <span class={class::MUTED}>{ "evalite is MIT or Apache-2.0" }</span>
                        <a href={GITHUB}>{ "GitHub" }</a>
                        <a href={DOCS}>{ "Documentation" }</a>
                        <a href={CRATES}>{ "crates.io" }</a>
                        <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_theme}>
                            { format!("Theme: {}", theme.name()) }
                        </Button>
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

    html! {
        <header>
            <Stack gap={Gap::Loose}>
                <h1 class={class::DISPLAY}>{ "evalite" }</h1>
                <p class={class::LEAD}>
                    { "Let your users write formulas like " }
                    <code class={class::CODE}>{ "min(wits, dexterity) + athletics" }</code>
                    { ". evalite type-checks them before they run, and nothing they type can crash or hang your program." }
                </p>
                <Cluster gap={Gap::Tight}>
                    <code class={classes!(class::CODE, class::INSTALL)}>{ INSTALL }</code>
                    <Button variant={ButtonVariant::Quiet} onclick={on_copy}>{ if *copied { "Copied" } else { "Copy" } }</Button>
                    <Button variant={ButtonVariant::Primary} href={DOCS}>{ "Read the docs" }</Button>
                    <Button variant={ButtonVariant::Ghost} href={GITHUB}>{ "GitHub" }</Button>
                </Cluster>
            </Stack>
        </header>
    }
}

#[derive(Properties, PartialEq)]
struct ClaimsProps {
    /// Load a claim's case, by the claim's index and the case's.
    on_load: Callback<(usize, usize)>,
}

/// What evalite does, each with examples that load into the playground.
#[function_component]
fn Claims(props: &ClaimsProps) -> Html {
    let claims = CLAIMS.iter().enumerate().map(|(c, claim)| {
        let cases = claim.cases.iter().enumerate().map(|(k, case)| {
            let onclick = props.on_load.reform(move |_: MouseEvent| (c, k));

            let gives = match &case.gives {
                Gives::Value(value) => html! { <span class={class::GIVES_VALUE}>{ format!("→ {value}") }</span> },
                Gives::Error(message) => html! { <span class={class::GIVES_ERROR}>{ format!("error: {message}") }</span> },
                Gives::Errors(messages) => html! {
                    <span class={class::GIVES_ERROR}>{ messages.iter().map(|m| format!("error: {m}")).collect::<Vec<_>>().join("; ") }</span>
                },
                Gives::Offers { some, not, more } => html! {
                    <>
                        <span class={class::GIVES_VALUE}>
                            { format!("offers {}{}", some.join(", "), if *more { ", …" } else { "" }) }
                        </span>
                        if !not.is_empty() {
                            <span class={class::MUTED}>{ format!("never {}", not.join(", ")) }</span>
                        }
                    </>
                },
            };

            html! {
                <li>
                    <button type="button" class={class::EXAMPLE} {onclick} title="Load into the playground">
                        <code class={class::CODE}>{ case.shown.unwrap_or(case.example.source) }</code>
                        { gives }
                    </button>
                </li>
            }
        });

        html! {
            <li class={class::CLAIM}>
                <div class={class::CLAIM_TEXT}>
                    <h3>{ claim.head }</h3>
                    <p>{ claim.body }</p>
                </div>
                <Stack gap={Gap::Tight} class={class::CLAIM_EXAMPLES}>
                    if !claim.cases.is_empty() {
                        <ul class={sockel::class::PLAIN_LIST}>{ for cases }</ul>
                    }
                    if let Some(rust) = claim.rust {
                        <pre class={classes!(class::CODE, class::RUST)}>{ rust }</pre>
                    }
                    if let Some(hint) = claim.hint {
                        <p class={class::MUTED}>{ hint }</p>
                    }
                </Stack>
            </li>
        }
    });

    html! {
        <section aria-label="What you get">
            <Stack gap={Gap::Loose}>
                <SectionHead title={html! { "What you get" }} />
                <p class={class::MUTED}>{ "Click an example to load it into the playground above." }</p>
                <ul class={classes!(sockel::class::PLAIN_LIST, class::CLAIMS)}>{ for claims }</ul>
            </Stack>
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct PlaygroundProps {
    input: UseStateHandle<Input>,
    vars_open: UseStateHandle<bool>,
}

#[function_component]
fn Playground(props: &PlaygroundProps) -> Html {
    let input = props.input.clone();
    let vars_open = props.vars_open.clone();
    let outcome = use_state(|| Computed::of(&input));
    let linked = use_state(|| false);
    let limits_open = use_state(|| false);
    let names_open = use_state(|| false);

    {
        let (outcome, linked) = (outcome.clone(), linked.clone());
        use_effect_with((*input).clone(), move |input| {
            let input = input.clone();
            let timeout = Timeout::new(DEBOUNCE_MS, move || {
                to_location(&input);
                linked.set(false);
                outcome.set(Computed::of(&input));
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

    let on_pick = {
        let input = input.clone();
        Callback::from(move |name: String| {
            let mut next = (*input).clone();
            next.source.push_str(&name);
            input.set(next);
        })
    };

    let examples = EXAMPLES.iter().map(|example| {
        let (input, vars_open) = (input.clone(), vars_open.clone());
        let onclick = Callback::from(move |_: MouseEvent| {
            vars_open.set(!example.vars.is_empty());
            input.set(Input::example(example));
        });

        html! {
            <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} {onclick}>{ example.title }</Button>
        }
    });

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

    let environment = use_memo(input.bindings(), |bindings| {
        eval::environment(bindings).unwrap_or_else(|_| Environment::with_builtins())
    });

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
    let used = outcome.outcome.as_ref().ok();
    let limits_summary = match &outcome.outcome {
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
        <section aria-label="Playground" id={PLAYGROUND_ID}>
            <Stack gap={Gap::Tight}>
                <SectionHead title={html! { "Try it" }} suffix={html! {
                    <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_share}>
                        { if *linked { "Link copied" } else { "Copy link" } }
                    </Button>
                }} />
                <Cluster gap={Gap::Hair}>
                    <span class={class::MUTED}>{ "Examples:" }</span>
                    { for examples }
                </Cluster>
                <ExpressionEditor source={AttrValue::from(input.source.clone())} on_input={on_source}
                    environment={environment.clone()} options={bounds.options()} />
                <Card class={class::OUTCOME} gap={Gap::Tight}>
                    <OutcomeView computed={(*outcome).clone()} />
                </Card>
                <Disclosure title={html! { "Variables" }} summary={html! { vars_summary }}
                    id="playground-variables" open={*vars_open} on_toggle={toggle(&vars_open)}
                    actions={html! {
                        <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_add}>
                            { "Add a variable" }
                        </Button>
                    }}>
                    <Stack tag="ul" gap={Gap::Hair} class={sockel::class::PLAIN_LIST}>{ for rows }</Stack>
                </Disclosure>
                <Disclosure title={html! { "Names" }} id="playground-names" open={*names_open}
                    on_toggle={toggle(&names_open)}>
                    <NamesReference environment={environment.clone()} {on_pick} />
                </Disclosure>
                <Disclosure title={html! { "Limits" }} summary={html! { limits_summary }}
                    id="playground-limits" open={*limits_open} on_toggle={toggle(&limits_open)}>
                    <Cluster gap={Gap::Medium} align={Align::Top}>
                        { limit("Fuel (steps)", "limit-fuel", i64::try_from(bounds.fuel).unwrap_or(i64::MAX), 1_000_000, 10, used.map(|d| d.steps),
                            |b, n| b.fuel = u64::try_from(n).unwrap_or(1)) }
                        { limit("Memory (words)", "limit-memory", i64::from(bounds.memory), 1 << 20, 64, used.map(|d| d.memory),
                            |b, n| b.memory = u32::try_from(n).unwrap_or(1)) }
                        { limit("Depth", "limit-depth", i64::try_from(bounds.depth).unwrap_or(i64::MAX), 256, 1, None,
                            |b, n| b.depth = usize::try_from(n).unwrap_or(1)) }
                        { limit("Length (bytes)", "limit-len", i64::try_from(bounds.len).unwrap_or(i64::MAX), 1 << 20, 1024, u64::try_from(input.source.len()).ok(),
                            |b, n| b.len = usize::try_from(n).unwrap_or(1)) }
                    </Cluster>
                </Disclosure>
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

/// One variable: its name, its type written out, its value,
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

    html! {
        <li>
            <Stack gap={Gap::Hair}>
                <Cluster gap={Gap::Tight}>
                    <TextInput class={classes!(class::CODE, sockel::class::GROW, sockel::class::BASIS_SM)}
                        label="Name" size={InputSize::Small} value={row.name.clone()} invalid={name_bad}
                        on_input={set(|r, t| r.name = t)} />
                    <TextInput class={classes!(class::CODE, sockel::class::GROW, sockel::class::BASIS_SM)}
                        label="Type" size={InputSize::Small} placeholder="Int" value={row.ty.clone()} invalid={ty_bad}
                        on_input={set(|r, t| r.ty = t)} />
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
    computed: Rc<Computed>,
}

#[function_component]
fn OutcomeView(props: &OutcomeProps) -> Html {
    match &props.computed.outcome {
        Ok(done) => {
            let ran = Rc::new(Ran::value(done.value.clone(), done.ty.clone()));

            html! { <ResultView {ran} class={class::VALUE} /> }
        }
        Err(Failed::Compile(diagnostics)) => html! {
            <ErrorView diagnostics={diagnostics.clone()} source={props.computed.source.clone()} />
        },
        Err(Failed::Run(ran)) => html! { <ResultView ran={ran.clone()} class={class::VALUE} /> },
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
            assert!(outcome.is_ok(), "{}: {:?}", example.title, outcome);
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
}
