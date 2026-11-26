use std::rc::Rc;

use evalite::Environment;
use phrasebook::yew023::use_phrases_with;
use sockel::{Button, ButtonSize, ButtonVariant, Gap, Stack, TextInput};
use yew::prelude::*;

use crate::{ExpressionPreview, L10N, TypeBadge};

#[derive(Properties)]
pub struct NamesReferenceProps {
    /// The names to list. Compared by pointer, so keep the same `Rc` while
    /// the names do not change, as for
    /// [`ExpressionEditor`](crate::ExpressionEditor).
    pub environment: Rc<Environment>,
    /// Called with a name when it is clicked, for the app to put into its
    /// [`ExpressionEditor`](crate::ExpressionEditor)'s source.
    pub on_pick: Callback<String>,
    /// The list's accessible name, else the catalog's.
    #[prop_or_default]
    pub label: Option<AttrValue>,
    #[prop_or_default]
    pub class: Classes,
}

impl PartialEq for NamesReferenceProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.environment, &other.environment)
            && self.on_pick == other.on_pick
            && self.label == other.label
            && self.class == other.class
    }
}

/// A filterable list of the values and functions an
/// [`Environment`] declares, each with its type or signatures. Clicking
/// one reports its name through `on_pick`.
#[function_component]
pub fn NamesReference(props: &NamesReferenceProps) -> Html {
    let phrases = use_phrases_with(&L10N);
    let filter = use_state(String::new);

    let on_input = {
        let filter = filter.clone();
        Callback::from(move |text: String| filter.set(text))
    };

    let needle = filter.trim().to_lowercase();
    let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);

    let pick = |name: &str| {
        let on_pick = props.on_pick.clone();
        let name = name.to_owned();
        Callback::from(move |_: MouseEvent| on_pick.emit(name.clone()))
    };

    let values = props.environment.values().filter(|(name, _)| matches(name)).map(|(name, ty)| {
        html! {
            <li key={name}>
                <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={pick(name)}>
                    <code>{ name }</code>
                    { " " }
                    <TypeBadge ty={ty.clone()} />
                </Button>
            </li>
        }
    });

    let functions = props.environment.functions().filter(|(name, _)| matches(name)).map(|(name, signatures)| {
        let signatures = signatures.into_iter().map(|signature| {
            html! { <div><ExpressionPreview source={signature} /></div> }
        });

        html! {
            <li key={name}>
                <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={pick(name)}>
                    <span>{ for signatures }</span>
                </Button>
            </li>
        }
    });

    let label = props
        .label
        .clone()
        .unwrap_or_else(|| AttrValue::from(phrases.text("evalite-names-label")));

    html! {
        <Stack gap={Gap::Tight} class={props.class.clone()}>
            <TextInput value={AttrValue::from((*filter).clone())} {on_input}
                label={Some(AttrValue::from(phrases.text("evalite-names-filter")))}
                placeholder={Some(AttrValue::from(phrases.text("evalite-names-filter")))} />
            <ul class={sockel::class::PLAIN_LIST} aria-label={label}>
                { for values }
                { for functions }
            </ul>
        </Stack>
    }
}
