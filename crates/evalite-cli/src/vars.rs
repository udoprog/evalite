//! Variables declared on the command line as `name:Type=value`.

use std::fmt;

use evalite::{Type, Value};

/// A declared variable and the value it is bound to.
#[derive(Debug, Clone)]
pub(crate) struct Var {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) value: Value,
}

/// Why a `--var` argument is not a variable.
#[derive(Debug)]
pub(crate) struct VarError(String);

impl fmt::Display for VarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VarError {}

impl VarError {
    pub(crate) fn new(message: String) -> Self {
        Self(message)
    }
}

fn fail<T>(message: impl Into<String>) -> Result<T, VarError> {
    Err(VarError(message.into()))
}

/// Parse `name:Type=value`, such as `strength:Int=3`,
/// `bonus:Option<Int>=Some(2)` or `track:{boxes: Int}={boxes: 7}`.
pub(crate) fn parse_var(text: &str) -> Result<Var, VarError> {
    let Some((name, rest)) = text.split_once(':') else {
        return fail("expected `name:Type=value`, such as `strength:Int=3`");
    };

    let name = name.trim();

    if name.is_empty() {
        return fail("the variable has no name before `:`");
    }

    let Some((ty, value)) = rest.split_once('=') else {
        return fail(format!(
            "`{name}` has no value, expected `{name}:Type=value`"
        ));
    };

    let ty = Type::parse(ty).map_err(|e| VarError(format!("the type of `{name}`: {e}")))?;
    let value = parse_value(&ty, value.trim())
        .map_err(|VarError(e)| VarError(format!("the value of `{name}`: {e}")))?;

    Ok(Var {
        name: name.to_owned(),
        ty,
        value,
    })
}

/// Parse a value of type `ty`, as [`Value::parse`] does, except that a
/// top-level `String` may also be given without quotes.
fn parse_value(ty: &Type, text: &str) -> Result<Value, VarError> {
    if ty.is_string() && !text.starts_with('"') {
        return Ok(Value::string(text));
    }

    Value::parse(text, ty).map_err(|e| VarError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values() {
        let v = parse_var("a:Option<List<Int>>=Some([1, -2])").unwrap();
        assert_eq!(
            v.value,
            Value::some(Value::list(vec![Value::int(1), Value::int(-2)]))
        );
        assert_eq!(
            parse_var("s:String=hi there").unwrap().value,
            Value::string("hi there")
        );
        assert_eq!(
            parse_var("b:Option<Int>=None").unwrap().value,
            Value::none()
        );
    }

    #[test]
    fn rejects_bad_vars() {
        assert!(parse_var("a").is_err());
        assert!(parse_var("a:Int").is_err());
        assert!(parse_var("a:Float=1").is_err());
        assert!(parse_var("a:Int=x").is_err());
        assert!(parse_var("a:Option<Option<Int>>=None").is_ok());
        assert!(parse_var("a:Option<Option<Int>>=Some(None)").is_ok());
        assert!(parse_var("a:Option<Option<Int>>=Some(Some(1))").is_ok());
        assert!(parse_var("a:Option<Option<Int>>=Some(1)").is_err());
        assert!(parse_var("a:Int=1 2").is_err());
        assert_eq!(
            parse_var("t:{boxes: Int}={boxes: 7}").unwrap().value,
            Value::record([("boxes", Value::int(7))])
        );
        assert_eq!(
            parse_var("s:String=\"quoted\"").unwrap().value,
            Value::string("quoted")
        );
    }
}
