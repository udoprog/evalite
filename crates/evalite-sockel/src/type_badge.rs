use evalite::Type;
use sockel::Badge;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct TypeBadgeProps {
    /// The type to show, as evalite writes it: `Option<Int>`, `{boxes: Int}`.
    pub ty: Type,
    #[prop_or_default]
    pub class: Classes,
}

/// An evalite [`Type`] in a sockel badge, such as `Option<Int>`.
#[function_component]
pub fn TypeBadge(props: &TypeBadgeProps) -> Html {
    html! {
        <Badge class={props.class.clone()}>{ props.ty.to_string() }</Badge>
    }
}
