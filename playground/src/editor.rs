//! The expression editor: the code editor with evalite's highlighting, the
//! error's span and evalite's completions for the word at the cursor (as
//! you type a name or after a `.`, or on Ctrl+Space).

use std::ops::Range;
use std::rc::Rc;

use evalite::highlight::{Class, highlight};
use sockel::{
    CodeEditor, CodeSpan, CodeToken, Completion, CompletionQuery, Completions, Diagnostic, Severity,
};
use yew::prelude::*;

use crate::complete;
use crate::eval::{self, Binding};

#[derive(Properties, PartialEq)]
pub struct EditorProps {
    pub source: AttrValue,
    pub on_change: Callback<String>,
    /// The characters the error points at, and its message.
    pub primary: Option<(Range<usize>, AttrValue)>,
    /// The variables the completions offer.
    pub bindings: Rc<Vec<Binding>>,
}

#[function_component]
pub fn Editor(props: &EditorProps) -> Html {
    let spans = use_memo(props.source.clone(), |source| spans_of(source));

    let diagnostics: Rc<[Diagnostic]> = props
        .primary
        .iter()
        .map(|(span, message)| {
            let range = byte_of(&props.source, span.start)..byte_of(&props.source, span.end);
            Diagnostic::new(range, Severity::Error, message.clone())
        })
        .collect();

    let complete = {
        let bindings = props.bindings.clone();
        Callback::from(move |ask: CompletionQuery| completions(&ask, &bindings).map(Rc::new))
    };

    html! {
        <CodeEditor label="Expression" value={props.source.clone()}
            on_input={props.on_change.clone()} spans={(*spans).clone()} {diagnostics}
            complete={Some(complete)} />
    }
}

fn spans_of(source: &str) -> Rc<[CodeSpan]> {
    highlight(source)
        .filter_map(|token| {
            let token_class = match token.class {
                Class::Keyword => CodeToken::Keyword,
                Class::Ident => CodeToken::Name,
                Class::Member => CodeToken::Member,
                Class::Number => CodeToken::Number,
                Class::String => CodeToken::String,
                Class::Punctuation | Class::Operator => CodeToken::Punctuation,
                Class::Error => CodeToken::Error,
                _ => return None,
            };

            Some(CodeSpan::new(token.span, token_class))
        })
        .collect()
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

fn completions(ask: &CompletionQuery, bindings: &[Binding]) -> Option<Completions> {
    if !ask.forced && !wants_offers(&ask.text, ask.cursor) {
        return None;
    }

    let cursor = ask.text.get(..ask.cursor)?.chars().count();
    let env = eval::environment(bindings).ok()?;
    let at = complete::offers(&ask.text, cursor, &env)?;

    let items = at
        .offers
        .iter()
        .map(|offer| {
            let (text, from, to) = complete::accept(&ask.text, &at, offer);
            let select = byte_of(&text, from)..byte_of(&text, to);
            let detail = match (&offer.signature, &offer.ty) {
                (Some(signature), _) => signature.clone(),
                (None, Some(ty)) => format!("{} · {ty}", offer.kind),
                (None, None) => offer.kind.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_skip_whitespace() {
        let spans = spans_of("a + 1");
        let classes = spans.iter().map(|s| s.token).collect::<Vec<_>>();
        assert_eq!(
            classes,
            [CodeToken::Name, CodeToken::Punctuation, CodeToken::Number]
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
}
