use std::ops::Range;
use std::rc::Rc;

use evalite::{Diagnostic, Diagnostics};
use phrasebook::yew023::use_phrases_with;
use sockel::{ErrorBox, Gap};
use yew::prelude::*;

use crate::L10N;

/// Keeps the snippet's columns: a browser collapses runs of plain spaces.
const SPACE: char = '\u{a0}';

#[derive(Properties, PartialEq)]
pub struct ErrorViewProps {
    pub diagnostics: Rc<Diagnostics>,
    /// The source the diagnostics are about.
    pub source: AttrValue,
    /// The list's accessible name, else the catalog's.
    #[prop_or_default]
    pub label: Option<AttrValue>,
    #[prop_or_default]
    pub class: Classes,
}

/// Every diagnostic of an expression: its message, then the source line it
/// is about with the span underlined, and each further label with its own
/// line. Under an [`ExpressionEditor`](crate::ExpressionEditor), or on its
/// own, such as for a stored formula that no longer compiles.
#[function_component]
pub fn ErrorView(props: &ErrorViewProps) -> Html {
    let phrases = use_phrases_with(&L10N);
    let label = props
        .label
        .clone()
        .unwrap_or_else(|| AttrValue::from(phrases.text("evalite-errors-label")));

    let items = props.diagnostics.iter().map(|diagnostic| {
        let causes = causes(&props.source, diagnostic);

        html! {
            <li>
                <ErrorBox {causes}>{ diagnostic.to_string() }</ErrorBox>
            </li>
        }
    });

    html! {
        <ul class={classes!(sockel::stack(Gap::Tight), sockel::class::PLAIN_LIST, props.class.clone())}
            aria-label={label}>
            { for items }
        </ul>
    }
}

/// The lines under a diagnostic's message: a snippet for each label with a
/// span, its message beside its marks unless it is the primary one, which
/// the message above already says.
fn causes(source: &str, diagnostic: &Diagnostic) -> Vec<AttrValue> {
    let mut causes = Vec::new();

    for label in diagnostic.labels() {
        let note = (!label.is_primary()).then(|| label.message().to_string());

        match label.span() {
            Some(span) => {
                let mark = if label.is_primary() { '^' } else { '-' };

                if let Some(lines) = snippet(source, span.range(), mark, note.as_deref()) {
                    causes.extend(lines.map(AttrValue::from));
                }
            }
            None => causes.extend(note.map(AttrValue::from)),
        }
    }

    causes
}

/// The line of `source` that `span` starts on, and a line marking the span
/// under it with `mark`, followed by `note`. A span running past its line is
/// marked to the line's end.
fn snippet(
    source: &str,
    span: Range<usize>,
    mark: char,
    note: Option<&str>,
) -> Option<[String; 2]> {
    let start = boundary(source, span.start);
    let end = boundary(source, span.end).max(start);

    let line_start = source
        .get(..start)?
        .rfind('\n')
        .map_or(0, |n| n.saturating_add(1));
    let line = source.get(line_start..)?.split('\n').next()?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    let line_end = line_start.saturating_add(line.len());

    let column = source.get(line_start..start)?.chars().count();
    let width = source
        .get(start..end.clamp(start, line_end.max(start)))?
        .chars()
        .count()
        .max(1);

    let shown = line
        .chars()
        .map(|c| if c.is_whitespace() { SPACE } else { c })
        .collect::<String>();

    let mut marks = std::iter::repeat_n(SPACE, column)
        .chain(std::iter::repeat_n(mark, width))
        .collect::<String>();

    if let Some(note) = note {
        marks.push(' ');
        marks.push_str(note);
    }

    Some([shown, marks])
}

/// `at`, or the nearest character boundary of `text` before it.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());

    while !text.is_char_boundary(at) {
        at = at.saturating_sub(1);
    }

    at
}

#[cfg(test)]
mod tests {
    use evalite::{Environment, Type};

    use super::*;

    fn plain(lines: Vec<AttrValue>) -> Vec<String> {
        lines.iter().map(|line| line.replace(SPACE, " ")).collect()
    }

    fn causes_of(source: &str) -> Vec<Vec<String>> {
        let mut env = Environment::new();
        env.value("rank", Type::int()).unwrap();
        let errors = evalite::compile(source, &env).unwrap_err();
        errors.iter().map(|e| plain(causes(source, e))).collect()
    }

    #[test]
    fn each_diagnostic_marks_its_span() {
        assert_eq!(
            causes_of("rank + true + nothing"),
            [
                vec!["rank + true + nothing", "^^^^^^^^^^^"],
                vec!["rank + true + nothing", "              ^^^^^^^"],
            ]
        );
    }

    #[test]
    fn an_empty_span_is_one_mark() {
        assert_eq!(causes_of("rank +"), [vec!["rank +", "      ^"]]);
    }

    #[test]
    fn a_secondary_label_has_its_note() {
        let causes = causes_of("Int::min(rank, 1");
        let first = causes.first().unwrap();
        assert_eq!(first.len(), 4, "{first:?}");
        assert!(
            first.get(3).unwrap().trim_start().starts_with('-'),
            "{first:?}"
        );
    }

    #[test]
    fn only_the_span_line_is_shown() {
        assert_eq!(
            causes_of("rank +\n  nothing\n+ 1"),
            [vec!["  nothing", "  ^^^^^^^"]]
        );
    }

    #[test]
    fn columns_count_characters() {
        let lines = snippet("\"é\" + x", 7..8, '^', None).unwrap();
        assert_eq!(
            lines.get(1).map(|l| l.replace(SPACE, " ")).unwrap(),
            "      ^"
        );
    }

    #[test]
    fn a_span_past_the_line_stops_at_its_end() {
        let lines = snippet("ab\ncd", 1..5, '^', None).unwrap();
        assert_eq!(lines.get(1).map(|l| l.replace(SPACE, " ")).unwrap(), " ^");
    }
}
