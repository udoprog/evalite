//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/evalite-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/evalite)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/evalite-sockel.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/evalite-sockel)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-evalite--sockel-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/evalite-sockel)
//!
//! Let your users write [evalite](https://docs.rs/evalite) expressions in a
//! [Yew](https://yew.rs) app, with an editor that highlights them, underlines
//! every mistake as they type and completes the names you declare.
//!
//! **[See it in the evalite playground](https://udoprog.github.io/evalite/)**,
//! which is built with it.
//!
//! - **An expression editor.** [`ExpressionEditor`] is
//!   [sockel](https://docs.rs/sockel)'s code editor with evalite's
//!   highlighting, every diagnostic underlined and marked in the gutter, and
//!   completions from your environment (as you type a name, after a `.`, or on
//!   Ctrl+Space). It compiles as you type, after a short pause, and hands you
//!   the [`Program`](https://docs.rs/evalite/latest/evalite/struct.Program.html)
//!   or the diagnostics.
//! - **Errors people can read.** [`ErrorView`] lists each diagnostic with the
//!   source line it is about and the span marked, under the editor or on its
//!   own, such as for a stored formula that no longer compiles.
//! - **Small pieces.** [`TypeBadge`], [`ExpressionPreview`] and
//!   [`NamesReference`], for tables of formulas, their types and the names an
//!   expression may use.
//! - **A formula field.** [`ExpressionField`] is the editor in a sockel
//!   `Field`: a label, a hint and the first diagnostic in the error line,
//!   for a formula in an ordinary form.
//! - **Results that stay put.** [`ResultView`] shows a run's value read by its
//!   type, or its error, in room that does not change with the value.
//! - **Sample inputs.** [`VariablesEditor`] is rows of name, type and value
//!   that say which variables the expression reads. Each value is a
//!   one-line [`ExpressionEditor`] compiled against the declared type, so
//!   `None`, `[]` and record literals take their types from it and it
//!   completes `Some(_)`, `None` and field names. [`ExpressionEditor`] takes
//!   such a type, `expected`, for any value of a known type.
//! - **Styled by sockel.** Nothing to style: the components use sockel's
//!   components and tokens, so they look like the rest of a sockel app.
//! - **Translated.** Every word comes from a catalog, [`L10N`].
//!
//! [`ExpressionEditor`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionEditor.html
//! [`ErrorView`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ErrorView.html
//! [`TypeBadge`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.TypeBadge.html
//! [`ExpressionPreview`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionPreview.html
//! [`NamesReference`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.NamesReference.html
//! [`ExpressionField`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ExpressionField.html
//! [`ResultView`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.ResultView.html
//! [`VariablesEditor`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/fn.VariablesEditor.html
//! [`L10N`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/static.L10N.html
//!
//! <br>
//!
//! ## Getting started
//!
//! Add evalite-sockel beside evalite, yew and sockel. It enables sockel's
//! `badge`, `button`, `code-editor`, `disclosure`, `field`, `input`, `layout` and
//! `notice` and `property-list` features; mount sockel's stylesheet as for any
//! sockel app.
//!
//! ```sh
//! cargo add evalite evalite-sockel
//! ```
//!
//! Keep the source in your state, the environment in an `Rc` that stays the
//! same while the names do, and the last compile's diagnostics to show:
//!
//! ```
//! use std::rc::Rc;
//!
//! use evalite::{Diagnostics, Environment, Type};
//! use evalite_sockel::{Compiled, ErrorView, ExpressionEditor};
//! use yew::prelude::*;
//!
//! #[function_component]
//! fn Formula() -> Html {
//!     let source = use_state(|| AttrValue::from("wits.min(3) + bonus"));
//!     let errors = use_state(|| None::<Rc<Diagnostics>>);
//!
//!     let environment = use_memo((), |_| {
//!         let mut env = Environment::new();
//!         let _ = env.value("wits", Type::int());
//!         let _ = env.value("bonus", Type::int());
//!         env
//!     });
//!
//!     let on_input = {
//!         let source = source.clone();
//!         Callback::from(move |text: String| source.set(text.into()))
//!     };
//!
//!     let on_compile = {
//!         let errors = errors.clone();
//!         Callback::from(move |compiled: Compiled| errors.set(compiled.err()))
//!     };
//!
//!     html! {
//!         <>
//!             <ExpressionEditor source={(*source).clone()} {on_input} {environment} {on_compile} />
//!             if let Some(diagnostics) = &*errors {
//!                 <ErrorView diagnostics={diagnostics.clone()} source={(*source).clone()} />
//!             }
//!         </>
//!     }
//! }
//! ```
//!
//! <br>
//!
//! ## Smaller pieces
//!
//! [`TypeBadge`] shows a [`Type`](https://docs.rs/evalite/latest/evalite/struct.Type.html)
//! such as `Option<Int>` as a sockel badge. [`ExpressionPreview`] shows an
//! expression highlighted like the editor's, read-only and inline, for a
//! table of stored formulas:
//!
//! ```
//! use evalite::Type;
//! use evalite_sockel::{ExpressionPreview, TypeBadge};
//! use yew::prelude::*;
//!
//! # fn rows() -> Html {
//! html! {
//!     <tr>
//!         <td><ExpressionPreview source="wits.min(3) + bonus" /></td>
//!         <td><TypeBadge ty={Type::option(Type::int())} /></td>
//!     </tr>
//! }
//! # }
//! ```
//!
//! [`NamesReference`] lists the values and functions of an environment, with
//! a filter; `on_pick` gets the name clicked, to put into the editor's source:
//!
//! ```
//! use std::rc::Rc;
//!
//! use evalite::Environment;
//! use evalite_sockel::NamesReference;
//! use yew::prelude::*;
//!
//! # fn names(environment: Rc<Environment>, source: UseStateHandle<AttrValue>) -> Html {
//! let on_pick = Callback::from(move |name: String| {
//!     source.set(format!("{}{name}", &**source).into());
//! });
//!
//! html! { <NamesReference {environment} {on_pick} /> }
//! # }
//! ```
//!
//! <br>
//!
//! ## A formula field and its result
//!
//! A field in a form compiles once the text settles and hands over the
//! program; run it, then show what it gave:
//!
//! ```
//! use std::rc::Rc;
//!
//! use evalite::{Environment, Globals, Limits, Machine, Type};
//! use evalite_sockel::{Compiled, ExpressionField, Ran, ResultView};
//! use yew::prelude::*;
//!
//! #[function_component]
//! fn Damage() -> Html {
//!     let source = use_state(|| AttrValue::from("1 + 2"));
//!     let ran = use_state(|| None::<Rc<Ran>>);
//!     let environment = use_memo((), |_| Environment::new());
//!
//!     let on_input = {
//!         let source = source.clone();
//!         Callback::from(move |text: String| source.set(text.into()))
//!     };
//!
//!     let on_compile = {
//!         let ran = ran.clone();
//!         let environment = environment.clone();
//!         Callback::from(move |compiled: Compiled| {
//!             let shown = compiled.ok().and_then(|program| {
//!                 let globals = Globals::new(&environment);
//!                 let mut machine = Machine::new(Limits::new());
//!                 let outcome = machine.load(&program, &globals).ok()?.run().ok()?;
//!                 Some(Rc::new(Ran::of(&outcome, &program)))
//!             });
//!
//!             ran.set(shown);
//!         })
//!     };
//!
//!     html! {
//!         <>
//!             <ExpressionField id="damage" label={html! { "Damage" }} source={(*source).clone()}
//!                 {on_input} {environment} {on_compile} />
//!             if let Some(ran) = &*ran {
//!                 <ResultView ran={ran.clone()} />
//!             }
//!         </>
//!     }
//! }
//! # let _ = Type::int();
//! ```
//!
//! <br>
//!
//! ## Styles and pruning
//!
//! The components write sockel's classes only, so sockel's stylesheet covers
//! them. An app that prunes sockel's CSS with
//! [`Config::prune`](https://docs.rs/sockel-build/latest/sockel_build/struct.Config.html#method.prune)
//! does not see the sockel components this crate renders, so it keeps their
//! classes with [`CLASSES`], adding evalite-sockel as a build dependency too
//! (`cargo add --build evalite-sockel`):
//!
//! ```ignore
//! sockel_build::Config::new()
//!     .prune(
//!         sockel_build::Usage::new()
//!             .source("src/")
//!             .foreign(evalite_sockel::CLASSES.iter().copied()),
//!     )
//!     .compile();
//! ```
//!
//! [`CLASSES`]: https://docs.rs/evalite-sockel/latest/evalite_sockel/constant.CLASSES.html
//!
//! <br>
//!
//! ## Translations
//!
//! The words are messages in [`L10N`], with every id under the `evalite-`
//! prefix, read through the same `ContextProvider<Phrases>` as sockel's. An
//! app adds the catalog beside sockel's, after its own, so its own
//! `evalite-*` messages win:
//!
//! ```
//! # use phrasebook::{Catalog, Lang, Phrasebook};
//! # static APP: Catalog = Catalog::new("app", &[]);
//! let book = Phrasebook::builder()
//!     .language(Lang::EN_US, "English")
//!     .catalog(&APP)
//!     .catalog(&evalite_sockel::L10N)
//!     .catalog(&sockel::phrasebookcatalog01::L10N)
//!     .build();
//! # let _ = book;
//! ```
//!
//! Label props (`label`) override a message for one use.

mod complete;
mod editor;
mod error_view;
mod field;
mod highlight;
mod names;
mod preview;
mod result_view;
mod type_badge;
mod variables;

pub use self::editor::{Compiled, ExpressionEditor, ExpressionEditorProps, Mark};
pub use self::error_view::{ErrorView, ErrorViewProps};
pub use self::field::{ExpressionField, ExpressionFieldProps};
pub use self::names::{NamesReference, NamesReferenceProps};
pub use self::preview::{ExpressionPreview, ExpressionPreviewProps};
pub use self::result_view::{Ran, ResultView, ResultViewProps};
pub use self::type_badge::{TypeBadge, TypeBadgeProps};
pub use self::variables::{
    Binding, Column, Variable, VariableError, VariablesEditor, VariablesEditorProps, bindings,
};

use phrasebook::{Catalog, Lang, Source};

/// evalite-sockel's messages, every id under the `evalite-` prefix.
pub static L10N: Catalog = Catalog::new(
    "evalite-sockel",
    &[Source::new(
        Lang::EN_US,
        include_str!("../locales/en-US.ftl"),
    )],
)
.with_prefix("evalite-");

/// The sockel classes this crate's components write, themselves and through
/// the sockel components they render, for
/// [`Usage::foreign`](https://docs.rs/sockel-build/latest/sockel_build/struct.Usage.html#method.foreign).
pub const CLASSES: &[&str] = &[
    "sockel-accent",
    "sockel-action-group",
    "sockel-active",
    "sockel-away",
    "sockel-badge",
    "sockel-band",
    "sockel-bare",
    "sockel-baseline",
    "sockel-basis-lg",
    "sockel-basis-sm",
    "sockel-between",
    "sockel-bottom",
    "sockel-boxed",
    "sockel-busy",
    "sockel-button",
    "sockel-button-after",
    "sockel-button-content",
    "sockel-button-icon",
    "sockel-button-label",
    "sockel-button-spinner",
    "sockel-card",
    "sockel-carried",
    "sockel-center",
    "sockel-checkbox",
    "sockel-checkbox-glyph",
    "sockel-checkbox-input",
    "sockel-checkbox-label",
    "sockel-checkbox-mark",
    "sockel-chrome",
    "sockel-cluster",
    "sockel-code-editor",
    "sockel-code-editor-caret",
    "sockel-code-editor-code",
    "sockel-code-editor-empty-mark",
    "sockel-code-editor-gutter",
    "sockel-code-editor-input",
    "sockel-code-editor-line",
    "sockel-code-editor-mark",
    "sockel-code-editor-marker",
    "sockel-code-editor-option",
    "sockel-code-token-comment",
    "sockel-code-token-error",
    "sockel-code-token-function",
    "sockel-code-token-keyword",
    "sockel-code-token-member",
    "sockel-code-token-name",
    "sockel-code-token-number",
    "sockel-code-token-operator",
    "sockel-code-token-punctuation",
    "sockel-code-token-string",
    "sockel-code-token-type",
    "sockel-compact",
    "sockel-confirm",
    "sockel-confirm-ask",
    "sockel-confirm-idle",
    "sockel-confirm-inline",
    "sockel-confirm-line",
    "sockel-confirm-no",
    "sockel-confirm-pair",
    "sockel-confirm-popover",
    "sockel-confirm-text",
    "sockel-confirm-yes",
    "sockel-contents",
    "sockel-context-target",
    "sockel-corner",
    "sockel-count",
    "sockel-crumb",
    "sockel-danger",
    "sockel-desktop-only",
    "sockel-dialog",
    "sockel-dialog-body",
    "sockel-dialog-foot",
    "sockel-dialog-head",
    "sockel-dialog-layer",
    "sockel-disabled",
    "sockel-disclosure",
    "sockel-disclosure-actions",
    "sockel-disclosure-body",
    "sockel-disclosure-detail",
    "sockel-disclosure-head",
    "sockel-disclosure-heading",
    "sockel-disclosure-mark",
    "sockel-disclosure-summary",
    "sockel-disclosure-title",
    "sockel-divided",
    "sockel-dot",
    "sockel-drag-handle",
    "sockel-drop-after",
    "sockel-drop-before",
    "sockel-empty",
    "sockel-end",
    "sockel-error",
    "sockel-error-box",
    "sockel-error-box-actions",
    "sockel-error-box-body",
    "sockel-error-box-cause",
    "sockel-error-box-causes",
    "sockel-error-box-close",
    "sockel-error-box-icon",
    "sockel-error-box-message",
    "sockel-error-box-title",
    "sockel-faint",
    "sockel-field",
    "sockel-field-check",
    "sockel-field-error",
    "sockel-field-head",
    "sockel-field-help",
    "sockel-field-hint",
    "sockel-field-inputs",
    "sockel-field-label",
    "sockel-field-required",
    "sockel-fieldset",
    "sockel-fill",
    "sockel-flanked",
    "sockel-flush",
    "sockel-fold",
    "sockel-fold-wide",
    "sockel-form-control",
    "sockel-form-label",
    "sockel-form-row",
    "sockel-form-rows",
    "sockel-ghost",
    "sockel-glyph",
    "sockel-glyph-only",
    "sockel-group-detached",
    "sockel-group-first",
    "sockel-group-last",
    "sockel-group-middle",
    "sockel-grow",
    "sockel-gutter-kept",
    "sockel-hair",
    "sockel-head-heading",
    "sockel-head-icon",
    "sockel-head-subtitle",
    "sockel-head-trail",
    "sockel-head-trail-item",
    "sockel-head-trail-list",
    "sockel-heading",
    "sockel-hidden",
    "sockel-hover-card",
    "sockel-icon",
    "sockel-info",
    "sockel-inline",
    "sockel-inline-edit",
    "sockel-inline-edit-actions",
    "sockel-inline-edit-editor",
    "sockel-inline-edit-field",
    "sockel-inline-edit-text",
    "sockel-input",
    "sockel-input-box",
    "sockel-input-clear",
    "sockel-input-control",
    "sockel-input-group",
    "sockel-input-mark",
    "sockel-input-unit",
    "sockel-invalid",
    "sockel-justify",
    "sockel-kbd",
    "sockel-kbd-key",
    "sockel-label",
    "sockel-legend",
    "sockel-lifted",
    "sockel-line",
    "sockel-lines",
    "sockel-link",
    "sockel-listbox",
    "sockel-listbox-group",
    "sockel-listbox-group-name",
    "sockel-listbox-option",
    "sockel-listbox-option-description",
    "sockel-listbox-option-icon",
    "sockel-listbox-option-label",
    "sockel-listbox-option-mark",
    "sockel-listbox-option-text",
    "sockel-loose",
    "sockel-match-anchor",
    "sockel-medium",
    "sockel-menu",
    "sockel-menu-heading",
    "sockel-menu-item",
    "sockel-menu-item-detail",
    "sockel-menu-item-end",
    "sockel-menu-item-hint",
    "sockel-menu-item-icon",
    "sockel-menu-item-label",
    "sockel-menu-item-mark",
    "sockel-menu-item-text",
    "sockel-menu-note",
    "sockel-menu-point",
    "sockel-menu-scrim",
    "sockel-menu-separator",
    "sockel-multiline",
    "sockel-native-select",
    "sockel-none",
    "sockel-notice",
    "sockel-notice-action",
    "sockel-notice-body",
    "sockel-notice-close",
    "sockel-notice-icon",
    "sockel-nowrap",
    "sockel-numeric",
    "sockel-off",
    "sockel-on",
    "sockel-one-row",
    "sockel-open",
    "sockel-overlay",
    "sockel-overlay-host",
    "sockel-page-head",
    "sockel-page-head-actions",
    "sockel-page-head-crumbs",
    "sockel-page-head-sep",
    "sockel-page-head-title",
    "sockel-pager",
    "sockel-pager-at",
    "sockel-pager-button",
    "sockel-pager-echo",
    "sockel-pager-edge",
    "sockel-pager-flip",
    "sockel-pager-gap",
    "sockel-pager-group",
    "sockel-pager-mark",
    "sockel-pager-position",
    "sockel-pager-range",
    "sockel-pager-shown",
    "sockel-pager-status",
    "sockel-pager-steps",
    "sockel-panel",
    "sockel-panel-box",
    "sockel-pick-row",
    "sockel-pick-row-detail",
    "sockel-pick-row-mark",
    "sockel-pick-row-name",
    "sockel-pill",
    "sockel-pill-button",
    "sockel-pill-label",
    "sockel-placed",
    "sockel-plain",
    "sockel-plain-list",
    "sockel-popover",
    "sockel-popover-body",
    "sockel-popover-head",
    "sockel-popover-scrim",
    "sockel-popover-title",
    "sockel-primary",
    "sockel-property-hint",
    "sockel-property-label",
    "sockel-property-list",
    "sockel-property-row",
    "sockel-property-value",
    "sockel-quantity",
    "sockel-quiet",
    "sockel-raised",
    "sockel-ranged",
    "sockel-reorder",
    "sockel-reorder-grab",
    "sockel-reorder-row",
    "sockel-reorder-tiled",
    "sockel-reserve",
    "sockel-row-action-danger",
    "sockel-row-actions",
    "sockel-row-list",
    "sockel-row-list-detail",
    "sockel-row-list-item",
    "sockel-row-list-main",
    "sockel-row-list-name",
    "sockel-ruled",
    "sockel-scroll-locked",
    "sockel-search",
    "sockel-search-select",
    "sockel-search-select-create",
    "sockel-search-select-detail",
    "sockel-search-select-group",
    "sockel-search-select-heading",
    "sockel-search-select-list",
    "sockel-search-select-more",
    "sockel-search-select-note",
    "sockel-search-select-typed",
    "sockel-section-head",
    "sockel-section-head-actions",
    "sockel-section-head-title",
    "sockel-select",
    "sockel-select-body",
    "sockel-select-chevron",
    "sockel-select-empty",
    "sockel-select-filter",
    "sockel-select-head",
    "sockel-select-options",
    "sockel-select-placeholder",
    "sockel-select-popup",
    "sockel-select-scrim",
    "sockel-select-sheet",
    "sockel-select-text",
    "sockel-select-title",
    "sockel-select-value",
    "sockel-selected",
    "sockel-sheet",
    "sockel-shortcut-help",
    "sockel-shortcut-help-body",
    "sockel-shortcut-help-empty",
    "sockel-shortcut-help-group",
    "sockel-shortcut-help-heading",
    "sockel-shortcut-help-keys",
    "sockel-shortcut-help-label",
    "sockel-shortcut-help-list",
    "sockel-shortcut-help-row",
    "sockel-small",
    "sockel-snug",
    "sockel-solid",
    "sockel-spin",
    "sockel-spinner",
    "sockel-split-button",
    "sockel-split-button-main",
    "sockel-split-button-toggle",
    "sockel-spread",
    "sockel-stack",
    "sockel-stacked",
    "sockel-stepper",
    "sockel-stepper-button",
    "sockel-stepper-control",
    "sockel-stepper-field",
    "sockel-stepper-unit",
    "sockel-stretch",
    "sockel-success",
    "sockel-summary",
    "sockel-switch",
    "sockel-switch-label",
    "sockel-switch-thumb",
    "sockel-switch-track",
    "sockel-tall",
    "sockel-textarea",
    "sockel-tight",
    "sockel-tint",
    "sockel-tip-anchor",
    "sockel-toast",
    "sockel-toast-action",
    "sockel-toast-body",
    "sockel-toast-close",
    "sockel-toast-host",
    "sockel-toast-icon",
    "sockel-toolbar",
    "sockel-toolbar-part",
    "sockel-toolbar-spacer",
    "sockel-tooltip",
    "sockel-top",
    "sockel-touch",
    "sockel-unset",
    "sockel-visually-hidden",
    "sockel-warning",
    "sockel-wide",
    "sockel-wide-labels",
    "sockel-wider",
    "sockel-zero",
];
