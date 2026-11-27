//! A store of values that runs read in place: the values of one
//! [`Environment`], set once and kept from run to run.

use std::collections::BTreeMap;
use std::fmt;

use crate::env::refused;
use crate::memory::{self, Words, WordsMut, count, fault, get, inside};
use crate::message::{
    Fault, Key, Labels, Message, Names, PathStep, Root, RunError, Seg, Shown, cut,
};
use crate::references::StepRepr;
use crate::types::{TypeId, TypeNode, Types};
use crate::value::Repr;
use crate::word::Word;
use crate::{Environment, Path, Program, ReferenceKind, Type, Value};

/// The words a [`Globals`] store may take for the parts of its values
/// unless [`Globals::with_limit`] says otherwise: 2^20 words, 8 MiB.
pub const DEFAULT_GLOBALS: u32 = 1 << 20;

/// The values of an [`Environment`], set once and read in place by every
/// run of every program compiled against it, as a sheet's values are by
/// its formulas.
///
/// Each value is held in its slot, where the environment declared it, and
/// its parts (a string's bytes, a list's entries, a record's fields, a
/// map's entries) after the slots. Setting a value checks it against its
/// declared type, and replaces the value before it, whose parts are freed
/// to be taken again by a part of the same size. A run reads the store and
/// never writes to it, so loading a program copies none of its values.
///
/// ```
/// use evalite::{Environment, Globals, Machine, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let mut globals = Globals::new(&env);
/// globals.set("strength", &Value::int(3))?;
/// globals.set("bonus", &Value::some(Value::int(2)))?;
///
/// let program = evalite::compile("strength + bonus.unwrap_or(0)", &env)?;
/// let mut machine = Machine::default();
/// assert_eq!(machine.load(&program, &globals)?.run()?.value(), &Value::int(5));
///
/// // Runs read the values as they are now.
/// globals.set("bonus", &Value::none())?;
/// assert_eq!(machine.load(&program, &globals)?.run()?.value(), &Value::int(3));
///
/// let error = globals.set("strength", &Value::bool(true)).unwrap_err();
/// assert_eq!(error.to_string(), "expected Int, found Bool");
/// assert_eq!(format!("{error:#}"), "expected Int, found Bool: while setting `strength`");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Globals {
    layout: Layout,
    heap: Heap,
    /// Whether the value whose slot starts at each word of the slots is
    /// set.
    set: Vec<bool>,
    /// The words the parts may take, as asked for.
    limit: u32,
}

/// A store's words.
#[derive(Debug, Clone)]
struct Heap {
    /// The slots, then the parts of the values in them.
    words: Vec<Word>,
    /// The words the slots take, where the parts start.
    slot_words: usize,
    /// The words the parts may take, kept inside what a reference can
    /// address.
    room: usize,
    /// Freed blocks of parts, by their size in words.
    free: BTreeMap<usize, Vec<usize>>,
}

impl Heap {
    /// The words the parts take, freed ones included.
    fn used(&self) -> usize {
        self.words.len().saturating_sub(self.slot_words)
    }
}

/// The values of a store, which name them in errors.
#[derive(Debug, Clone)]
struct Layout {
    fingerprint: u64,
    types: Types,
    /// In declaration order, which is slot order.
    slots: Vec<Held>,
    /// Where each value is in `slots`, by name.
    names: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
struct Held {
    name: String,
    ty: Type,
    /// Its type in the store's table, which a declared type always has.
    id: Option<TypeId>,
    at: u32,
}

impl Names for Layout {
    fn types(&self) -> &Types {
        &self.types
    }

    fn global(&self, global: u32) -> Option<(&str, TypeId)> {
        let held = self.slots.get(usize::try_from(global).ok()?)?;
        Some((held.name.as_str(), held.id?))
    }

    fn function(&self, _: u32) -> Option<&str> {
        None
    }
}

impl Globals {
    /// A store of `env`'s values, none of them set yet, whose parts may
    /// take [`DEFAULT_GLOBALS`] words.
    ///
    /// It holds the values `env` declares now: one declared after it is
    /// made is not in it, and a program reading it is refused.
    pub fn new(env: &Environment) -> Self {
        Self::with_limit(env, DEFAULT_GLOBALS)
    }

    /// A store of `env`'s values whose parts may take `words` words,
    /// freed ones included. The slots themselves are not counted.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("name", Type::string())?;
    ///
    /// let mut globals = Globals::with_limit(&env, 2);
    /// globals.set("name", &Value::string("Ada Lovelace"))?;
    ///
    /// // Its 4 words would not fit in the 2 of the name it replaces, and no
    /// // more are left.
    /// let error = globals.set("name", &Value::string("Ada, Countess of Lovelace")).unwrap_err();
    /// assert_eq!(error.to_string(), "the value needs 4 words of memory, 0 are left");
    /// assert_eq!(globals.get("name"), Some(Value::string("Ada Lovelace")));
    ///
    /// // A value of the same size takes the words of the one it replaces.
    /// globals.set("name", &Value::string("Ada Byron"))?;
    /// assert_eq!(globals.get("name"), Some(Value::string("Ada Byron")));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_limit(env: &Environment, words: u32) -> Self {
        let mut types = Types::new();
        let slots = env
            .slots
            .iter()
            .map(|slot| Held {
                name: slot.name.clone(),
                ty: slot.ty.clone(),
                id: types.intern(&slot.ty),
                at: slot.at,
            })
            .collect::<Vec<_>>();

        let slot_words = usize::try_from(env.slot_words()).unwrap_or(usize::MAX);
        // Every word, the end included, is addressable by a reference.
        let room = usize::try_from(u32::MAX)
            .unwrap_or(usize::MAX)
            .saturating_sub(slot_words)
            .min(usize::try_from(words).unwrap_or(usize::MAX));

        Self {
            layout: Layout {
                fingerprint: env.fingerprint(),
                types,
                slots,
                names: env.names.clone(),
            },
            heap: Heap {
                words: vec![Word::int(0); slot_words],
                slot_words,
                room,
                free: BTreeMap::new(),
            },
            set: vec![false; slot_words],
            limit: words,
        }
    }

    /// The words its values' parts may take.
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// The [`fingerprint`](Environment::fingerprint) of the environment it
    /// holds the values of.
    pub fn fingerprint(&self) -> u64 {
        self.layout.fingerprint
    }

    /// Whether it holds the values `program` reads where and as it reads
    /// them, as [`Environment::accepts`] says of the environment it was
    /// made from. [`Machine::load`](crate::Machine::load) refuses a
    /// program it does not accept.
    pub fn accepts(&self, program: &Program) -> bool {
        self.refused(program).is_none()
    }

    /// The first of the values `program` reads that this store does not
    /// hold where and as it reads it.
    pub(crate) fn refused(&self, program: &Program) -> Option<usize> {
        refused(self.layout.fingerprint, program, |name| {
            let held = self.held(name)?.1;
            Some((held.at, &held.ty))
        })
    }

    /// Set the value `name` to `value`, replacing the value before it.
    ///
    /// It takes the words `value`'s parts need, reusing those of the value
    /// it replaces and of values replaced before; only what they cannot
    /// give counts against the [`limit`](Self::limit). It takes as long as
    /// `value` and the value it replaces are large.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`], leaving the value as it was, when `name`
    /// is not declared, `value` is not of its declared type (the error
    /// names the path into `value`), or its parts do not fit in the words
    /// left.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("track", Type::record([("boxes", Type::int()), ("marks", Type::list(Type::bool()))]))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// let track = Value::record([
    ///     ("boxes", Value::int(5)),
    ///     ("marks", Value::list([Value::bool(true), Value::int(1)])),
    /// ]);
    /// let error = globals.set("track", &track).unwrap_err();
    /// let labels: Vec<String> = error.labels().map(|l| l.message().to_string()).collect();
    /// assert_eq!(labels, ["expected Bool, found Int", "at `track.marks[1]`", "while setting `track`"]);
    ///
    /// let error = globals.set("tracks", &track).unwrap_err();
    /// assert_eq!(error.to_string(), "no value `tracks` is declared");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn set(&mut self, name: &str, value: &Value) -> Result<(), GlobalsError> {
        let Some((n, held)) = self.held(name) else {
            let error = RunError::new(Message::NotDeclared { name: name.into() });
            return Err(self.error(&error));
        };

        let global = u32::try_from(n).unwrap_or(u32::MAX);
        let (id, at) = (held.id, held.at);

        self.set_slot(n, id, at, value)
            .map_err(|e| self.error(&e.context(Message::Setting { global })))
    }

    fn set_slot(
        &mut self,
        n: usize,
        id: Option<TypeId>,
        at: u32,
        value: &Value,
    ) -> Result<(), RunError> {
        let id = id.ok_or_else(|| fault(Fault::NoSuchType))?;
        let global = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))?;
        let at = usize::try_from(at).map_err(|_| fault(Fault::PastAddressSpace))?;
        let set = self.set.get(at).copied().unwrap_or(false);

        let place = Place {
            global,
            at,
            ty: id,
            segs: Vec::new(),
        };

        self.replace(&place, value, set)?;

        if let Some(set) = self.set.get_mut(at) {
            *set = true;
        }

        Ok(())
    }

    /// Write `value` at `place`, replacing the value there when `held`,
    /// whose parts are freed.
    fn replace(&mut self, place: &Place, value: &Value, held: bool) -> Result<(), RunError> {
        let types = &self.layout.types;
        let Place {
            global, at, ty: id, ..
        } = *place;

        memory::measure_at(types, value, id, Root::Global(global), &place.segs)?;
        let sizes = memory::parts(types, value, id)?;

        let old = if held {
            memory::blocks(&self.heap, types, at, id)?
        } else {
            Vec::new()
        };

        // What the free blocks and the old value's cannot give is taken
        // past the parts: found before anything is changed, so a value
        // that does not fit leaves the store as it was.
        let needed = self.bump(&sizes, &old);
        let left = self.heap.room.saturating_sub(self.heap.used());

        if needed > left {
            return Err(RunError::new(Message::TooLarge {
                needed: count(needed),
                left: count(left),
            }));
        }

        for (start, words) in old {
            self.heap.free.entry(words).or_default().push(start);
        }

        memory::write(&mut self.heap, types, value, id, at)
    }

    /// The words that blocks of `sizes` take past the parts, once the free
    /// blocks and `old` give what they can of the same sizes.
    fn bump(&self, sizes: &[usize], old: &[(usize, usize)]) -> usize {
        // Of each size: the blocks there are to take, then those taken.
        let mut blocks = BTreeMap::<usize, (usize, usize)>::new();

        for (_, words) in old {
            let (there, _) = blocks.entry(*words).or_insert_with(|| {
                let free = self.heap.free.get(words).map_or(0, Vec::len);
                (free, 0)
            });
            *there = there.saturating_add(1);
        }

        let mut needed = 0usize;

        for words in sizes {
            let (there, taken) = blocks.entry(*words).or_insert_with(|| {
                let free = self.heap.free.get(words).map_or(0, Vec::len);
                (free, 0)
            });

            if *taken < *there {
                *taken = taken.saturating_add(1);
            } else {
                needed = needed.saturating_add(*words);
            }
        }

        needed
    }

    /// The value `name`, or `None` when it is not declared or not set.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("tags", Type::map(Type::string(), Type::int()))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// assert_eq!(globals.get("tags"), None);
    ///
    /// let tags = Value::map([(Value::string("fire"), Value::int(2))]);
    /// globals.set("tags", &tags)?;
    /// assert_eq!(globals.get("tags"), Some(tags));
    /// assert_eq!(globals.get("other"), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn get(&self, name: &str) -> Option<Value> {
        let (_, held) = self.held(name)?;
        let at = usize::try_from(held.at).ok()?;

        if !self.set.get(at).copied().unwrap_or(false) {
            return None;
        }

        memory::read(&self.heap, &self.layout.types, at, held.id?).ok()
    }

    /// Set the part at `path` of a value already set to `value`, replacing
    /// the part there: a record's field, a list's entry or the value of a
    /// map's key. [`set`](Self::set) a value as a whole by its name alone,
    /// as `set_at` of a path without steps does.
    ///
    /// A path steps through a present option, as `bonus!.dice` reads the
    /// field of what `bonus` holds, so `Path::value("bonus").field("dice")`
    /// is that field. Only the part is written: the value around it stays
    /// where it is, and runs read the new part from then on. It takes the
    /// words the new part needs as [`set`](Self::set) does, reusing those of
    /// the part it replaces, so a part of the same size takes no more. It
    /// takes as long as the path is long, and the old and new parts are
    /// large, plus a binary search of each map the path takes a key of.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`], leaving the store as it was, when:
    ///
    /// - the path is of what a function gives, or names a value that is not
    ///   declared or, past it, not set;
    /// - a step is not to a part the value has: a field its record does not
    ///   have, a key its map does not have, an index past either end of its
    ///   list, a step of the wrong kind, or one through an absent option;
    /// - `value` is not of the part's type, or its parts do not fit in the
    ///   words left.
    ///
    /// Inserting a key, or an entry of a list, is not setting one.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// let skill = Type::record([("dots", Type::int()), ("specialties", Type::list(Type::string()))]);
    /// env.value("skills", Type::map(Type::string(), skill))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// let skill = |dots, specialties: &[&str]| {
    ///     let specialties = specialties.iter().map(|s| Value::string(*s));
    ///     Value::record([("dots", Value::int(dots)), ("specialties", Value::list(specialties))])
    /// };
    /// globals.set("skills", &Value::map([
    ///     (Value::string("brawl"), skill(2, &[])),
    ///     (Value::string("occult"), skill(3, &["ghosts"])),
    /// ]))?;
    ///
    /// let occult = Path::value("skills").string_key("occult");
    /// globals.set_at(&occult.clone().field("dots"), &Value::int(4))?;
    /// globals.set_at(&occult.clone().field("specialties").index(0), &Value::string("rituals"))?;
    /// assert_eq!(globals.get_at(&occult), Some(skill(4, &["rituals"])));
    ///
    /// let program = evalite::compile(r#"skills["occult"].dots + skills["brawl"].dots"#, &env)?;
    /// assert_eq!(Machine::default().load(&program, &globals)?.run()?.value(), &Value::int(6));
    ///
    /// let brawl = Path::value("skills").string_key("brawl");
    /// let error = globals.set_at(&brawl.clone().field("specialties").index(0), &Value::string("grappling"));
    /// assert_eq!(
    ///     format!("{:#}", error.unwrap_err()),
    ///     r#"index 0 is out of range of a list of 0 entries: at `skills["brawl"].specialties`: while setting `skills["brawl"].specialties[0]`"#,
    /// );
    ///
    /// let error = globals.set_at(&brawl.field("dots"), &Value::bool(true)).unwrap_err();
    /// assert_eq!(format!("{error:#}"), r#"expected Int, found Bool: while setting `skills["brawl"].dots`"#);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_at(&mut self, path: &Path, value: &Value) -> Result<(), GlobalsError> {
        if path.kind == ReferenceKind::Value && path.steps.is_empty() {
            return self.set(&path.name, value);
        }

        let result = match self.locate(path) {
            Ok(place) => self.replace(&place, value, true),
            Err(error) => Err(error),
        };

        result.map_err(|e| {
            let path = path.to_string().into_boxed_str();
            self.error(&e.context(Message::SettingAt { path }))
        })
    }

    /// The part at `path` of a value, as [`set_at`](Self::set_at) finds it,
    /// or `None` when there is no such part, or the value is not set. A
    /// path without steps is the value [`get`](Self::get) gives.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("bonus", Type::option(Type::record([("dice", Type::int())])))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// let dice = Path::value("bonus").field("dice");
    /// assert_eq!(globals.get_at(&dice), None);
    ///
    /// globals.set("bonus", &Value::some(Value::record([("dice", Value::int(2))])))?;
    /// assert_eq!(globals.get_at(&dice), Some(Value::int(2)));
    ///
    /// globals.set("bonus", &Value::none())?;
    /// assert_eq!(globals.get_at(&dice), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn get_at(&self, path: &Path) -> Option<Value> {
        let place = self.locate(path).ok()?;
        memory::read(&self.heap, &self.layout.types, place.at, place.ty).ok()
    }

    /// Where the part at `path` is held, and its type.
    ///
    /// Each step is checked against the type of the part before it, and
    /// reads the reference word it steps through: a constant number of
    /// words for a field or an index, a binary search for a key.
    fn locate(&self, path: &Path) -> Result<Place, RunError> {
        if path.kind != ReferenceKind::Value {
            let name = cut(&path.name);
            return Err(RunError::new(Message::FunctionPath { name }));
        }

        let Some((n, held)) = self.held(&path.name) else {
            let name = path.name.as_str().into();
            return Err(RunError::new(Message::NotDeclared { name }));
        };

        let global = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))?;
        let mut at = usize::try_from(held.at).map_err(|_| fault(Fault::PastAddressSpace))?;
        let mut ty = held.id.ok_or_else(|| fault(Fault::NoSuchType))?;

        if !self.set.get(at).copied().unwrap_or(false) {
            return Err(RunError::new(Message::NotSet { global }));
        }

        let types = &self.layout.types;
        let words = &self.heap;
        let mut segs = Vec::with_capacity(path.steps.len());

        for step in &path.steps {
            // Through each present option to what it holds.
            while let Some(TypeNode::Option(inner)) = types.get(ty) {
                let word = get(words, Some(at))?;

                let present = if types.tagged(ty).is_some() {
                    at = at
                        .checked_add(1)
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    word.as_bool()?
                } else {
                    !word.is_none()
                };

                if !present {
                    return Err(off_path(Message::NoneOnPath, global, &segs));
                }

                ty = *inner;
            }

            let node = types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))?;

            let (seg, next, next_ty) = match (step, node) {
                (StepRepr::Field(name), TypeNode::Record(fields)) => {
                    let Some((n, offset, field)) = types.field_index(ty, name) else {
                        let step = PathStep::Field(cut(name));
                        return Err(off_path(Message::NoStep { ty, step }, global, &segs));
                    };

                    let (start, len) = get(words, Some(at))?.as_reference()?;

                    if len != types.record_words(fields) {
                        return Err(fault(Fault::RecordWrongSize));
                    }

                    inside(words, start, Some(len))?;
                    let field_ty = field.ty;
                    let seg = Seg::Field {
                        record: ty,
                        field: u32::try_from(n).map_err(|_| fault(Fault::FieldPastRecord))?,
                    };
                    (seg, start.checked_add(offset), field_ty)
                }
                (StepRepr::Key(key), TypeNode::Map { key: key_ty, value }) => {
                    let shown = match (&key.0, types.get(*key_ty)) {
                        (Repr::Int(n), Some(TypeNode::Int)) => Key::Int(*n),
                        (Repr::String(s), Some(TypeNode::String)) => Key::string(s),
                        _ => {
                            let mistyped = Message::KeyMistyped {
                                expected: *key_ty,
                                found: key.describe(),
                            };
                            return Err(off_path(mistyped, global, &segs));
                        }
                    };

                    let (start, len) = get(words, Some(at))?.as_reference()?;
                    let stride = types
                        .width(*value)
                        .checked_add(1)
                        .ok_or_else(|| fault(Fault::ListTooLarge))?;

                    let Some(entry) = memory::find_key(words, start, len, stride, key)? else {
                        let missing = Message::NoSuchKey { key: shown };
                        return Err(off_path(missing, global, &segs));
                    };

                    (Seg::Key(shown), entry.checked_add(1), *value)
                }
                (StepRepr::Index(index), TypeNode::List(entry)) => {
                    let (start, len) = get(words, Some(at))?.as_reference()?;
                    let width = types.width(*entry);

                    let Some(n) = usize::try_from(*index).ok().filter(|n| *n < len) else {
                        let range = Message::IndexOutOfRange {
                            index: *index,
                            len: count(len),
                        };
                        return Err(off_path(range, global, &segs));
                    };

                    inside(words, start, len.checked_mul(width))?;
                    let next = n.checked_mul(width).and_then(|n| n.checked_add(start));
                    (Seg::Index(count(n)), next, *entry)
                }
                (StepRepr::Dynamic, _) => return Err(fault(Fault::DynamicStep)),
                (step, _) => {
                    let step = match step {
                        StepRepr::Field(name) => PathStep::Field(cut(name)),
                        StepRepr::Key(key) => match (&key.0, key.as_int()) {
                            (_, Some(n)) => PathStep::Key(Key::Int(n)),
                            (Repr::String(s), _) => PathStep::Key(Key::string(s)),
                            _ => return Err(fault(Fault::ValueChangedType)),
                        },
                        StepRepr::Index(index) => PathStep::Index(*index),
                        StepRepr::Dynamic => return Err(fault(Fault::DynamicStep)),
                    };

                    return Err(off_path(Message::NoStep { ty, step }, global, &segs));
                }
            };

            at = next.ok_or_else(|| fault(Fault::OutsideMemory))?;
            ty = next_ty;
            segs.push(seg);
        }

        Ok(Place {
            global,
            at,
            ty,
            segs,
        })
    }

    /// The value `name` and where it is among the slots.
    fn held(&self, name: &str) -> Option<(usize, &Held)> {
        let n = *self.layout.names.get(name)?;
        Some((n, self.layout.slots.get(n)?))
    }

    /// The words the parts take, freed ones included.
    #[cfg(test)]
    pub(crate) fn heap_words(&self) -> usize {
        self.heap.used()
    }

    /// Every word: the slots, then the parts.
    pub(crate) fn words(&self) -> &[Word] {
        &self.heap.words
    }

    /// Whether the value whose slot starts at each word of the slots is
    /// set.
    pub(crate) fn set_flags(&self) -> &[bool] {
        &self.set
    }

    /// `error` with the names of this store's values.
    #[cold]
    #[inline(never)]
    fn error(&self, error: &RunError) -> GlobalsError {
        let frames = error
            .frames()
            .iter()
            .map(|frame| {
                Shown {
                    message: &frame.message,
                    names: Some(&self.layout),
                }
                .to_string()
                .into_boxed_str()
            })
            .collect();

        GlobalsError { frames }
    }
}

impl Words for Heap {
    #[inline]
    fn word(&self, at: usize) -> Option<Word> {
        self.words.get(at).copied()
    }

    fn end(&self) -> usize {
        self.words.len()
    }
}

impl WordsMut for Heap {
    fn set_word(&mut self, at: usize, word: Word) -> Option<()> {
        *self.words.get_mut(at)? = word;
        Some(())
    }

    fn alloc(&mut self, n: usize) -> Result<usize, RunError> {
        if n == 0 {
            return Ok(self.words.len());
        }

        if let Some(at) = self.free.get_mut(&n).and_then(Vec::pop) {
            return Ok(at);
        }

        let at = self.words.len();
        let heap = self.used().checked_add(n).filter(|heap| *heap <= self.room);
        let end = heap.and_then(|_| at.checked_add(n));
        let end = end.ok_or_else(|| fault(Fault::StorePastLimit))?;
        self.words.resize(end, Word::int(0));
        Ok(at)
    }
}

/// Where a part of a value is held: the value it is part of, the word it
/// starts at, its type, and the path to it.
struct Place {
    global: u32,
    at: usize,
    ty: TypeId,
    segs: Vec<Seg>,
}

/// `message` about a step of a path, from the part at `segs` of the value
/// `global`.
#[cold]
#[inline(never)]
fn off_path(message: Message, global: u32, segs: &[Seg]) -> RunError {
    RunError::new(message).context(Message::AtPath {
        root: Root::Global(global),
        path: segs.into(),
    })
}

/// Why a value could not be set in a [`Globals`] store.
///
/// It is opaque: [`Display`](fmt::Display) gives its message, `{:#}` the
/// whole chain of what was being done, and [`labels`](Self::labels) each
/// part, naming the store's values. None has a span, as a value has no
/// source text.
#[derive(Clone, PartialEq, Eq)]
pub struct GlobalsError {
    /// The primary message, then each context outward, shown.
    frames: Box<[Box<str>]>,
}

impl GlobalsError {
    /// The primary message, then each context outward, as [`Label`]s
    /// without spans.
    ///
    /// [`Label`]: crate::Label
    pub fn labels(&self) -> Labels<'_> {
        Labels::shown(&self.frames)
    }
}

impl fmt::Debug for GlobalsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlobalsError")
            .field("frames", &self.frames)
            .finish()
    }
}

impl fmt::Display for GlobalsError {
    /// The primary message; with `{:#}`, every frame, joined by `: `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = if f.alternate() {
            &*self.frames
        } else {
            self.frames.get(..1).unwrap_or_default()
        };

        for (n, frame) in shown.iter().enumerate() {
            if n > 0 {
                f.write_str(": ")?;
            }

            f.write_str(frame)?;
        }

        Ok(())
    }
}

impl std::error::Error for GlobalsError {}
