//! The evalite playground: the project's page, with an editor that runs
//! evalite in the browser.

mod bench;
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
/// The square mark, inlined so its tile follows the page's theme.
const LOGO: &str = include_str!("../../assets/logo.svg");

use std::rc::Rc;

use evalite::{Environment, Program};
use evalite_sockel::{
    Compiled, ErrorView, ExpressionEditor, Mark, NamesReference, Ran, ResultView, Variable,
    VariablesEditor,
};
use gloo_timers::callback::Timeout;
use yew::prelude::*;

use sockel::{
    Align, Button, ButtonSize, ButtonVariant, Card, Cluster, Disclosure, Field, Gap, OverlayHost,
    SectionHead, Stack, Stepper, TabItem, Table, TableCell, TableRow, Tabs,
};

use crate::bench::Stage;
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

/// The page's state: what the outcome is computed from.
#[derive(Clone, PartialEq)]
struct Input {
    source: String,
    rows: Rc<Vec<Variable>>,
    bounds: Bounds,
}

impl Input {
    fn example(example: &Example) -> Self {
        let rows = example
            .vars
            .iter()
            .zip(0u32..)
            .map(|(&(name, ty, value), id)| Variable::new(id, name, ty, value))
            .collect::<Vec<_>>();

        let mut bounds = Bounds::default();

        if let Some(fuel) = example.fuel {
            bounds.fuel = fuel;
        }

        Self {
            source: example.source.into(),
            rows: Rc::new(rows),
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
                rows: Rc::new(Vec::new()),
                bounds: Bounds::default(),
            })
    }

    fn shared(shared: Shared) -> Self {
        let defaults = Bounds::default();

        Self {
            source: shared.source,
            rows: Rc::new(
                shared
                    .vars
                    .iter()
                    .zip(0u32..)
                    .map(|((name, ty, value), id)| Variable::new(id, name, ty, value))
                    .collect(),
            ),
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

    fn bindings(&self) -> Vec<Binding> {
        evalite_sockel::bindings(&self.rows)
            .into_iter()
            .map(|b| Binding {
                name: b.name,
                ty: b.ty,
                value: b.value,
            })
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
                <Benchmarks />
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
                <Cluster tag="h1" class={class::DISPLAY} gap={Gap::Tight} nowrap=true>
                    <span class={class::LOGO} aria-hidden="true">{ Html::from_html_unchecked(LOGO.into()) }</span>
                    { "evalite" }
                </Cluster>
                <p class={class::LEAD}>
                    { "Let your users write formulas like " }
                    <code class={class::CODE}>{ "wits.min(dexterity) + athletics" }</code>
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

/// The engine's colour, as a modifier on a bar or a legend swatch.
fn engine_class(engine: &str) -> Option<&'static str> {
    match engine {
        "evalite" => Some(class::EVALITE),
        "evalexpr" => Some(class::EVALEXPR),
        "fasteval" => Some(class::FASTEVAL),
        _ => None,
    }
}

/// How evalite compares with its peers, from `crates/evalite-compare`'s
/// snapshot: compile and evaluate times as separate views.
#[function_component]
fn Benchmarks() -> Html {
    let results = use_memo((), |_| bench::results());
    let stage = use_state(|| Stage::Evaluate);

    let Some(results) = results.as_ref() else {
        return Html::default();
    };

    let tabs = vec![
        TabItem::new("evaluate", "Evaluate"),
        TabItem::new("compile", "Compile"),
    ];

    let on_stage = {
        let stage = stage.clone();
        Callback::from(move |value: AttrValue| {
            stage.set(if value == "compile" {
                Stage::Compile
            } else {
                Stage::Evaluate
            });
        })
    };

    let (value, verb) = match *stage {
        Stage::Compile => ("compile", "to compile"),
        Stage::Evaluate => ("evaluate", "to evaluate"),
    };

    let legend = results.engines.iter().map(|engine| {
        html! {
            <Cluster gap={Gap::Tight} nowrap=true>
                <span class={classes!(class::SWATCH, engine_class(&engine.name))} aria-hidden="true"></span>
                <span>{ &engine.name }</span>
                <span class={class::MUTED}>{ &engine.version }</span>
            </Cluster>
        }
    });

    let rows = results.expressions.iter().map(|expression| {
        let stage = *stage;
        let longest = expression
            .results
            .iter()
            .map(|m| stage.of(m))
            .fold(0.0, f64::max);

        let bars = expression.results.iter().map(|measured| {
            let ns = stage.of(measured);
            let share = if longest > 0.0 { ns / longest * 100.0 } else { 0.0 };
            let time = bench::duration(ns);

            html! {
                <div class={class::BAR_ROW} title={format!("{}: {time} {verb}\n{}", measured.engine, measured.source)}>
                    <span>{ &measured.engine }</span>
                    <span class={class::BAR_TRACK}>
                        <span class={classes!(class::BAR, engine_class(&measured.engine))}
                            style={format!("inline-size: {share:.1}%")}></span>
                    </span>
                    <span class={class::BAR_VALUE}>{ time }</span>
                </div>
            }
        });

        html! {
            <TableRow value={expression.name.clone()}>
                <TableCell title=true wrap=true scope="row">
                    <Stack gap={Gap::Hair}>
                        <span>{ &expression.name }</span>
                        <span class={class::MUTED}>{ &expression.description }</span>
                    </Stack>
                </TableCell>
                <TableCell full=true>
                    <div class={class::BARS}>{ for bars }</div>
                </TableCell>
            </TableRow>
        }
    });

    let head = html! {
        <>
            <TableCell head=true>{ "Expression" }</TableCell>
            <TableCell head=true>{ format!("Time {verb}, shorter is faster") }</TableCell>
        </>
    };

    html! {
        <section aria-label="Benchmarks">
            <Stack gap={Gap::Loose}>
                <SectionHead title={html! { "How fast it is" }} />
                <p class={class::PROSE}>
                    { "evalite is the slowest of the three to compile, because it type-checks an expression and verifies the program it generates before anything runs. \
                       It is built to compile once and evaluate many times: there it is faster than evalexpr on every expression measured here, \
                       while fasteval, which only has floating point numbers, is faster still on all but the longest formula." }
                </p>
                <Cluster gap={Gap::Loose}>
                    <Tabs {tabs} value={AttrValue::from(value)} on_change={on_stage}
                        label="Stage" segmented=true />
                    <Cluster gap={Gap::Loose}>{ for legend }</Cluster>
                </Cluster>
                <Table {head} columns={2} framed=true cards=true top=true
                    label={format!("Time {verb} each expression, by engine")}>
                    { for rows }
                </Table>
                <p class={class::MUTED}>
                    { format!("Medians measured {} on {}, {}. Bars are scaled within each expression; differences under about 15% are noise. ",
                        bench::date(&results.measured_at), results.machine.cpu, results.machine.os) }
                    <a href={bench::README}>{ "How it is measured" }</a>
                </p>
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

    let on_rows = {
        let input = input.clone();
        Callback::from(move |rows: Rc<Vec<Variable>>| {
            let mut next = (*input).clone();
            next.rows = rows;
            input.set(next);
        })
    };

    let program = use_state(|| None::<Rc<Program>>);
    let on_compile = {
        let program = program.clone();
        Callback::from(move |compiled: Compiled| program.set(compiled.ok()))
    };

    let toggle = |open: &UseStateHandle<bool>| {
        let open = open.clone();
        Callback::from(move |next: bool| open.set(next))
    };

    let environment = use_memo(input.bindings(), |bindings| {
        eval::environment(bindings).unwrap_or_else(|_| Environment::new())
    });

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

    let editor_marks: Rc<[Mark]> = match &outcome.outcome {
        Err(Failed::Run(_, marks)) => marks.clone(),
        _ => Rc::from([]),
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
                    environment={environment.clone()} options={bounds.options()} marks={editor_marks}
                    {on_compile} />
                <Card class={class::OUTCOME} gap={Gap::Tight}>
                    <OutcomeView computed={(*outcome).clone()} />
                </Card>
                <VariablesEditor variables={input.rows.clone()} on_change={on_rows}
                    program={(*program).clone()} id="playground-variables"
                    open={*vars_open} on_toggle={toggle(&vars_open)} input_class={class::CODE} />
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
        Err(Failed::Run(ran, _)) => html! { <ResultView ran={ran.clone()} class={class::VALUE} /> },
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

            for row in input.rows.iter() {
                assert!(
                    row.binding().is_ok(),
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
