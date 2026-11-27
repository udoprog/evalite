//! Completions at the editor's cursor, from evalite's own
//! [`Structure::complete`]: the word under the cursor becomes a hole, and
//! what fits that hole is offered, filtered by what was typed of it.

use evalite::highlight::{Class, highlight};
use evalite::{CompletionKind, Environment, Structure, Type};

/// The most offers the popup lists.
const MAX_OFFERS: usize = 40;

/// What a type annotation such as `None::<Int>` offers, each with holes for
/// the types it holds.
const TYPES: [&str; 6] = ["Int", "Bool", "String", "Option<_>", "List<_>", "Map<_, _>"];

/// One thing the popup offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) label: String,
    pub(crate) kind: CompletionKind,
    pub(crate) ty: Option<String>,
    /// The function's or method's signature, as `Option<T>.unwrap_or(T) -> T`.
    pub(crate) signature: Option<String>,
}

/// What the cursor is in: the word it replaces (in characters) and what
/// fits there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offers {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) offers: Vec<Offer>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What fits at character `cursor` of `source`, with `env` declared and
/// `root` the type the whole expression must have, if any. `None`
/// when the text around the cursor does not parse with a hole there, or
/// nothing fits.
pub(crate) fn offers(
    source: &str,
    cursor: usize,
    env: &Environment,
    root: Option<&Type>,
) -> Option<Offers> {
    let chars = source.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());

    let mut start = cursor;

    while let Some(before) = start.checked_sub(1)
        && chars.get(before).is_some_and(|&c| is_word(c))
    {
        start = before;
    }

    let mut end = cursor;

    while chars.get(end).is_some_and(|&c| is_word(c)) {
        end = end.saturating_add(1);
    }

    let prefix = chars.get(start..cursor)?.iter().collect::<String>();

    // A number is not a name to complete.
    if prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    let before = chars.get(..start)?.iter().collect::<String>();
    let after = chars.get(end..)?.iter().collect::<String>();
    let prefix = prefix.to_lowercase();

    if in_annotation(&before) {
        let offers = TYPES
            .iter()
            .filter(|ty| ty.to_lowercase().starts_with(&prefix))
            .map(|ty| Offer {
                label: (*ty).to_owned(),
                kind: CompletionKind::Type,
                ty: None,
                signature: None,
            })
            .collect::<Vec<_>>();

        return (!offers.is_empty()).then_some(Offers { start, end, offers });
    }

    let index = holes_in(&before);

    let structure = holed(&before, &after)?;
    let hole = *structure.holes().get(index)?;
    let completions = structure.complete(env, hole, root).ok()?;

    let offers = completions
        .iter()
        .filter(|c| prefix.is_empty() || c.label().to_lowercase().starts_with(&prefix))
        .take(MAX_OFFERS)
        .map(|c| Offer {
            label: c.label().to_owned(),
            kind: c.kind(),
            ty: c.ty().map(|ty| ty.to_string()),
            signature: c.signature().map(str::to_owned),
        })
        .collect::<Vec<_>>();

    if offers.is_empty() {
        return None;
    }

    Some(Offers { start, end, offers })
}

/// `source` with `offer` in place of the word, and the characters to
/// select after it: its first hole, or the place after it.
pub(crate) fn accept(source: &str, at: &Offers, offer: &Offer) -> (String, usize, usize) {
    let chars = source.chars().collect::<Vec<_>>();
    let mut start = at.start.min(chars.len());
    let end = at.end.clamp(start, chars.len());

    // A field that is not a name is read with an index, `m["Brawl (bar)"]`,
    // so the dot before it goes.
    if offer.label.starts_with('[')
        && let Some(dot) = start.checked_sub(1)
        && chars.get(dot) == Some(&'.')
    {
        start = dot;
    }

    let before = chars
        .get(..start)
        .unwrap_or_default()
        .iter()
        .collect::<String>();
    let after = chars
        .get(end..)
        .unwrap_or_default()
        .iter()
        .collect::<String>();
    let text = format!("{before}{}{after}", offer.label);

    let label = offer.label.chars().collect::<Vec<_>>();
    let hole = (0..label.len()).find(|&i| {
        label.get(i) == Some(&'_')
            && !i
                .checked_sub(1)
                .and_then(|j| label.get(j))
                .is_some_and(|&c| is_word(c))
            && !label.get(i.saturating_add(1)).is_some_and(|&c| is_word(c))
    });

    match hole {
        Some(i) => {
            let at = start.saturating_add(i);
            (text, at, at.saturating_add(1))
        }
        None => {
            let at = start.saturating_add(label.len());
            (text, at, at)
        }
    }
}

/// The structure of `before` and `after` with a hole between them. Text being
/// typed is rarely whole: groups are left open, and a map key has no `:`
/// yet. So the hole is also tried as a key with a value, and with the
/// unclosed groups closed.
fn holed(before: &str, after: &str) -> Option<Structure> {
    let closers = closers(before, after);

    for hole in ["_", "_: _"] {
        for close in ["", closers.as_str()] {
            let text = format!("{before}{hole}{after}{close}");

            if let Ok(structure) = Structure::from_text(&text) {
                return Some(structure);
            }
        }
    }

    None
}

/// The closing brackets for the groups `before` and `after` leave open.
fn closers(before: &str, after: &str) -> String {
    let mut open = Vec::new();
    let mut string = false;
    let mut escaped = false;

    for c in before.chars().chain(after.chars()) {
        if string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => string = false,
                _ => {}
            }
            continue;
        }

        match c {
            '"' => string = true,
            '(' => open.push(')'),
            '[' => open.push(']'),
            '{' => open.push('}'),
            ')' | ']' | '}' if open.last() == Some(&c) => {
                open.pop();
            }
            _ => {}
        }
    }

    open.into_iter().rev().collect()
}

/// Whether a type goes at the end of `before`, inside a type annotation such
/// as `None::<Map<String, `: after its `<`, a `Map`'s `,` or a record
/// field's `:`.
fn in_annotation(before: &str) -> bool {
    // The open `<` and `{` of the annotation, innermost last.
    let mut open = Vec::new();
    let mut after_path = false;
    let mut last = None;

    for token in highlight(before) {
        let text = before.get(token.span.clone()).unwrap_or_default();

        match (token.class, text) {
            (Class::Whitespace, _) => continue,
            (Class::Punctuation, "<") if after_path || !open.is_empty() => open.push('<'),
            (Class::Punctuation, "{") if !open.is_empty() => open.push('{'),
            (Class::Punctuation, ">" | "}") => {
                open.pop();
            }
            (Class::Punctuation | Class::Type | Class::Member | Class::String, _) => {}
            _ => open.clear(),
        }

        after_path = text == "::";
        last = Some(text);
    }

    matches!(
        (open.last(), last),
        (Some('<'), Some("<" | ",")) | (Some('{'), Some(":"))
    )
}

/// How many holes (`_` standing alone, outside strings) `text` has.
fn holes_in(text: &str) -> usize {
    let mut count = 0usize;
    let mut word = String::new();
    let mut string = false;
    let mut escaped = false;

    for c in text.chars().chain(std::iter::once(' ')) {
        if string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => string = false,
                _ => {}
            }
            continue;
        }

        if is_word(c) {
            word.push(c);
            continue;
        }

        if word == "_" {
            count = count.saturating_add(1);
        }

        word.clear();
        string = c == '"';
    }

    count
}

#[cfg(test)]
mod tests {
    use evalite::Type;

    use super::*;

    fn env() -> Environment {
        let mut env = Environment::new();
        env.value("wits", Type::int()).unwrap();
        env.value("bonus", Type::option(Type::int())).unwrap();
        env.function(
            "count",
            [Type::record([
                ("boxes", Type::int()),
                ("bonus", Type::int()),
                ("count", Type::int()),
            ])],
            Type::int(),
        )
        .unwrap();
        env.function(
            "roll",
            [Type::record([
                ("athletics", Type::int()),
                ("Brawl (bar)", Type::int()),
            ])],
            Type::int(),
        )
        .unwrap();
        env.value(
            "m",
            Type::record([("athletics", Type::int()), ("Brawl (bar)", Type::int())]),
        )
        .unwrap();
        env
    }

    fn labels(source: &str, cursor: usize) -> Vec<String> {
        offers(source, cursor, &env(), None)
            .map(|o| o.offers.into_iter().map(|o| o.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn types_in_an_annotation() {
        assert_eq!(
            labels("None::<", 7),
            ["Int", "Bool", "String", "Option<_>", "List<_>", "Map<_, _>"]
        );
        assert_eq!(labels("None::<Li", 9), ["List<_>"]);
        assert_eq!(labels("[{boxes: None::<Li", 18), ["List<_>"]);
        assert_eq!(labels("None::<Map<String, ", 19).len(), 6);
        assert_eq!(labels("None::<{a: ", 11).len(), 6);
        assert!(labels("None::<{a", 9).is_empty());
        assert!(!labels("1 < wi", 6).iter().any(|l| l == "Int"));
    }

    #[test]
    fn a_prefix_filters() {
        let labels = labels("1 + wi", 6);
        assert!(labels.iter().any(|l| l == "wits"), "{labels:?}");
        assert!(labels.iter().all(|l| l.starts_with("wi")), "{labels:?}");
    }

    #[test]
    fn around_an_annotated_none() {
        let names = labels("None::<Int> + wi", 16);
        assert!(names.iter().any(|l| l == "wits"), "{names:?}");

        let members = labels("None::<Int>.un", 14);
        assert!(members.iter().any(|l| l == "unwrap_or(_)"), "{members:?}");
    }

    fn rooted(source: &str, cursor: usize, root: &Type) -> Vec<String> {
        offers(source, cursor, &Environment::new(), Some(root))
            .map(|o| o.offers.into_iter().map(|o| o.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_root_type_is_offered() {
        let option = Type::option(Type::int());
        let names = rooted("", 0, &option);
        assert!(names.iter().any(|l| l == "None"), "{names:?}");
        assert!(names.iter().any(|l| l == "Some(_)"), "{names:?}");
        assert_eq!(rooted("N", 1, &option), ["None"]);

        let record = Type::record([("boxes", option.clone()), ("count", Type::int())]);
        assert_eq!(rooted("{", 1, &record), ["boxes", "count"]);
        assert_eq!(rooted("{boxes: None, ", 14, &record), ["count"]);
        assert_eq!(rooted("{boxes: N", 9, &record), ["None"]);
    }

    #[test]
    fn members_after_a_dot() {
        let labels = labels("bonus.", 6);
        assert!(labels.iter().any(|l| l == "unwrap_or(_)"), "{labels:?}");
    }

    #[test]
    fn the_right_hole_among_others() {
        let source = "_ + Int::max(_, wi)";
        let labels = labels(source, 13);
        assert!(labels.iter().any(|l| l == "wits"), "{labels:?}");
    }

    #[test]
    fn calls_carry_their_signature() {
        let at = offers("bonus.un", 8, &env(), None).unwrap();
        let unwrap_or = at
            .offers
            .iter()
            .find(|o| o.label == "unwrap_or(_)")
            .unwrap();
        assert_eq!(
            unwrap_or.signature.as_deref(),
            Some("Option<T>.unwrap_or(T) -> T")
        );
    }

    #[test]
    fn nothing_in_a_number() {
        assert!(labels("12", 2).is_empty());
    }

    #[test]
    fn accept_selects_the_first_hole() {
        let at = offers("1 + bo", 6, &env(), None).unwrap();
        let offer = at
            .offers
            .iter()
            .find(|o| o.label == "bonus.unwrap_or(_)")
            .unwrap();
        let (text, from, to) = accept("1 + bo", &at, offer);
        assert_eq!(text, "1 + bonus.unwrap_or(_)");
        assert_eq!((from, to), (20, 21));
    }

    #[test]
    fn accept_an_index_field_drops_the_dot() {
        let at = offers("m.", 2, &env(), None).unwrap();
        let offer = at.offers.iter().find(|o| o.label.starts_with('[')).unwrap();
        let (text, from, to) = accept("m.", &at, offer);
        assert_eq!(text, r#"m["Brawl (bar)"]"#);
        assert_eq!(from, to);
    }

    #[test]
    fn holes_skip_strings_and_names() {
        assert_eq!(holes_in(r#"_ + "_" + my_var + _"#), 2);
    }

    #[test]
    fn record_keys_of_a_first_key() {
        for (source, cursor) in [("count({", 7), ("count({}", 7), ("count({bo", 9)] {
            let labels = labels(source, cursor);
            assert!(labels.iter().any(|l| l == "boxes"), "{source}: {labels:?}");
        }

        assert_eq!(labels("count({bo", 9), ["bonus", "boxes"]);
    }

    #[test]
    fn record_keys_of_a_later_key() {
        assert_eq!(labels("count({boxes: 1, co", 19), ["count"]);
        assert_eq!(labels("count({boxes: 1, ", 17), ["bonus", "count"]);
    }

    #[test]
    fn record_keys_skip_unwritable_and_mixed() {
        assert_eq!(labels("roll({", 6), ["athletics"]);
        let labels = labels("count({\"boxes\": 1, ", 20);
        assert!(
            !labels.iter().any(|l| l == "bonus" || l == "boxes"),
            "{labels:?}"
        );
    }
}
