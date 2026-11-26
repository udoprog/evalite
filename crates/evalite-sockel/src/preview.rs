use evalite::highlight::highlight;
use yew::prelude::*;

use crate::highlight::token_class;

#[derive(Properties, PartialEq)]
pub struct ExpressionPreviewProps {
    /// The expression.
    pub source: AttrValue,
    #[prop_or_default]
    pub class: Classes,
}

/// An expression highlighted as [`ExpressionEditor`](crate::ExpressionEditor)
/// does, read-only and inline, for a table of stored formulas or a history
/// list. It does not compile the source, so it shows an expression that no
/// longer compiles as it is written.
#[function_component]
pub fn ExpressionPreview(props: &ExpressionPreviewProps) -> Html {
    let source = props.source.as_str();

    let parts = highlight(source).map(|token| {
        let text = source.get(token.span).unwrap_or_default();

        match token_class(token.class) {
            Some(class) => html! { <span class={class}>{ text }</span> },
            None => html! { { text } },
        }
    });

    html! {
        <code class={props.class.clone()}>{ for parts }</code>
    }
}
