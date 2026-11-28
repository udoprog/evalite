use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use musli::{Decode, Encode};

use crate::Type;
use crate::message::{Invalid, Message, StoreError};
use crate::types::{FxBuildHasher, TypeId, TypeNode, Types, is_value_name};

/// The version of the stored form [`Program::to_bytes`] writes.
pub(crate) const FORMAT: u32 = 13;

/// How many nodes the globals' and host functions' types may have
/// together, each written out in full, as [`Program::globals`] and
/// [`Program::functions`] hand them out.
pub(crate) const MAX_GLOBAL_TYPE_NODES: usize = 1 << 21;

/// One instruction of a stack machine over words of memory. A value takes
/// the words its type's layout gives it: one for an `Int`, a `Bool` (0 or
/// 1), and a string, list, record or map (a reference packing where its
/// parts are and how many there are). A map's parts are its entries in key
/// order, each its key's word then its value's words, so a key is found by
/// binary search; `Int` keys are ordered by value and `String` keys by
/// their bytes. An option of a `Bool`, string, list, record or map is one
/// word with [`Word::NONE`](crate::word::Word::NONE) for
/// absent; an option of an `Int` or of an option is tagged, a word for
/// present or not, then its value's words, so `Option<Int>` is two words
/// and `Option<Option<Int>>` three. Operands are pushed in
/// source order, so a binary operator finds its right operand on top.
///
/// Instructions whose work depends on a type carry it, and the verifier
/// checks it against its model of the stack, so the machine never looks
/// at what a word is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub(crate) enum Instr {
    /// Push an integer.
    Int(i64),
    /// Push a boolean.
    Bool(bool),
    /// Push a reference to a constant.
    Const(u32),
    /// Push the value of type `ty` held in the slot at word `at` of the
    /// globals, where the [`Environment`](crate::Environment) holds it.
    Global {
        at: u32,
        ty: TypeId,
    },
    /// `-a`, `!a`.
    Neg,
    Not,
    /// `a + b` and the rest, on `Int`s.
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// `a == b`, `a != b` on two `Int`s or two `Bool`s, one word each.
    Eq,
    Ne,
    /// Orderings on `Int`s.
    Lt,
    Le,
    Gt,
    Ge,
    /// `a && b`: with `false` on top, skip this many instructions, keeping
    /// it; otherwise pop it and go on to `b`.
    And(u32),
    /// `a || b`: the same, skipping on `true`.
    Or(u32),
    /// A record's field of type `ty`, `at` words from the record's start.
    Field {
        at: u32,
        ty: TypeId,
    },
    /// `list[index]` of a list of `T`s, an error out of range.
    Index(TypeId),
    /// Make a list of `len` entries of type `entry`, at least one, on the
    /// heap.
    List {
        len: u32,
        entry: TypeId,
    },
    /// The least or greatest of this many `Int`s, at least one.
    Min(u32),
    Max(u32),
    Abs,
    /// `value.clamp(low, high)`.
    Clamp,
    /// `list.get(index)` of a list of `T`s: an `Option<T>`, absent when
    /// the index is out of range.
    Get(TypeId),
    /// `option.unwrap_or(default)` of this option type.
    UnwrapOr(TypeId),
    /// `option!` of this option type, failing the run when it is absent.
    Unwrap(TypeId),
    IsSome(TypeId),
    IsNone(TypeId),
    /// A list's length, or a map's entries.
    Len,
    /// Call one of the program's host functions.
    Call(u32),
    /// `map[key]` of this map type, an error when the key is missing.
    MapIndex(TypeId),
    /// `map.get(key)` of this map type: an `Option` of its value type,
    /// absent when the key is missing.
    MapGet(TypeId),
    /// Make a map of this map type on the heap, of the keys of the constant
    /// list `keys`, which the verifier checks are in key order and each
    /// given once, and a value of each, popped in that order.
    Map {
        keys: u32,
        map: TypeId,
    },
    /// `Some(value)` of this option type: its value on top of the stack
    /// made present.
    Some(TypeId),
    /// `None` of this option type.
    None(TypeId),
    /// An empty list or map of this type, `[]` or `{}`.
    Empty(TypeId),
    /// Make a record of this record type, of at least one field, on the
    /// heap: its fields' values, popped in name order, the first deepest.
    Record(TypeId),
    /// `a == b`, `a != b` on two options of this type, which hold an
    /// `Int`, a `Bool` or another such option: equal when both are absent
    /// at the same depth, or present with equal values.
    OptionEq(TypeId),
    OptionNe(TypeId),
    /// `MapIndex` of a key written as a hexadecimal literal, which an
    /// error shows as written.
    MapIndexHex(TypeId),
}

/// A value in a constant list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub(crate) enum Scalar {
    Int(i64),
    Bool(bool),
    /// An earlier constant.
    Const(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) enum Constant {
    String(String),
    /// A non-empty list whose entries are of type `entry`.
    List {
        entry: TypeId,
        items: Vec<Scalar>,
    },
}

impl Constant {
    /// The words it takes in memory: one per entry (constant entries are
    /// `Int`s, `Bool`s, strings and lists, a word each), or one per 8
    /// bytes of a string.
    fn words(&self) -> usize {
        match self {
            Self::String(s) => s.len().div_ceil(8),
            Self::List { items, .. } => items.len(),
        }
    }

    /// The length its reference holds: a string's bytes or a list's
    /// entries.
    fn len(&self) -> usize {
        match self {
            Self::String(s) => s.len(),
            Self::List { items, .. } => items.len(),
        }
    }
}

/// A value the program reads, which the caller binds before it runs,
/// held in the slot at word `slot` of the globals.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct Global {
    pub(crate) name: String,
    pub(crate) ty: TypeId,
    pub(crate) slot: u32,
}

/// A function of the caller's that a program calls.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct HostFunction {
    pub(crate) name: String,
    pub(crate) params: Vec<TypeId>,
    pub(crate) ret: TypeId,
}

/// The most memory a program uses as it runs, in 64-bit words, from
/// [`Program::memory`].
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let program = evalite::compile("[1, 2, 3][bonus.unwrap_or(0)]", &env)?;
/// let memory = program.memory();
/// assert_eq!(memory.constants(), 3); // the constant list's entries
/// assert!(memory.stack() > 0);
/// assert_eq!(memory.heap(), 0);
/// assert_eq!(
///     memory.total(),
///     u64::from(memory.stack() + memory.heap() + memory.constants()),
/// );
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub struct Memory {
    pub(crate) stack: u32,
    pub(crate) heap: u32,
    pub(crate) constants: u32,
}

impl Memory {
    /// The deepest the stack gets.
    pub fn stack(&self) -> u32 {
        self.stack
    }

    /// The entries of lists built as it runs.
    pub fn heap(&self) -> u32 {
        self.heap
    }

    /// The program's constant lists and strings, copied into the machine
    /// when the program is loaded.
    pub fn constants(&self) -> u32 {
        self.constants
    }

    /// Every word of the machine's it takes, the parts of returned values
    /// aside. The values it reads are not among them: they are in the
    /// [`Globals`](crate::Globals) store it runs with.
    pub fn total(&self) -> u64 {
        u64::from(self.stack)
            .saturating_add(u64::from(self.heap))
            .saturating_add(u64::from(self.constants))
    }
}

#[derive(Encode, Decode)]
pub(crate) struct Stored {
    pub(crate) types: Vec<TypeNode>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) globals: Vec<Global>,
    pub(crate) functions: Vec<HostFunction>,
    pub(crate) code: Vec<Instr>,
    pub(crate) result: TypeId,
    pub(crate) memory: Memory,
    /// The fingerprint of the layout it was compiled against.
    pub(crate) layout: u64,
}

/// A compiled expression: checked, typed, and ready to store or run.
///
/// It names the values it reads and the caller's functions it calls, with
/// their types, so it runs without the [`Environment`](crate::Environment)
/// it was compiled with. [`Program::from_bytes`] checks stored bytes
/// before giving one back.
#[derive(Debug, Clone)]
pub struct Program {
    /// The fingerprint of the layout it was compiled against.
    pub(crate) layout: u64,
    pub(crate) types: Types,
    pub(crate) constants: Vec<Constant>,
    pub(crate) globals: Vec<Global>,
    pub(crate) functions: Vec<HostFunction>,
    pub(crate) code: Vec<Instr>,
    pub(crate) result: TypeId,
    pub(crate) memory: Memory,
    /// The type of each global, built the first time they are listed, not
    /// when the program is made: loading compares the store's types with
    /// the table without building them.
    pub(crate) global_types: OnceLock<Vec<Type>>,
    /// Where each constant is, in words from the first, and the length
    /// its reference holds.
    pub(crate) constant_at: Vec<(u32, u32)>,
}

impl Program {
    /// The type of the value it gives.
    pub fn result_type(&self) -> Type {
        self.type_of(self.result)
    }

    /// The most memory it uses as it runs.
    pub fn memory(&self) -> Memory {
        self.memory
    }

    /// The values it reads, which a [`Globals`](crate::Globals) store
    /// holds for it, with their types: each declared name the expression
    /// uses, once, and none it does not.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("wits", Type::int())?;
    /// env.value("bonus", Type::option(Type::int()))?;
    /// env.value("unused", Type::bool())?;
    ///
    /// let program = evalite::compile("wits + bonus.unwrap_or(wits)", &env)?;
    /// let mut names: Vec<_> = program.globals().map(|(name, _)| name).collect();
    /// names.sort();
    /// assert_eq!(names, ["bonus", "wits"]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn globals(&self) -> impl ExactSizeIterator<Item = (&str, &Type)> + Clone {
        let types = self.global_types.get_or_init(|| {
            let ids: Vec<TypeId> = self.globals.iter().map(|g| g.ty).collect();
            // A program's global types are checked, and bounded, when it is
            // made, so they build.
            self.types
                .to_types(&ids)
                .unwrap_or_else(|| ids.iter().map(|_| Type::int()).collect())
        });

        self.globals
            .iter()
            .zip(types)
            .map(|(g, ty)| (g.name.as_str(), ty))
    }

    /// The [`fingerprint`](crate::Environment::fingerprint) of the
    /// environment it was compiled against. It is a hint, not a proof:
    /// [`Environment::accepts`](crate::Environment::accepts) checks each
    /// value the program reads however the fingerprints compare.
    pub fn fingerprint(&self) -> u64 {
        self.layout
    }

    /// The caller's functions it calls: each name, parameter types and
    /// return type.
    pub fn functions(&self) -> Vec<(&str, Vec<Type>, Type)> {
        // Built together, so the table is walked once, not once a type.
        let ids: Vec<TypeId> = function_types(&self.functions).collect();
        let mut built = self.types.to_types(&ids).unwrap_or_default().into_iter();
        let mut next = || built.next().unwrap_or(Type::int());

        self.functions
            .iter()
            .map(|f| {
                let params = f.params.iter().map(|_| next()).collect();
                (f.name.as_str(), params, next())
            })
            .collect()
    }

    pub(crate) fn type_of(&self, id: TypeId) -> Type {
        // A program's ids are checked when it is made.
        self.types.to_type(id).unwrap_or(Type::int())
    }

    /// The program's stored form, which [`Program::from_bytes`] reads.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the program cannot be encoded, which a
    /// program from [`compile`](crate::compile) or [`Program::from_bytes`] does
    /// not do in practice.
    pub fn to_bytes(&self) -> Result<Vec<u8>, StoreError> {
        let stored = Stored {
            types: self.types.nodes().to_vec(),
            constants: self.constants.clone(),
            globals: self.globals.clone(),
            functions: self.functions.clone(),
            code: self.code.clone(),
            result: self.result,
            memory: self.memory,
            layout: self.layout,
        };

        crate::framed::to_bytes(FORMAT, &stored).map_err(|e| {
            StoreError::new(Message::CannotEncode)
                .with_source(e.to_string())
                .context(Message::StoringProgram)
        })
    }

    /// Read a stored program, checking that it is one evalite would compile:
    /// well formed, well typed, and using exactly the memory it declares.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when `bytes` do not decode, are of another
    /// format version, or are a program that fails verification: ill typed, out
    /// of bounds, or using other memory than it declares. Its `{:#}` chain says
    /// where, such as "at instruction 0: while verifying the program".
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let not_decoded = |e: musli::storage::Error| {
            StoreError::new(Message::NotDecoded)
                .with_source(e.to_string())
                .context(Message::DecodingProgram)
        };
        let (format, body) = crate::framed::split(bytes).map_err(not_decoded)?;

        if format != FORMAT {
            return Err(StoreError::new(Message::WrongFormat {
                found: format,
                reads: FORMAT,
            })
            .context(Message::DecodingProgram));
        }

        let stored: Stored = musli::storage::from_slice(body).map_err(not_decoded)?;

        let verifying = |e: StoreError| e.context(Message::VerifyingProgram);
        let types = Types::from_stored(stored.types).map_err(verifying)?;

        let program = Self::verify(
            stored.layout,
            types,
            stored.constants,
            stored.globals,
            stored.functions,
            stored.code,
            stored.result,
        )
        .map_err(verifying)?;

        if program.memory != stored.memory {
            return Err(verifying(
                Invalid::MemoryMismatch {
                    declared: stored.memory.total(),
                    used: program.memory.total(),
                }
                .into(),
            ));
        }

        Ok(program)
    }

    /// Check a program's parts and work out the memory it uses, by running
    /// its code over the types of the values instead of values.
    pub(crate) fn verify(
        layout: u64,
        types: Types,
        constants: Vec<Constant>,
        globals: Vec<Global>,
        functions: Vec<HostFunction>,
        code: Vec<Instr>,
        result: TypeId,
    ) -> Result<Self, StoreError> {
        let mut verifier = Verifier {
            types: &types,
            constants: &constants,
            constant_types: Vec::new(),
        };

        let mut constant_at = Vec::with_capacity(constants.len());
        let mut constant_words = 0usize;

        for (at, constant) in constants.iter().enumerate() {
            let ty = verifier
                .constant(constant)
                .map_err(|e| e.context(Message::AtConstant { at: count(at) }))?;
            verifier.constant_types.push(ty);
            constant_at.push((words(constant_words)?, words(constant.len())?));
            constant_words = constant_words.saturating_add(constant.words());
        }

        let hasher = types.hasher();
        let mut names = HashSet::with_capacity_and_hasher(globals.len(), hasher);
        // Each global's slot, where it ends and which it is.
        let mut slots = Vec::with_capacity(globals.len());

        for (at, global) in globals.iter().enumerate() {
            let at_global = |e: StoreError| e.context(Message::AtGlobal { at: count(at) });

            if !is_value_name(&global.name) || !names.insert(global.name.as_str()) {
                return Err(at_global(Invalid::BadGlobal.into()));
            }

            verifier.known(global.ty).map_err(at_global)?;
            let end = usize::try_from(global.slot)
                .ok()
                .and_then(|slot| slot.checked_add(types.width(global.ty)));
            let end = end.ok_or_else(|| at_global(Invalid::TooMuchToCount.into()))?;
            slots.push((global.slot, words(end).map_err(at_global)?, at));
        }

        // No two globals may share a word, or writing one would change
        // what another reads.
        slots.sort_unstable();

        for pair in slots.windows(2) {
            if let [(_, end, _), (slot, _, at)] = *pair
                && slot < end
            {
                return Err(StoreError::from(Invalid::GlobalsOverlap)
                    .context(Message::AtGlobal { at: count(at) }));
            }
        }

        let mut signatures = HashSet::with_hasher(hasher);

        for (at, function) in functions.iter().enumerate() {
            let at_function = |e: StoreError| e.context(Message::AtFunction { at: count(at) });

            if !is_value_name(&function.name)
                || !signatures.insert((function.name.as_str(), function.params.as_slice()))
            {
                return Err(at_function(Invalid::BadFunction.into()));
            }

            for ty in function.params.iter().chain([&function.ret]) {
                verifier.known(*ty).map_err(at_function)?;
            }
        }

        verifier.known(result)?;

        let mut global_ids = HashMap::with_capacity_and_hasher(globals.len(), hasher);

        for global in &globals {
            global_ids.insert(global.slot, global.ty);
        }

        let memory = verifier.code(&code, &global_ids, &functions, result)?;

        let held: Vec<TypeId> = globals
            .iter()
            .map(|g| g.ty)
            .chain(function_types(&functions))
            .collect();

        // The globals' and functions' types are handed out written out, and
        // building them holds every type they reach too, so both are
        // bounded together, not only one by one.
        let nodes = held
            .iter()
            .fold(0usize, |n, id| n.saturating_add(types.size(*id)));
        let reached = types
            .reached_size(&held)
            .ok_or_else(|| StoreError::from(Invalid::BadGlobal))?;

        if nodes > MAX_GLOBAL_TYPE_NODES || reached > MAX_GLOBAL_TYPE_NODES {
            return Err(Invalid::TypeTooLarge.into());
        }

        Ok(Self {
            memory: Memory {
                constants: words(constant_words)?,
                ..memory
            },
            layout,
            types,
            constants,
            globals,
            functions,
            code,
            result,
            global_types: OnceLock::new(),
            constant_at,
        })
    }
}

/// Each function's parameter types, then its return type, in order.
fn function_types(functions: &[HostFunction]) -> impl Iterator<Item = TypeId> + '_ {
    functions
        .iter()
        .flat_map(|f| f.params.iter().copied().chain([f.ret]))
}

fn words(n: usize) -> Result<u32, StoreError> {
    u32::try_from(n).map_err(|_| Invalid::TooMuchToCount.into())
}

/// An index into stored parts, which have fewer than `u32::MAX` once they
/// decode, as a message holds it.
fn count(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

struct Verifier<'a> {
    types: &'a Types,
    constants: &'a [Constant],
    constant_types: Vec<TypeId>,
}

impl Verifier<'_> {
    fn known(&self, ty: TypeId) -> Result<&TypeNode, StoreError> {
        self.types
            .get(ty)
            .ok_or_else(|| Invalid::UnknownType.into())
    }

    fn find(&self, node: TypeNode) -> Result<TypeId, StoreError> {
        self.types
            .find(&node)
            .ok_or_else(|| Invalid::MissingType.into())
    }

    fn constant(&self, constant: &Constant) -> Result<TypeId, StoreError> {
        match constant {
            Constant::String(_) => self.find(TypeNode::String),
            Constant::List { entry, items } => {
                if items.is_empty() {
                    return Err(Invalid::EmptyConstantList.into());
                }

                self.known(*entry)?;

                for item in items {
                    let ty = match item {
                        Scalar::Int(_) => self.find(TypeNode::Int)?,
                        Scalar::Bool(_) => self.find(TypeNode::Bool)?,
                        Scalar::Const(c) => *usize::try_from(*c)
                            .ok()
                            .and_then(|c| self.constant_types.get(c))
                            .ok_or(Invalid::ConstantRefersLater)?,
                    };

                    if ty != *entry {
                        return Err(Invalid::ConstantEntryMistyped.into());
                    }
                }

                self.find(TypeNode::List(*entry))
            }
        }
    }

    fn code(
        &self,
        code: &[Instr],
        globals: &HashMap<u32, TypeId, FxBuildHasher>,
        functions: &[HostFunction],
        result: TypeId,
    ) -> Result<Memory, StoreError> {
        let int = self.find(TypeNode::Int).ok();
        let bool = self.find(TypeNode::Bool).ok();
        let int_ty = || int.ok_or_else(|| StoreError::from(Invalid::MissingType));
        let bool_ty = || bool.ok_or_else(|| StoreError::from(Invalid::MissingType));

        let mut stack = Model::new(self.types, code.len());
        let mut heap = 0usize;
        // The stack each pending jump expects where it lands.
        let mut targets = BTreeMap::<usize, usize>::new();
        // The key constants found sorted, which maps share.
        let mut sorted = HashSet::with_hasher(self.types.hasher());

        for (at, instr) in code.iter().enumerate() {
            let fail = |invalid: Invalid| {
                StoreError::from(invalid).context(Message::AtInstruction { pc: count(at) })
            };

            if let Some(expected) = targets.remove(&at)
                && expected != stack.top
            {
                return Err(fail(Invalid::TwoStacks));
            }

            let need = |ty: Option<TypeId>, want: Option<TypeId>| match ty {
                None => Err(fail(Invalid::PopsEmpty)),
                Some(ty) if Some(ty) == want => Ok(()),
                Some(_) => Err(fail(Invalid::WrongOperand)),
            };

            let push = match *instr {
                Instr::Int(_) => int_ty()?,
                Instr::Bool(_) => bool_ty()?,
                Instr::Const(c) => {
                    *index(&self.constant_types, c).ok_or_else(|| fail(Invalid::NoSuchConstant))?
                }
                Instr::Global { at, ty } => {
                    if globals.get(&at) != Some(&ty) {
                        return Err(fail(Invalid::NoSuchGlobal));
                    }

                    ty
                }
                Instr::Neg | Instr::Abs => {
                    need(stack.pop(), int)?;
                    int_ty()?
                }
                Instr::Not => {
                    need(stack.pop(), bool)?;
                    bool_ty()?
                }
                Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Rem => {
                    need(stack.pop(), int)?;
                    need(stack.pop(), int)?;
                    int_ty()?
                }
                Instr::Lt | Instr::Le | Instr::Gt | Instr::Ge => {
                    need(stack.pop(), int)?;
                    need(stack.pop(), int)?;
                    bool_ty()?
                }
                Instr::Eq | Instr::Ne => {
                    let right = stack.pop();
                    let left = stack.pop();

                    if left.is_none() || left != right || (left != int && left != bool) {
                        return Err(fail(Invalid::MixedCompare));
                    }

                    bool_ty()?
                }
                Instr::And(skip) | Instr::Or(skip) => {
                    // It lands with the operand it keeps.
                    let landing = stack.top;
                    need(stack.pop(), bool)?;

                    let target = usize::try_from(skip)
                        .ok()
                        .and_then(|skip| at.checked_add(skip)?.checked_add(1))
                        .filter(|target| *target <= code.len())
                        .ok_or_else(|| fail(Invalid::JumpPastEnd))?;

                    if targets.get(&target).is_some_and(|other| *other != landing) {
                        return Err(fail(Invalid::JumpsDisagree));
                    }

                    targets.insert(target, landing);
                    continue;
                }
                Instr::Field { at, ty } => {
                    let record = stack.pop().ok_or_else(|| fail(Invalid::PopsEmpty))?;

                    if !matches!(self.types.get(record), Some(TypeNode::Record(_))) {
                        return Err(fail(Invalid::FieldOfNonRecord));
                    }

                    let at = usize::try_from(at).unwrap_or(usize::MAX);

                    if self.types.field_at(record, at).is_none_or(|f| f.ty != ty) {
                        return Err(fail(Invalid::NoSuchField));
                    }

                    ty
                }
                Instr::Index(entry) => {
                    need(stack.pop(), int)?;
                    need(stack.pop().and_then(|l| self.entry(l)), Some(entry))
                        .map_err(|_| fail(Invalid::IndexNonList))?;
                    entry
                }
                Instr::Get(entry) => {
                    need(stack.pop(), int)?;
                    need(stack.pop().and_then(|l| self.entry(l)), Some(entry))
                        .map_err(|_| fail(Invalid::GetNonList))?;

                    self.find(TypeNode::Option(entry))?
                }
                Instr::List { len, entry } => {
                    self.known(entry)?;

                    if len == 0 {
                        return Err(fail(Invalid::EmptyList));
                    }

                    for _ in 0..len {
                        need(stack.pop(), Some(entry))
                            .map_err(|_| fail(Invalid::ListEntriesMissing))?;
                    }

                    let n = usize::try_from(len).unwrap_or(usize::MAX);
                    heap = heap.saturating_add(n.saturating_mul(self.types.width(entry)));
                    self.find(TypeNode::List(entry))?
                }
                Instr::Map { keys, map } => {
                    let Some(TypeNode::Map { key, value }) = self.types.get(map) else {
                        return Err(fail(Invalid::MakeNonMap));
                    };

                    let n = self.map_keys(keys, *key, &mut sorted).map_err(fail)?;

                    for _ in 0..n {
                        need(stack.pop(), Some(*value))
                            .map_err(|_| fail(Invalid::MapEntriesMissing))?;
                    }

                    let stride = self.types.width(*value).saturating_add(1);
                    heap = heap.saturating_add(n.saturating_mul(stride));
                    map
                }
                Instr::Record(record) => {
                    let Some(TypeNode::Record(fields)) = self.types.get(record) else {
                        return Err(fail(Invalid::MakeNonRecord));
                    };

                    if fields.is_empty() {
                        return Err(fail(Invalid::EmptyRecord));
                    }

                    for field in fields.iter().rev() {
                        need(stack.pop(), Some(field.ty))
                            .map_err(|_| fail(Invalid::RecordFieldsMissing))?;
                    }

                    heap = heap.saturating_add(self.types.record_words(fields));
                    record
                }
                Instr::Min(n) | Instr::Max(n) => {
                    if n == 0 {
                        return Err(fail(Invalid::NoOperands));
                    }

                    for _ in 0..n {
                        need(stack.pop(), int)?;
                    }

                    self.find(TypeNode::Int)?
                }
                Instr::Clamp => {
                    for _ in 0..3 {
                        need(stack.pop(), int)?;
                    }

                    self.find(TypeNode::Int)?
                }
                Instr::UnwrapOr(option) => {
                    let Some(TypeNode::Option(value)) = self.types.get(option) else {
                        return Err(fail(Invalid::UnwrapNonOption));
                    };

                    need(stack.pop(), Some(*value))?;
                    need(stack.pop(), Some(option))?;
                    *value
                }
                Instr::Unwrap(option) => {
                    let Some(TypeNode::Option(value)) = self.types.get(option) else {
                        return Err(fail(Invalid::UnwrapNonOption));
                    };

                    need(stack.pop(), Some(option))?;
                    *value
                }
                Instr::IsSome(option) | Instr::IsNone(option) => {
                    if !matches!(self.types.get(option), Some(TypeNode::Option(..))) {
                        return Err(fail(Invalid::TestNonOption));
                    }

                    need(stack.pop(), Some(option))?;
                    self.find(TypeNode::Bool)?
                }
                Instr::Some(option) => {
                    let Some(TypeNode::Option(value)) = self.types.get(option) else {
                        return Err(fail(Invalid::WrapNonOption));
                    };

                    need(stack.pop(), Some(*value))?;
                    option
                }
                Instr::None(option) => {
                    if !matches!(self.types.get(option), Some(TypeNode::Option(..))) {
                        return Err(fail(Invalid::WrapNonOption));
                    }

                    option
                }
                Instr::Empty(ty) => {
                    if !matches!(
                        self.types.get(ty),
                        Some(TypeNode::List(..) | TypeNode::Map { .. })
                    ) {
                        return Err(fail(Invalid::EmptyNonList));
                    }

                    ty
                }
                Instr::OptionEq(option) | Instr::OptionNe(option) => {
                    let fits = matches!(self.types.get(option), Some(TypeNode::Option(..)))
                        && self.types.comparable(option);

                    if !fits {
                        return Err(fail(Invalid::MixedCompare));
                    }

                    need(stack.pop(), Some(option))?;
                    need(stack.pop(), Some(option))?;
                    self.find(TypeNode::Bool)?
                }
                Instr::Len => {
                    let counted = stack.pop().and_then(|ty| match self.types.get(ty)? {
                        TypeNode::List(_) | TypeNode::Map { .. } => Some(()),
                        _ => None,
                    });

                    counted.ok_or_else(|| fail(Invalid::LenNonList))?;
                    self.find(TypeNode::Int)?
                }
                Instr::MapIndex(map) | Instr::MapIndexHex(map) | Instr::MapGet(map) => {
                    let invalid = match instr {
                        Instr::MapIndex(_) | Instr::MapIndexHex(_) => Invalid::IndexNonMap,
                        _ => Invalid::GetNonMap,
                    };

                    let Some(TypeNode::Map { key, value }) = self.types.get(map) else {
                        return Err(fail(invalid));
                    };

                    need(stack.pop(), Some(*key))?;
                    need(stack.pop(), Some(map)).map_err(|_| fail(invalid))?;

                    match instr {
                        Instr::MapIndex(_) | Instr::MapIndexHex(_) => *value,
                        _ => self.find(TypeNode::Option(*value))?,
                    }
                }
                Instr::Call(f) => {
                    let function =
                        index(functions, f).ok_or_else(|| fail(Invalid::NoSuchFunction))?;

                    for param in function.params.iter().rev() {
                        need(stack.pop(), Some(*param))?;
                    }

                    function.ret
                }
            };

            stack.push(push);
        }

        if let Some(expected) = targets.remove(&code.len())
            && expected != stack.top
        {
            return Err(Invalid::EndStackDisagrees.into());
        }

        if !targets.is_empty() {
            return Err(Invalid::JumpIntoNothing.into());
        }

        let values = stack.len();

        if values != 1 || stack.pop() != Some(result) {
            return Err(Invalid::NotOneResult {
                values: u64::try_from(values).unwrap_or(u64::MAX),
            }
            .into());
        }

        Ok(Memory {
            stack: words(stack.max)?,
            heap: words(heap)?,
            constants: 0,
        })
    }

    /// How many keys the constant `keys` holds, a list of `key`s in key
    /// order, each once, as a map's lookup needs them: `Int`s by value,
    /// strings by their bytes.
    /// The entries of a map made from the key constant `keys`, checking
    /// that its keys are of type `key`, and sorted unless `sorted` says it
    /// was found so before: string keys are compared byte by byte, and
    /// many maps may share one constant.
    fn map_keys(
        &self,
        keys: u32,
        key: TypeId,
        sorted: &mut HashSet<u32, FxBuildHasher>,
    ) -> Result<usize, Invalid> {
        let list = index(&self.constant_types, keys).ok_or(Invalid::NoSuchConstant)?;

        let (Some(Constant::List { items, .. }), true) =
            (index(self.constants, keys), self.entry(*list) == Some(key))
        else {
            return Err(Invalid::MapKeysMistyped);
        };

        if sorted.contains(&keys) {
            return Ok(items.len());
        }

        // Only `Int`s and strings are keys, a map type's key is one of them.
        let text = |c: u32| match index(self.constants, c) {
            Some(Constant::String(s)) => Some(s.as_bytes()),
            _ => None,
        };

        let in_order = items.windows(2).all(|pair| match *pair {
            [Scalar::Int(a), Scalar::Int(b)] => a < b,
            [Scalar::Const(a), Scalar::Const(b)] => matches!(
                (text(a), text(b)),
                (Some(a), Some(b)) if a < b
            ),
            _ => false,
        });

        if items.is_empty() {
            return Err(Invalid::EmptyMap);
        }

        if !in_order {
            return Err(Invalid::MapKeysUnsorted);
        }

        sorted.insert(keys);
        Ok(items.len())
    }

    fn entry(&self, list: TypeId) -> Option<TypeId> {
        match self.types.get(list)? {
            TypeNode::List(entry) => Some(*entry),
            _ => None,
        }
    }
}

/// The verifier's model of the stack: the type of each value on it, and
/// the words they take. It is a persistent stack in which equal stacks
/// share one id, so a jump records and compares the stack it lands with
/// in constant time, however deep it is.
struct Model<'a> {
    types: &'a Types,
    /// Every stack seen, by id; the first is the empty stack.
    frames: Vec<Frame>,
    /// The id of each stack by the id of the stack below its top and the
    /// type on top.
    ids: HashMap<(usize, TypeId), usize, FxBuildHasher>,
    /// The id of the stack as it is now.
    top: usize,
    max: usize,
}

#[derive(Clone, Copy)]
struct Frame {
    below: usize,
    ty: TypeId,
    words: usize,
    len: usize,
}

impl<'a> Model<'a> {
    fn new(types: &'a Types, code_len: usize) -> Self {
        let cap = code_len.min(64);
        let mut frames = Vec::with_capacity(cap.saturating_add(1));
        frames.push(Frame {
            below: 0,
            ty: TypeId(0),
            words: 0,
            len: 0,
        });

        Self {
            types,
            frames,
            ids: HashMap::with_capacity_and_hasher(cap, types.hasher()),
            top: 0,
            max: 0,
        }
    }

    fn frame(&self) -> Option<Frame> {
        self.frames.get(self.top).copied()
    }

    fn len(&self) -> usize {
        self.frame().map_or(0, |f| f.len)
    }

    fn pop(&mut self) -> Option<TypeId> {
        let frame = self.frame().filter(|f| f.len > 0)?;
        self.top = frame.below;
        Some(frame.ty)
    }

    fn push(&mut self, ty: TypeId) {
        let (words, len) = self.frame().map_or((0, 0), |f| (f.words, f.len));

        let frame = Frame {
            below: self.top,
            ty,
            words: words.saturating_add(self.types.width(ty)),
            len: len.saturating_add(1),
        };

        let next = self.frames.len();
        let id = *self.ids.entry((self.top, ty)).or_insert(next);

        if id == next {
            self.frames.push(frame);
        }

        self.top = id;
        self.max = self.max.max(frame.words);
    }
}

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}
