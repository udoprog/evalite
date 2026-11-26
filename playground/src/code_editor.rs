//! A code editor that knows no language: an uncontrolled textarea, so the
//! browser keeps its undo, under an `aria-hidden` `<pre>` repeating its text
//! in highlighted spans, beside a gutter of line numbers. The highlighting,
//! the error span and the completions come in through props. Offsets are
//! bytes of the source.

use std::ops::Range;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlDocument, HtmlTextAreaElement};
use yew::prelude::*;

use sockel::{
    Dismiss, PopoverAlign, Side, listbox, listbox_option, option_id, use_overlays, use_popover,
};

use crate::class;

const LIST_ID: &str = "playground-completions";

/// A highlighted span of source, worn as `class`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub class: &'static str,
}

/// One completion: what the popup lists, and the text and selection
/// accepting it leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub text: String,
    pub select: Range<usize>,
}

/// The completions at the cursor, with the popup anchored at `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completions {
    pub start: usize,
    pub items: Vec<Completion>,
}

/// What the completions are asked for: the text, the cursor and whether
/// they were asked for (Ctrl+Space) rather than offered while typing.
pub struct Ask {
    pub text: String,
    pub cursor: usize,
    pub forced: bool,
}

#[derive(Properties, PartialEq)]
pub struct CodeEditorProps {
    pub label: AttrValue,
    pub source: AttrValue,
    pub on_change: Callback<String>,
    /// The highlighted spans of `source`, in order.
    pub spans: Rc<[Span]>,
    /// The span an error points at; an empty span marks the place after it.
    #[prop_or_default]
    pub error: Option<Range<usize>>,
    #[prop_or_default]
    pub complete: Option<Callback<Ask, Option<Rc<Completions>>>>,
}

#[function_component]
pub fn CodeEditor(props: &CodeEditorProps) -> Html {
    let area = use_node_ref();
    let frame = use_node_ref();
    let anchor = use_node_ref();
    let list = use_node_ref();
    let offers = use_state(|| None::<Rc<Completions>>);
    let active = use_state(|| 0usize);
    let caret = use_state(|| 0usize);
    // Set while an accepted completion is inserted, so its `input` does not
    // offer completions again.
    let accepting = use_mut_ref(|| false);
    let overlays = use_overlays();

    let open = offers.is_some();

    let close = {
        let offers = offers.clone();
        Callback::from(move |_: Dismiss| offers.set(None))
    };

    let placed = use_popover(
        open,
        anchor.clone(),
        list.clone(),
        Side::Below,
        PopoverAlign::Start,
        close,
    );

    // The textarea is uncontrolled: only a source from outside (an example,
    // a link) is written into it, which typing never needs.
    {
        let area = area.clone();
        use_effect_with(props.source.clone(), move |source| {
            if let Some(el) = area.cast::<HtmlTextAreaElement>()
                && el.value() != source.as_str()
            {
                el.set_value(source);
            }
        });
    }

    {
        let (frame, caret) = (frame.clone(), *caret);
        use_effect_with((caret, props.source.clone()), move |_| follow_caret(&frame));
    }

    let track = {
        let (area, caret) = (area.clone(), caret.clone());
        Rc::new(move || {
            if let Some(el) = area.cast::<HtmlTextAreaElement>() {
                caret.set(cursor(&el));
            }
        })
    };

    let refresh = {
        let (area, offers, active, complete) = (
            area.clone(),
            offers.clone(),
            active.clone(),
            props.complete.clone(),
        );
        Rc::new(move |forced: bool| {
            let (Some(el), Some(complete)) = (area.cast::<HtmlTextAreaElement>(), &complete) else {
                return;
            };

            let ask = Ask {
                cursor: cursor(&el),
                text: el.value(),
                forced,
            };

            active.set(0);
            offers.set(complete.emit(ask));
        })
    };

    let accept = {
        let (area, offers, accepting) = (area.clone(), offers.clone(), accepting.clone());
        Rc::new(move |index: usize| {
            let (Some(el), Some(at)) = (area.cast::<HtmlTextAreaElement>(), (*offers).clone())
            else {
                return;
            };
            let Some(item) = at.items.get(index) else {
                return;
            };

            let value = el.value();
            let (replace, text) = difference(&value, &item.text);
            *accepting.borrow_mut() = true;
            apply(
                &el,
                &value,
                &Edit {
                    replace,
                    text: text.to_owned(),
                    select: item.select.clone(),
                },
            );
            *accepting.borrow_mut() = false;
            let _ = el.focus();
            offers.set(None);
        })
    };

    let on_input = {
        let (on_change, refresh, track, accepting) = (
            props.on_change.clone(),
            refresh.clone(),
            track.clone(),
            accepting.clone(),
        );
        Callback::from(move |e: InputEvent| {
            let Some(el) = e.target_dyn_into::<HtmlTextAreaElement>() else {
                return;
            };

            on_change.emit(el.value());
            track();

            if !*accepting.borrow() {
                refresh(false);
            }
        })
    };

    let on_keydown = {
        let (area, offers, active, refresh, accept) = (
            area.clone(),
            offers.clone(),
            active.clone(),
            refresh.clone(),
            accept.clone(),
        );
        Callback::from(move |e: KeyboardEvent| {
            let key = e.key();

            if e.ctrl_key() && key == " " {
                e.prevent_default();
                refresh(true);
                return;
            }

            if let Some(at) = &*offers {
                let count = at.items.len().max(1);

                match key.as_str() {
                    "ArrowDown" => {
                        e.prevent_default();
                        active.set(active.saturating_add(1).checked_rem(count).unwrap_or(0));
                        return;
                    }
                    "ArrowUp" => {
                        e.prevent_default();
                        active.set(active.checked_sub(1).unwrap_or(count.saturating_sub(1)));
                        return;
                    }
                    "Enter" | "Tab" => {
                        e.prevent_default();
                        accept(*active);
                        return;
                    }
                    "Escape" => {
                        e.prevent_default();
                        offers.set(None);
                        return;
                    }
                    _ => {}
                }
            }

            if let Some(el) = area.cast::<HtmlTextAreaElement>()
                && keydown(&el, &e)
            {
                e.prevent_default();
            }
        })
    };

    let on_caret = {
        let track = track.clone();
        Callback::from(move |_: Event| track())
    };

    let on_blur = {
        let offers = offers.clone();
        Callback::from(move |_: FocusEvent| offers.set(None))
    };

    // The textarea is as large as its text and never scrolls; the frame
    // does.
    let on_scroll = Callback::from(|e: Event| {
        if let Some(el) = e.target_dyn_into::<Element>() {
            el.set_scroll_top(0);
            el.set_scroll_left(0);
        }
    });

    let popup = offers.as_ref().map(|at| {
        let style = placed.map(|p| {
            format!(
                "--popover-left: {}px; --popover-top: {}px; --popover-max-height: {}px;",
                p.left, p.top, p.max_height
            )
        });

        let rows = at.items.iter().enumerate().map(|(index, item)| {
            let onclick = {
                let accept = accept.clone();
                Callback::from(move |_: MouseEvent| accept(index))
            };
            let onmousemove = {
                let active = active.clone();
                Callback::from(move |_: MouseEvent| {
                    if *active != index {
                        active.set(index);
                    }
                })
            };

            html! {
                <div id={option_id(LIST_ID, index)} role="option"
                    class={listbox_option(index == *active)}
                    aria-selected={(index == *active).then_some("true")}
                    {onclick} {onmousemove}>
                    <span class={sockel::class::LISTBOX_OPTION_TEXT}>
                        <span class={classes!(sockel::class::LISTBOX_OPTION_LABEL, class::CODE)}>
                            { item.label.clone() }
                        </span>
                        <span class={sockel::class::LISTBOX_OPTION_DESCRIPTION}>{ item.detail.clone() }</span>
                    </span>
                </div>
            }
        });

        // A press in the list keeps the keyboard in the textarea.
        let onmousedown = Callback::from(|e: MouseEvent| e.prevent_default());

        overlays.portal(html! {
            <div ref={list.clone()} id={LIST_ID} role="listbox" aria-label="Completions"
                class={listbox(placed.is_some())} {style} {onmousedown}>
                { for rows }
            </div>
        })
    });

    let source = props.source.as_str();
    let anchor_at = offers.as_ref().map(|at| at.start);
    let pieces = pieces(
        source,
        &props.spans,
        props.error.clone(),
        &[Some(*caret), anchor_at],
    );

    let code = pieces.into_iter().map(|piece| match piece {
        Piece::Text {
            range,
            class,
            marked,
        } => {
            let text = source.get(range).unwrap_or_default().to_owned();
            let html = match class {
                Some(class) => html! { <span class={class}>{ text }</span> },
                None => html! { { text } },
            };

            if marked {
                html! { <span class={class::MARK}>{ html }</span> }
            } else {
                html
            }
        }
        Piece::Point(0) => html! { <span class={class::EDITOR_CARET}></span> },
        Piece::Point(_) => html! { <span ref={anchor.clone()}></span> },
        Piece::EmptyMark => html! {
            <span class={classes!(class::MARK, class::EDITOR_EMPTY_MARK)}></span>
        },
    });

    let error_line = props.error.as_ref().map(|span| line_of(source, span.start));
    let gutter = (0..source.split('\n').count()).map(|line| {
        let number = line.saturating_add(1);

        if Some(line) == error_line {
            html! {
                <div class={classes!(class::EDITOR_LINE, class::ERROR)}>
                    <span class={class::EDITOR_MARKER}></span>{ number }
                </div>
            }
        } else {
            html! { <div class={class::EDITOR_LINE}>{ number }</div> }
        }
    });

    let invalid = props.error.is_some();

    html! {
        <div ref={frame}
            class={classes!(class::EDITOR, invalid.then_some(class::INVALID))}>
            // First: yew matches unkeyed children from the end, so the
            // popup coming and going there would rebuild the textarea.
            { for popup }
            <div class={class::EDITOR_GUTTER} aria-hidden="true">{ for gutter }</div>
            // The spare line keeps the textarea's text inside its box, so
            // that it never has anything to scroll.
            <pre class={classes!(class::CODE, class::EDITOR_CODE)} aria-hidden="true">
                { for code }{ "\n " }
            </pre>
            <textarea ref={area} class={classes!(class::CODE, class::EDITOR_INPUT)}
                aria-label={props.label.clone()}
                aria-invalid={invalid.then_some("true")}
                aria-controls={open.then_some(LIST_ID)}
                aria-activedescendant={open.then(|| option_id(LIST_ID, *active))}
                rows="1" wrap="off" spellcheck="false" autocapitalize="off" autocomplete="off"
                oninput={on_input} onkeydown={on_keydown}
                onkeyup={on_caret.reform(|e: KeyboardEvent| e.into())}
                onclick={on_caret.reform(|e: MouseEvent| e.into())}
                onselect={on_caret.clone()}
                onblur={on_blur} onscroll={on_scroll} />
        </div>
    }
}

/// One stretch of the highlighted code, or a point in it.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    Text {
        range: Range<usize>,
        class: Option<&'static str>,
        marked: bool,
    },
    /// The index of a point asked for.
    Point(usize),
    /// An empty error span.
    EmptyMark,
}

/// `source` cut into pieces at the spans, the error and the points, which
/// are placed before the text at them.
fn pieces(
    source: &str,
    spans: &[Span],
    error: Option<Range<usize>>,
    points: &[Option<usize>],
) -> Vec<Piece> {
    let len = source.len();
    let floor = |at: usize| floor(source, at.min(len));
    let error = error.map(|e| floor(e.start)..floor(e.end));

    let mut cuts = vec![0, len];
    cuts.extend(
        spans
            .iter()
            .flat_map(|s| [floor(s.range.start), floor(s.range.end)]),
    );
    cuts.extend(error.iter().flat_map(|e| [e.start, e.end]));
    cuts.extend(points.iter().flatten().map(|&p| floor(p)));
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = Vec::new();
    let mut spans = spans.iter().peekable();

    for (index, &at) in cuts.iter().enumerate() {
        for (point, p) in points.iter().enumerate() {
            if p.map(floor) == Some(at) {
                out.push(Piece::Point(point));
            }
        }

        if let Some(e) = &error
            && e.is_empty()
            && e.start == at
        {
            out.push(Piece::EmptyMark);
        }

        let Some(&end) = cuts.get(index.saturating_add(1)) else {
            break;
        };

        while spans.next_if(|s| floor(s.range.end) <= at).is_some() {}

        let class = spans
            .peek()
            .filter(|s| floor(s.range.start) <= at)
            .map(|s| s.class);
        let marked = error
            .as_ref()
            .is_some_and(|e| e.start <= at && end <= e.end);

        out.push(Piece::Text {
            range: at..end,
            class,
            marked,
        });
    }

    out
}

fn floor(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at = at.saturating_sub(1);
    }

    at
}

/// The line, from zero, byte `at` is on.
fn line_of(text: &str, at: usize) -> usize {
    text.get(..floor(text, at.min(text.len())))
        .unwrap_or_default()
        .matches('\n')
        .count()
}

/// The byte offset of `utf16` units into `text`, as the DOM counts.
fn byte_offset(text: &str, utf16: u32) -> usize {
    let mut count = 0u32;

    for (index, c) in text.char_indices() {
        if count >= utf16 {
            return index;
        }

        count = count.saturating_add(u32::try_from(c.len_utf16()).unwrap_or(2));
    }

    text.len()
}

/// The UTF-16 units of `text` up to byte `at`.
fn utf16_at(text: &str, at: usize) -> u32 {
    let units = text
        .get(..floor(text, at.min(text.len())))
        .unwrap_or_default()
        .encode_utf16()
        .count();
    u32::try_from(units).unwrap_or(u32::MAX)
}

fn cursor(area: &HtmlTextAreaElement) -> usize {
    let at = area.selection_start().ok().flatten().unwrap_or(0);
    byte_offset(&area.value(), at)
}

/// The part of `old` to replace, and the text to put there, to make `new`.
fn difference<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|&((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);

    let suffix = old
        .get(prefix..)
        .unwrap_or_default()
        .chars()
        .rev()
        .zip(new.get(prefix..).unwrap_or_default().chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum::<usize>();

    let old_end = old.len().saturating_sub(suffix).max(prefix);
    let new_end = new.len().saturating_sub(suffix).max(prefix);
    (
        prefix..old_end,
        new.get(prefix..new_end).unwrap_or_default(),
    )
}

/// An edit: `replace` (bytes of the text before) becomes `text`, and then
/// `select` (bytes of the text after) is selected.
#[derive(Debug, PartialEq, Eq)]
struct Edit {
    replace: Range<usize>,
    text: String,
    select: Range<usize>,
}

/// The edit for Tab (`shift`: Shift+Tab) on the selection `start..end` of
/// `value`: a tab at the cursor, or the selected lines indented or
/// outdented. `None` when nothing changes.
fn indent(value: &str, start: usize, end: usize, shift: bool) -> Option<Edit> {
    let selected = value.get(start..end)?;
    let multiline = selected.contains('\n');

    if !shift && !multiline {
        let at = start.saturating_add(1);
        return Some(Edit {
            replace: start..end,
            text: "\t".to_owned(),
            select: at..at,
        });
    }

    let line_start = value
        .get(..start)?
        .rfind('\n')
        .map_or(0, |i| i.saturating_add(1));

    let block_end = if end > start && value.get(..end)?.ends_with('\n') {
        end.saturating_sub(1)
    } else {
        value
            .get(end..)?
            .find('\n')
            .map_or(value.len(), |i| end.saturating_add(i))
    };

    let block = value.get(line_start..block_end)?;
    let mut removed = 0usize;

    let lines = block
        .split('\n')
        .enumerate()
        .map(|(n, line)| {
            if !shift {
                return format!("\t{line}");
            }

            let strip = if line.starts_with('\t') {
                1
            } else {
                line.len()
                    .saturating_sub(line.trim_start_matches(' ').len())
                    .min(4)
            };

            if n == 0 {
                removed = strip;
            }

            line.get(strip..).unwrap_or(line).to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");

    if lines == block {
        return None;
    }

    let select = if multiline {
        line_start..line_start.saturating_add(lines.len())
    } else {
        let at = start.saturating_sub(removed).max(line_start);
        at..at
    };

    Some(Edit {
        replace: line_start..block_end,
        text: lines,
        select,
    })
}

/// The edit for Enter at the selection `start..end`: a new line keeping
/// the indent of the one the cursor is on.
fn newline(value: &str, start: usize, end: usize) -> Option<Edit> {
    let line_start = value
        .get(..start)?
        .rfind('\n')
        .map_or(0, |i| i.saturating_add(1));
    let line = value.get(line_start..start)?;
    let indent = line
        .get(
            ..line
                .len()
                .saturating_sub(line.trim_start_matches([' ', '\t']).len()),
        )
        .unwrap_or_default();
    let text = format!("\n{indent}");
    let at = start.saturating_add(text.len());

    Some(Edit {
        replace: start..end,
        text,
        select: at..at,
    })
}

/// Handle the keys which edit beyond inserting a character. Whether the
/// browser's own action is to be prevented.
fn keydown(area: &HtmlTextAreaElement, event: &KeyboardEvent) -> bool {
    if event.is_composing() || event.ctrl_key() || event.meta_key() || event.alt_key() {
        return false;
    }

    let value = area.value();
    let s16 = area.selection_start().ok().flatten().unwrap_or(0);
    let e16 = area.selection_end().ok().flatten().unwrap_or(s16);
    let (start, end) = (byte_offset(&value, s16), byte_offset(&value, e16));

    let edit = match event.key().as_str() {
        "Tab" => {
            let Some(edit) = indent(&value, start, end, event.shift_key()) else {
                return true;
            };
            edit
        }
        "Enter" if !event.shift_key() => match newline(&value, start, end) {
            Some(edit) => edit,
            None => return false,
        },
        _ => return false,
    };

    apply(area, &value, &edit);
    true
}

/// Make `edit` through the browser's editing commands, so that it lands on
/// its undo stack and fires `input` as typing does.
fn apply(area: &HtmlTextAreaElement, value: &str, edit: &Edit) {
    let from = utf16_at(value, edit.replace.start);
    let to = utf16_at(value, edit.replace.end);
    let _ = area.set_selection_range(from, to);

    let document = area
        .owner_document()
        .and_then(|d| d.dyn_into::<HtmlDocument>().ok());

    let done = match document {
        Some(document) if edit.text.is_empty() => {
            from == to || document.exec_command("delete").unwrap_or(false)
        }
        Some(document) => document
            .exec_command_with_show_ui_and_value("insertText", false, &edit.text)
            .unwrap_or(false),
        None => false,
    };

    if !done {
        let _ = area.set_range_text(&edit.text);

        if let Ok(event) = web_sys::InputEvent::new("input") {
            let _ = area.dispatch_event(&event);
        }
    }

    let after = area.value();
    let _ = area.set_selection_range(
        utf16_at(&after, edit.select.start),
        utf16_at(&after, edit.select.end),
    );
}

/// Scroll the frame, when it scrolls, so that the caret can be seen: the
/// textarea is as large as its text and never scrolls itself, so the
/// browser does not do this on its own.
fn follow_caret(frame: &NodeRef) {
    let Some(frame) = frame.cast::<Element>() else {
        return;
    };

    let selector = format!(".{}", class::EDITOR_CARET);

    let Ok(Some(caret)) = frame.query_selector(&selector) else {
        return;
    };

    let gutter = frame
        .query_selector(&format!(".{}", class::EDITOR_GUTTER))
        .ok()
        .flatten()
        .map_or(0.0, |g| g.get_bounding_client_rect().width());

    let view = frame.get_bounding_client_rect();
    let at = caret.get_bounding_client_rect();
    let margin = at.height().max(16.0) * 2.0;

    let left = view.left() + gutter + margin;
    let right = view.right() - margin;

    let by = if at.left() < left {
        at.left() - left
    } else if at.left() > right {
        at.left() - right
    } else {
        return;
    };

    frame.set_scroll_left(frame.scroll_left().saturating_add(by.round() as i32));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: Range<usize>, class: &'static str) -> Span {
        Span { range, class }
    }

    fn text(range: Range<usize>, class: Option<&'static str>, marked: bool) -> Piece {
        Piece::Text {
            range,
            class,
            marked,
        }
    }

    #[test]
    fn pieces_follow_spans() {
        let spans = [span(0..1, "a"), span(2..3, "b")];
        assert_eq!(
            pieces("x y", &spans, None, &[]),
            [
                text(0..1, Some("a"), false),
                text(1..2, None, false),
                text(2..3, Some("b"), false),
            ]
        );
    }

    #[test]
    fn pieces_cut_at_the_error_and_points() {
        let spans = [span(0..4, "a")];
        assert_eq!(
            pieces("abcd", &spans, Some(1..3), &[Some(2), None]),
            [
                text(0..1, Some("a"), false),
                text(1..2, Some("a"), true),
                Piece::Point(0),
                text(2..3, Some("a"), true),
                text(3..4, Some("a"), false),
            ]
        );
    }

    #[test]
    fn pieces_place_points_and_empty_marks_at_the_end() {
        assert_eq!(
            pieces("ab", &[], Some(2..2), &[Some(2), Some(9)]),
            [
                text(0..2, None, false),
                Piece::Point(0),
                Piece::Point(1),
                Piece::EmptyMark,
            ]
        );
    }

    #[test]
    fn pieces_of_nothing() {
        assert_eq!(pieces("", &[], None, &[Some(0)]), [Piece::Point(0)]);
    }

    #[test]
    fn pieces_keep_to_char_boundaries() {
        let source = "é+1";
        let got = pieces(source, &[], Some(1..2), &[Some(1)]);
        assert!(got.iter().all(|p| match p {
            Piece::Text { range, .. } => source.get(range.clone()).is_some(),
            _ => true,
        }));
    }

    #[test]
    fn utf16_round_trips() {
        let text = "a😀é\nb";
        assert_eq!(utf16_at(text, 1), 1);
        assert_eq!(utf16_at(text, 5), 3);
        assert_eq!(utf16_at(text, 7), 4);
        assert_eq!(byte_offset(text, 3), 5);
        assert_eq!(byte_offset(text, 4), 7);
        assert_eq!(byte_offset(text, 99), text.len());
    }

    #[test]
    fn lines() {
        assert_eq!(line_of("a\nb\nc", 0), 0);
        assert_eq!(line_of("a\nb\nc", 2), 1);
        assert_eq!(line_of("a\nb\nc", 99), 2);
    }

    #[test]
    fn differences() {
        assert_eq!(difference("a + wi", "a + wits"), (6..6, "ts"));
        assert_eq!(difference("a.le + 1", "a.len() + 1"), (4..4, "n()"));
        assert_eq!(difference("x.[", "x[\"a b\"]"), (1..3, "[\"a b\"]"));
        assert_eq!(difference("same", "same"), (4..4, ""));
    }

    #[test]
    fn tab_inserts_a_tab() {
        assert_eq!(
            indent("ab", 1, 1, false),
            Some(Edit {
                replace: 1..1,
                text: "\t".into(),
                select: 2..2
            })
        );
    }

    #[test]
    fn tab_indents_selected_lines() {
        assert_eq!(
            indent("a\nb\nc", 0, 3, false),
            Some(Edit {
                replace: 0..3,
                text: "\ta\n\tb".into(),
                select: 0..5
            })
        );
    }

    #[test]
    fn shift_tab_outdents() {
        assert_eq!(
            indent("x\n    ab", 7, 7, true),
            Some(Edit {
                replace: 2..8,
                text: "ab".into(),
                select: 3..3
            })
        );
        assert_eq!(indent("ab", 1, 1, true), None);
    }

    #[test]
    fn enter_keeps_the_indent() {
        assert_eq!(
            newline("  a", 3, 3),
            Some(Edit {
                replace: 3..3,
                text: "\n  ".into(),
                select: 6..6
            })
        );
    }
}
