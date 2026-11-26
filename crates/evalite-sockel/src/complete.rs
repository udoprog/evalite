//! Completions at the editor's cursor, from evalite's own
//! [`Structure::complete`]: the word under the cursor becomes a hole, and
//! what fits that hole is offered, filtered by what was typed of it.

use evalite::{CompletionKind, Environment, Structure};

/// The most offers the popup lists.
const MAX_OFFERS: usize = 40;

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

/// What fits at character `cursor` of `source`, with `env` declared. `None`
/// when the text around the cursor does not parse with a hole there, or
/// nothing fits.
pub(crate) fn offers(source: &str, cursor: usize, env: &Environment) -> Option<Offers> {
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
    let index = holes_in(&before);

    let structure = Structure::from_text(&format!("{before}_{after}")).ok()?;
    let hole = *structure.holes().get(index)?;
    let completions = structure.complete(env, hole, None).ok()?;

    let prefix = prefix.to_lowercase();
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
        let mut env = Environment::with_builtins();
        env.value("wits", Type::int()).unwrap();
        env.value("bonus", Type::option(Type::int())).unwrap();
        env.value(
            "m",
            Type::record([("athletics", Type::int()), ("Brawl (bar)", Type::int())]),
        )
        .unwrap();
        env
    }

    fn labels(source: &str, cursor: usize) -> Vec<String> {
        offers(source, cursor, &env())
            .map(|o| o.offers.into_iter().map(|o| o.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_prefix_filters() {
        let labels = labels("1 + wi", 6);
        assert!(labels.iter().any(|l| l == "wits"), "{labels:?}");
        assert!(labels.iter().all(|l| l.starts_with("wi")), "{labels:?}");
    }

    #[test]
    fn members_after_a_dot() {
        let labels = labels("bonus.", 6);
        assert!(labels.iter().any(|l| l == "unwrap_or(_)"), "{labels:?}");
    }

    #[test]
    fn the_right_hole_among_others() {
        let source = "_ + max(_, wi)";
        let labels = labels(source, 13);
        assert!(labels.iter().any(|l| l == "wits"), "{labels:?}");
    }

    #[test]
    fn calls_carry_their_signature() {
        let at = offers("bonus.un", 8, &env()).unwrap();
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
        let at = offers("1 + bo", 6, &env()).unwrap();
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
        let at = offers("m.", 2, &env()).unwrap();
        let offer = at.offers.iter().find(|o| o.label.starts_with('[')).unwrap();
        let (text, from, to) = accept("m.", &at, offer);
        assert_eq!(text, r#"m["Brawl (bar)"]"#);
        assert_eq!(from, to);
    }

    #[test]
    fn holes_skip_strings_and_names() {
        assert_eq!(holes_in(r#"_ + "_" + my_var + _"#), 2);
    }
}
