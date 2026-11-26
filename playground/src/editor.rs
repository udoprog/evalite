//! The expression editor: the code editor with evalite's highlighting, the
//! error's span and evalite's completions for the word at the cursor (as
//! you type a name or after a `.`, or on Ctrl+Space).

use std::ops::Range;
use std::rc::Rc;

use evalite::highlight::{Class, highlight};
use yew::prelude::*;

use crate::class;
use crate::code_editor::{Ask, CodeEditor, Completion, Completions, Span};
use crate::complete;
use crate::eval::{self, Binding};

#[derive(Properties, PartialEq)]
pub struct EditorProps {
    pub source: AttrValue,
    pub on_change: Callback<String>,
    /// The characters the error points at.
    pub primary: Option<Range<usize>>,
    /// The variables the completions offer.
    pub bindings: Rc<Vec<Binding>>,
}

#[function_component]
pub fn Editor(props: &EditorProps) -> Html {
    let spans = use_memo(props.source.clone(), |source| spans_of(source));

    let error = props
        .primary
        .as_ref()
        .map(|span| byte_of(&props.source, span.start)..byte_of(&props.source, span.end));

    let complete = {
        let bindings = props.bindings.clone();
        Callback::from(move |ask: Ask| completions(&ask, &bindings).map(Rc::new))
    };

    html! {
        <CodeEditor label="Expression" source={props.source.clone()}
            on_change={props.on_change.clone()} spans={(*spans).clone()} {error}
            complete={Some(complete)} />
    }
}

fn spans_of(source: &str) -> Rc<[Span]> {
    highlight(source)
        .filter_map(|token| {
            let class = match token.class {
                Class::Keyword => class::TOKEN_KEYWORD,
                Class::Ident => class::TOKEN_IDENT,
                Class::Member => class::TOKEN_MEMBER,
                Class::Number => class::TOKEN_NUMBER,
                Class::String => class::TOKEN_STRING,
                Class::Punctuation | Class::Operator => class::TOKEN_PUNCTUATION,
                Class::Error => class::TOKEN_ERROR,
                _ => return None,
            };

            Some(Span {
                range: token.span,
                class,
            })
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

fn completions(ask: &Ask, bindings: &[Binding]) -> Option<Completions> {
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
            let detail = match &offer.ty {
                Some(ty) => format!("{} · {ty}", offer.kind),
                None => offer.kind.to_owned(),
            };

            Completion {
                label: offer.label.clone(),
                detail,
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
        let classes = spans.iter().map(|s| s.class).collect::<Vec<_>>();
        assert_eq!(
            classes,
            [
                class::TOKEN_IDENT,
                class::TOKEN_PUNCTUATION,
                class::TOKEN_NUMBER
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
}
