use std::collections::BTreeSet;
use std::rc::Rc;

use evalite::{
    DEFAULT_FUEL, DEFAULT_MAX_LEN, DEFAULT_MEMORY, Environment, Globals, Limits, Machine, Options,
    Program, Type, Value,
};
use phrasebook::yew023::use_phrases_with;
use sockel::{
    Badge, Button, ButtonSize, ButtonVariant, Cluster, Disclosure, Gap, InputSize, Stack,
    TextInput, Tone,
};
use yew::prelude::*;

use crate::L10N;
use crate::editor::ExpressionEditor;

/// The column of a [`Variable`] that is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    /// The variable has no name.
    Name,
    /// The type does not parse.
    Type,
    /// The value does not compile as a constant expression of the type, or
    /// running it fails.
    Value,
}

/// What is wrong with a [`Variable`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableError {
    column: Column,
    message: Option<String>,
}

impl VariableError {
    /// The column the error is in.
    pub fn column(&self) -> Column {
        self.column
    }

    /// evalite's message, or `None` for a missing name, which has no message
    /// of evalite's.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// A variable ready to be declared and bound: its name, its type and its
/// value of that type.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The name, without surrounding space.
    pub name: String,
    /// The parsed type.
    pub ty: Type,
    /// The parsed value.
    pub value: Value,
}

/// One row of the [`VariablesEditor`], as typed: the type in the syntax
/// evalite prints (`Int`, `Option<Int>`, `{boxes: Int}`) and the value as an
/// expression of that type with no names in scope (`3`, `Some(2)`,
/// `{boxes: None}`, `[1, 2 * 3]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    /// Tells the row apart from the others while the list changes; keep it
    /// unique in a list.
    pub id: u32,
    /// The name, as typed.
    pub name: String,
    /// The type, as typed.
    pub ty: String,
    /// The value, as typed.
    pub value: String,
}

impl Variable {
    /// A row.
    pub fn new(id: u32, name: &str, ty: &str, value: &str) -> Self {
        Self {
            id,
            name: name.into(),
            ty: ty.into(),
            value: value.into(),
        }
    }

    /// The binding the row stands for, or what is wrong with it.
    ///
    /// The value is compiled with [`evalite::compile_as`] against an empty
    /// environment, with the declared type as the type it must have, and run
    /// once. So `None` and `[]` take their types from the declared type, and
    /// no row reads another.
    ///
    /// # Examples
    ///
    /// ```
    /// use evalite_sockel::{Column, Variable};
    ///
    /// let ok = Variable::new(0, "wits", "Int", "3").binding()?;
    /// assert_eq!(ok.name, "wits");
    ///
    /// let ok = Variable::new(0, "pool", "{boxes: Option<Int>}", "{boxes: None}").binding()?;
    /// assert_eq!(ok.value.to_string(), "{boxes: None}");
    ///
    /// let bad = Variable::new(0, "wits", "Int", "true").binding();
    /// assert_eq!(bad.map_err(|e| e.column()), Err(Column::Value));
    /// # Ok::<_, evalite_sockel::VariableError>(())
    /// ```
    pub fn binding(&self) -> Result<Binding, VariableError> {
        let name = self.name.trim();

        if name.is_empty() {
            return Err(VariableError {
                column: Column::Name,
                message: None,
            });
        }

        let ty = Type::parse(&self.ty).map_err(|e| VariableError {
            column: Column::Type,
            message: Some(e.to_string()),
        })?;

        let value = constant(&self.value, &ty).map_err(|message| VariableError {
            column: Column::Value,
            message: Some(message),
        })?;

        Ok(Binding {
            name: name.to_owned(),
            ty,
            value,
        })
    }
}

/// What a constant of the longest source the parser allows can need: a word
/// of memory and four steps per byte of source (a list entry takes at least
/// two bytes and about a word), on top of the defaults. Bounded, as the
/// source is.
fn limits() -> Limits {
    let len = DEFAULT_MAX_LEN;
    Limits::new()
        .memory(DEFAULT_MEMORY.saturating_add(u32::try_from(len).unwrap_or(u32::MAX)))
        .fuel(DEFAULT_FUEL.saturating_add((len as u64).saturating_mul(4)))
}

/// `source` compiled as a constant of type `ty` and run once, or the first
/// error's message.
fn constant(source: &str, ty: &Type) -> Result<Value, String> {
    let env = Environment::new();

    let program = evalite::compile_as(source, &env, &Options::new(), ty).map_err(|errors| {
        errors
            .iter()
            .next()
            .map_or_else(|| errors.to_string(), ToString::to_string)
    })?;

    let globals = Globals::new(&env);
    let mut machine = Machine::new(limits());
    let outcome = machine
        .load(&program, &globals)
        .and_then(|run| run.run())
        .map_err(|error| error.to_string())?;

    Ok(outcome.value().clone())
}

/// The bindings of every row that has one, in order; the rows with an error
/// are skipped.
pub fn bindings(variables: &[Variable]) -> Vec<Binding> {
    variables.iter().filter_map(|v| v.binding().ok()).collect()
}

#[derive(Properties)]
pub struct VariablesEditorProps {
    /// The rows.
    pub variables: Rc<Vec<Variable>>,
    /// Called with the new rows on every edit, addition and removal.
    pub on_change: Callback<Rc<Vec<Variable>>>,
    /// The compiled expression, to mark which variables it reads. Without
    /// it no row is marked.
    #[prop_or_default]
    pub program: Option<Rc<Program>>,
    /// Whether the section shows its rows.
    pub open: bool,
    /// Called with the state a press on the section's head asks for.
    pub on_toggle: Callback<bool>,
    /// The body's id.
    #[prop_or_default]
    pub id: Option<AttrValue>,
    /// Classes for the three text inputs, such as a monospace face.
    #[prop_or_default]
    pub input_class: Classes,
    #[prop_or_default]
    pub class: Classes,
}

impl PartialEq for VariablesEditorProps {
    fn eq(&self, other: &Self) -> bool {
        self.variables == other.variables
            && self.on_change == other.on_change
            && match (&self.program, &other.program) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.open == other.open
            && self.on_toggle == other.on_toggle
            && self.id == other.id
            && self.input_class == other.input_class
            && self.class == other.class
    }
}

/// Rows of name, type and value for trying an expression against sample
/// inputs, in a section that opens and shuts. Each row says whether the
/// expression reads it, and has its error under it when the type does not
/// parse or the value is not a constant of that type. The value is an
/// [`ExpressionEditor`] of one line, compiled against the declared type, so
/// it is highlighted, its errors are underlined where they are, and it
/// completes `Some(_)`, `None` and record field names.
///
/// It is controlled: it shows `variables` and reports every change through
/// `on_change`. Take the bindings of the rows with [`bindings`].
#[function_component]
pub fn VariablesEditor(props: &VariablesEditorProps) -> Html {
    let phrases = use_phrases_with(&L10N);
    // Values are constants: no names are in scope.
    let empty = use_memo((), |_| Environment::new());

    let used: Option<BTreeSet<String>> = props
        .program
        .as_ref()
        .map(|p| p.globals().map(|(name, _)| name.to_owned()).collect());

    let is_used = |v: &Variable| used.as_ref().map(|u| u.contains(v.name.trim()));

    let summary = if props.variables.is_empty() {
        phrases.text("evalite-variables-none").to_string()
    } else if let Some(used) = &used {
        let count = props
            .variables
            .iter()
            .filter(|v| used.contains(v.name.trim()))
            .count();
        let (used, total) = (count.to_string(), props.variables.len().to_string());
        phrases.msg(
            "evalite-variables-summary",
            &phrasebook::args!(used = used.as_str(), count = total.as_str()),
        )
    } else {
        props
            .variables
            .iter()
            .map(|v| v.name.trim())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let on_add = {
        let (variables, on_change) = (props.variables.clone(), props.on_change.clone());
        let open = props.open;
        let on_toggle = props.on_toggle.clone();
        Callback::from(move |_: MouseEvent| {
            let mut next = (*variables).clone();
            let id = next
                .iter()
                .map(|v| v.id)
                .max()
                .map_or(0, |id| id.saturating_add(1));
            next.push(Variable::new(id, &format!("v{id}"), "Int", "0"));
            on_change.emit(Rc::new(next));

            if !open {
                on_toggle.emit(true);
            }
        })
    };

    let rows = props.variables.iter().map(|variable| {
        let id = variable.id;

        let edit = |f: fn(&mut Variable, String)| {
            let (variables, on_change) = (props.variables.clone(), props.on_change.clone());
            Callback::from(move |text: String| {
                let mut next = (*variables).clone();

                if let Some(v) = next.iter_mut().find(|v| v.id == id) {
                    f(v, text);
                }

                on_change.emit(Rc::new(next));
            })
        };

        let on_remove = {
            let (variables, on_change) = (props.variables.clone(), props.on_change.clone());
            Callback::from(move |_: MouseEvent| {
                let mut next = (*variables).clone();
                next.retain(|v| v.id != id);
                on_change.emit(Rc::new(next));
            })
        };

        let declared = Type::parse(&variable.ty).ok();
        let error = variable.binding().err();
        let bad = |column| error.as_ref().is_some_and(|e| e.column() == column);
        let message = error
            .as_ref()
            .map(|e| match e.message() {
                Some(message) => message.to_owned(),
                None => phrases.text("evalite-variables-no-name").to_string(),
            })
            .unwrap_or_default();

        let mark = is_used(variable).map(|used| {
            let (tone, text) = if used {
                (Tone::Success, "evalite-variables-used")
            } else {
                (Tone::Neutral, "evalite-variables-unused")
            };

            html! { <Badge {tone}>{ phrases.text(text).to_string() }</Badge> }
        });

        let input_class = classes!(
            props.input_class.clone(),
            sockel::class::GROW,
            sockel::class::BASIS_SM
        );
        // The value is the longest of the three, so it starts wider.
        let value_class = classes!(
            props.input_class.clone(),
            sockel::class::GROW,
            sockel::class::BASIS_LG
        );
        let value_label = AttrValue::from(phrases.text("evalite-variables-value").to_string());

        html! {
            <li key={id}>
                <Stack gap={Gap::Hair}>
                    <Cluster gap={Gap::Tight}>
                        <TextInput class={input_class.clone()} size={InputSize::Small}
                            label={phrases.text("evalite-variables-name").to_string()}
                            value={variable.name.clone()} invalid={bad(Column::Name)}
                            on_input={edit(|v, t| v.name = t)} />
                        <TextInput class={input_class} size={InputSize::Small}
                            label={phrases.text("evalite-variables-type").to_string()}
                            placeholder="Int" value={variable.ty.clone()} invalid={bad(Column::Type)}
                            on_input={edit(|v, t| v.ty = t)} />
                        <ExpressionEditor class={value_class} label={Some(value_label)}
                            source={AttrValue::from(variable.value.clone())}
                            on_input={edit(|v, t| v.value = t)} environment={empty.clone()}
                            check={declared.is_some()} expected={declared} rows={1} />
                        { mark }
                        <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_remove}>
                            { phrases.text("evalite-variables-remove").to_string() }
                        </Button>
                    </Cluster>
                    <p class={sockel::field_error(true)} role="status">{ message }</p>
                </Stack>
            </li>
        }
    });

    html! {
        <Disclosure title={html! { phrases.text("evalite-variables-title").to_string() }}
            summary={html! { summary }} id={props.id.clone()} open={props.open}
            on_toggle={props.on_toggle.clone()} class={props.class.clone()}
            actions={html! {
                <Button variant={ButtonVariant::Quiet} size={ButtonSize::Small} onclick={on_add}>
                    { phrases.text("evalite-variables-add").to_string() }
                </Button>
            }}>
            <Stack tag="ul" gap={Gap::Hair} class={sockel::class::PLAIN_LIST}>{ for rows }</Stack>
        </Disclosure>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_values_bind() {
        let list = format!("[{}]", vec!["1"; 1100].join(", "));
        assert!(Variable::new(0, "xs", "List<Int>", &list).binding().is_ok());

        let big = format!("\"{}\"", "a".repeat(60000));
        assert!(Variable::new(0, "s", "String", &big).binding().is_ok());
    }

    #[test]
    fn value_past_the_source_limit_is_refused() {
        let big = format!("\"{}\"", "a".repeat(DEFAULT_MAX_LEN + 1));
        let err = Variable::new(0, "s", "String", &big).binding().unwrap_err();
        assert_eq!(err.column(), Column::Value);
    }
}
