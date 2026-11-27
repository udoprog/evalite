//! Values in words of memory: checking a [`Value`] against its type, writing
//! it in the layout of the `word` module, reading it back, and finding the
//! blocks its parts take. A run's memory and a [`Globals`](crate::Globals)
//! store both hold values this way, so the code is shared through
//! [`Words`].
//!
//! Every walk uses an explicit work list, never recursion, and visits each
//! part of a value once: values are trees, so no part is reached twice.

use std::cmp::Ordering;

use crate::message::{Fault, Found, Key, Message, Root, RunError, Seg};
use crate::types::{TypeId, TypeNode, Types};
use crate::value::{Repr, Value, key_order};
use crate::word::Word;

/// Memory values are read from.
pub(crate) trait Words {
    /// The word at `at`, when it is inside memory.
    fn word(&self, at: usize) -> Option<Word>;

    /// One past the last word that may be read.
    fn end(&self) -> usize;
}

/// Memory values are written to, taking new memory for their parts.
pub(crate) trait WordsMut: Words {
    /// Set the word at `at`, when it may be written.
    fn set_word(&mut self, at: usize, word: Word) -> Option<()>;

    /// Take `n` words, giving where they start. The caller has checked
    /// they fit, so running out is a fault.
    fn alloc(&mut self, n: usize) -> Result<usize, RunError>;

    /// Take the words of `len` entries of `stride` words each, a list's or
    /// a map's, giving where the entries start. A store puts a capacity
    /// header before them ([`HEADER`]); a run's memory does not.
    fn alloc_entries(&mut self, len: usize, stride: usize) -> Result<usize, RunError> {
        let n = len
            .checked_mul(stride)
            .ok_or_else(|| fault(Fault::ListTooLarge))?;
        self.alloc(n)
    }
}

/// The words of the capacity header a store keeps just before the entries
/// of a list or a map that has any: an `Int`, the entries the block has
/// room for. The reference word keeps (start, len) as a run reads it, and
/// a run never reads the header.
pub(crate) const HEADER: usize = 1;

/// The words of a store's block of `cap` entries of `stride` words: none
/// when it has no room, else the header and the entries.
pub(crate) fn entries_block(cap: usize, stride: usize) -> Option<usize> {
    if cap == 0 {
        return Some(0);
    }

    cap.checked_mul(stride)?.checked_add(HEADER)
}

/// The capacity of a store's list or map of `len` entries from `start`:
/// the header before them, or none for an empty one, which has no block.
pub(crate) fn capacity(
    words: &(impl Words + ?Sized),
    start: usize,
    len: usize,
) -> Result<usize, RunError> {
    if len == 0 {
        return Ok(0);
    }

    let header = get(words, start.checked_sub(HEADER))?.as_int();

    usize::try_from(header)
        .ok()
        .filter(|cap| *cap >= len)
        .ok_or_else(|| fault(Fault::BadCapacity))
}

/// A run's steps: those it may take, and those left. Reading a value
/// spends a step per word of its parts, before it reads them, so a read
/// past the steps left stops without doing the work.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fuel {
    fuel: u64,
    left: u64,
}

impl Fuel {
    /// No limit, for reads outside a run.
    pub(crate) const UNLIMITED: Self = Self::new(u64::MAX);

    /// `fuel` steps, none taken.
    pub(crate) const fn new(fuel: u64) -> Self {
        Self { fuel, left: fuel }
    }

    /// The steps taken.
    pub(crate) const fn spent(&self) -> u64 {
        self.fuel.saturating_sub(self.left)
    }

    /// Take `steps` steps, or fail with nothing taken when fewer are left.
    #[inline]
    pub(crate) fn spend(&mut self, steps: usize) -> Result<(), RunError> {
        match self.left.checked_sub(count(steps)) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => Err(RunError::new(Message::OutOfFuel { fuel: self.fuel })),
        }
    }
}

#[cold]
#[inline(never)]
pub(crate) fn fault(fault: Fault) -> RunError {
    RunError::from(fault)
}

/// The word at `at`.
#[inline]
pub(crate) fn get(words: &(impl Words + ?Sized), at: Option<usize>) -> Result<Word, RunError> {
    at.and_then(|at| words.word(at))
        .ok_or_else(|| fault(Fault::OutsideMemory))
}

/// Set the word at `at`.
#[inline]
pub(crate) fn put(
    words: &mut (impl WordsMut + ?Sized),
    at: Option<usize>,
    word: Word,
) -> Result<(), RunError> {
    at.and_then(|at| words.set_word(at, word))
        .ok_or_else(|| fault(Fault::OutsideMemory))
}

/// Check the `len` words from `at` are inside memory.
pub(crate) fn inside(
    words: &(impl Words + ?Sized),
    at: usize,
    len: Option<usize>,
) -> Result<(), RunError> {
    len.and_then(|len| at.checked_add(len))
        .filter(|end| *end <= words.end())
        .map(|_| ())
        .ok_or_else(|| fault(Fault::OutsideMemory))
}

/// Pack `bytes` 8 to a word from `at`.
pub(crate) fn put_bytes(
    words: &mut (impl WordsMut + ?Sized),
    at: usize,
    bytes: &[u8],
) -> Result<(), RunError> {
    for (n, chunk) in bytes.chunks(8).enumerate() {
        let mut word = [0u8; 8];

        for (b, c) in word.iter_mut().zip(chunk) {
            *b = *c;
        }

        put(words, at.checked_add(n), Word::bytes(word))?;
    }

    Ok(())
}

/// The string of `len` bytes packed from `at`.
pub(crate) fn string(
    words: &(impl Words + ?Sized),
    at: usize,
    len: usize,
) -> Result<String, RunError> {
    inside(words, at, Some(len.div_ceil(8)))?;
    let mut bytes = Vec::with_capacity(len);

    for n in 0..len.div_ceil(8) {
        bytes.extend_from_slice(&get(words, at.checked_add(n))?.as_bytes());
    }

    bytes.truncate(len);
    String::from_utf8(bytes).map_err(|_| fault(Fault::NotUtf8))
}

/// Check `value` is of type `ty`, and that no string, list or map in it is
/// too long for a reference, giving the words its parts take; a mismatch
/// names its path from `root`.
///
/// The walk keeps no path, as most values are of their type; a mismatch is
/// found again with one, to name where it is.
pub(crate) fn measure(
    types: &Types,
    value: &Value,
    ty: TypeId,
    root: Root,
) -> Result<usize, RunError> {
    measure_at(types, value, ty, root, &[])
}

/// As [`measure`], for a value held at `prefix` from `root`, which a
/// mismatch's path starts with.
pub(crate) fn measure_at(
    types: &Types,
    value: &Value,
    ty: TypeId,
    root: Root,
    prefix: &[Seg],
) -> Result<usize, RunError> {
    match walk(types, value, ty, &mut NoPaths, &mut ()) {
        Ok(total) => Ok(total),
        Err(Stop::Fault(error)) => Err(error),
        Err(Stop::Mismatch(_)) => {
            let mut nodes = Nodes::default();

            match walk(types, value, ty, &mut nodes, &mut ()) {
                Err(Stop::Mismatch(mismatch)) => Err(mistyped(mismatch, root, prefix, &nodes.0)),
                Err(Stop::Fault(error)) => Err(error),
                // The same walk, so the same mismatch.
                Ok(_) => Err(fault(Fault::ValueChangedType)),
            }
        }
    }
}

/// The words of each block the parts of `value`, measured against `ty`
/// already, take when a store writes it, one entry per block of at least a
/// word, in no particular order: a list's and a map's include the
/// [`HEADER`].
pub(crate) fn parts(types: &Types, value: &Value, ty: TypeId) -> Result<Vec<usize>, RunError> {
    let mut blocks = Vec::new();

    match walk(types, value, ty, &mut NoPaths, &mut blocks) {
        Ok(_) => Ok(blocks),
        Err(Stop::Fault(error)) => Err(error),
        Err(Stop::Mismatch(_)) => Err(fault(Fault::ValueChangedType)),
    }
}

/// The longest string, in bytes, or list or map, in entries, a value may
/// hold: a reference word keeps its length in 32 bits. A walk refuses a
/// longer one, so a set fails before it changes the store, not when
/// [`write`] makes the reference.
#[cfg(not(test))]
fn max_len() -> usize {
    usize::try_from(u32::MAX).unwrap_or(usize::MAX)
}

#[cfg(test)]
thread_local! {
    /// [`max_len`], which a test lowers to reach it without gigabytes.
    pub(crate) static MAX_LEN: std::cell::Cell<usize> =
        std::cell::Cell::new(usize::try_from(u32::MAX).unwrap_or(usize::MAX));
}

#[cfg(test)]
fn max_len() -> usize {
    MAX_LEN.with(std::cell::Cell::get)
}

/// Refuse a part `len` long, past [`max_len`].
#[inline]
pub(crate) fn check_len(len: usize) -> Result<(), RunError> {
    let max = max_len();

    if len > max {
        return Err(RunError::new(Message::TooLong {
            len: count(len),
            max: count(max),
        }));
    }

    Ok(())
}

/// What a walk does with the size of each block of parts it finds.
trait Blocks {
    fn block(&mut self, words: usize);

    /// A list's or a map's entries, `words` of them.
    fn entries(&mut self, words: usize);
}

impl Blocks for () {
    #[inline]
    fn block(&mut self, _: usize) {}

    #[inline]
    fn entries(&mut self, _: usize) {}
}

impl Blocks for Vec<usize> {
    fn block(&mut self, words: usize) {
        if words > 0 {
            self.push(words);
        }
    }

    fn entries(&mut self, words: usize) {
        if words > 0 {
            self.push(words.saturating_add(HEADER));
        }
    }
}

/// Walk `value` of type `ty`, giving the words its parts take, with
/// `trail` keeping the path to each part and `blocks` told of each block.
fn walk<T: Trail>(
    types: &Types,
    value: &Value,
    ty: TypeId,
    trail: &mut T,
    blocks: &mut impl Blocks,
) -> Result<usize, Stop> {
    let mut work = Vec::new();
    let mut next = Some((value, ty, 0usize, None));
    let mut total = 0usize;

    while let Some((value, ty, node, option)) = next.take().or_else(|| work.pop()) {
        let ty_node = types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))?;

        let found = match (ty_node, &value.0) {
            (TypeNode::Option(_), Repr::None)
            | (TypeNode::Int, Repr::Int(_))
            | (TypeNode::Bool, Repr::Bool(_)) => continue,
            (TypeNode::Option(inner), Repr::Some(value)) => {
                next = Some((value, *inner, node, Some(ty)));
                continue;
            }
            (TypeNode::String, Repr::String(s)) => {
                check_len(s.len())?;
                let words = s.len().div_ceil(8);
                blocks.block(words);
                total = total.saturating_add(words);
                continue;
            }
            (TypeNode::List(entry), Repr::List(items)) => {
                check_len(items.len())?;
                let words = items.len().saturating_mul(types.width(*entry));
                blocks.entries(words);
                total = total.saturating_add(words);
                work.reserve(items.len());
                trail.reserve(items.len());

                for (i, item) in items.iter().enumerate() {
                    let child = trail.child(node, || Seg::Index(count(i)));
                    work.push((item, *entry, child, None));
                }

                continue;
            }
            (TypeNode::Map { key, value }, Repr::Map(entries)) => {
                check_len(entries.len())?;
                let width = types.width(*value).saturating_add(1);
                let words = entries.len().saturating_mul(width);
                blocks.entries(words);
                total = total.saturating_add(words);
                let parts = entries.len().saturating_mul(2);
                work.reserve(parts);
                trail.reserve(parts);

                for (i, (k, v)) in entries.iter().enumerate() {
                    let key_node = trail.child(node, || Seg::KeyOf(count(i)));
                    let value_node = trail.child(node, || match &k.0 {
                        Repr::Int(n) => Seg::Key(Key::Int(*n)),
                        Repr::String(s) => Seg::Key(Key::string(s)),
                        _ => Seg::KeyOf(count(i)),
                    });

                    // The key is checked first.
                    work.push((v, *value, value_node, None));
                    work.push((k, *key, key_node, None));
                }

                continue;
            }
            // Both are sorted by name, so equal names pairwise is an exact
            // match, found without a lookup.
            (TypeNode::Record(fields), Repr::Record(map))
                if fields.len() == map.len()
                    && fields.iter().zip(map.keys()).all(|(f, k)| f.name == *k) =>
            {
                let words = types.record_words(fields);
                blocks.block(words);
                total = total.saturating_add(words);
                work.reserve(fields.len());
                trail.reserve(fields.len());

                for (n, (field, value)) in fields.iter().zip(map.values()).enumerate() {
                    let child = trail.child(node, || Seg::Field {
                        record: ty,
                        field: u32::try_from(n).unwrap_or(u32::MAX),
                    });
                    work.push((value, field.ty, child, None));
                }

                continue;
            }
            (TypeNode::Record(fields), Repr::Record(map)) => {
                let missing = fields.iter().position(|f| !map.contains_key(&f.name));
                let extra = map.keys().any(|k| !fields.iter().any(|f| f.name == **k));

                match (missing, extra) {
                    (Some(field), _) => Found::RecordWithout {
                        record: ty,
                        field: u32::try_from(field).unwrap_or(u32::MAX),
                    },
                    (None, true) => Found::RecordWithExtra,
                    (None, false) => {
                        let words = types.record_words(fields);
                        blocks.block(words);
                        total = total.saturating_add(words);
                        work.reserve(fields.len());
                        trail.reserve(fields.len());

                        for (n, field) in fields.iter().enumerate() {
                            if let Some(value) = map.get(&field.name) {
                                let child = trail.child(node, || Seg::Field {
                                    record: ty,
                                    field: u32::try_from(n).unwrap_or(u32::MAX),
                                });
                                work.push((value, field.ty, child, None));
                            }
                        }

                        continue;
                    }
                }
            }
            _ => value.describe(),
        };

        return Err(Stop::Mismatch(Mismatch {
            expected: option.unwrap_or(ty),
            found,
            in_some: option.is_some(),
            node,
        }));
    }

    Ok(total)
}

/// Write `value`, already measured against `ty`, at `dest`, taking memory
/// for its parts.
pub(crate) fn write(
    words: &mut (impl WordsMut + ?Sized),
    types: &Types,
    value: &Value,
    ty: TypeId,
    dest: usize,
) -> Result<(), RunError> {
    let mut work = Vec::new();
    let mut next = Some((value, ty, dest));

    while let Some((value, ty, dest)) = next.take().or_else(|| work.pop()) {
        let ty_node = types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))?;

        let word = match (ty_node, &value.0) {
            (TypeNode::Option(_), Repr::None) => match types.tagged(ty) {
                Some(width) => {
                    put(words, Some(dest), Word::present(false))?;

                    for n in 1..=width {
                        put(words, dest.checked_add(n), Word::int(0))?;
                    }

                    continue;
                }
                None => Word::NONE,
            },
            (TypeNode::Option(inner), Repr::Some(value)) => {
                let dest = if types.tagged(ty).is_some() {
                    put(words, Some(dest), Word::present(true))?;
                    dest.checked_add(1)
                        .ok_or_else(|| fault(Fault::OutsideMemory))?
                } else {
                    dest
                };

                next = Some((value, *inner, dest));
                continue;
            }
            (TypeNode::Int, Repr::Int(n)) => Word::int(*n),
            (TypeNode::Bool, Repr::Bool(b)) => Word::bool(*b),
            (TypeNode::String, Repr::String(s)) => {
                let at = words.alloc(s.len().div_ceil(8))?;
                put_bytes(words, at, s.as_bytes())?;
                Word::reference(at, s.len())?
            }
            (TypeNode::List(entry), Repr::List(items)) => {
                let width = types.width(*entry);
                let at = words.alloc_entries(items.len(), width)?;
                work.reserve(items.len());

                for (n, item) in items.iter().enumerate() {
                    let dest = n
                        .checked_mul(width)
                        .and_then(|n| n.checked_add(at))
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    work.push((item, *entry, dest));
                }

                Word::reference(at, items.len())?
            }
            (TypeNode::Map { key, value }, Repr::Map(entries)) => {
                // A map's entries are in key order, each key once, as
                // `Value::map` keeps them, for the lookup to find them.
                let sorted = entries.windows(2).all(|pair| match pair {
                    [(a, _), (b, _)] => key_order(a, b) == Ordering::Less,
                    _ => true,
                });

                if !sorted {
                    return Err(fault(Fault::MapUnsorted));
                }

                let width = types.width(*value);
                let stride = width
                    .checked_add(1)
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                let at = words.alloc_entries(entries.len(), stride)?;
                work.reserve(entries.len().saturating_mul(2));

                for (n, (k, v)) in entries.iter().enumerate() {
                    let dest = n
                        .checked_mul(stride)
                        .and_then(|n| n.checked_add(at))
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    let value_dest = dest
                        .checked_add(1)
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    work.push((k, *key, dest));
                    work.push((v, *value, value_dest));
                }

                Word::reference(at, entries.len())?
            }
            (TypeNode::Record(fields), Repr::Record(map)) => {
                let n = types.record_words(fields);
                let at = words.alloc(n)?;

                let mut entries = map.iter();
                work.reserve(fields.len());

                for (offset, field) in types.field_offsets(fields) {
                    // In step with the fields, as `measure` found them.
                    let value = match entries.next() {
                        Some((name, value)) if *name == field.name => value,
                        _ => map
                            .get(&field.name)
                            .ok_or_else(|| fault(Fault::RecordLostField))?,
                    };
                    let dest = at
                        .checked_add(offset)
                        .ok_or_else(|| fault(Fault::OutsideMemory))?;
                    work.push((value, field.ty, dest));
                }

                Word::reference(at, n)?
            }
            _ => return Err(fault(Fault::ValueChangedType)),
        };

        put(words, Some(dest), word)?;
    }

    Ok(())
}

enum Read {
    Visit(usize, TypeId),
    List(usize),
    /// A map of this many entries.
    Map(usize),
    Record(TypeId),
    Some,
}

/// The value of type `ty` held at `at`, spending from `fuel` a step per
/// word of its parts (what [`measure`] gives for it) before reading each
/// part, so the work is bounded by the steps it is allowed.
pub(crate) fn read(
    words: &(impl Words + ?Sized),
    types: &Types,
    at: usize,
    ty: TypeId,
    fuel: &mut Fuel,
) -> Result<Value, RunError> {
    let mut steps = vec![Read::Visit(at, ty)];
    let mut out = Vec::<Value>::new();

    while let Some(step) = steps.pop() {
        match step {
            Read::Visit(at, ty) => {
                let word = get(words, Some(at))?;

                match types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))? {
                    TypeNode::Option(inner) => {
                        let at = if types.tagged(ty).is_some() {
                            if !word.as_bool()? {
                                out.push(Value::none());
                                continue;
                            }

                            at.checked_add(1)
                                .ok_or_else(|| fault(Fault::OutsideMemory))?
                        } else if word.is_none() {
                            out.push(Value::none());
                            continue;
                        } else {
                            at
                        };

                        steps.push(Read::Some);
                        steps.push(Read::Visit(at, *inner));
                    }
                    TypeNode::Int => out.push(Value::int(word.as_int())),
                    TypeNode::Bool => out.push(Value::bool(word.as_bool()?)),
                    TypeNode::String => {
                        let (at, len) = word.as_reference()?;
                        fuel.spend(len.div_ceil(8))?;
                        out.push(Value::string(string(words, at, len)?));
                    }
                    TypeNode::List(entry) => {
                        let (start, len) = word.as_reference()?;
                        let width = types.width(*entry);
                        fuel.spend(len.saturating_mul(width))?;
                        inside(words, start, len.checked_mul(width))?;
                        steps.push(Read::List(len));

                        for n in (0..len).rev() {
                            let at = n.saturating_mul(width).saturating_add(start);
                            steps.push(Read::Visit(at, *entry));
                        }
                    }
                    TypeNode::Map { key, value } => {
                        let (start, len) = word.as_reference()?;
                        let stride = types.width(*value).saturating_add(1);
                        fuel.spend(len.saturating_mul(stride))?;
                        inside(words, start, len.checked_mul(stride))?;
                        steps.push(Read::Map(len));

                        for n in (0..len).rev() {
                            let at = n.saturating_mul(stride).saturating_add(start);
                            steps.push(Read::Visit(at.saturating_add(1), *value));
                            steps.push(Read::Visit(at, *key));
                        }
                    }
                    TypeNode::Record(fields) => {
                        let (start, len) = word.as_reference()?;

                        if len != types.record_words(fields) {
                            return Err(fault(Fault::RecordWrongSize));
                        }

                        fuel.spend(len)?;
                        inside(words, start, Some(len))?;
                        steps.push(Read::Record(ty));

                        for (offset, field) in types
                            .field_offsets(fields)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                        {
                            steps.push(Read::Visit(start.saturating_add(offset), field.ty));
                        }
                    }
                }
            }
            Read::List(n) => {
                let at = out
                    .len()
                    .checked_sub(n)
                    .ok_or_else(|| fault(Fault::ListLostEntries))?;
                let items = out.split_off(at);
                out.push(Value(Repr::List(items)));
            }
            Read::Map(n) => {
                let at = n
                    .checked_mul(2)
                    .and_then(|parts| out.len().checked_sub(parts))
                    .ok_or_else(|| fault(Fault::MapLostEntries))?;
                let mut parts = out.split_off(at).into_iter();
                let mut entries = Vec::with_capacity(n);

                while let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                    entries.push((key, value));
                }

                // Written in key order, so read back in it.
                out.push(Value(Repr::Map(entries)));
            }
            Read::Record(ty) => {
                let Some(TypeNode::Record(fields)) = types.get(ty) else {
                    return Err(fault(Fault::RecordNotRecordType));
                };

                let at = out
                    .len()
                    .checked_sub(fields.len())
                    .ok_or_else(|| fault(Fault::RecordLostFields))?;
                let values = out.split_off(at);

                out.push(Value(Repr::Record(
                    fields.iter().map(|f| f.name.clone()).zip(values).collect(),
                )));
            }
            Read::Some => {
                let value = out.pop().ok_or_else(|| fault(Fault::OptionLostValue))?;
                out.push(Value::some(value));
            }
        }
    }

    match (out.pop(), out.is_empty()) {
        (Some(value), true) => Ok(value),
        _ => Err(fault(Fault::ReadAsSeveral)),
    }
}

/// Each block of at least a word that the parts of the value of type `ty`
/// held at `at` in a store take, as (start, words), in no particular
/// order: what [`write`] took for them, a list's and a map's from its
/// [`HEADER`] to the end of its capacity.
pub(crate) fn blocks(
    words: &(impl Words + ?Sized),
    types: &Types,
    at: usize,
    ty: TypeId,
) -> Result<Vec<(usize, usize)>, RunError> {
    let mut work = vec![(at, ty)];
    let mut out = Vec::new();

    let mut block = |start: Option<usize>, n: Option<usize>| -> Result<(), RunError> {
        let (Some(start), Some(n)) = (start, n) else {
            return Err(fault(Fault::OutsideMemory));
        };

        inside(words, start, Some(n))?;

        if n > 0 {
            out.push((start, n));
        }

        Ok(())
    };

    // The block of a list's or a map's entries: from its header to the end
    // of its capacity, none when it is empty.
    let entries = |start: usize, len: usize, stride: usize| {
        inside(words, start, len.checked_mul(stride))?;

        if len == 0 {
            return Ok((Some(start), Some(0)));
        }

        let cap = capacity(words, start, len)?;
        Ok::<_, RunError>((start.checked_sub(HEADER), entries_block(cap, stride)))
    };

    while let Some((at, ty)) = work.pop() {
        let word = get(words, Some(at))?;

        match types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))? {
            TypeNode::Int | TypeNode::Bool => {}
            TypeNode::Option(inner) => {
                if types.tagged(ty).is_some() {
                    if word.as_bool()? {
                        let at = at
                            .checked_add(1)
                            .ok_or_else(|| fault(Fault::OutsideMemory))?;
                        work.push((at, *inner));
                    }
                } else if !word.is_none() {
                    work.push((at, *inner));
                }
            }
            TypeNode::String => {
                let (start, len) = word.as_reference()?;
                block(Some(start), Some(len.div_ceil(8)))?;
            }
            TypeNode::List(entry) => {
                let (start, len) = word.as_reference()?;
                let width = types.width(*entry);
                let (from, n) = entries(start, len, width)?;
                block(from, n)?;

                for n in 0..len {
                    work.push((n.saturating_mul(width).saturating_add(start), *entry));
                }
            }
            TypeNode::Map { key, value } => {
                let (start, len) = word.as_reference()?;
                let stride = types.width(*value).saturating_add(1);
                let (from, n) = entries(start, len, stride)?;
                block(from, n)?;

                for n in 0..len {
                    let at = n.saturating_mul(stride).saturating_add(start);
                    work.push((at, *key));
                    work.push((at.saturating_add(1), *value));
                }
            }
            TypeNode::Record(fields) => {
                let (start, len) = word.as_reference()?;

                if len != types.record_words(fields) {
                    return Err(fault(Fault::RecordWrongSize));
                }

                block(Some(start), Some(len))?;

                for (offset, field) in types.field_offsets(fields) {
                    work.push((start.saturating_add(offset), field.ty));
                }
            }
        }
    }

    Ok(out)
}

/// Where the entry of `key`, an `Int` or a `String`, starts in the map of
/// `len` entries of `stride` words from `start`, found by binary search
/// over its sorted keys: a probe per halving, and a word per 8 bytes of a
/// `String` key compared. `None` when the map has no such key, or `key` is
/// of neither type.
pub(crate) fn find_key(
    words: &(impl Words + ?Sized),
    start: usize,
    len: usize,
    stride: usize,
    key: &Value,
) -> Result<Option<usize>, RunError> {
    let Some(Ok(n)) = search_key(words, start, len, stride, key)? else {
        return Ok(None);
    };

    n.checked_mul(stride)
        .and_then(|at| at.checked_add(start))
        .map(Some)
        .ok_or_else(|| fault(Fault::OutsideMemory))
}

/// As [`find_key`], giving the index of the entry of `key` as `Ok`, or as
/// `Err` the index its entry would take to keep the keys in order. `None`
/// when `key` is neither an `Int` nor a `String`.
pub(crate) fn search_key(
    words: &(impl Words + ?Sized),
    start: usize,
    len: usize,
    stride: usize,
    key: &Value,
) -> Result<Option<Result<usize, usize>>, RunError> {
    inside(words, start, len.checked_mul(stride))?;

    let mut low = 0usize;
    let mut high = len;

    while low < high {
        let mid = low.saturating_add(high.saturating_sub(low) / 2);
        let at = mid
            .checked_mul(stride)
            .and_then(|at| at.checked_add(start))
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        let probe = get(words, Some(at))?;

        let order = match &key.0 {
            Repr::Int(n) => probe.as_int().cmp(n),
            Repr::String(s) => compare_string(words, probe, s.as_bytes())?,
            _ => return Ok(None),
        };

        match order {
            Ordering::Less => low = mid.saturating_add(1),
            Ordering::Greater => high = mid,
            Ordering::Equal => return Ok(Some(Ok(mid))),
        }
    }

    match &key.0 {
        Repr::Int(_) | Repr::String(_) => Ok(Some(Err(low))),
        _ => Ok(None),
    }
}

/// The order of the string `word` refers to and `bytes`, by their bytes.
fn compare_string(
    words: &(impl Words + ?Sized),
    word: Word,
    bytes: &[u8],
) -> Result<Ordering, RunError> {
    let (at, len) = word.as_reference()?;
    let common = len.min(bytes.len());

    for (n, theirs) in bytes
        .get(..common)
        .unwrap_or_default()
        .chunks(8)
        .enumerate()
    {
        let ours = get(words, at.checked_add(n))?.as_bytes();
        let order = ours.get(..theirs.len()).cmp(&Some(theirs));

        if order != Ordering::Equal {
            return Ok(order);
        }
    }

    Ok(len.cmp(&bytes.len()))
}

/// Where a walk found a value not of its type: the type `expected`, what
/// was `found` instead, and the `node` of its path.
struct Mismatch {
    expected: TypeId,
    found: Found,
    in_some: bool,
    node: usize,
}

/// Why a walk stopped.
enum Stop {
    Mismatch(Mismatch),
    Fault(RunError),
}

impl From<RunError> for Stop {
    fn from(error: RunError) -> Self {
        Self::Fault(error)
    }
}

impl From<Fault> for Stop {
    fn from(error: Fault) -> Self {
        Self::Fault(error.into())
    }
}

/// The paths into a value a walk keeps, one node to each part it queues.
trait Trail {
    /// Make room for `n` more nodes.
    fn reserve(&mut self, n: usize);

    /// The node of the part reached from `parent` by the step `seg` gives.
    fn child(&mut self, parent: usize, seg: impl FnOnce() -> Seg) -> usize;
}

/// Keeps none, as a walk that finds no mismatch needs none.
struct NoPaths;

impl Trail for NoPaths {
    fn reserve(&mut self, _: usize) {}

    fn child(&mut self, _: usize, _: impl FnOnce() -> Seg) -> usize {
        0
    }
}

/// Each node's parent and the step to it; the root, which has none, is
/// added with the first part.
#[derive(Default)]
struct Nodes(Vec<(usize, Option<Seg>)>);

impl Trail for Nodes {
    fn reserve(&mut self, n: usize) {
        self.0.reserve(n.saturating_add(1));
    }

    fn child(&mut self, parent: usize, seg: impl FnOnce() -> Seg) -> usize {
        if self.0.is_empty() {
            self.0.push((0, None));
        }

        let node = self.0.len();
        self.0.push((parent, Some(seg())));
        node
    }
}

/// The error of a value found not to be of its type, at its node of the
/// path from `root`.
#[cold]
#[inline(never)]
fn mistyped(
    mismatch: Mismatch,
    root: Root,
    prefix: &[Seg],
    nodes: &[(usize, Option<Seg>)],
) -> RunError {
    let Mismatch {
        expected,
        found,
        in_some,
        node,
    } = mismatch;
    let mut node = node;
    let mut path = Vec::new();

    while let Some((parent, Some(seg))) = nodes.get(node) {
        path.push(seg.clone());
        node = *parent;
    }

    // A whole value is named by the setting's or the function's context.
    if path.is_empty() {
        return RunError::new(Message::Mistyped {
            expected,
            found,
            in_some,
        });
    }

    path.extend(prefix.iter().rev().cloned());
    path.reverse();

    let error = RunError::new(Message::Mistyped {
        expected,
        found,
        in_some,
    });

    error.context(Message::AtPath {
        root,
        path: path.into_boxed_slice(),
    })
}

pub(crate) fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
