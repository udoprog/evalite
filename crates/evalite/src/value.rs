//! Values bound to a program and given back by one.

use std::collections::BTreeMap;
use std::collections::btree_map;
use std::fmt;
use std::iter::FusedIterator;

use crate::message::Found;
use crate::types::is_field_name;

/// A value bound to a program or given back by one: an `Int`, a `Bool`, a
/// `String`, a list, a record, or an option.
///
/// It is opaque. Build one with [`Value::int`], [`Value::bool`],
/// [`Value::string`], [`Value::list`], [`Value::record`], [`Value::none`]
/// and [`Value::some`] (or `From`), and read one back with the `as_*`
/// try-decoders, which give `None` when the value is of another shape and
/// never panic.
///
/// Decoding is a family of borrowing try-functions, one per shape, rather
/// than a single `decode::<T>()`: each one maps to one of evalite's types,
/// reads without copying, and nested values decode by chaining them inside
/// an `Option`-returning closure or function with `?`.
///
/// An `Option<T>` is [`Value::none`] or [`Value::some`] of a `T`, never a
/// bare `T`.
///
/// ```
/// use evalite::Value;
///
/// let health = Value::record([
///     ("boxes", Value::int(7)),
///     ("count", Value::list([Value::int(3), Value::int(1)])),
/// ]);
///
/// let count = health.field("count").and_then(Value::as_list);
/// let count: Option<Vec<i64>> = count.map(|c| c.iter().filter_map(Value::as_int).collect());
/// assert_eq!(count, Some(vec![3, 1]));
/// assert_eq!(health.to_string(), "{boxes: 7, count: [3, 1]}");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value(pub(crate) Repr);

/// What a [`Value`] holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Repr {
    Int(i64),
    Bool(bool),
    String(String),
    List(Vec<Value>),
    /// A record's fields by name.
    Record(BTreeMap<String, Value>),
    /// An absent `Option<T>`.
    None,
    /// A present `Option<T>`.
    Some(Box<Value>),
}

impl Value {
    /// An `Int`.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::int(i64::MIN).as_int(), Some(i64::MIN));
    /// assert_eq!(Value::int(i64::MAX).as_int(), Some(i64::MAX));
    /// assert_eq!(Value::int(3), Value::from(3));
    /// ```
    pub const fn int(value: i64) -> Self {
        Self(Repr::Int(value))
    }

    /// A `Bool`.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::bool(true).as_bool(), Some(true));
    /// assert_eq!(Value::bool(false), Value::from(false));
    /// ```
    pub const fn bool(value: bool) -> Self {
        Self(Repr::Bool(value))
    }

    /// A `String`.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::string("Essence").as_str(), Some("Essence"));
    /// assert_eq!(Value::string(String::from("a")), Value::from("a"));
    /// ```
    pub fn string(value: impl Into<String>) -> Self {
        Self(Repr::String(value.into()))
    }

    /// A list of these entries, which should all be of one type for it to
    /// bind; binding checks that.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// let list = Value::list([Value::int(1), Value::int(2)]);
    /// assert_eq!(list.as_list().map(<[Value]>::len), Some(2));
    /// assert_eq!(list.to_string(), "[1, 2]");
    /// ```
    pub fn list(entries: impl IntoIterator<Item = Value>) -> Self {
        Self(Repr::List(entries.into_iter().collect()))
    }

    /// A record of these fields. A name given twice keeps its last value.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// let record = Value::record([("boxes", Value::int(7)), ("Brawl (bar)", Value::int(1))]);
    /// assert_eq!(record.field("boxes"), Some(&Value::int(7)));
    /// assert_eq!(record.to_string(), r#"{"Brawl (bar)": 1, boxes: 7}"#);
    /// ```
    pub fn record<K>(fields: impl IntoIterator<Item = (K, Value)>) -> Self
    where
        K: Into<String>,
    {
        Self(Repr::Record(
            fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        ))
    }

    /// An absent `Option<T>`.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::none().as_option(), Some(None));
    /// assert_eq!(Value::none().to_string(), "none");
    /// ```
    pub const fn none() -> Self {
        Self(Repr::None)
    }

    /// A present `Option<T>`.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// let value = Value::some(Value::int(2));
    /// assert_eq!(value.as_option(), Some(Some(&Value::int(2))));
    /// assert_eq!(value.to_string(), "some(2)");
    /// ```
    pub fn some(value: Value) -> Self {
        Self(Repr::Some(Box::new(value)))
    }

    /// The `Int` it is, or `None` if it is anything else.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::int(-4).as_int(), Some(-4));
    /// assert_eq!(Value::int(i64::MIN).as_int(), Some(i64::MIN));
    /// assert_eq!(Value::int(i64::MAX).as_int(), Some(i64::MAX));
    ///
    /// assert_eq!(Value::bool(true).as_int(), None);
    /// assert_eq!(Value::string("1").as_int(), None);
    /// assert_eq!(Value::list([Value::int(1)]).as_int(), None);
    /// assert_eq!(Value::record([("a", Value::int(1))]).as_int(), None);
    /// assert_eq!(Value::none().as_int(), None);
    /// // An option is never its value: unwrap it with `as_option` first.
    /// assert_eq!(Value::some(Value::int(1)).as_int(), None);
    /// ```
    pub fn as_int(&self) -> Option<i64> {
        match self.0 {
            Repr::Int(n) => Some(n),
            _ => None,
        }
    }

    /// The `Bool` it is, or `None` if it is anything else.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::bool(true).as_bool(), Some(true));
    /// assert_eq!(Value::bool(false).as_bool(), Some(false));
    ///
    /// assert_eq!(Value::int(1).as_bool(), None);
    /// assert_eq!(Value::string("true").as_bool(), None);
    /// assert_eq!(Value::list([Value::bool(true)]).as_bool(), None);
    /// assert_eq!(Value::record([("a", Value::bool(true))]).as_bool(), None);
    /// assert_eq!(Value::none().as_bool(), None);
    /// assert_eq!(Value::some(Value::bool(true)).as_bool(), None);
    /// ```
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            Repr::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// The `String` it is, or `None` if it is anything else.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::string("Brawl (bar)").as_str(), Some("Brawl (bar)"));
    /// assert_eq!(Value::string("").as_str(), Some(""));
    ///
    /// assert_eq!(Value::int(1).as_str(), None);
    /// assert_eq!(Value::bool(true).as_str(), None);
    /// assert_eq!(Value::list([Value::string("a")]).as_str(), None);
    /// assert_eq!(Value::record([("a", Value::string("a"))]).as_str(), None);
    /// assert_eq!(Value::none().as_str(), None);
    /// assert_eq!(Value::some(Value::string("a")).as_str(), None);
    /// ```
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            Repr::String(s) => Some(s),
            _ => None,
        }
    }

    /// The entries of the list it is, or `None` if it is anything else.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// // A `List<Option<Int>>`.
    /// let list = Value::list([Value::some(Value::int(i64::MAX)), Value::none()]);
    /// let entries: Option<Vec<Option<i64>>> = list
    ///     .as_list()
    ///     .and_then(|l| l.iter().map(|v| v.as_option().map(|o| o.and_then(Value::as_int))).collect());
    /// assert_eq!(entries, Some(vec![Some(i64::MAX), None]));
    ///
    /// assert_eq!(Value::list([]).as_list(), Some(&[][..]));
    ///
    /// assert_eq!(Value::int(1).as_list(), None);
    /// assert_eq!(Value::bool(true).as_list(), None);
    /// assert_eq!(Value::string("[1]").as_list(), None);
    /// assert_eq!(Value::record([("a", Value::list([]))]).as_list(), None);
    /// assert_eq!(Value::none().as_list(), None);
    /// assert_eq!(Value::some(Value::list([])).as_list(), None);
    /// ```
    pub fn as_list(&self) -> Option<&[Value]> {
        match &self.0 {
            Repr::List(items) => Some(items),
            _ => None,
        }
    }

    /// The fields of the record it is, by name in order, or `None` if it is
    /// anything else.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// let record = Value::record([("count", Value::list([Value::int(3)])), ("boxes", Value::int(7))]);
    /// let fields: Vec<(&str, String)> = record
    ///     .as_record()
    ///     .map(|fields| fields.map(|(name, value)| (name, value.to_string())).collect())
    ///     .unwrap_or_default();
    /// assert_eq!(fields, [("boxes", "7".to_owned()), ("count", "[3]".to_owned())]);
    /// assert_eq!(record.as_record().map(|f| f.len()), Some(2));
    ///
    /// assert!(Value::int(1).as_record().is_none());
    /// assert!(Value::bool(true).as_record().is_none());
    /// assert!(Value::string("{}").as_record().is_none());
    /// assert!(Value::list([]).as_record().is_none());
    /// assert!(Value::none().as_record().is_none());
    /// assert!(Value::some(Value::record([("a", Value::int(1))])).as_record().is_none());
    /// ```
    pub fn as_record(&self) -> Option<Fields<'_>> {
        match &self.0 {
            Repr::Record(fields) => Some(Fields {
                iter: fields.iter(),
            }),
            _ => None,
        }
    }

    /// The field `name` of the record it is, or `None` if it has no such
    /// field or is not a record.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// let health = Value::record([
    ///     ("boxes", Value::int(7)),
    ///     ("count", Value::list([Value::int(3), Value::int(i64::MIN)])),
    /// ]);
    /// assert_eq!(health.field("boxes").and_then(Value::as_int), Some(7));
    /// let last = health.field("count").and_then(Value::as_list).and_then(|c| c.last());
    /// assert_eq!(last.and_then(Value::as_int), Some(i64::MIN));
    ///
    /// assert_eq!(health.field("missing"), None);
    /// assert_eq!(Value::int(1).field("boxes"), None);
    /// assert_eq!(Value::bool(true).field("boxes"), None);
    /// assert_eq!(Value::string("boxes").field("boxes"), None);
    /// assert_eq!(Value::list([]).field("boxes"), None);
    /// assert_eq!(Value::none().field("boxes"), None);
    /// assert_eq!(Value::some(health.clone()).field("boxes"), None);
    /// ```
    pub fn field(&self, name: &str) -> Option<&Value> {
        match &self.0 {
            Repr::Record(fields) => fields.get(name),
            _ => None,
        }
    }

    /// The option it is: `Some(None)` when absent, `Some(Some(value))` when
    /// present, and `None` if it is not an option.
    ///
    /// ```
    /// use evalite::Value;
    ///
    /// assert_eq!(Value::none().as_option(), Some(None));
    /// let some = Value::some(Value::int(i64::MIN));
    /// assert_eq!(some.as_option().flatten().and_then(Value::as_int), Some(i64::MIN));
    ///
    /// // An `Option<List<Int>>`.
    /// let some = Value::some(Value::list([Value::int(1)]));
    /// assert_eq!(some.as_option().flatten().and_then(Value::as_list).map(<[Value]>::len), Some(1));
    ///
    /// assert_eq!(Value::int(1).as_option(), None);
    /// assert_eq!(Value::bool(false).as_option(), None);
    /// assert_eq!(Value::string("none").as_option(), None);
    /// assert_eq!(Value::list([]).as_option(), None);
    /// assert_eq!(Value::record([("a", Value::none())]).as_option(), None);
    /// ```
    pub fn as_option(&self) -> Option<Option<&Value>> {
        match &self.0 {
            Repr::None => Some(None),
            Repr::Some(value) => Some(Some(value)),
            _ => None,
        }
    }

    /// What it is, as an error names it.
    pub(crate) fn describe(&self) -> Found {
        match self.0 {
            Repr::Int(_) => Found::Int,
            Repr::Bool(_) => Found::Bool,
            Repr::String(_) => Found::String,
            Repr::List(_) => Found::List,
            Repr::Record(_) => Found::Record,
            Repr::None => Found::None,
            Repr::Some(_) => Found::Some,
        }
    }
}

/// The fields of a record [`Value`], by name in order, from
/// [`Value::as_record`].
///
/// ```
/// use evalite::Value;
///
/// let record = Value::record([("b", Value::int(2)), ("a", Value::int(1))]);
/// let mut fields = record.as_record().into_iter().flatten();
/// assert_eq!(fields.next(), Some(("a", &Value::int(1))));
/// assert_eq!(fields.next(), Some(("b", &Value::int(2))));
/// assert_eq!(fields.next(), None);
/// ```
#[derive(Debug, Clone)]
pub struct Fields<'a> {
    iter: btree_map::Iter<'a, String, Value>,
}

impl<'a> Iterator for Fields<'a> {
    type Item = (&'a str, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        let (name, value) = self.iter.next()?;
        Some((name, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl DoubleEndedIterator for Fields<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let (name, value) = self.iter.next_back()?;
        Some((name, value))
    }
}

impl ExactSizeIterator for Fields<'_> {}

impl FusedIterator for Fields<'_> {}

impl From<i64> for Value {
    /// An `Int`, as [`Value::int`].
    fn from(value: i64) -> Self {
        Self::int(value)
    }
}

impl From<bool> for Value {
    /// A `Bool`, as [`Value::bool`].
    fn from(value: bool) -> Self {
        Self::bool(value)
    }
}

impl From<&str> for Value {
    /// A `String`, as [`Value::string`].
    fn from(value: &str) -> Self {
        Self::string(value)
    }
}

impl From<String> for Value {
    /// A `String`, as [`Value::string`].
    fn from(value: String) -> Self {
        Self::string(value)
    }
}

enum Piece<'a> {
    Value(&'a Value),
    Text(&'static str),
    Key(&'a str),
}

impl fmt::Display for Value {
    /// Values as evalite writes them: `3`, `true`, `"Brawl"`, `[1, 2]`,
    /// `{athletics: 2, "Brawl (bar)": 1}`, `none` and `some(2)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut stack = vec![Piece::Value(self)];

        while let Some(piece) = stack.pop() {
            match piece {
                Piece::Text(text) => f.write_str(text)?,
                Piece::Key(name) if is_field_name(name) => write!(f, "{name}: ")?,
                Piece::Key(name) => {
                    write_string(f, name)?;
                    f.write_str(": ")?;
                }
                Piece::Value(value) => match &value.0 {
                    Repr::Int(n) => write!(f, "{n}")?,
                    Repr::Bool(b) => write!(f, "{b}")?,
                    Repr::String(s) => write_string(f, s)?,
                    Repr::None => f.write_str("none")?,
                    Repr::Some(value) => {
                        f.write_str("some(")?;
                        stack.push(Piece::Text(")"));
                        stack.push(Piece::Value(value));
                    }
                    Repr::List(items) => {
                        f.write_str("[")?;
                        stack.push(Piece::Text("]"));

                        for (n, item) in items.iter().enumerate().rev() {
                            stack.push(Piece::Value(item));

                            if n > 0 {
                                stack.push(Piece::Text(", "));
                            }
                        }
                    }
                    Repr::Record(fields) => {
                        f.write_str("{")?;
                        stack.push(Piece::Text("}"));

                        for (n, (name, value)) in fields.iter().enumerate().rev() {
                            stack.push(Piece::Value(value));
                            stack.push(Piece::Key(name));

                            if n > 0 {
                                stack.push(Piece::Text(", "));
                            }
                        }
                    }
                },
            }
        }

        Ok(())
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;

    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            c => write!(f, "{c}")?,
        }
    }

    f.write_str("\"")
}
