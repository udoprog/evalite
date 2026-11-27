//! A store of values that runs read in place: the values of one
//! [`Environment`], set once and kept from run to run.

use std::collections::BTreeMap;
use std::fmt;

use crate::env::refused;
use crate::memory::{self, Words, WordsMut, count, fault};
use crate::message::{Fault, Labels, Message, Names, Root, RunError, Shown};
use crate::types::{TypeId, Types};
use crate::word::Word;
use crate::{Environment, Program, Type, Value};

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
        let types = &self.layout.types;
        let id = id.ok_or_else(|| fault(Fault::NoSuchType))?;
        let global = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))?;
        let at = usize::try_from(at).map_err(|_| fault(Fault::PastAddressSpace))?;

        memory::measure(types, value, id, Root::Global(global))?;
        let sizes = memory::parts(types, value, id)?;

        let old = if self.set.get(at).copied().unwrap_or(false) {
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

        memory::write(&mut self.heap, types, value, id, at)?;

        if let Some(set) = self.set.get_mut(at) {
            *set = true;
        }

        Ok(())
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
