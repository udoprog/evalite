use std::cell::RefCell;
use std::rc::Rc;

use evalite::highlight::highlight;
use evalite::{CompletionKind, Diagnostics, Environment, Options, Program};
use gloo_timers::callback::Timeout;
use phrasebook::Phrases;
use phrasebook::yew023::use_phrases_with;
use sockel::{
    CodeEditor, CodeSpan, CodeToken, Completion, CompletionQuery, Completions, Diagnostic, Severity,
};
use yew::prelude::*;

use crate::L10N;
use crate::complete;
use crate::highlight::token_class;

/// What compiling the editor's source gives.
pub type Compiled = Result<Rc<Program>, Rc<Diagnostics>>;

#[derive(Properties)]
pub struct ExpressionEditorProps {
    /// The expression.
    pub source: AttrValue,
    /// Called with the new text on every edit.
    pub on_input: Callback<String>,
    /// The names the expression may use. Compared by pointer, so keep the
    /// same `Rc` while the names do not change.
    pub environment: Rc<Environment>,
    #[prop_or_default]
    pub options: Options,
    /// Called with each compile's result, once the source is still for
    /// `debounce_ms`.
    #[prop_or_default]
    pub on_compile: Callback<Compiled>,
    /// How long after an edit the source is compiled; 0 compiles on every
    /// edit. Highlighting never waits.
    #[prop_or(150)]
    pub debounce_ms: u32,
    /// The editor's accessible name, else the catalog's.
    #[prop_or_default]
    pub label: Option<AttrValue>,
    #[prop_or(4)]
    pub rows: u32,
    #[prop_or_default]
    pub readonly: bool,
    #[prop_or_default]
    pub id: Option<AttrValue>,
    #[prop_or_default]
    pub describedby: Option<AttrValue>,
    #[prop_or_default]
    pub class: Classes,
}

impl PartialEq for ExpressionEditorProps {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.on_input == other.on_input
            && Rc::ptr_eq(&self.environment, &other.environment)
            && self.options == other.options
            && self.on_compile == other.on_compile
            && self.debounce_ms == other.debounce_ms
            && self.label == other.label
            && self.rows == other.rows
            && self.readonly == other.readonly
            && self.id == other.id
            && self.describedby == other.describedby
            && self.class == other.class
    }
}

/// What a compile depends on.
#[derive(Clone)]
struct Input {
    source: AttrValue,
    environment: Rc<Environment>,
    options: Options,
}

impl PartialEq for Input {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Rc::ptr_eq(&self.environment, &other.environment)
            && self.options == other.options
    }
}

/// sockel's code editor for an evalite expression: evalite's highlighting,
/// every diagnostic underlined and marked in the gutter, and completions for
/// the name at the cursor (as you type one, after a `.`, or on Ctrl+Space).
///
/// It is controlled: it shows `source` and reports edits through
/// `on_input`. It compiles the source itself and reports the result through
/// `on_compile`; show the diagnostics in full with
/// [`ErrorView`](crate::ErrorView).
#[function_component]
pub fn ExpressionEditor(props: &ExpressionEditorProps) -> Html {
    let phrases = use_phrases_with(&L10N);
    let spans = use_memo(props.source.clone(), |source| spans_of(source));
    let diagnostics = use_state(|| None::<Rc<Diagnostics>>);

    let on_compile = use_mut_ref(Callback::<Compiled>::noop);
    *on_compile.borrow_mut() = props.on_compile.clone();

    {
        let diagnostics = diagnostics.clone();
        let input = Input {
            source: props.source.clone(),
            environment: props.environment.clone(),
            options: props.options,
        };
        let debounce_ms = props.debounce_ms;

        use_effect_with(input, move |input| {
            let input = input.clone();
            let run = move || compile(&input, &diagnostics, &on_compile);

            let timeout = if debounce_ms == 0 {
                run();
                None
            } else {
                Some(Timeout::new(debounce_ms, run))
            };

            move || drop(timeout)
        });
    }

    let marks: Rc<[Diagnostic]> = match &*diagnostics {
        Some(diagnostics) => marks_of(&props.source, diagnostics),
        None => Rc::from([]),
    };

    let complete = {
        let environment = props.environment.clone();
        let phrases = phrases.clone();
        Callback::from(move |ask: CompletionQuery| {
            completions(&ask, &environment, &phrases).map(Rc::new)
        })
    };

    let label = props
        .label
        .clone()
        .unwrap_or_else(|| AttrValue::from(phrases.text("evalite-editor-label")));

    html! {
        <CodeEditor {label} value={props.source.clone()} on_input={props.on_input.clone()}
            spans={(*spans).clone()} diagnostics={marks} complete={Some(complete)}
            rows={props.rows} readonly={props.readonly} id={props.id.clone()}
            describedby={props.describedby.clone()} class={props.class.clone()} />
    }
}

fn compile(
    input: &Input,
    diagnostics: &UseStateHandle<Option<Rc<Diagnostics>>>,
    on_compile: &Rc<RefCell<Callback<Compiled>>>,
) {
    let compiled = evalite::compile_with(&input.source, &input.environment, &input.options)
        .map(Rc::new)
        .map_err(Rc::new);

    diagnostics.set(compiled.as_ref().err().cloned());
    let on_compile = on_compile.borrow().clone();
    on_compile.emit(compiled);
}

fn spans_of(source: &str) -> Rc<[CodeSpan]> {
    highlight(source)
        .filter_map(|token| {
            let class = token_class(token.class)?;
            Some(CodeSpan::new(token.span, CodeToken::Class(class)))
        })
        .collect()
}

/// Each diagnostic's primary span in `source`, which may have changed since
/// it was compiled, so a span is cut to what is still there.
fn marks_of(source: &str, diagnostics: &Diagnostics) -> Rc<[Diagnostic]> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let range = diagnostic.span().range();
            let start = boundary(source, range.start);
            let end = boundary(source, range.end).max(start);
            Diagnostic::new(start..end, Severity::Error, diagnostic.to_string())
        })
        .collect()
}

/// `at`, or the nearest character boundary of `text` before it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());

    while !text.is_char_boundary(at) {
        at = at.saturating_sub(1);
    }

    at
}

/// The byte offset of character `at` of `text`.
fn byte_of(text: &str, at: usize) -> usize {
    text.char_indices().nth(at).map_or(text.len(), |(i, _)| i)
}

/// Whether typing just now should open the popup: the cursor is after a
/// letter of a name or a `.`.
fn wants_offers(text: &str, cursor: usize) -> bool {
    text.get(..cursor)
        .and_then(|before| before.chars().next_back())
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '.')
}

fn completions(ask: &CompletionQuery, env: &Environment, phrases: &Phrases) -> Option<Completions> {
    if !ask.forced && !wants_offers(&ask.text, ask.cursor) {
        return None;
    }

    let cursor = ask.text.get(..ask.cursor)?.chars().count();
    let at = complete::offers(&ask.text, cursor, env)?;

    let items = at
        .offers
        .iter()
        .map(|offer| {
            let (text, from, to) = complete::accept(&ask.text, &at, offer);
            let select = byte_of(&text, from)..byte_of(&text, to);
            let kind = phrases.text(kind_id(offer.kind));
            let detail = match (&offer.signature, &offer.ty) {
                (Some(signature), _) => signature.clone(),
                (None, Some(ty)) => phrases.msg(
                    "evalite-completion-typed",
                    &phrasebook::args!(kind = &*kind, "type" = ty.as_str()),
                ),
                (None, None) => kind.to_string(),
            };

            Completion {
                label: offer.label.clone().into(),
                detail: Some(detail.into()),
                text,
                select,
            }
        })
        .collect();

    Some(Completions {
        start: byte_of(&ask.text, at.start),
        items,
    })
}

fn kind_id(kind: CompletionKind) -> &'static str {
    match kind {
        CompletionKind::Value => "evalite-completion-value",
        CompletionKind::Field => "evalite-completion-field",
        CompletionKind::Method => "evalite-completion-method",
        CompletionKind::Function => "evalite-completion-function",
        CompletionKind::Literal => "evalite-completion-literal",
        CompletionKind::Operator => "evalite-completion-operator",
        CompletionKind::Unwrap => "evalite-completion-unwrap",
        _ => "evalite-completion-other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_skip_whitespace() {
        let spans = spans_of("a + 1");
        let classes = spans.iter().map(|s| s.token).collect::<Vec<_>>();
        assert_eq!(
            classes,
            [
                CodeToken::Class(sockel::class::CODE_TOKEN_NAME),
                CodeToken::Class(sockel::class::CODE_TOKEN_PUNCTUATION),
                CodeToken::Class(sockel::class::CODE_TOKEN_NUMBER),
            ]
        );
    }

    #[test]
    fn bytes_of_characters() {
        assert_eq!(byte_of("é+1", 1), 2);
        assert_eq!(byte_of("é+1", 9), 4);
    }

    #[test]
    fn offers_after_a_letter_or_dot() {
        assert!(wants_offers("a.", 2));
        assert!(wants_offers("é", 2));
        assert!(!wants_offers("a ", 2));
        assert!(!wants_offers("", 0));
    }

    #[test]
    fn every_diagnostic_is_marked() {
        let env = Environment::with_builtins();
        let errors = evalite::compile("a + b", &env).unwrap_err();
        let marks = marks_of("a + b", &errors);
        let ranges = marks.iter().map(|m| m.range.clone()).collect::<Vec<_>>();
        assert_eq!(ranges, [0..1, 4..5]);
    }

    #[test]
    fn stale_marks_stay_in_the_source() {
        let env = Environment::with_builtins();
        let errors = evalite::compile("é + nothing", &env).unwrap_err();
        let marks = marks_of("é", &errors);
        assert!(marks.iter().all(|m| m.range.end <= 2));
    }
}
