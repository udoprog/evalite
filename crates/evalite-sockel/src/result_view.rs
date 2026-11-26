use evalite::{Labels, Outcome, Program, Type, Value};
use phrasebook::Phrases;
use phrasebook::yew023::use_phrases_with;
use sockel::{ErrorBox, PropertyList, PropertyRow};
use std::rc::Rc;
use yew::prelude::*;

use crate::L10N;

/// How many characters wide a value's room is, unless the props say.
const RESERVE: u32 = 12;

/// One line of a run error: its message, and whether it is the error itself
/// rather than a note on it. The first line of an error is primary, and the
/// ones after it explain it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cause {
    message: String,
    primary: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Value { value: Value, ty: Type },
    Failed(Vec<Cause>),
}

/// What a run gave, for [`ResultView`] to show: a value and the type its
/// program said it has, or the run's error. Opaque, so how it is kept can
/// change.
#[derive(Clone, Debug, PartialEq)]
pub struct Ran {
    kind: Kind,
}

impl Ran {
    /// The value of an outcome of `program`.
    pub fn of(outcome: &Outcome, program: &Program) -> Self {
        Self::value(outcome.value().clone(), program.result_type())
    }

    /// `value`, which has type `ty`, for apps that have no [`Outcome`].
    pub fn value(value: Value, ty: Type) -> Self {
        Self {
            kind: Kind::Value { value, ty },
        }
    }

    /// A failed run: an overflow, an index out of range, too many steps, too
    /// much memory, from its [`labels`](evalite::RunError::labels).
    pub fn failed(labels: Labels<'_>) -> Self {
        let causes = labels
            .map(|label| Cause {
                message: label.message().to_string(),
                primary: label.is_primary(),
            })
            .collect();

        Self {
            kind: Kind::Failed(causes),
        }
    }

    /// A failed run that is only `message`, for what is not a [`RunError`](evalite::RunError).
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            kind: Kind::Failed(vec![Cause {
                message: message.into(),
                primary: true,
            }]),
        }
    }

    /// The messages of the errors of a failed run, without their notes; none
    /// for a value.
    pub fn errors(&self) -> Vec<&str> {
        match &self.kind {
            Kind::Failed(causes) => causes
                .iter()
                .filter(|c| c.primary)
                .map(|c| c.message.as_str())
                .collect(),
            Kind::Value { .. } => Vec::new(),
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct ResultViewProps {
    /// What the run gave.
    pub ran: Rc<Ran>,
    /// How many characters wide a value's room is, which a shorter value
    /// does not shrink, so the page does not move as results change.
    #[prop_or(RESERVE)]
    pub reserve: u32,
    /// The list's accessible name, else the catalog's.
    #[prop_or_default]
    pub label: Option<AttrValue>,
    #[prop_or_default]
    pub class: Classes,
}

/// A run's result: its value written as evalite writes it and read by its
/// type (an `Int`, a `Bool`, text, an option, a list or a record, each part
/// of one), with the type beside it, or the run's error with its causes.
///
/// A value keeps its room (`reserve`) and its numerals their width, so the
/// page does not jitter as results change.
#[function_component]
pub fn ResultView(props: &ResultViewProps) -> Html {
    let phrases = use_phrases_with(&L10N);

    match &props.ran.kind {
        Kind::Value { value, ty } => {
            let label = props
                .label
                .clone()
                .unwrap_or_else(|| AttrValue::from(phrases.text("evalite-result-label")));
            let style = format!("min-width: {}ch", props.reserve);

            html! {
                <PropertyList {label} class={props.class.clone()}>
                    <PropertyRow label={phrases.text("evalite-result-value").to_string()}>
                        <span {style}>{ show(value, ty, &phrases) }</span>
                    </PropertyRow>
                    <PropertyRow label={phrases.text("evalite-result-type").to_string()}>
                        <code>{ ty.to_string() }</code>
                    </PropertyRow>
                </PropertyList>
            }
        }
        Kind::Failed(causes) => {
            // Each primary line starts an error, which the lines after it
            // explain.
            let mut groups: Vec<(&Cause, Vec<AttrValue>)> = Vec::new();

            for cause in causes {
                match groups.last_mut() {
                    Some((_, notes)) if !cause.primary => {
                        notes.push(AttrValue::from(cause.message.clone()));
                    }
                    _ => groups.push((cause, Vec::new())),
                }
            }

            let title = AttrValue::from(phrases.text("evalite-result-error").to_string());

            let boxes = groups.into_iter().map(|(cause, causes)| {
                html! {
                    <ErrorBox title={title.clone()} {causes}>{ cause.message.clone() }</ErrorBox>
                }
            });

            html! { <div class={props.class.clone()}>{ for boxes }</div> }
        }
    }
}

/// `value`, read as `ty` says it is. A value that is not what its type says
/// is written as it is.
fn show(value: &Value, ty: &Type, phrases: &Phrases) -> Html {
    if let Some(entry) = ty.as_list() {
        return match value.as_list() {
            Some([]) => word("evalite-result-empty-list", phrases),
            Some(entries) => html! {
                <ol class={sockel::class::PLAIN_LIST}>
                    { for entries.iter().map(|v| html! { <li>{ show(v, entry, phrases) }</li> }) }
                </ol>
            },
            None => leaf(value),
        };
    }

    if let Some(inner) = ty.as_option() {
        return match value.as_option() {
            Some(None) => word("evalite-result-none", phrases),
            Some(Some(v)) => html! {
                <>
                    { word("evalite-result-some", phrases) }{ " " }{ show(v, inner, phrases) }
                </>
            },
            None => leaf(value),
        };
    }

    if let Some(types) = ty.as_record() {
        let rows = types.filter_map(|(name, ty)| {
            let field = value.field(name)?;
            Some(html! {
                <PropertyRow label={name.to_string()}>{ show(field, ty, phrases) }</PropertyRow>
            })
        });

        return html! { <PropertyList>{ for rows }</PropertyList> };
    }

    leaf(value)
}

/// An `Int`, a `Bool` or text, as evalite writes it.
fn leaf(value: &Value) -> Html {
    html! { <code class={sockel::class::NUMERIC}>{ value.to_string() }</code> }
}

fn word(id: &str, phrases: &Phrases) -> Html {
    html! { <em>{ phrases.text(id).to_string() }</em> }
}
