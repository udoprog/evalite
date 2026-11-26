//! The expression editor: a textarea under a layer that underlines the
//! error's span, and a popup of evalite's completions for the word at the
//! cursor (as you type a name or after a `.`, or on Ctrl+Space).

use std::ops::Range;
use std::rc::Rc;

use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

use sockel::{
    Dismiss, PopoverAlign, Side, Textarea, listbox, listbox_option, option_id, use_overlays,
    use_popover,
};

use crate::class;
use crate::complete::{self, Offers};
use crate::eval::{self, Binding};

const LIST_ID: &str = "playground-completions";

#[derive(Properties, PartialEq)]
pub struct EditorProps {
    pub source: AttrValue,
    pub on_change: Callback<String>,
    /// The characters the error points at.
    pub primary: Option<Range<usize>>,
    /// The variables the completions offer.
    pub bindings: Rc<Vec<Binding>>,
}

/// The cursor, in characters, of a textarea whose selection is in UTF-16
/// units.
fn cursor_of(area: &HtmlTextAreaElement) -> usize {
    let text = area.value();
    let at = area.selection_start().ok().flatten().unwrap_or(0);
    let at = usize::try_from(at).unwrap_or(usize::MAX);
    let mut units = 0usize;

    text.chars()
        .take_while(|c| {
            units = units.saturating_add(c.len_utf16());
            units <= at
        })
        .count()
}

/// Character `at` of `text` in UTF-16 units.
fn utf16_at(text: &str, at: usize) -> u32 {
    let units = text.chars().take(at).map(char::len_utf16).sum::<usize>();
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// Whether typing just now should open the popup: the cursor is after a
/// letter of a name or a `.`.
fn wants_offers(text: &str, cursor: usize) -> bool {
    cursor
        .checked_sub(1)
        .and_then(|i| text.chars().nth(i))
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '.')
}

#[function_component]
pub fn Editor(props: &EditorProps) -> Html {
    let area = use_node_ref();
    let anchor = use_node_ref();
    let list = use_node_ref();
    let offers = use_state(|| None::<Rc<Offers>>);
    let active = use_state(|| 0usize);
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
        close.clone(),
    );

    let refresh = {
        let (area, offers, active, bindings) = (
            area.clone(),
            offers.clone(),
            active.clone(),
            props.bindings.clone(),
        );
        Rc::new(move |force: bool| {
            let Some(el) = area.cast::<HtmlTextAreaElement>() else {
                return;
            };

            let text = el.value();
            let cursor = cursor_of(&el);

            if !force && !wants_offers(&text, cursor) {
                offers.set(None);
                return;
            }

            let next = eval::environment(&bindings)
                .ok()
                .and_then(|env| complete::offers(&text, cursor, &env));
            active.set(0);
            offers.set(next.map(Rc::new));
        })
    };

    // The selection an accepted completion leaves, in UTF-16 units, put
    // back once the new text has rendered (rendering it moves the cursor).
    let pending = use_mut_ref(|| None::<(String, u32, u32)>);

    {
        let (area, pending) = (area.clone(), pending.clone());
        use_effect_with(props.source.clone(), move |source| {
            let Some(el) = area.cast::<HtmlTextAreaElement>() else {
                return;
            };

            let taken = pending.borrow_mut().take();

            if let Some((text, from, to)) = taken
                && text == source.as_str()
            {
                let _ = el.focus();
                let _ = el.set_selection_range(from, to);
            }
        });
    }

    let accept = {
        let (area, offers, on_change, pending) = (
            area.clone(),
            offers.clone(),
            props.on_change.clone(),
            pending.clone(),
        );
        Rc::new(move |index: usize| {
            let (Some(el), Some(at)) = (area.cast::<HtmlTextAreaElement>(), (*offers).clone())
            else {
                return;
            };
            let Some(offer) = at.offers.get(index) else {
                return;
            };

            let (text, from, to) = complete::accept(&el.value(), &at, offer);
            let (from, to) = (utf16_at(&text, from), utf16_at(&text, to));
            el.set_value(&text);
            let _ = el.set_selection_range(from, to);
            let _ = el.focus();
            *pending.borrow_mut() = Some((text.clone(), from, to));
            offers.set(None);
            on_change.emit(text);
        })
    };

    let on_input = {
        let (on_change, refresh) = (props.on_change.clone(), refresh.clone());
        Callback::from(move |text: String| {
            on_change.emit(text);
            refresh(false);
        })
    };

    let on_keydown = {
        let (offers, active, refresh, accept) = (
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

            let Some(at) = &*offers else {
                return;
            };

            let count = at.offers.len().max(1);

            match key.as_str() {
                "ArrowDown" => {
                    e.prevent_default();
                    active.set(active.saturating_add(1).checked_rem(count).unwrap_or(0));
                }
                "ArrowUp" => {
                    e.prevent_default();
                    active.set(active.checked_sub(1).unwrap_or(count.saturating_sub(1)));
                }
                "Enter" | "Tab" => {
                    e.prevent_default();
                    accept(*active);
                }
                "Escape" => {
                    e.prevent_default();
                    offers.set(None);
                }
                _ => {}
            }
        })
    };

    let on_blur = {
        let offers = offers.clone();
        Callback::from(move |_: FocusEvent| offers.set(None))
    };

    let popup = offers.as_ref().map(|at| {
        let style = placed.map(|p| {
            format!(
                "--popover-left: {}px; --popover-top: {}px; --popover-max-height: {}px;",
                p.left, p.top, p.max_height
            )
        });

        let rows = at.offers.iter().enumerate().map(|(index, offer)| {
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
            let detail = match &offer.ty {
                Some(ty) => format!("{} · {ty}", offer.kind),
                None => offer.kind.to_owned(),
            };

            html! {
                <div id={option_id(LIST_ID, index)} role="option"
                    class={listbox_option(index == *active)}
                    aria-selected={(index == *active).then_some("true")}
                    {onclick} {onmousemove}>
                    <span class={sockel::class::LISTBOX_OPTION_TEXT}>
                        <span class={classes!(sockel::class::LISTBOX_OPTION_LABEL, class::CODE)}>
                            { offer.label.clone() }
                        </span>
                        <span class={sockel::class::LISTBOX_OPTION_DESCRIPTION}>{ detail }</span>
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

    // The text up to the word being completed, in a layer laid over the
    // textarea as the marks are, ending in the popup's anchor.
    let caret = offers.as_ref().map(|at| {
        let before = props.source.chars().take(at.start).collect::<String>();

        html! {
            <div class={classes!(class::CODE, class::EDITOR_MARKS)} aria-hidden="true">
                { before }<span ref={anchor.clone()} class={class::EDITOR_CARET}>{ "\u{200b}" }</span>
            </div>
        }
    });

    {
        let (area, open, active) = (area.clone(), open, *active);
        use_effect_with((open, active), move |&(open, active)| {
            if let Some(el) = area.cast::<HtmlTextAreaElement>() {
                let _ = if open {
                    el.set_attribute("aria-activedescendant", &option_id(LIST_ID, active))
                } else {
                    el.remove_attribute("aria-activedescendant")
                };
            }
        });
    }

    html! {
        <div class={class::EDITOR} onblur={on_blur}>
            <Textarea class={classes!(class::CODE, class::EDITOR_INPUT)} label="Expression"
                value={props.source.clone()} {on_input} {on_keydown} rows={4}
                node_ref={area} invalid={props.primary.is_some()} />
            <div class={classes!(class::CODE, class::EDITOR_MARKS)} aria-hidden="true">
                { marked(&props.source, props.primary.clone()) }
            </div>
            { for caret }
            { for popup }
        </div>
    }
}

/// `source` with the characters of `span` wrapped in a mark. An empty span
/// (the end of the expression) marks a space after it.
fn marked(source: &str, span: Option<Range<usize>>) -> Html {
    let Some(span) = span else {
        return html! { <>{ source.to_owned() }{ "\u{200b}" }</> };
    };

    let before = source.chars().take(span.start).collect::<String>();
    let inside = source
        .chars()
        .skip(span.start)
        .take(span.end.saturating_sub(span.start))
        .collect::<String>();
    let after = source.chars().skip(span.end).collect::<String>();
    let inside = if inside.is_empty() {
        " ".to_owned()
    } else {
        inside
    };

    html! {
        <>{ before }<span class={class::MARK}>{ inside }</span>{ after }{ "\u{200b}" }</>
    }
}
