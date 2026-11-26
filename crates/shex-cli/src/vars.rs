//! Variables declared on the command line as `name:Type=value`.

use std::fmt;

use shex::{Type, Value};

/// The deepest a type written on the command line may nest.
const MAX_TYPE_DEPTH: usize = 16;

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

/// Parse `name:Type=value`, such as `strength:Int=3` or
/// `bonus:Option<Int>=some(2)`.
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

    let ty = parse_type(ty.trim())?;
    let value = parse_value(&ty, value.trim())
        .map_err(|VarError(e)| VarError(format!("the value of `{name}` is not {ty}: {e}")))?;

    Ok(Var {
        name: name.to_owned(),
        ty,
        value,
    })
}

/// Parse `Int`, `Bool`, `String`, `Option<T>` or `List<T>`.
fn parse_type(text: &str) -> Result<Type, VarError> {
    parse_type_at(text, 0)
}

fn parse_type_at(text: &str, depth: usize) -> Result<Type, VarError> {
    if depth > MAX_TYPE_DEPTH {
        return fail(format!(
            "the type nests deeper than {MAX_TYPE_DEPTH} levels"
        ));
    }

    let text = text.trim();

    match text {
        "Int" => return Ok(Type::Int),
        "Bool" => return Ok(Type::Bool),
        "String" => return Ok(Type::String),
        _ => {}
    }

    for (head, make) in [
        ("Option", Type::option as fn(Type) -> Type),
        ("List", Type::list as fn(Type) -> Type),
    ] {
        if let Some(inner) = text
            .strip_prefix(head)
            .map(str::trim_start)
            .and_then(|t| t.strip_prefix('<'))
            .and_then(|t| t.strip_suffix('>'))
        {
            let inner = parse_type_at(inner, depth.saturating_add(1))?;

            if head == "Option" && matches!(inner, Type::Option(..)) {
                return fail("`Option<Option<T>>` is not a type");
            }

            return Ok(make(inner));
        }
    }

    fail(format!(
        "unknown type `{text}`, expected Int, Bool, String, Option<T> or List<T>"
    ))
}

/// Parse a value of type `ty`.
fn parse_value(ty: &Type, text: &str) -> Result<Value, VarError> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut cursor = Cursor {
        chars: &chars,
        at: 0,
    };
    let value = cursor.value(ty, true)?;
    cursor.skip_space();

    if cursor.at < chars.len() {
        return fail(format!(
            "unexpected `{}` after the value",
            chars.iter().skip(cursor.at).collect::<String>()
        ));
    }

    Ok(value)
}

struct Cursor<'a> {
    chars: &'a [char],
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_space();

        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Result<(), VarError> {
        if self.eat(c) {
            Ok(())
        } else {
            fail(format!("expected `{c}`"))
        }
    }

    fn word(&mut self) -> String {
        self.skip_space();
        let mut word = String::new();

        while let Some(c) = self
            .peek()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        {
            word.push(c);
            self.bump();
        }

        word
    }

    /// A value of `ty`; `top` allows an unquoted string to run to the end.
    fn value(&mut self, ty: &Type, top: bool) -> Result<Value, VarError> {
        self.skip_space();

        match ty {
            Type::Int => {
                let word = self.word();
                match word.parse::<i64>() {
                    Ok(n) => Ok(Value::Int(n)),
                    Err(_) => fail(format!("`{word}` is not an Int")),
                }
            }
            Type::Bool => match self.word().as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                other => fail(format!("`{other}` is not `true` or `false`")),
            },
            Type::String => {
                if self.peek() == Some('"') {
                    self.quoted()
                } else if top {
                    let rest = self.chars.iter().skip(self.at).collect::<String>();
                    self.at = self.chars.len();
                    Ok(Value::String(rest))
                } else {
                    fail("a string inside a list or `some(..)` must be in quotes")
                }
            }
            Type::Option(inner) => {
                let word = self.word();

                match word.as_str() {
                    "none" => Ok(Value::None),
                    "some" => {
                        self.expect('(')?;
                        let value = self.value(inner, false)?;
                        self.expect(')')?;
                        Ok(Value::some(value))
                    }
                    other => fail(format!("`{other}` is not `none` or `some(..)`")),
                }
            }
            Type::List(entry) => {
                self.expect('[')?;
                let mut items = Vec::new();

                if self.eat(']') {
                    return Ok(Value::List(items));
                }

                loop {
                    items.push(self.value(entry, false)?);

                    if self.eat(']') {
                        return Ok(Value::List(items));
                    }

                    self.expect(',')?;
                }
            }
            other => fail(format!(
                "{other} values cannot be given on the command line"
            )),
        }
    }

    fn quoted(&mut self) -> Result<Value, VarError> {
        self.bump();
        let mut out = String::new();

        loop {
            match self.peek() {
                None => return fail("the string is never closed, expected a `\"`"),
                Some('"') => {
                    self.bump();
                    return Ok(Value::String(out));
                }
                Some('\\') => {
                    self.bump();

                    match self.peek() {
                        Some(c @ ('"' | '\\')) => out.push(c),
                        _ => return fail("a string may only escape `\\\"` and `\\\\`"),
                    }

                    self.bump();
                }
                Some(c) => {
                    out.push(c);
                    self.bump();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values() {
        let v = parse_var("a:Option<List<Int>>=some([1, -2])").unwrap();
        assert_eq!(
            v.value,
            Value::some(Value::List(vec![Value::Int(1), Value::Int(-2)]))
        );
        assert_eq!(
            parse_var("s:String=hi there").unwrap().value,
            Value::String("hi there".into())
        );
        assert_eq!(parse_var("b:Option<Int>=none").unwrap().value, Value::None);
    }

    #[test]
    fn rejects_bad_vars() {
        assert!(parse_var("a").is_err());
        assert!(parse_var("a:Int").is_err());
        assert!(parse_var("a:Float=1").is_err());
        assert!(parse_var("a:Int=x").is_err());
        assert!(parse_var("a:Option<Option<Int>>=none").is_err());
        assert!(parse_var("a:Int=1 2").is_err());
    }
}
