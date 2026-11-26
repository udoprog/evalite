//! Context variables as the page takes them: a type written as evalite
//! prints one (`Int`, `Option<Int>`, `List<Int>`, `{boxes: Int}`), and a
//! value in the same literal syntax as `evalite-cli`'s `--var`, extended
//! with records (`3`, `true`, `"text"`, `none`, `some(2)`, `[1, 2]`,
//! `{boxes: 7}`).

use evalite::{MAX_TYPE_DEPTH, Type, Value};

type Result<T> = std::result::Result<T, String>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into())
}

/// Parse a type, such as `Option<List<Int>>` or `{boxes: Int, count: List<Int>}`.
pub fn parse_type(text: &str) -> Result<Type> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut cursor = Cursor {
        chars: &chars,
        at: 0,
    };
    let ty = cursor.ty(0)?;
    cursor.end()?;
    Ok(ty)
}

/// Parse a value of type `ty`.
pub fn parse_value(ty: &Type, text: &str) -> Result<Value> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut cursor = Cursor {
        chars: &chars,
        at: 0,
    };
    let value = cursor.value(ty)?;
    cursor.end()?;
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

    fn end(&mut self) -> Result<()> {
        self.skip_space();

        if self.at < self.chars.len() {
            let rest = self.chars.iter().skip(self.at).collect::<String>();
            return fail(format!("unexpected `{rest}` at the end"));
        }

        Ok(())
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

    fn expect(&mut self, c: char) -> Result<()> {
        if self.eat(c) {
            Ok(())
        } else {
            match self.peek() {
                Some(found) => fail(format!("expected `{c}`, found `{found}`")),
                None => fail(format!("expected `{c}`, found the end")),
            }
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

    /// A record field's name: a word, or a quoted string.
    fn field_name(&mut self) -> Result<String> {
        self.skip_space();

        if self.peek() == Some('"') {
            return self.quoted();
        }

        let word = self.word();

        if word.is_empty() {
            return fail("expected a field name");
        }

        Ok(word)
    }

    fn ty(&mut self, depth: usize) -> Result<Type> {
        if depth > MAX_TYPE_DEPTH {
            return fail(format!(
                "the type nests deeper than {MAX_TYPE_DEPTH} levels"
            ));
        }

        let depth = depth.saturating_add(1);

        if self.eat('{') {
            let mut fields = Vec::new();

            loop {
                let name = self.field_name()?;
                self.expect(':')?;
                fields.push((name, self.ty(depth)?));

                if self.eat('}') {
                    return Ok(Type::record(fields));
                }

                self.expect(',')?;
            }
        }

        let word = self.word();

        match word.as_str() {
            "Int" => Ok(Type::int()),
            "Bool" => Ok(Type::bool()),
            "String" => Ok(Type::string()),
            "Option" | "List" => {
                self.expect('<')?;
                let inner = self.ty(depth)?;
                self.expect('>')?;

                if word == "List" {
                    Ok(Type::list(inner))
                } else {
                    Ok(Type::option(inner))
                }
            }
            "" => fail("expected a type"),
            other => fail(format!(
                "unknown type `{other}`, expected Int, Bool, String, Option<T>, List<T> or {{name: T}}"
            )),
        }
    }

    fn value(&mut self, ty: &Type) -> Result<Value> {
        self.skip_space();

        if ty.is_int() {
            let negative = self.eat('-');
            let word = self.word();
            let text = if negative { format!("-{word}") } else { word };

            return match text.parse::<i64>() {
                Ok(n) => Ok(Value::int(n)),
                Err(_) => fail(format!("`{text}` is not an Int")),
            };
        }

        if ty.is_bool() {
            return match self.word().as_str() {
                "true" => Ok(Value::bool(true)),
                "false" => Ok(Value::bool(false)),
                other => fail(format!("`{other}` is not `true` or `false`")),
            };
        }

        if ty.is_string() {
            return if self.peek() == Some('"') {
                self.quoted().map(Value::string)
            } else {
                fail("a string is written in quotes, such as `\"Brawl\"`")
            };
        }

        if let Some(inner) = ty.as_option() {
            return match self.word().as_str() {
                "none" => Ok(Value::none()),
                "some" => {
                    self.expect('(')?;
                    let value = self.value(inner)?;
                    self.expect(')')?;
                    Ok(Value::some(value))
                }
                other => fail(format!("`{other}` is not `none` or `some(..)`")),
            };
        }

        if let Some(entry) = ty.as_list() {
            self.expect('[')?;
            let mut items = Vec::new();

            if self.eat(']') {
                return Ok(Value::list(items));
            }

            loop {
                items.push(self.value(entry)?);

                if self.eat(']') {
                    return Ok(Value::list(items));
                }

                self.expect(',')?;
            }
        }

        if let Some(fields) = ty.as_record() {
            self.expect('{')?;
            let mut values = Vec::new();

            if !self.eat('}') {
                loop {
                    let name = self.field_name()?;
                    self.expect(':')?;

                    let Some((_, field)) = fields.clone().find(|(n, _)| *n == name) else {
                        return fail(format!("`{ty}` has no field `{name}`"));
                    };

                    values.push((name, self.value(field)?));

                    if self.eat('}') {
                        break;
                    }

                    self.expect(',')?;
                }
            }

            if let Some((missing, _)) = fields
                .clone()
                .find(|(n, _)| !values.iter().any(|(v, _)| v == n))
            {
                return fail(format!("the field `{missing}` has no value"));
            }

            return Ok(Value::record(values));
        }

        fail(format!("{ty} values cannot be written here"))
    }

    fn quoted(&mut self) -> Result<String> {
        self.bump();
        let mut out = String::new();

        loop {
            match self.peek() {
                None => return fail("the string is never closed, expected a `\"`"),
                Some('"') => {
                    self.bump();
                    return Ok(out);
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
    fn parses_types() {
        assert_eq!(
            parse_type("Option<List<Int>>").unwrap(),
            Type::option(Type::list(Type::int()))
        );
        let track = parse_type("{boxes: Int, count: List<Int>}").unwrap();
        assert_eq!(track.to_string(), "{boxes: Int, count: List<Int>}");
        let nested = parse_type("Option<Option<Int>>").unwrap();
        assert_eq!(nested.to_string(), "Option<Option<Int>>");
        assert_eq!(
            parse_value(&nested, "some(none)").unwrap().to_string(),
            "some(none)"
        );
        assert!(parse_value(&nested, "some(1)").is_err());
        assert!(parse_type("Float").is_err());
        assert!(parse_type("Int x").is_err());
    }

    #[test]
    fn parses_values() {
        let ty = parse_type("Option<List<Int>>").unwrap();
        assert_eq!(
            parse_value(&ty, "some([1, -2])").unwrap(),
            Value::some(Value::list([Value::int(1), Value::int(-2)]))
        );
        let track = parse_type("{boxes: Int, count: List<Int>}").unwrap();
        assert_eq!(
            parse_value(&track, "{count: [1], boxes: 7}").unwrap(),
            Value::record([
                ("boxes", Value::int(7)),
                ("count", Value::list([Value::int(1)]))
            ])
        );
        assert!(parse_value(&track, "{boxes: 7}").is_err());
        assert!(parse_value(&Type::int(), "x").is_err());
        assert!(parse_value(&Type::string(), "bare").is_err());
    }
}
