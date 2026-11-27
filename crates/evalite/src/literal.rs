//! Types and values written as evalite prints them, such as
//! `{boxes: Int, count: List<Int>}` and `{boxes: 7, count: [3, 1]}`.
//!
//! Both parsers are iterative over an explicit stack of open groups, so no
//! input nests the call stack, and they refuse input longer than
//! [`DEFAULT_MAX_LEN`] before reading it.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use syntree::Span;

use crate::error::{Diagnostic, ErrorKind};
use crate::types::Kind;
use crate::value::key_order;
use crate::{DEFAULT_MAX_LEN, MAX_TYPE_DEPTH, Type, Value};

/// The most of the input an error quotes.
const MAX_QUOTED: usize = 32;

/// What was found where something else was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiteralFound {
    End,
    /// A word or a single character, at most [`MAX_QUOTED`] characters.
    Text(String),
}

impl fmt::Display for LiteralFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::End => f.write_str("the end"),
            Self::Text(text) => write!(f, "`{text}`"),
        }
    }
}

/// What a value of a type starts with, as an error asks for it.
pub(crate) struct Expected<'a>(pub(crate) &'a Type);

impl fmt::Display for Expected<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ty = self.0;

        if ty.is_int() {
            f.write_str("an Int")
        } else if ty.is_bool() {
            f.write_str("`true` or `false`")
        } else if ty.is_string() {
            f.write_str("a string in quotes, such as `\"Brawl\"`")
        } else if ty.as_option().is_some() {
            f.write_str("`None` or `Some(..)`")
        } else if ty.as_list().is_some() {
            write!(f, "a {ty} such as `[..]`")
        } else {
            write!(f, "a {ty} such as `{{..}}`")
        }
    }
}

impl Type {
    /// Parse a type as it is displayed: `Int`, `Bool`, `String`,
    /// `List<T>`, `Option<T>`, `Map<K, V>` or a record such as
    /// `{boxes: Int, "Brawl (bar)": Option<Int>}`. Whitespace may surround
    /// any part.
    ///
    /// # Errors
    ///
    /// A [`Diagnostic`] spanning the first mistake in `text`, when it is
    /// not a type, nests more than [`MAX_TYPE_DEPTH`] levels, names a
    /// record field twice, keys a map by other than `Int` or `String`, or
    /// is longer than [`DEFAULT_MAX_LEN`] bytes.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// let ty = Type::parse("{count: List<Int>, boxes: Option<Int>}")?;
    /// assert_eq!(ty, Type::record([
    ///     ("boxes", Type::option(Type::int())),
    ///     ("count", Type::list(Type::int())),
    /// ]));
    /// assert_eq!(Type::parse(&ty.to_string())?, ty);
    /// assert_eq!("Option<Bool>".parse::<Type>()?, Type::option(Type::bool()));
    /// assert_eq!(Type::parse("Map<Int, {dice: Int}>")?, Type::map(Type::int(), Type::record([("dice", Type::int())])));
    ///
    /// let error = Type::parse("List<Float>").unwrap_err();
    /// assert_eq!(error.to_string(), "unknown type `Float`, expected Int, Bool, String, Option<T>, List<T>, Map<K, V> or {name: T}");
    /// assert_eq!(error.span().range(), 5..10);
    ///
    /// let error = Type::parse("Map<Bool, Int>").unwrap_err();
    /// assert_eq!(error.to_string(), "a map's keys are Int or String, found Bool");
    /// assert_eq!(error.span().range(), 4..8);
    ///
    /// let error = Type::parse("Option<Int").unwrap_err();
    /// assert_eq!(error.to_string(), "expected `>`, found the end");
    ///
    /// let error = Type::parse("{a: Int, a: Bool}").unwrap_err();
    /// assert_eq!(error.to_string(), "the field `a` is given twice");
    ///
    /// let deep = format!("{}Int{}", "List<".repeat(64), ">".repeat(64));
    /// assert_eq!(Type::parse(&deep).unwrap_err().to_string(), "this type nests more than 64 levels");
    /// # Ok::<_, evalite::Diagnostic>(())
    /// ```
    pub fn parse(text: &str) -> Result<Type, Diagnostic> {
        let mut cursor = Cursor::new(text)?;
        let ty = cursor.ty()?;
        cursor.end()?;
        Ok(ty)
    }
}

impl FromStr for Type {
    type Err = Diagnostic;

    /// Parse a type as [`Type::parse`] does.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Value {
    /// Parse a value of type `ty` as it is displayed: `3`, `-2`, `true`,
    /// `"text"`, `None`, `Some(2)`, `[1, 2]`, `{boxes: 7}`, or a map such
    /// as `{1: "a", 42: "b"}`. A string is in quotes, escaping only `\"`
    /// and `\\`, a record gives each of its fields once, in any order, and
    /// a map each of its keys at most once, in any order. Whitespace may
    /// surround any part.
    ///
    /// # Errors
    ///
    /// A [`Diagnostic`] spanning the first mistake in `text`, when it is
    /// not a value of `ty` or is longer than [`DEFAULT_MAX_LEN`] bytes.
    ///
    /// ```
    /// use evalite::{Type, Value};
    ///
    /// let ty = Type::parse("{boxes: Int, count: List<Option<Int>>}")?;
    /// let value = Value::parse("{count: [Some(-1), None], boxes: 7}", &ty)?;
    /// assert_eq!(value.to_string(), "{boxes: 7, count: [Some(-1), None]}");
    /// assert_eq!(Value::parse(&value.to_string(), &ty)?, value);
    /// assert_eq!(Value::parse(r#""say \"hi\"""#, &Type::string())?, Value::string("say \"hi\""));
    ///
    /// let error = Value::parse("{boxes: 7}", &ty).unwrap_err();
    /// assert_eq!(error.to_string(), "the field `count` has no value");
    /// assert_eq!(error.span().range(), 9..10);
    ///
    /// let error = Value::parse("[1, x]", &Type::list(Type::int())).unwrap_err();
    /// assert_eq!(error.to_string(), "expected an Int, found `x`");
    /// assert_eq!(error.span().range(), 4..5);
    ///
    /// let error = Value::parse("bare", &Type::string()).unwrap_err();
    /// assert_eq!(error.to_string(), "expected a string in quotes, such as `\"Brawl\"`, found `bare`");
    ///
    /// let ty = Type::map(Type::int(), Type::string());
    /// let value = Value::parse(r#"{42: "b", 1: "a"}"#, &ty)?;
    /// assert_eq!(value, Value::map([(Value::int(1), Value::from("a")), (Value::int(42), Value::from("b"))]));
    /// assert_eq!(Value::parse(&value.to_string(), &ty)?, value);
    ///
    /// let error = Value::parse(r#"{1: "a", 1: "b"}"#, &ty).unwrap_err();
    /// assert_eq!(error.to_string(), "the key 1 is given twice");
    /// assert_eq!(error.span().range(), 9..10);
    ///
    /// let error = Value::parse("1 2", &Type::int()).unwrap_err();
    /// assert_eq!(error.to_string(), "expected the end, found `2`");
    ///
    /// let error = Value::parse("99999999999999999999", &Type::int()).unwrap_err();
    /// assert_eq!(error.to_string(), "this integer is too large, the largest is 9223372036854775807");
    /// # Ok::<_, evalite::Diagnostic>(())
    /// ```
    pub fn parse(text: &str, ty: &Type) -> Result<Value, Diagnostic> {
        let mut cursor = Cursor::new(text)?;
        let value = cursor.value(ty)?;
        cursor.end()?;
        Ok(value)
    }
}

/// A type group still open while its contents are parsed.
enum TypeFrame {
    Option,
    List,
    /// `Map<`, its key type being parsed, which starts at `at`.
    MapKey {
        at: usize,
    },
    /// `Map<K, `, its value type being parsed.
    MapValue {
        key: Type,
    },
    Record {
        fields: BTreeMap<String, Type>,
        /// The field whose type is being parsed, and where its name is.
        name: String,
        at: Span<u32>,
    },
}

/// A value group still open while its contents are parsed.
enum ValueFrame<'t> {
    Some,
    List {
        entry: &'t Type,
        items: Vec<Value>,
    },
    Record {
        ty: &'t Type,
        values: BTreeMap<String, Value>,
        name: String,
        at: Span<u32>,
    },
    /// A map's entries so far, and where each key is; the key next is
    /// parsed when `key` is `None`, else its value.
    Map {
        key_ty: &'t Type,
        value_ty: &'t Type,
        entries: Vec<(Value, Value)>,
        spans: Vec<Span<u32>>,
        key: Option<Value>,
    },
}

struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Result<Self, Diagnostic> {
        if text.len() > DEFAULT_MAX_LEN {
            return Err(Diagnostic::new(
                Span::new(0, 0),
                ErrorKind::LiteralTooLong {
                    len: text.len(),
                    max: DEFAULT_MAX_LEN,
                },
            ));
        }

        Ok(Self { text, at: 0 })
    }

    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.at = self.at.saturating_add(c.len_utf8());
        }
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn span(start: usize, end: usize) -> Span<u32> {
        // The input is at most `DEFAULT_MAX_LEN` bytes, so this fits.
        let index = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Span::new(index(start), index(end))
    }

    fn fail<T>(&self, start: usize, end: usize, kind: ErrorKind) -> Result<T, Diagnostic> {
        Err(Diagnostic::new(Self::span(start, end), kind))
    }

    /// What is next, and its span: a word, a single character, or the end.
    fn found(&mut self) -> (LiteralFound, Span<u32>) {
        self.skip_space();
        let start = self.at;
        let len = Some(word_len(self.rest()))
            .filter(|n| *n > 0)
            .or_else(|| self.peek().map(char::len_utf8));

        match len {
            None => (LiteralFound::End, Self::span(start, start)),
            Some(len) => {
                let end = start.saturating_add(len);
                let text = self.text.get(start..end).unwrap_or_default();
                let quoted = text.chars().take(MAX_QUOTED).collect();
                (LiteralFound::Text(quoted), Self::span(start, end))
            }
        }
    }

    fn unexpected<T>(
        &mut self,
        kind: impl FnOnce(LiteralFound) -> ErrorKind,
    ) -> Result<T, Diagnostic> {
        let (found, span) = self.found();
        Err(Diagnostic::new(span, kind(found)))
    }

    fn end(&mut self) -> Result<(), Diagnostic> {
        self.skip_space();

        if self.at < self.text.len() {
            return self.unexpected(ErrorKind::ExpectedEnd);
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

    fn expect(&mut self, c: char) -> Result<(), Diagnostic> {
        if self.eat(c) {
            return Ok(());
        }

        self.unexpected(|found| ErrorKind::ExpectedChar { expected: c, found })
    }

    /// The word next, of ASCII letters, digits and `_`, and where it is.
    fn word(&mut self) -> (&'a str, usize) {
        self.skip_space();
        let start = self.at;
        let len = word_len(self.rest());
        self.at = start.saturating_add(len);
        (self.text.get(start..self.at).unwrap_or_default(), start)
    }

    /// A record field's name, a word or a quoted string, and its span.
    fn field_name(&mut self) -> Result<(String, Span<u32>), Diagnostic> {
        self.skip_space();
        let start = self.at;

        let name = if self.peek() == Some('"') {
            self.quoted()?
        } else {
            let (word, _) = self.word();

            if word.is_empty() {
                return self.unexpected(ErrorKind::ExpectedFieldName);
            }

            word.to_owned()
        };

        Ok((name, Self::span(start, self.at)))
    }

    /// A string in quotes, escaping only `\"` and `\\`.
    fn quoted(&mut self) -> Result<String, Diagnostic> {
        let start = self.at;
        self.bump();
        let mut out = String::new();

        loop {
            let Some(c) = self.peek() else {
                return self.fail(start, self.at, ErrorKind::UnterminatedString);
            };

            let escape = self.at;
            self.bump();

            match c {
                '"' => return Ok(out),
                '\\' => match self.peek() {
                    Some(c @ ('"' | '\\')) => {
                        out.push(c);
                        self.bump();
                    }
                    Some(c) => {
                        let end = self.at.saturating_add(c.len_utf8());
                        return self.fail(escape, end, ErrorKind::InvalidEscape(c));
                    }
                    None => return self.fail(start, self.at, ErrorKind::UnterminatedString),
                },
                c => out.push(c),
            }
        }
    }

    fn ty(&mut self) -> Result<Type, Diagnostic> {
        let mut stack = Vec::<TypeFrame>::new();

        'open: loop {
            self.skip_space();
            let start = self.at;

            // The type about to be read is one level below every open group.
            if stack.len() >= MAX_TYPE_DEPTH {
                return self.fail(start, start, ErrorKind::LiteralTooDeep);
            }

            let mut done = if self.eat('{') {
                if self.eat('}') {
                    Type::record(Vec::<(String, Type)>::new())
                } else {
                    let (name, at) = self.field_name()?;
                    self.expect(':')?;
                    stack.push(TypeFrame::Record {
                        fields: BTreeMap::new(),
                        name,
                        at,
                    });
                    continue 'open;
                }
            } else {
                let (word, start) = self.word();

                match word {
                    "Int" => Type::int(),
                    "Bool" => Type::bool(),
                    "String" => Type::string(),
                    "Map" => {
                        self.expect('<')?;
                        self.skip_space();
                        stack.push(TypeFrame::MapKey { at: self.at });
                        continue 'open;
                    }
                    "Option" | "List" => {
                        self.expect('<')?;
                        stack.push(if word == "List" {
                            TypeFrame::List
                        } else {
                            TypeFrame::Option
                        });
                        continue 'open;
                    }
                    "" => return self.unexpected(ErrorKind::ExpectedType),
                    other => {
                        let end = start.saturating_add(other.len());
                        let name = other.chars().take(MAX_QUOTED).collect();
                        return self.fail(start, end, ErrorKind::UnknownType(name));
                    }
                }
            };

            // Close every group `done` completes.
            loop {
                match stack.pop() {
                    None => return Ok(done),
                    Some(TypeFrame::Option) => {
                        self.expect('>')?;
                        done = Type::option(done);
                    }
                    Some(TypeFrame::List) => {
                        self.expect('>')?;
                        done = Type::list(done);
                    }
                    Some(TypeFrame::MapKey { at }) => {
                        if !done.is_int() && !done.is_string() {
                            return self.fail(at, self.at, ErrorKind::MapKeyType(done));
                        }

                        self.expect(',')?;
                        stack.push(TypeFrame::MapValue { key: done });
                        continue 'open;
                    }
                    Some(TypeFrame::MapValue { key }) => {
                        self.expect('>')?;
                        done = Type::map(key, done);
                    }
                    Some(TypeFrame::Record {
                        mut fields,
                        name,
                        at,
                    }) => {
                        if fields.contains_key(&name) {
                            return Err(Diagnostic::new(at, ErrorKind::DuplicateField(name)));
                        }

                        fields.insert(name, done);

                        if self.eat(',') {
                            let (name, at) = self.field_name()?;
                            self.expect(':')?;
                            stack.push(TypeFrame::Record { fields, name, at });
                            continue 'open;
                        }

                        if !self.eat('}') {
                            return self.unexpected(|found| ErrorKind::ExpectedChar {
                                expected: '}',
                                found,
                            });
                        }

                        done = Type::record(fields);
                    }
                }
            }
        }
    }

    fn value(&mut self, ty: &Type) -> Result<Value, Diagnostic> {
        let mut stack = Vec::<ValueFrame<'_>>::new();
        let mut ty = ty;

        'open: loop {
            self.skip_space();

            let mut done = if ty.is_int() {
                let start = self.at;

                if self.peek() == Some('-') {
                    self.bump();
                }

                let len = word_len(self.rest());
                let digits = self.rest().get(..len).unwrap_or_default();

                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    self.at = start;
                    return self.unexpected(|found| ErrorKind::ExpectedValue {
                        ty: ty.clone(),
                        found,
                    });
                }

                self.at = self.at.saturating_add(len);
                let text = self.text.get(start..self.at).unwrap_or_default();

                match text.parse::<i64>() {
                    Ok(n) => Value::int(n),
                    Err(_) if text.starts_with('-') => {
                        return self.fail(start, self.at, ErrorKind::IntegerTooSmall);
                    }
                    Err(_) => return self.fail(start, self.at, ErrorKind::IntegerTooLarge),
                }
            } else if ty.is_bool() {
                match self.word() {
                    ("true", _) => Value::bool(true),
                    ("false", _) => Value::bool(false),
                    (_, start) => {
                        self.at = start;
                        return self.unexpected(|found| ErrorKind::ExpectedValue {
                            ty: ty.clone(),
                            found,
                        });
                    }
                }
            } else if ty.is_string() {
                if self.peek() != Some('"') {
                    return self.unexpected(|found| ErrorKind::ExpectedValue {
                        ty: ty.clone(),
                        found,
                    });
                }

                Value::string(self.quoted()?)
            } else if let Some(inner) = ty.as_option() {
                match self.word() {
                    ("None", _) => Value::none(),
                    ("Some", _) => {
                        self.expect('(')?;
                        stack.push(ValueFrame::Some);
                        ty = inner;
                        continue 'open;
                    }
                    (_, start) => {
                        self.at = start;
                        return self.unexpected(|found| ErrorKind::ExpectedValue {
                            ty: ty.clone(),
                            found,
                        });
                    }
                }
            } else if let Some(entry) = ty.as_list() {
                if !self.eat('[') {
                    return self.unexpected(|found| ErrorKind::ExpectedValue {
                        ty: ty.clone(),
                        found,
                    });
                }

                if self.eat(']') {
                    Value::list([])
                } else {
                    stack.push(ValueFrame::List {
                        entry,
                        items: Vec::new(),
                    });
                    ty = entry;
                    continue 'open;
                }
            } else if let Some((key_ty, value_ty)) = ty.as_map() {
                if !self.eat('{') {
                    return self.unexpected(|found| ErrorKind::ExpectedValue {
                        ty: ty.clone(),
                        found,
                    });
                }

                if self.eat('}') {
                    Value::map([])
                } else {
                    self.skip_space();
                    stack.push(ValueFrame::Map {
                        key_ty,
                        value_ty,
                        entries: Vec::new(),
                        spans: vec![Self::span(self.at, self.at)],
                        key: None,
                    });
                    ty = key_ty;
                    continue 'open;
                }
            } else {
                if !self.eat('{') {
                    return self.unexpected(|found| ErrorKind::ExpectedValue {
                        ty: ty.clone(),
                        found,
                    });
                }

                let values = BTreeMap::new();

                if self.eat('}') {
                    self.finish_record(ty, values)?
                } else {
                    let record = ty;
                    let (name, at) = self.field_name()?;
                    ty = field_type(record, &name, at)?;
                    self.expect(':')?;
                    stack.push(ValueFrame::Record {
                        ty: record,
                        values,
                        name,
                        at,
                    });
                    continue 'open;
                }
            };

            // Close every group `done` completes.
            loop {
                match stack.pop() {
                    None => return Ok(done),
                    Some(ValueFrame::Some) => {
                        self.expect(')')?;
                        done = Value::some(done);
                    }
                    Some(ValueFrame::List { entry, mut items }) => {
                        items.push(done);

                        if self.eat(',') {
                            stack.push(ValueFrame::List { entry, items });
                            ty = entry;
                            continue 'open;
                        }

                        if !self.eat(']') {
                            return self.unexpected(|found| ErrorKind::ExpectedChar {
                                expected: ']',
                                found,
                            });
                        }

                        done = Value::list(items);
                    }
                    Some(ValueFrame::Record {
                        ty: record,
                        mut values,
                        name,
                        at,
                    }) => {
                        if values.contains_key(&name) {
                            return Err(Diagnostic::new(at, ErrorKind::DuplicateField(name)));
                        }

                        values.insert(name, done);

                        if self.eat(',') {
                            let (name, at) = self.field_name()?;
                            ty = field_type(record, &name, at)?;
                            self.expect(':')?;
                            stack.push(ValueFrame::Record {
                                ty: record,
                                values,
                                name,
                                at,
                            });
                            continue 'open;
                        }

                        if !self.eat('}') {
                            return self.unexpected(|found| ErrorKind::ExpectedChar {
                                expected: '}',
                                found,
                            });
                        }

                        done = self.finish_record(record, values)?;
                    }
                    Some(ValueFrame::Map {
                        key_ty,
                        value_ty,
                        entries,
                        mut spans,
                        key: None,
                    }) => {
                        // The key's span runs from where it starts to here.
                        if let Some(span) = spans.last_mut() {
                            *span = Span::new(span.start, Self::span(self.at, self.at).end);
                        }

                        self.expect(':')?;
                        stack.push(ValueFrame::Map {
                            key_ty,
                            value_ty,
                            entries,
                            spans,
                            key: Some(done),
                        });
                        ty = value_ty;
                        continue 'open;
                    }
                    Some(ValueFrame::Map {
                        key_ty,
                        value_ty,
                        mut entries,
                        mut spans,
                        key: Some(key),
                    }) => {
                        entries.push((key, done));

                        if self.eat(',') {
                            self.skip_space();
                            spans.push(Self::span(self.at, self.at));
                            stack.push(ValueFrame::Map {
                                key_ty,
                                value_ty,
                                entries,
                                spans,
                                key: None,
                            });
                            ty = key_ty;
                            continue 'open;
                        }

                        if !self.eat('}') {
                            return self.unexpected(|found| ErrorKind::ExpectedChar {
                                expected: '}',
                                found,
                            });
                        }

                        done = finish_map(entries, &spans)?;
                    }
                }
            }
        }
    }

    /// The record of `values`, which the `}` just read closes, if it gives
    /// every field of `ty`.
    fn finish_record(
        &self,
        ty: &Type,
        values: BTreeMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        let missing = ty
            .as_record()
            .into_iter()
            .flatten()
            .find(|(name, _)| !values.contains_key(*name));

        if let Some((name, _)) = missing {
            let close = self.at.saturating_sub(1);
            return self.fail(close, self.at, ErrorKind::MissingField(name.to_owned()));
        }

        Ok(Value::record(values))
    }
}

/// The map of `entries`, whose keys are at `spans`, if no key is given
/// twice; the second of a repeated key is the one refused.
fn finish_map(entries: Vec<(Value, Value)>, spans: &[Span<u32>]) -> Result<Value, Diagnostic> {
    let mut order = (0..entries.len()).collect::<Vec<_>>();
    let key = |n: usize| entries.get(n).map(|(k, _)| k);
    // Stable, so of a repeated key the earlier is first.
    order.sort_by(|a, b| match (key(*a), key(*b)) {
        (Some(a), Some(b)) => key_order(a, b),
        _ => std::cmp::Ordering::Equal,
    });

    for pair in order.windows(2) {
        if let [a, b] = pair
            && let (Some(first), Some(second)) = (key(*a), key(*b))
            && key_order(first, second) == std::cmp::Ordering::Equal
        {
            let at = spans.get(*b).copied().unwrap_or(Span::new(0, 0));
            return Err(Diagnostic::new(
                at,
                ErrorKind::DuplicateKey(second.to_string()),
            ));
        }
    }

    Ok(Value::map(entries))
}

/// The type of field `name` of the record `ty`.
fn field_type<'t>(ty: &'t Type, name: &str, at: Span<u32>) -> Result<&'t Type, Diagnostic> {
    let field = match &ty.0 {
        Kind::Record(fields) => fields.get(name),
        _ => None,
    };

    field.ok_or_else(|| {
        Diagnostic::new(
            at,
            ErrorKind::NoSuchField {
                ty: ty.clone(),
                field: name.to_owned(),
            },
        )
    })
}

/// How many bytes of ASCII letters, digits and `_` `text` starts with.
fn word_len(text: &str) -> usize {
    text.bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count()
}
