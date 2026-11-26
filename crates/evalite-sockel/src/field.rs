use std::rc::Rc;

use evalite::{Diagnostics, Environment, Options};
use phrasebook::yew023::use_phrases_with;
use sockel::Field;
use yew::prelude::*;

use crate::L10N;
use crate::editor::{Compiled, ExpressionEditor};

#[derive(Properties)]
pub struct ExpressionFieldProps {
    /// The field's label.
    pub label: Html,
    /// The expression.
    pub source: AttrValue,
    /// Called with the new text on every edit.
    pub on_input: Callback<String>,
    /// The names the expression may use. Compared by pointer, so keep the
    /// same `Rc` while the names do not change.
    pub environment: Rc<Environment>,
    #[prop_or_default]
    pub options: Options,
    /// Called with each compile's result: the
    /// [`Program`](evalite::Program), which
    /// [`to_bytes`](evalite::Program::to_bytes) turns into what to store, or
    /// the diagnostics.
    #[prop_or_default]
    pub on_compile: Callback<Compiled>,
    /// A hint under the editor.
    #[prop_or_default]
    pub hint: Option<Html>,
    /// Marks the label with `*`.
    #[prop_or_default]
    pub required: bool,
    /// The editor's id, which the label names.
    pub id: AttrValue,
    #[prop_or_default]
    pub readonly: bool,
    #[prop_or(1)]
    pub rows: u32,
    #[prop_or(150)]
    pub debounce_ms: u32,
    #[prop_or_default]
    pub class: Classes,
}

impl PartialEq for ExpressionFieldProps {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
            && self.source == other.source
            && self.on_input == other.on_input
            && Rc::ptr_eq(&self.environment, &other.environment)
            && self.options == other.options
            && self.on_compile == other.on_compile
            && self.hint == other.hint
            && self.required == other.required
            && self.id == other.id
            && self.readonly == other.readonly
            && self.rows == other.rows
            && self.debounce_ms == other.debounce_ms
            && self.class == other.class
    }
}

/// An [`ExpressionEditor`] in sockel's [`Field`], for a formula in an
/// ordinary form: the label, an optional hint, and the first diagnostic of
/// the last compile in the field's error line, which keeps its room so the
/// form does not move when one appears.
///
/// Like the editor, it is controlled and reports each compile through
/// `on_compile`, which hands over the compiled program to store.
#[function_component]
pub fn ExpressionField(props: &ExpressionFieldProps) -> Html {
    let phrases = use_phrases_with(&L10N);
    let diagnostics = use_state(|| None::<Rc<Diagnostics>>);

    let on_compile = {
        let diagnostics = diagnostics.clone();
        let forward = props.on_compile.clone();

        Callback::from(move |compiled: Compiled| {
            diagnostics.set(compiled.as_ref().err().cloned());
            forward.emit(compiled);
        })
    };

    let error = diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.iter().next())
        .map(|diagnostic| html! { diagnostic.to_string() });

    let describedby = sockel::field_describedby(&props.id, props.hint.is_some(), true);
    let label = AttrValue::from(phrases.text("evalite-field-label").to_string());

    html! {
        <Field id={props.id.clone()} label={props.label.clone()} hint={props.hint.clone()}
            {error} reserve_error={true} required={props.required} class={props.class.clone()}>
            <ExpressionEditor {label} source={props.source.clone()} on_input={props.on_input.clone()}
                environment={props.environment.clone()} options={props.options}
                on_compile={on_compile} debounce_ms={props.debounce_ms}
                rows={props.rows} readonly={props.readonly}
                id={Some(props.id.clone())} {describedby} />
        </Field>
    }
}
