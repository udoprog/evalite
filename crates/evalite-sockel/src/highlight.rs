//! The class a highlighted span is drawn with, shared by the editor and the
//! preview so the two look alike.

use evalite::highlight::Class;

/// sockel's `code-token` class for a span of `class`, or `None` for one that
/// is drawn plain, such as whitespace.
pub(crate) fn token_class(class: Class) -> Option<&'static str> {
    Some(match class {
        Class::Keyword => sockel::class::CODE_TOKEN_KEYWORD,
        Class::Ident => sockel::class::CODE_TOKEN_NAME,
        Class::Member => sockel::class::CODE_TOKEN_MEMBER,
        Class::Number => sockel::class::CODE_TOKEN_NUMBER,
        Class::String => sockel::class::CODE_TOKEN_STRING,
        Class::Punctuation | Class::Operator => sockel::class::CODE_TOKEN_PUNCTUATION,
        Class::Error => sockel::class::CODE_TOKEN_ERROR,
        _ => return None,
    })
}
