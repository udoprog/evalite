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
/// to be taken again. When more than half the words its parts take are
/// free, the store moves its values' parts together, so it takes at most
/// twice the words its values do ([`stats`](Self::stats)). A run reads the
/// store and never writes to it, so loading a program copies none of its
/// values.
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
    /// Freed blocks of parts, by their size in words. A block is taken
    /// from the smallest size that holds it, and what it leaves of a larger
    /// one is freed again.
    free: BTreeMap<usize, Vec<usize>>,
    /// The words of the free blocks.
    free_words: usize,
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
                free_words: 0,
            },
            set: vec![false; slot_words],
            limit: words,
        }
    }

    /// The words its values' parts may take.
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// The words its values' parts take, and those it has freed. It takes
    /// as long as its values are large.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("names", Type::list(Type::string()))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("names", &Value::list([Value::string("Ada Lovelace")]))?;
    ///
    /// // A header, an entry, and the name's 2 words.
    /// let stats = globals.stats();
    /// assert_eq!((stats.live(), stats.spare(), stats.free()), (4, 0, 0));
    ///
    /// // Pushing moves the list to room for 4 entries, freeing its 2 words,
    /// // and the new name takes one of them.
    /// globals.push(&Path::value("names"), &Value::string("Bea"))?;
    /// let stats = globals.stats();
    /// assert_eq!((stats.live(), stats.spare(), stats.free()), (8, 2, 1));
    /// assert_eq!(stats.heap(), 9);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn stats(&self) -> GlobalsStats {
        let (mut live, mut spare) = (0usize, 0usize);

        for (at, id) in self.roots() {
            let _ = memory::each_block(&self.heap, &self.layout.types, at, id, |_, n, s| {
                live = live.saturating_add(n);
                spare = spare.saturating_add(s);
            });
        }

        let words = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);

        GlobalsStats {
            live: words(live),
            spare: words(spare),
            free: words(self.heap.free_words),
            heap: words(self.heap.used()),
        }
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
        refused(program, |name| {
            let held = self.held(name)?.1;
            Some((held.at, &held.ty))
        })
    }

    /// Set the value `name` to `value`, replacing the value before it.
    ///
    /// It takes the words `value`'s parts need, reusing those of the value
    /// it replaces and of values replaced before; only what they cannot
    /// give counts against the [`limit`](Self::limit). It takes as long as
    /// `value` and the value it replaces are large, plus, when it leaves
    /// more than half the store's words free, as its values are large, to
    /// move them together.
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
            .map_err(|e| self.error(&e.context(Message::Setting { global })))?;
        self.tidy();
        Ok(())
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

        // The old parts are freed and the new ones' blocks taken before a
        // word is written, so a value that does not fit is undone and
        // leaves the store as it was.
        let mut plan = Plan::default();

        for (start, words) in old {
            self.heap.give(&mut plan, start, words);
        }

        let pool = self.heap.take_all(&mut plan, sizes);
        self.heap.settle(plan)?;

        // Measured, so it is a fault for the write to fail. The old parts
        // are freed by then, so the value is left unset rather than
        // pointing at them, never to be freed twice.
        let written = self.heap.write(pool, types, value, id, at);

        if written.is_err() {
            let slot = usize::try_from(global)
                .ok()
                .and_then(|n| self.layout.slots.get(n))
                .and_then(|held| usize::try_from(held.at).ok());

            if let Some(set) = slot.and_then(|at| self.set.get_mut(at)) {
                *set = false;
            }
        }

        written
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

        let mut fuel = memory::Fuel::UNLIMITED;
        memory::read(&self.heap, &self.layout.types, at, held.id?, &mut fuel).ok()
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
        })?;
        self.tidy();
        Ok(())
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
        let mut fuel = memory::Fuel::UNLIMITED;
        memory::read(
            &self.heap,
            &self.layout.types,
            place.at,
            place.ty,
            &mut fuel,
        )
        .ok()
    }

    /// Insert `value` at `path`, a list's index or a map's key, moving the
    /// entries after it along.
    ///
    /// - A list's index may be from 0 to its length, which pushes onto it.
    /// - A map's key goes where it keeps the keys in order. A key the map
    ///   has already is set to `value`, as [`set_at`](Self::set_at) sets it.
    ///
    /// The path steps through present options as `set_at`'s does, and
    /// through those of the list or map it ends in. Lists and maps in a
    /// store have room for more entries than they hold: one that is full
    /// moves to a block of twice its room, at least 4 entries, so an
    /// insert takes amortized O(1) words. It takes as long as the path, a
    /// binary search for a key, `value`, and moving the entries after it
    /// (a copy of words), or all of them when the list or map moves.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`], leaving the store as it was, when:
    ///
    /// - the path does not end with a list's index or a map's key, or
    ///   [`set_at`](Self::set_at) would refuse the path to the list or map;
    /// - an index is past the list's length, or a key is not of the map's
    ///   key type;
    /// - `value` is not of the entry's type, or it, and the list or map
    ///   when it moves, do not fit in the words left.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("skills", Type::map(Type::string(), Type::int()))?;
    /// env.value("tracks", Type::list(Type::int()))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("skills", &Value::map([(Value::string("occult"), Value::int(3))]))?;
    /// globals.set("tracks", &Value::list([Value::int(1), Value::int(3)]))?;
    ///
    /// globals.insert(&Path::value("skills").string_key("brawl"), &Value::int(2))?;
    /// globals.insert(&Path::value("tracks").index(1), &Value::int(2))?;
    /// globals.push(&Path::value("tracks"), &Value::int(4))?;
    ///
    /// let program = evalite::compile(r#"skills["brawl"] + tracks[1] + tracks.len()"#, &env)?;
    /// assert_eq!(Machine::default().load(&program, &globals)?.run()?.value(), &Value::int(8));
    ///
    /// let error = globals.insert(&Path::value("tracks").index(5), &Value::int(5)).unwrap_err();
    /// assert_eq!(
    ///     format!("{error:#}"),
    ///     "index 5 is out of range of a list of 4 entries: at `tracks`: while inserting at `tracks[5]`",
    /// );
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn insert(&mut self, path: &Path, value: &Value) -> Result<(), GlobalsError> {
        self.insert_at(path, value).map_err(|e| {
            let path = path.to_string().into_boxed_str();
            self.error(&e.context(Message::InsertingAt { path }))
        })?;
        self.tidy();
        Ok(())
    }

    /// Push `value` onto the list at `path`, as [`insert`](Self::insert)
    /// at its length does.
    ///
    /// # Errors
    ///
    /// As [`insert`](Self::insert), and when the path is not to a list.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("marks", Type::list(Type::bool()))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("marks", &Value::list([]))?;
    ///
    /// for _ in 0..3 {
    ///     globals.push(&Path::value("marks"), &Value::bool(true))?;
    /// }
    ///
    /// assert_eq!(globals.get("marks"), Some(Value::list(vec![Value::bool(true); 3])));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn push(&mut self, path: &Path, value: &Value) -> Result<(), GlobalsError> {
        self.push_at(path, value).map_err(|e| {
            let path = path.to_string().into_boxed_str();
            self.error(&e.context(Message::PushingOnto { path }))
        })?;
        self.tidy();
        Ok(())
    }

    /// Remove the entry at `path`, a list's index or a map's key, moving
    /// the entries after it back, and give its value.
    ///
    /// Its parts are freed. A list or map that falls below a quarter of its
    /// room frees the second half of it, keeping at least 4 entries' room,
    /// and one left empty frees its block. The words freed are taken again
    /// by blocks they hold, or compacted away when more than half the
    /// store's are free, so its memory follows its data. A remove takes no
    /// words, so it never fails for want of them. It takes as long as the
    /// path, a binary search for a key, the removed value, and moving the
    /// entries after it, plus a compaction's work when there is one.
    ///
    /// # Errors
    ///
    /// Returns a [`GlobalsError`], leaving the store as it was, when the
    /// path does not end with a list's index or a map's key, or
    /// [`set_at`](Self::set_at) would refuse it.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Path, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("tags", Type::map(Type::string(), Type::int()))?;
    ///
    /// let mut globals = Globals::new(&env);
    /// let tags = [("air", 1), ("fire", 2), ("water", 3)];
    /// globals.set("tags", &Value::map(tags.map(|(k, v)| (Value::string(k), Value::int(v)))))?;
    ///
    /// let fire = Path::value("tags").string_key("fire");
    /// assert_eq!(globals.remove(&fire)?, Value::int(2));
    /// assert_eq!(globals.get_at(&fire), None);
    ///
    /// let error = globals.remove(&fire).unwrap_err();
    /// assert_eq!(format!("{error:#}"), r#"the map has no key "fire": at `tags`: while removing `tags["fire"]`"#);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove(&mut self, path: &Path) -> Result<Value, GlobalsError> {
        let removed = self.remove_at(path).map_err(|e| {
            let path = path.to_string().into_boxed_str();
            self.error(&e.context(Message::RemovingAt { path }))
        })?;
        self.tidy();
        Ok(removed)
    }

    fn insert_at(&mut self, path: &Path, value: &Value) -> Result<(), RunError> {
        let (last, entries) = self.locate_entries(path)?;
        let Entries {
            place,
            kind,
            start,
            len,
            stride,
            ..
        } = &entries;
        let global = place.global;

        match (last, *kind) {
            (StepRepr::Index(index), Kind::List(entry)) => {
                let Some(n) = usize::try_from(*index).ok().filter(|n| *n <= *len) else {
                    let range = Message::IndexOutOfRange {
                        index: *index,
                        len: count(*len),
                    };
                    return Err(off_path(range, global, &place.segs));
                };

                self.open(&entries, n, &[(value, entry, Seg::Index(count(n)))])
            }
            (
                StepRepr::Key(key),
                Kind::Map {
                    key: key_ty,
                    value: value_ty,
                },
            ) => {
                let shown = self.shown_key(key, key_ty, global, &place.segs)?;

                match memory::search_key(&self.heap, *start, *len, *stride, key)? {
                    Some(Ok(n)) => {
                        let mut segs = place.segs.clone();
                        segs.push(Seg::Key(shown));
                        let at = n
                            .checked_mul(*stride)
                            .and_then(|at| at.checked_add(*start))
                            .and_then(|at| at.checked_add(1))
                            .ok_or_else(|| fault(Fault::OutsideMemory))?;
                        let place = Place {
                            global,
                            at,
                            ty: value_ty,
                            segs,
                        };
                        self.replace(&place, value, true)
                    }
                    Some(Err(n)) => self.open(
                        &entries,
                        n,
                        &[
                            (key, key_ty, Seg::KeyOf(count(n))),
                            (value, value_ty, Seg::Key(shown)),
                        ],
                    ),
                    None => Err(fault(Fault::ValueChangedType)),
                }
            }
            (last, _) => Err(self.not_entry(last, &entries)),
        }
    }

    fn push_at(&mut self, path: &Path, value: &Value) -> Result<(), RunError> {
        let place = self.locate(path)?;
        let (at, ty) = self.through_options(place.at, place.ty, place.global, &place.segs)?;
        let place = Place { at, ty, ..place };

        let Some(TypeNode::List(entry)) = self.layout.types.get(ty) else {
            return Err(RunError::new(Message::NotList { ty }));
        };

        let entry = *entry;
        let entries = self.entries(place)?;
        let n = entries.len;
        self.open(&entries, n, &[(value, entry, Seg::Index(count(n)))])
    }

    fn remove_at(&mut self, path: &Path) -> Result<Value, RunError> {
        let (last, entries) = self.locate_entries(path)?;
        let types = &self.layout.types;
        let mut fuel = memory::Fuel::UNLIMITED;
        let Entries {
            place,
            kind,
            start,
            len,
            stride,
            ..
        } = &entries;
        let global = place.global;

        let (n, removed, freed) = match (last, *kind) {
            (StepRepr::Index(index), Kind::List(entry)) => {
                let Some(n) = usize::try_from(*index).ok().filter(|n| *n < *len) else {
                    let range = Message::IndexOutOfRange {
                        index: *index,
                        len: count(*len),
                    };
                    return Err(off_path(range, global, &place.segs));
                };

                let at = n
                    .checked_mul(*stride)
                    .and_then(|at| at.checked_add(*start))
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                let removed = memory::read(&self.heap, types, at, entry, &mut fuel)?;
                (n, removed, memory::blocks(&self.heap, types, at, entry)?)
            }
            (
                StepRepr::Key(key),
                Kind::Map {
                    key: key_ty,
                    value: value_ty,
                },
            ) => {
                let shown = self.shown_key(key, key_ty, global, &place.segs)?;

                let n = match memory::search_key(&self.heap, *start, *len, *stride, key)? {
                    Some(Ok(n)) => n,
                    Some(Err(_)) => {
                        let missing = Message::NoSuchKey { key: shown };
                        return Err(off_path(missing, global, &place.segs));
                    }
                    None => return Err(fault(Fault::ValueChangedType)),
                };

                let at = n
                    .checked_mul(*stride)
                    .and_then(|at| at.checked_add(*start))
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                let value_at = at
                    .checked_add(1)
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                let removed = memory::read(&self.heap, types, value_at, value_ty, &mut fuel)?;
                let mut freed = memory::blocks(&self.heap, types, at, key_ty)?;
                freed.extend(memory::blocks(&self.heap, types, value_at, value_ty)?);
                (n, removed, freed)
            }
            (last, _) => return Err(self.not_entry(last, &entries)),
        };

        self.close(&entries, n, freed)?;
        Ok(removed)
    }

    /// The list or map a path to insert or remove at ends in, and the step
    /// into it.
    fn locate_entries<'a>(&self, path: &'a Path) -> Result<(&'a StepRepr, Entries), RunError> {
        let Some((last, steps)) = path.steps.split_last() else {
            return Err(RunError::new(Message::NotEntryPath));
        };

        let place = self.locate_steps(path, steps)?;
        let (at, ty) = self.through_options(place.at, place.ty, place.global, &place.segs)?;
        let place = Place { at, ty, ..place };

        if let StepRepr::Field(_) = last {
            return Err(RunError::new(Message::NotEntryPath));
        }

        match self.layout.types.get(ty) {
            Some(TypeNode::List(_) | TypeNode::Map { .. }) => Ok((last, self.entries(place)?)),
            _ => Err(no_step(last, ty, place.global, &place.segs)),
        }
    }

    /// Why the step `last` cannot be taken into `entries`.
    #[cold]
    fn not_entry(&self, last: &StepRepr, entries: &Entries) -> RunError {
        let place = &entries.place;

        match last {
            StepRepr::Field(_) => RunError::new(Message::NotEntryPath),
            _ => no_step(last, place.ty, place.global, &place.segs),
        }
    }

    /// The list or map held at `place`.
    fn entries(&self, place: Place) -> Result<Entries, RunError> {
        let types = &self.layout.types;

        let (kind, stride) = match types.get(place.ty) {
            Some(TypeNode::List(entry)) => (Kind::List(*entry), types.width(*entry)),
            Some(TypeNode::Map { key, value }) => {
                let stride = types
                    .width(*value)
                    .checked_add(1)
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                (
                    Kind::Map {
                        key: *key,
                        value: *value,
                    },
                    stride,
                )
            }
            _ => return Err(fault(Fault::ValueChangedType)),
        };

        let (start, len) = get(&self.heap, Some(place.at))?.as_reference()?;
        inside(&self.heap, start, len.checked_mul(stride))?;
        let cap = memory::capacity(&self.heap, start, len)?;

        Ok(Entries {
            place,
            kind,
            start,
            len,
            cap,
            stride,
        })
    }

    /// Open the entry `n` of `entries`, moving those from it along, and
    /// write `parts` into it: each value of its type at its offset in the
    /// entry (a map's key, then its value). A full list or map first moves
    /// to a block of twice its room.
    ///
    /// Each part is checked and the words they and a move take are found
    /// before anything changes, so an error leaves the store as it was.
    fn open(
        &mut self,
        entries: &Entries,
        n: usize,
        parts: &[(&Value, TypeId, Seg)],
    ) -> Result<(), RunError> {
        let &Entries {
            ref place,
            start,
            len,
            cap,
            stride,
            ..
        } = entries;
        let types = &self.layout.types;

        let mut sizes = Vec::new();

        for (value, ty, seg) in parts {
            let mut segs = place.segs.clone();
            segs.push(seg.clone());
            memory::measure_at(types, value, *ty, Root::Global(place.global), &segs)?;
            sizes.extend(memory::parts(types, value, *ty)?);
        }

        // Refused as a set refuses a list or map too long for a reference,
        // before anything changes.
        let new_len = len.saturating_add(1);
        memory::check_len(new_len)?;

        // A full one moves to a block of twice its room, taken before its
        // own is freed, as its entries are copied out of it.
        let grown = if new_len > cap {
            let new_cap = cap
                .checked_mul(2)
                .ok_or_else(|| fault(Fault::ListTooLarge))?
                .max(MIN_CAPACITY);
            let words =
                memory::entries_block(new_cap, stride).ok_or_else(|| fault(Fault::ListTooLarge))?;
            Some((new_cap, words, self.block(start, cap, stride)?))
        } else {
            None
        };

        let mut plan = Plan::default();

        let grown = grown.map(|(new_cap, words, old)| {
            let block = self.heap.take(&mut plan, words);

            if let Some((start, words)) = old {
                self.heap.give(&mut plan, start, words);
            }

            (new_cap, block)
        });

        let pool = self.heap.take_all(&mut plan, sizes);
        self.heap.settle(plan)?;

        let before = n
            .checked_mul(stride)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let after = len
            .checked_sub(n)
            .and_then(|after| after.checked_mul(stride))
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let from = start
            .checked_add(before)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;

        let opening = Opening {
            grown,
            pool,
            before,
            after,
            from,
            new_len,
        };
        let written = self.open_at(entries, parts, opening);
        self.unset_on_fault(place.global, written)
    }

    /// The changes of [`open`](Self::open), once it has found they fit.
    fn open_at(
        &mut self,
        entries: &Entries,
        parts: &[(&Value, TypeId, Seg)],
        opening: Opening,
    ) -> Result<(), RunError> {
        let Entries {
            ref place,
            start,
            stride,
            ..
        } = *entries;
        let Opening {
            grown,
            pool,
            before,
            after,
            from,
            new_len,
        } = opening;
        let types = &self.layout.types;

        let start = match grown {
            Some((new_cap, block)) => {
                let new_start = self.heap.header(block, new_cap)?;
                self.heap.copy(start, before, new_start)?;
                let to = new_start
                    .checked_add(before)
                    .and_then(|to| to.checked_add(stride))
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                self.heap.copy(from, after, to)?;
                new_start
            }
            None => {
                let to = from
                    .checked_add(stride)
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                self.heap.copy(from, after, to)?;
                start
            }
        };

        let entry = start
            .checked_add(before)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;

        let mut planned = Planned {
            heap: &mut self.heap,
            pool,
        };

        let written = parts
            .iter()
            .enumerate()
            .try_for_each(|(offset, (value, ty, _))| {
                let dest = entry
                    .checked_add(offset)
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                memory::write(&mut planned, types, value, *ty, dest)
            });

        planned.finish();
        written?;
        let reference = Word::reference(start, new_len)?;
        self.heap.put(place.at, reference)
    }

    /// `result`, leaving the value `global` unset when it failed: checked
    /// before anything changed, so a fault, and its parts may be freed by
    /// then, so it is never to point at them, as [`replace`](Self::replace)
    /// leaves one.
    fn unset_on_fault(
        &mut self,
        global: u32,
        result: Result<(), RunError>,
    ) -> Result<(), RunError> {
        if result.is_err() {
            let slot = usize::try_from(global)
                .ok()
                .and_then(|n| self.layout.slots.get(n))
                .and_then(|held| usize::try_from(held.at).ok());

            if let Some(set) = slot.and_then(|at| self.set.get_mut(at)) {
                *set = false;
            }
        }

        result
    }

    /// Close the entry `n` of `entries`, freeing `freed`, its parts' blocks,
    /// and moving those after it back. One left below a quarter of its room
    /// frees the second half of its block, keeping room for at least
    /// [`MIN_CAPACITY`] entries, and one left empty frees its block.
    fn close(
        &mut self,
        entries: &Entries,
        n: usize,
        freed: Vec<(usize, usize)>,
    ) -> Result<(), RunError> {
        let &Entries {
            ref place,
            start,
            len,
            cap,
            stride,
            ..
        } = entries;

        let new_len = len
            .checked_sub(1)
            .ok_or_else(|| fault(Fault::ListLostEntries))?;
        let to = n
            .checked_mul(stride)
            .and_then(|to| to.checked_add(start))
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let from = to
            .checked_add(stride)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let after = new_len
            .checked_sub(n)
            .and_then(|after| after.checked_mul(stride))
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let block = self.block(start, cap, stride)?;

        for (start, words) in freed {
            self.heap.free(start, words);
        }

        let closed = self.close_at(entries, new_len, block, (from, after, to));
        self.unset_on_fault(place.global, closed)
    }

    /// The moves of [`close`](Self::close), once its parts are freed.
    fn close_at(
        &mut self,
        entries: &Entries,
        new_len: usize,
        block: Option<(usize, usize)>,
        (from, after, to): (usize, usize, usize),
    ) -> Result<(), RunError> {
        let Entries {
            ref place,
            start,
            cap,
            stride,
            ..
        } = *entries;
        self.heap.copy(from, after, to)?;

        if new_len == 0 {
            if let Some((block, words)) = block {
                self.heap.free(block, words);
            }
        } else if cap > MIN_CAPACITY && new_len.saturating_mul(4) < cap {
            let new_cap = (cap / 2).max(MIN_CAPACITY);
            let tail = new_cap
                .checked_mul(stride)
                .and_then(|tail| tail.checked_add(start))
                .ok_or_else(|| fault(Fault::OutsideMemory))?;
            let words = cap
                .checked_sub(new_cap)
                .and_then(|n| n.checked_mul(stride))
                .ok_or_else(|| fault(Fault::OutsideMemory))?;
            let block = start
                .checked_sub(memory::HEADER)
                .ok_or_else(|| fault(Fault::OutsideMemory))?;
            self.heap.header(block, new_cap)?;
            self.heap.free(tail, words);
        }

        let reference = Word::reference(start, new_len)?;
        self.heap.put(place.at, reference)
    }

    /// The block, from its header, of a list's or map's room for `cap`
    /// entries of `stride` words from `start`: none when it has no room.
    fn block(
        &self,
        start: usize,
        cap: usize,
        stride: usize,
    ) -> Result<Option<(usize, usize)>, RunError> {
        if cap == 0 {
            return Ok(None);
        }

        let block = start.checked_sub(memory::HEADER);
        let words = memory::entries_block(cap, stride);

        match (block, words) {
            (Some(block), Some(words)) => Ok(Some((block, words))),
            _ => Err(fault(Fault::OutsideMemory)),
        }
    }

    /// Where the part at `path` is held, and its type.
    ///
    /// Each step is checked against the type of the part before it, and
    /// reads the reference word it steps through: a constant number of
    /// words for a field or an index, a binary search for a key.
    fn locate(&self, path: &Path) -> Result<Place, RunError> {
        self.locate_steps(path, &path.steps)
    }

    /// As [`locate`](Self::locate), the part at the first `steps` of
    /// `path`.
    fn locate_steps(&self, path: &Path, steps: &[StepRepr]) -> Result<Place, RunError> {
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
        let mut segs = Vec::with_capacity(steps.len());

        for step in steps {
            (at, ty) = self.through_options(at, ty, global, &segs)?;

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
                    let shown = self.shown_key(key, *key_ty, global, &segs)?;
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
                (step, _) => return Err(no_step(step, ty, global, &segs)),
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

    /// From the part of type `ty` at `at`, through each present option to
    /// what it holds; an absent one is `NoneOnPath`, from `segs`.
    fn through_options(
        &self,
        mut at: usize,
        mut ty: TypeId,
        global: u32,
        segs: &[Seg],
    ) -> Result<(usize, TypeId), RunError> {
        let types = &self.layout.types;

        while let Some(TypeNode::Option(inner)) = types.get(ty) {
            let word = get(&self.heap, Some(at))?;

            let present = if types.tagged(ty).is_some() {
                at = at
                    .checked_add(1)
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                word.as_bool()?
            } else {
                !word.is_none()
            };

            if !present {
                return Err(off_path(Message::NoneOnPath, global, segs));
            }

            ty = *inner;
        }

        Ok((at, ty))
    }

    /// `key` as a step's key of a map whose keys are `key_ty`, refused
    /// when of another type, from `segs`.
    fn shown_key(
        &self,
        key: &Value,
        key_ty: TypeId,
        global: u32,
        segs: &[Seg],
    ) -> Result<Key, RunError> {
        match (&key.0, self.layout.types.get(key_ty)) {
            (Repr::Int(n), Some(TypeNode::Int)) => Ok(Key::Int(*n)),
            (Repr::String(s), Some(TypeNode::String)) => Ok(Key::string(s)),
            _ => {
                let mistyped = Message::KeyMistyped {
                    expected: key_ty,
                    found: key.describe(),
                };
                Err(off_path(mistyped, global, segs))
            }
        }
    }

    /// The slot word and type of each value set.
    fn roots(&self) -> Vec<(usize, TypeId)> {
        self.layout
            .slots
            .iter()
            .filter_map(|held| {
                let at = usize::try_from(held.at).ok()?;
                let set = self.set.get(at).copied().unwrap_or(false);
                set.then_some((at, held.id?))
            })
            .collect()
    }

    /// Move the values' parts together when more than half the words they
    /// take are free, so the store takes at most twice the words of its
    /// values. A fault leaves it as it was.
    fn tidy(&mut self) {
        if self.heap.free_words.saturating_mul(2) <= self.heap.used() {
            return;
        }

        if let Ok(words) = self.heap.compacted(&self.layout.types, self.roots()) {
            self.heap.words = words;
            self.heap.free.clear();
            self.heap.free_words = 0;
        }
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

    /// The entries the list or map at `path` holds, and has room for.
    #[cfg(test)]
    pub(crate) fn room_at(&self, path: &Path) -> Option<(usize, usize)> {
        let place = self.locate(path).ok()?;
        let (at, ty) = self
            .through_options(place.at, place.ty, place.global, &place.segs)
            .ok()?;
        let entries = self.entries(Place { at, ty, ..place }).ok()?;
        Some((entries.len, entries.cap))
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

impl Heap {
    fn put(&mut self, at: usize, word: Word) -> Result<(), RunError> {
        let slot = self
            .words
            .get_mut(at)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        *slot = word;
        Ok(())
    }

    /// Write the header of a block of `cap` entries from `block`, giving
    /// where its entries start.
    fn header(&mut self, block: usize, cap: usize) -> Result<usize, RunError> {
        let cap = i64::try_from(cap).map_err(|_| fault(Fault::ListTooLarge))?;
        self.put(block, Word::int(cap))?;
        block
            .checked_add(memory::HEADER)
            .ok_or_else(|| fault(Fault::OutsideMemory))
    }

    /// Free the `words` words from `start`, for a block to take.
    fn free(&mut self, start: usize, words: usize) {
        if words > 0 {
            self.free.entry(words).or_default().push(start);
            self.free_words = self.free_words.saturating_add(words);
        }
    }

    /// As [`free`](Self::free), as part of `plan`.
    fn give(&mut self, plan: &mut Plan, start: usize, words: usize) {
        if words > 0 {
            self.free(start, words);
            plan.changes.push(Change::Freed(words, start));
        }
    }

    /// Take a block of `n` words as part of `plan`: the free block of the
    /// smallest size that holds it, freeing what it leaves, or else past
    /// the heap's end, where it is taken only when `plan` is
    /// [`settle`](Self::settle)d. A log per size of free blocks.
    fn take(&mut self, plan: &mut Plan, n: usize) -> usize {
        let fit = self.free.range_mut(n..).next().map(|(size, starts)| {
            let start = starts.pop();
            (*size, start, starts.is_empty())
        });

        if let Some((size, Some(start), empty)) = fit {
            if empty {
                self.free.remove(&size);
            }

            self.free_words = self.free_words.saturating_sub(size);
            plan.changes.push(Change::Took(size, start));
            // Both inside the block, so neither overflows.
            let rest = start.saturating_add(n);
            self.give(plan, rest, size.saturating_sub(n));
            return start;
        }

        // A plan past what the heap can address is refused by `settle`
        // before any of it is used.
        let at = self.words.len().saturating_add(plan.past);
        plan.past = plan.past.saturating_add(n);
        at
    }

    /// Take a block of each of `sizes` as part of `plan`, the largest
    /// first, so that smaller ones take what they leave.
    fn take_all(&mut self, plan: &mut Plan, mut sizes: Vec<usize>) -> Pool {
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        let mut pool = Pool::new();

        for n in sizes {
            let at = self.take(plan, n);
            pool.entry(n).or_default().push(at);
        }

        pool
    }

    /// Keep `plan` when what it takes past the heap's end fits in the words
    /// left, taking them; else undo it, leaving the free blocks as they
    /// were.
    fn settle(&mut self, plan: Plan) -> Result<(), RunError> {
        let left = self.room.saturating_sub(self.used());

        if plan.past > left {
            let needed = plan.past;
            self.undo(plan);

            return Err(RunError::new(Message::TooLarge {
                needed: count(needed),
                left: count(left),
            }));
        }

        let end = self.words.len().saturating_add(plan.past);
        self.words.resize(end, Word::int(0));
        Ok(())
    }

    /// Undo the changes of `plan` to the free blocks, the last first.
    fn undo(&mut self, plan: Plan) {
        for change in plan.changes.into_iter().rev() {
            match change {
                Change::Took(size, start) => {
                    self.free.entry(size).or_default().push(start);
                    self.free_words = self.free_words.saturating_add(size);
                }
                Change::Freed(size, start) => {
                    let Some(starts) = self.free.get_mut(&size) else {
                        continue;
                    };

                    // The last freed, as it is undone first.
                    if let Some(n) = starts.iter().rposition(|s| *s == start) {
                        starts.remove(n);
                        self.free_words = self.free_words.saturating_sub(size);
                    }

                    if starts.is_empty() {
                        self.free.remove(&size);
                    }
                }
            }
        }
    }

    /// Write `value`, measured against `ty`, at `dest`, its parts taking the
    /// blocks of `pool`.
    fn write(
        &mut self,
        pool: Pool,
        types: &Types,
        value: &Value,
        ty: TypeId,
        dest: usize,
    ) -> Result<(), RunError> {
        let mut planned = Planned { heap: self, pool };
        let written = memory::write(&mut planned, types, value, ty, dest);
        planned.finish();
        written
    }

    /// The words of the slots and of each part of the values of `roots`
    /// (each a slot's word and type), the parts moved together after the
    /// slots, in the order they are reached, and the references to them
    /// rewritten. Lists and maps keep their room. O(live words), with an
    /// explicit work list.
    fn compacted(
        &self,
        types: &Types,
        mut work: Vec<(usize, TypeId)>,
    ) -> Result<Vec<Word>, RunError> {
        let mut out = self
            .words
            .get(..self.slot_words)
            .ok_or_else(|| fault(Fault::OutsideMemory))?
            .to_vec();
        out.reserve(self.used().saturating_sub(self.free_words));

        // Values share no parts, so each block is copied once and the
        // words copied are at most the heap's; more is a fault, and the
        // store is left as it was.
        let most = self.words.len();

        let copy = |out: &mut Vec<Word>, start: usize, n: usize| {
            let end = start.checked_add(n);
            let block = end.and_then(|end| self.words.get(start..end));
            let to = out.len();

            match block {
                Some(block) if to.checked_add(n).is_some_and(|end| end <= most) => {
                    out.extend_from_slice(block);
                    Ok(to)
                }
                _ => Err(fault(Fault::OutsideMemory)),
            }
        };

        let put = |out: &mut Vec<Word>, at: usize, word: Word| {
            let slot = out.get_mut(at).ok_or_else(|| fault(Fault::OutsideMemory))?;
            *slot = word;
            Ok::<_, RunError>(())
        };

        let step = |at: usize, by: usize| {
            at.checked_add(by)
                .ok_or_else(|| fault(Fault::OutsideMemory))
        };

        while let Some((at, ty)) = work.pop() {
            let word = *out.get(at).ok_or_else(|| fault(Fault::OutsideMemory))?;

            match types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))? {
                TypeNode::Int | TypeNode::Bool => {}
                TypeNode::Option(inner) => {
                    if types.tagged(ty).is_some() {
                        if word.as_bool()? {
                            work.push((step(at, 1)?, *inner));
                        }
                    } else if !word.is_none() {
                        work.push((at, *inner));
                    }
                }
                TypeNode::String => {
                    let (start, len) = word.as_reference()?;
                    let to = copy(&mut out, start, len.div_ceil(8))?;
                    put(&mut out, at, Word::reference(to, len)?)?;
                }
                TypeNode::Record(fields) => {
                    let (start, len) = word.as_reference()?;

                    if len != types.record_words(fields) {
                        return Err(fault(Fault::RecordWrongSize));
                    }

                    let to = copy(&mut out, start, len)?;
                    put(&mut out, at, Word::reference(to, len)?)?;

                    for (offset, field) in types.field_offsets(fields) {
                        work.push((step(to, offset)?, field.ty));
                    }
                }
                node @ (TypeNode::List(_) | TypeNode::Map { .. }) => {
                    let (start, len) = word.as_reference()?;

                    let (stride, parts) = match node {
                        TypeNode::Map { key, value } => {
                            let stride = types.width(*value).saturating_add(1);
                            (stride, [Some((0, *key)), Some((1, *value))])
                        }
                        TypeNode::List(entry) => (types.width(*entry), [Some((0, *entry)), None]),
                        _ => return Err(fault(Fault::ValueChangedType)),
                    };

                    if len == 0 {
                        let to = out.len();
                        put(&mut out, at, Word::reference(to, 0)?)?;
                        continue;
                    }

                    let cap = memory::capacity(self, start, len)?;
                    let block = start
                        .checked_sub(memory::HEADER)
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    let words = memory::entries_block(cap, stride)
                        .ok_or_else(|| fault(Fault::ListTooLarge))?;
                    let to = step(copy(&mut out, block, words)?, memory::HEADER)?;
                    put(&mut out, at, Word::reference(to, len)?)?;

                    for (offset, ty) in parts.into_iter().flatten() {
                        if !holds_parts(types, ty) {
                            continue;
                        }

                        work.reserve(len);

                        for n in 0..len {
                            let entry = n
                                .checked_mul(stride)
                                .and_then(|at| at.checked_add(to))
                                .and_then(|at| at.checked_add(offset))
                                .ok_or_else(|| fault(Fault::OutsideMemory))?;
                            work.push((entry, ty));
                        }
                    }
                }
            }
        }

        Ok(out)
    }

    /// Copy the `n` words from `from` to `to`, which may overlap.
    fn copy(&mut self, from: usize, n: usize, to: usize) -> Result<(), RunError> {
        if n == 0 {
            return Ok(());
        }

        let end = from.checked_add(n);
        let fits = |at: usize| at.checked_add(n).is_some_and(|end| end <= self.words.len());

        match end {
            Some(end) if fits(from) && fits(to) => {
                self.words.copy_within(from..end, to);
                Ok(())
            }
            _ => Err(fault(Fault::OutsideMemory)),
        }
    }
}

/// The room a list or a map in a store has for entries when it grows
/// past what it holds, and that it keeps when it shrinks.
const MIN_CAPACITY: usize = 4;

/// What [`Globals::open`] found it is to change: the list's or map's room
/// and new block when it grows, the blocks taken for the parts written,
/// the words of the entries before the one opened and from it, where
/// those start, and the new length.
struct Opening {
    grown: Option<(usize, usize)>,
    pool: Pool,
    before: usize,
    after: usize,
    from: usize,
    new_len: usize,
}

/// The blocks a [`Plan`] took for the parts of values, by their size in
/// words, for the write of those values to take.
type Pool = BTreeMap<usize, Vec<usize>>;

/// The changes an operation makes to a store's free blocks, made before it
/// writes a word, so that one which does not fit is undone and leaves the
/// store as it was.
#[derive(Default)]
struct Plan {
    /// In the order they were made.
    changes: Vec<Change>,
    /// The words it takes past the heap's end.
    past: usize,
}

/// A change to the free blocks, by size and start.
enum Change {
    Took(usize, usize),
    Freed(usize, usize),
}

/// A store's heap as a write takes its blocks, from those planned.
struct Planned<'a> {
    heap: &'a mut Heap,
    pool: Pool,
}

impl Planned<'_> {
    /// Free the blocks the write did not take, which it takes all of but
    /// for a fault, so none is lost.
    fn finish(self) {
        for (words, starts) in self.pool {
            for start in starts {
                self.heap.free(start, words);
            }
        }
    }
}

impl Words for Planned<'_> {
    #[inline]
    fn word(&self, at: usize) -> Option<Word> {
        self.heap.word(at)
    }

    fn end(&self) -> usize {
        self.heap.end()
    }
}

impl WordsMut for Planned<'_> {
    fn set_word(&mut self, at: usize, word: Word) -> Option<()> {
        *self.heap.words.get_mut(at)? = word;
        Some(())
    }

    fn alloc(&mut self, n: usize) -> Result<usize, RunError> {
        if n == 0 {
            return Ok(self.heap.words.len());
        }

        self.pool
            .get_mut(&n)
            .and_then(Vec::pop)
            .ok_or_else(|| fault(Fault::StorePastLimit))
    }

    fn alloc_entries(&mut self, len: usize, stride: usize) -> Result<usize, RunError> {
        if len == 0 {
            return Ok(self.heap.words.len());
        }

        let n = memory::entries_block(len, stride).ok_or_else(|| fault(Fault::ListTooLarge))?;
        let block = self.alloc(n)?;
        self.heap.header(block, len)
    }
}

/// Whether a value of type `ty` may have parts, for a walk to visit.
fn holds_parts(types: &Types, ty: TypeId) -> bool {
    !matches!(types.get(ty), Some(TypeNode::Int | TypeNode::Bool))
}

/// The words a [`Globals`] store's values take, as
/// [`Globals::stats`] gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalsStats {
    live: u32,
    spare: u32,
    free: u32,
    heap: u32,
}

impl GlobalsStats {
    /// The words the parts of the values set take, the headers and room of
    /// their lists and maps included.
    pub fn live(&self) -> u32 {
        self.live
    }

    /// Of [`live`](Self::live), the room of lists and maps past their
    /// entries.
    pub fn spare(&self) -> u32 {
        self.spare
    }

    /// The words freed and not yet taken again.
    pub fn free(&self) -> u32 {
        self.free
    }

    /// The words the parts take, free ones included: what counts against
    /// the [`limit`](Globals::limit), and at most twice
    /// [`live`](Self::live).
    pub fn heap(&self) -> u32 {
        self.heap
    }
}

/// A list or a map held in a store.
struct Entries {
    /// Where its reference word is.
    place: Place,
    kind: Kind,
    start: usize,
    len: usize,
    /// The entries it has room for.
    cap: usize,
    /// The words of an entry.
    stride: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    List(TypeId),
    Map { key: TypeId, value: TypeId },
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

/// That a value of type `ty` has no part at `step`, from the part at
/// `segs` of the value `global`.
#[cold]
#[inline(never)]
fn no_step(step: &StepRepr, ty: TypeId, global: u32, segs: &[Seg]) -> RunError {
    let step = match step {
        StepRepr::Field(name) => PathStep::Field(cut(name)),
        StepRepr::Key(key) => match (&key.0, key.as_int()) {
            (_, Some(n)) => PathStep::Key(Key::Int(n)),
            (Repr::String(s), _) => PathStep::Key(Key::string(s)),
            _ => return fault(Fault::ValueChangedType),
        },
        StepRepr::Index(index) => PathStep::Index(*index),
        StepRepr::Dynamic => return fault(Fault::DynamicStep),
    };

    off_path(Message::NoStep { ty, step }, global, segs)
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
