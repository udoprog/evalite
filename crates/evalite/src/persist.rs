//! A store's values as bytes ([`Globals::to_bytes`] and
//! [`Globals::from_bytes`]), and carried over to another environment's
//! layout ([`Globals::migrate`]).
//!
//! The bytes hold values, not the store's words: each set value by name,
//! as a flat list of its parts in pre-order, so reading them back nests no
//! deeper than the list does and checks every value against the
//! environment as [`Globals::set`] does.

use std::collections::BTreeMap;
use std::fmt;

use musli::{Decode, Encode};

use super::{Globals, GlobalsError};
use crate::memory;
use crate::message::{Fault, Invalid, Message, RunError};
use crate::types::Kind;
use crate::value::{Repr, key_order};
use crate::{DEFAULT_GLOBALS, Environment, MAX_TYPE_DEPTH, Type, Value};

/// The version of the stored form [`Globals::to_bytes`] writes.
pub(crate) const FORMAT: u32 = 1;

#[derive(Debug, Encode, Decode)]
pub(crate) struct Stored {
    pub(crate) format: u32,
    /// Each set value, in slot order.
    pub(crate) values: Vec<StoredValue>,
}

#[derive(Debug, Encode, Decode)]
pub(crate) struct StoredValue {
    pub(crate) name: String,
    /// The value's parts in pre-order: each container, then what it holds.
    pub(crate) parts: Vec<Part>,
}

/// One part of a stored value.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) enum Part {
    Int(i64),
    Bool(bool),
    String(String),
    /// A list of this many entries, which follow.
    List(u64),
    /// A record of these fields, sorted and unique, whose values follow in
    /// that order.
    Record(Vec<String>),
    /// A map of this many entries, which follow as a key then its value, in
    /// key order.
    Map(u64),
    /// An absent option.
    None,
    /// A present option, whose value follows.
    Some,
}

impl Globals {
    /// Its values as bytes, which [`from_bytes`](Self::from_bytes) reads:
    /// each set value by name, in a versioned form of its own. The words
    /// the store keeps them in are not stored, so how a store lays out its
    /// values may change without changing the bytes, and the bytes take as
    /// long to write as its values are large.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`] when the values cannot be encoded, which
    /// a store does not do in practice.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("name", Type::string())?;
    /// env.value("dots", Type::map(Type::string(), Type::int()))?;
    /// env.value("unset", Type::int())?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("name", &Value::string("Ada"))?;
    /// globals.set("dots", &Value::map([(Value::string("occult"), Value::int(3))]))?;
    ///
    /// let read = Globals::from_bytes(&env, &globals.to_bytes()?)?;
    /// assert_eq!(read.get("name"), Some(Value::string("Ada")));
    /// assert_eq!(read.get("dots"), globals.get("dots"));
    /// assert_eq!(read.get("unset"), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn to_bytes(&self) -> Result<Vec<u8>, GlobalsError> {
        let mut values = Vec::new();

        for held in &self.layout.slots {
            let Ok(at) = usize::try_from(held.at) else {
                continue;
            };

            if !self.set.get(at).copied().unwrap_or(false) {
                continue;
            }

            let value = held
                .id
                .ok_or_else(|| memory::fault(Fault::NoSuchType))
                .and_then(|id| {
                    let mut fuel = memory::Fuel::UNLIMITED;
                    memory::read(&self.heap, &self.layout.types, at, id, &mut fuel)
                })
                .map_err(|e| self.error(&e.context(Message::StoringValues)))?;

            values.push(StoredValue {
                name: held.name.clone(),
                parts: to_parts(&value),
            });
        }

        let stored = Stored {
            format: FORMAT,
            values,
        };

        musli::storage::to_vec(&stored).map_err(|_| {
            self.error(&RunError::new(Message::CannotEncode).context(Message::StoringValues))
        })
    }

    /// A store of `env`'s values read from `bytes`, which
    /// [`to_bytes`](Self::to_bytes) wrote, whose parts may take
    /// [`DEFAULT_GLOBALS`] words.
    ///
    /// The bytes are checked as values a caller sets are: each is set by
    /// name, and must be of the type `env` declares it with and fit in the
    /// store's words. A value `env` does not declare, or declares with
    /// another type, is refused; [`migrate`](Self::migrate) carries values
    /// over to a changed environment. The values are laid out anew, with no
    /// words free between them. It takes as long as the bytes are large,
    /// and holds no more than a constant times their size as it reads them.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`] when `bytes` do not decode, are of another
    /// format version, or hold a value that `env` does not declare, that is
    /// not of its declared type, that is stored twice or malformed, or that
    /// does not fit. Its `{:#}` chain names the value.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("level", Type::int())?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("level", &Value::int(3))?;
    /// let bytes = globals.to_bytes()?;
    ///
    /// let mut other = Environment::new();
    /// other.value("level", Type::string())?;
    /// let error = Globals::from_bytes(&other, &bytes).unwrap_err();
    /// assert_eq!(
    ///     format!("{error:#}"),
    ///     "expected String, found Int: while setting `level`: while reading stored values",
    /// );
    ///
    /// let error = Globals::from_bytes(&env, &bytes[..2]).unwrap_err();
    /// assert_eq!(format!("{error:#}"), "the bytes do not decode: while reading stored values");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn from_bytes(env: &Environment, bytes: &[u8]) -> Result<Self, GlobalsError> {
        Self::from_bytes_with_limit(env, bytes, DEFAULT_GLOBALS)
    }

    /// As [`from_bytes`](Self::from_bytes), into a store whose parts may
    /// take `words` words, as [`with_limit`](Self::with_limit) makes.
    ///
    /// # Errors
    ///
    /// As [`from_bytes`](Self::from_bytes).
    pub fn from_bytes_with_limit(
        env: &Environment,
        bytes: &[u8],
        words: u32,
    ) -> Result<Self, GlobalsError> {
        let mut globals = Self::with_limit(env, words);

        match globals.read_stored(bytes) {
            Ok(()) => Ok(globals),
            Err(error) => Err(globals.error(&error.context(Message::DecodingValues))),
        }
    }

    fn read_stored(&mut self, bytes: &[u8]) -> Result<(), RunError> {
        let stored: Stored = musli::storage::from_slice(bytes)
            .map_err(|e| RunError::new(Message::NotDecoded).with_source(e.to_string()))?;

        if stored.format != FORMAT {
            return Err(RunError::new(Message::WrongFormat {
                found: stored.format,
                reads: FORMAT,
            }));
        }

        for stored in stored.values {
            let Some((n, held)) = self.held(&stored.name) else {
                return Err(RunError::new(Message::NotDeclared {
                    name: stored.name.into(),
                }));
            };

            let (id, at) = (held.id, held.at);
            let global = u32::try_from(n).unwrap_or(u32::MAX);
            let setting = |e: RunError| e.context(Message::Setting { global });
            let set = usize::try_from(at)
                .ok()
                .and_then(|at| self.set.get(at).copied())
                .unwrap_or(false);

            if set {
                return Err(setting(invalid(Invalid::ValueTwice)));
            }

            let value = from_parts(stored.parts).map_err(setting)?;
            self.set_slot(n, id, at, &value).map_err(setting)?;
        }

        Ok(())
    }

    /// A store of `env`'s values carrying over `old`'s, with the limit
    /// `old` has, and the values it could not carry over.
    ///
    /// A value is carried over when `env` declares its name with the type
    /// `old` holds it with, or with that type changed only by fields
    /// removed from its records, anywhere inside it: they are left out.
    /// Every other set value is left unset and given back with why:
    ///
    /// - `env` does not declare its name ([`DropReason::NotDeclared`]);
    /// - `env` declares it with another type ([`DropReason::Retyped`]);
    /// - a record in its type gains a field, which it has no value for
    ///   ([`DropReason::NewField`]), however many such records it holds,
    ///   none included;
    /// - it does not fit ([`DropReason::Refused`]), which a value of
    ///   `old`'s does not do in practice, as it takes no more words there.
    ///
    /// A value `env` declares that `old` does not, or that `old` has not
    /// set, is left unset, and is not given back. It takes as long as the
    /// values and their types are large.
    ///
    /// ```
    /// use evalite::{DropReason, Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("name", Type::string())?;
    /// env.value("health", Type::record([("boxes", Type::int()), ("bashing", Type::int())]))?;
    /// env.value("speed", Type::int())?;
    ///
    /// let mut old = Globals::new(&env);
    /// old.set("name", &Value::string("Ada"))?;
    /// old.set("health", &Value::record([("boxes", Value::int(7)), ("bashing", Value::int(1))]))?;
    /// old.set("speed", &Value::int(5))?;
    ///
    /// // `health` loses `bashing` and `speed` becomes optional.
    /// let mut new = Environment::new();
    /// new.value("name", Type::string())?;
    /// new.value("health", Type::record([("boxes", Type::int())]))?;
    /// new.value("speed", Type::option(Type::int()))?;
    ///
    /// let (globals, dropped) = Globals::migrate(&old, &new);
    /// assert_eq!(globals.get("name"), Some(Value::string("Ada")));
    /// assert_eq!(globals.get("health"), Some(Value::record([("boxes", Value::int(7))])));
    /// assert_eq!(globals.get("speed"), None);
    ///
    /// assert_eq!(dropped.len(), 1);
    /// assert_eq!(dropped[0].name(), "speed");
    /// assert_eq!(dropped[0].value(), &Value::int(5));
    /// assert!(matches!(dropped[0].reason(), DropReason::Retyped { .. }));
    /// assert_eq!(dropped[0].to_string(), "`speed` was declared as Int, and is now Option<Int>");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn migrate(old: &Globals, env: &Environment) -> (Globals, Vec<Dropped>) {
        let mut globals = Self::with_limit(env, old.limit);
        let mut dropped = Vec::new();

        for held in &old.layout.slots {
            let Some(mut value) = old.get(&held.name) else {
                continue;
            };

            let mut dropping = |reason| {
                dropped.push(Dropped {
                    name: held.name.clone(),
                    value: value.clone(),
                    reason,
                });
            };

            let Some(new) = env.value_type(&held.name) else {
                dropping(DropReason::NotDeclared);
                continue;
            };

            if held.ty != *new {
                if let Err(reason) = carries(&held.ty, new) {
                    dropping(reason);
                    continue;
                }

                prune(&mut value, new);
            }

            if let Err(error) = globals.set(&held.name, &value) {
                dropped.push(Dropped {
                    name: held.name.clone(),
                    value,
                    reason: DropReason::Refused(error),
                });
            }
        }

        (globals, dropped)
    }
}

/// A value [`Globals::migrate`] could not carry over: its name, the value
/// it had, and why.
///
/// The value is given back so a caller can make one of the new type from
/// it, such as a record with the field its type gains, and
/// [`set`](Globals::set) it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    name: String,
    value: Value,
    reason: DropReason,
}

impl Dropped {
    /// The value's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value it had in the old store.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Why it was not carried over.
    pub fn reason(&self) -> &DropReason {
        &self.reason
    }
}

impl fmt::Display for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;

        match &self.reason {
            DropReason::NotDeclared => write!(f, "`{name}` is no longer declared"),
            DropReason::Retyped { old, new } => {
                write!(f, "`{name}` was declared as {old}, and is now {new}")
            }
            DropReason::NewField { record, field } => write!(
                f,
                "`{name}` holds a record of type {record}, whose field `{field}` it has no value for"
            ),
            DropReason::Refused(error) => write!(f, "`{name}` could not be set: {error}"),
        }
    }
}

/// Why [`Globals::migrate`] could not carry a value over.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropReason {
    /// The new environment does not declare its name.
    NotDeclared,
    /// The new environment declares it with another type, which is not
    /// the old one with record fields added or removed.
    Retyped {
        /// The type the old store held it with.
        old: Type,
        /// The type the new environment declares it with.
        new: Type,
    },
    /// Its new type is the old one with fields added to a record, and
    /// `field` of `record`, as the new type has it, is new, which the value
    /// has no value for.
    NewField {
        /// The record type, in the new type, that gains the field.
        record: Type,
        /// The field it gains.
        field: String,
    },
    /// Setting it in the new store failed.
    Refused(GlobalsError),
}

fn invalid(invalid: Invalid) -> RunError {
    RunError::new(Message::Invalid(invalid))
}

/// Whether a value of type `old` is carried over to type `new`: they are
/// alike but for fields removed from records, or the reason it is not. A
/// change of a part's type outweighs a new field.
fn carries(old: &Type, new: &Type) -> Result<(), DropReason> {
    let mut work = vec![(old, new)];
    let mut new_field = None;

    while let Some((a, b)) = work.pop() {
        match (&a.0, &b.0) {
            (Kind::Int, Kind::Int) | (Kind::Bool, Kind::Bool) | (Kind::String, Kind::String) => {}
            (Kind::List(a), Kind::List(b)) | (Kind::Option(a), Kind::Option(b)) => {
                work.push((a, b));
            }
            (Kind::Map(ka, va), Kind::Map(kb, vb)) => {
                work.push((ka, kb));
                work.push((va, vb));
            }
            (Kind::Record(fa), Kind::Record(fb)) => {
                for (name, tb) in fb {
                    match fa.get(name) {
                        Some(ta) => work.push((ta, tb)),
                        None => {
                            new_field.get_or_insert_with(|| DropReason::NewField {
                                record: b.clone(),
                                field: name.clone(),
                            });
                        }
                    }
                }
            }
            _ => {
                return Err(DropReason::Retyped {
                    old: old.clone(),
                    new: new.clone(),
                });
            }
        }
    }

    new_field.map_or(Ok(()), Err)
}

/// Leave out of `value` every record field `ty` does not have, `value`
/// being of a type [`carries`] to `ty`.
fn prune(value: &mut Value, ty: &Type) {
    let mut work = vec![(value, ty)];

    while let Some((value, ty)) = work.pop() {
        match (&mut value.0, &ty.0) {
            (Repr::List(items), Kind::List(entry)) => {
                work.extend(items.iter_mut().map(|item| (item, &**entry)));
            }
            (Repr::Some(inner), Kind::Option(inner_ty)) => work.push((&mut **inner, inner_ty)),
            (Repr::Map(entries), Kind::Map(_, value_ty)) => {
                work.extend(entries.iter_mut().map(|(_, v)| (v, &**value_ty)));
            }
            (Repr::Record(fields), Kind::Record(types)) => {
                fields.retain(|name, _| types.contains_key(name));
                work.extend(
                    fields
                        .iter_mut()
                        .filter_map(|(name, v)| Some((v, types.get(name)?))),
                );
            }
            _ => {}
        }
    }
}

/// `value`'s parts in pre-order.
pub(crate) fn to_parts(value: &Value) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut work = vec![value];

    while let Some(value) = work.pop() {
        match &value.0 {
            Repr::Int(n) => parts.push(Part::Int(*n)),
            Repr::Bool(b) => parts.push(Part::Bool(*b)),
            Repr::String(s) => parts.push(Part::String(s.clone())),
            Repr::List(items) => {
                parts.push(Part::List(memory::count(items.len())));
                work.extend(items.iter().rev());
            }
            Repr::Record(fields) => {
                parts.push(Part::Record(fields.keys().cloned().collect()));
                work.extend(fields.values().rev());
            }
            Repr::Map(entries) => {
                parts.push(Part::Map(memory::count(entries.len())));

                for (key, value) in entries.iter().rev() {
                    work.push(value);
                    work.push(key);
                }
            }
            Repr::None => parts.push(Part::None),
            Repr::Some(inner) => {
                parts.push(Part::Some);
                work.push(inner);
            }
        }
    }

    parts
}

/// A container being read, waiting for what it holds.
enum Open {
    List {
        left: u64,
        items: Vec<Value>,
    },
    Record {
        names: std::vec::IntoIter<String>,
        fields: BTreeMap<String, Value>,
    },
    Map {
        left: u64,
        key: Option<Value>,
        entries: Vec<(Value, Value)>,
    },
    Some,
}

impl Open {
    /// The container `part` opens, which holds at least one value.
    fn new(part: Part) -> Result<Self, RunError> {
        Ok(match part {
            Part::List(left) => Self::List {
                left,
                items: Vec::new(),
            },
            Part::Map(left) => Self::Map {
                left,
                key: None,
                entries: Vec::new(),
            },
            Part::Record(names) => {
                if names
                    .windows(2)
                    .any(|pair| matches!(pair, [a, b] if a >= b))
                {
                    return Err(invalid(Invalid::ValueFieldsUnsorted));
                }

                Self::Record {
                    names: names.into_iter(),
                    fields: BTreeMap::new(),
                }
            }
            Part::Some => Self::Some,
            // Not containers, read where they are.
            Part::Int(_) | Part::Bool(_) | Part::String(_) | Part::None => {
                return Err(memory::fault(Fault::ValueChangedType));
            }
        })
    }

    /// Take `value`, the next one it holds, giving back the container's
    /// value when that was its last.
    fn take(&mut self, value: Value) -> Result<Option<Value>, RunError> {
        match self {
            Self::List { left, items } => {
                items.push(value);
                *left = left.saturating_sub(1);
                Ok((*left == 0).then(|| Value(Repr::List(std::mem::take(items)))))
            }
            Self::Record { names, fields } => {
                if let Some(name) = names.next() {
                    fields.insert(name, value);
                }

                Ok((names.len() == 0).then(|| Value(Repr::Record(std::mem::take(fields)))))
            }
            Self::Map { left, key, entries } => {
                let Some(key) = key.take() else {
                    *key = Some(value);
                    return Ok(None);
                };

                // Strictly after the last key, so sorted and unique as a
                // map holds them.
                if let Some((last, _)) = entries.last()
                    && key_order(last, &key).is_ge()
                {
                    return Err(invalid(Invalid::ValueKeysUnsorted));
                }

                entries.push((key, value));
                *left = left.saturating_sub(1);
                Ok((*left == 0).then(|| Value(Repr::Map(std::mem::take(entries)))))
            }
            Self::Some => Ok(Some(Value(Repr::Some(Box::new(value))))),
        }
    }
}

/// The value of `parts`, in pre-order, without recursing: an open
/// container is kept on a list, at most [`MAX_TYPE_DEPTH`] deep, which no
/// value of a declared type nests past. What it holds is reserved for as
/// it is read, never as its count says, so it holds no more than a
/// constant times the parts.
pub(crate) fn from_parts(parts: Vec<Part>) -> Result<Value, RunError> {
    let mut open = Vec::<Open>::new();
    let mut parts = parts.into_iter();

    while let Some(part) = parts.next() {
        let mut value = match part {
            Part::Int(n) => Value::int(n),
            Part::Bool(b) => Value::bool(b),
            Part::String(s) => Value::string(s),
            Part::None => Value::none(),
            Part::List(0) => Value(Repr::List(Vec::new())),
            Part::Map(0) => Value(Repr::Map(Vec::new())),
            Part::Record(names) if names.is_empty() => Value(Repr::Record(BTreeMap::new())),
            part => {
                if open.len() >= MAX_TYPE_DEPTH {
                    return Err(invalid(Invalid::ValueTooDeep));
                }

                open.push(Open::new(part)?);
                continue;
            }
        };

        // Close each container the value was the last of.
        loop {
            let Some(top) = open.last_mut() else {
                return match parts.next() {
                    None => Ok(value),
                    Some(_) => Err(invalid(Invalid::ValuePastEnd)),
                };
            };

            match top.take(value)? {
                Some(closed) => {
                    open.pop();
                    value = closed;
                }
                None => break,
            }
        }
    }

    Err(invalid(Invalid::ValueEndsEarly))
}
