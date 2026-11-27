use std::collections::{BTreeMap, HashMap, HashSet};

use musli::{Decode, Encode};

use crate::Type;
use crate::message::{Invalid, Message, StoreError};
use crate::types::{TypeId, TypeNode, Types, is_value_name};

/// The version of the stored form [`Program::to_bytes`] writes.
pub(crate) const FORMAT: u32 = 3;

/// How many nodes the globals' types may have together, each written out
/// in full.
const MAX_GLOBAL_TYPE_NODES: usize = 1 << 21;

/// One instruction of a stack machine over words of memory. A value takes
/// the words its type's layout gives it: one for an `Int`, a `Bool` (0 or
/// 1), and a string, list or record (a reference packing where its parts
/// are and how many there are). An option of a `Bool`, string, list or
/// record is one word with [`Word::NONE`](crate::word::Word::NONE) for
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
    /// Push the value of type `ty` bound to the global `at` words into the
    /// globals.
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
    /// A list's length.
    Len,
    /// Call one of the program's host functions.
    Call(u32),
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

/// A value the caller binds before a program runs.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct Global {
    pub(crate) name: String,
    pub(crate) ty: TypeId,
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
/// assert_eq!(memory.globals(), 2); // an `Option<Int>` is held in two words
/// assert_eq!(memory.constants(), 3); // the constant list's entries
/// assert!(memory.stack() > 0);
/// assert_eq!(memory.heap(), 0);
/// assert_eq!(
///     memory.total(),
///     u64::from(memory.stack() + memory.heap() + memory.constants() + memory.globals()),
/// );
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub struct Memory {
    pub(crate) stack: u32,
    pub(crate) heap: u32,
    pub(crate) constants: u32,
    pub(crate) globals: u32,
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

    /// The values it reads, as they are held: a word each, two for an
    /// `Option<Int>`. Their parts, such as a bound list's entries, are the
    /// bindings' to fit in the memory left.
    pub fn globals(&self) -> u32 {
        self.globals
    }

    /// Every word, the parts of bound and returned values aside.
    pub fn total(&self) -> u64 {
        u64::from(self.stack)
            .saturating_add(u64::from(self.heap))
            .saturating_add(u64::from(self.constants))
            .saturating_add(u64::from(self.globals))
    }
}

#[derive(Encode, Decode)]
pub(crate) struct Stored {
    pub(crate) format: u32,
    pub(crate) types: Vec<TypeNode>,
    pub(crate) constants: Vec<Constant>,
    pub(crate) globals: Vec<Global>,
    pub(crate) functions: Vec<HostFunction>,
    pub(crate) code: Vec<Instr>,
    pub(crate) result: TypeId,
    pub(crate) memory: Memory,
}

/// A compiled expression: checked, typed, and ready to store or run.
///
/// It names the values it reads and the caller's functions it calls, with
/// their types, so it runs without the [`Environment`](crate::Environment)
/// it was compiled with. [`Program::from_bytes`] checks stored bytes
/// before giving one back.
#[derive(Debug, Clone)]
pub struct Program {
    pub(crate) types: Types,
    pub(crate) constants: Vec<Constant>,
    pub(crate) globals: Vec<Global>,
    pub(crate) functions: Vec<HostFunction>,
    pub(crate) code: Vec<Instr>,
    pub(crate) result: TypeId,
    pub(crate) memory: Memory,
    /// The type of each global, built once so listing them does not.
    pub(crate) global_types: Vec<Type>,
    /// Where each global is held, in words from the first.
    pub(crate) global_at: Vec<u32>,
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

    /// The values it reads, which are bound before it runs, with their
    /// types: each declared name the expression uses, once, and none it
    /// does not.
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
        self.globals
            .iter()
            .zip(&self.global_types)
            .map(|(g, ty)| (g.name.as_str(), ty))
    }

    /// The caller's functions it calls: each name, parameter types and
    /// return type.
    pub fn functions(&self) -> Vec<(&str, Vec<Type>, Type)> {
        self.functions
            .iter()
            .map(|f| {
                let params = f.params.iter().map(|t| self.type_of(*t)).collect();
                (f.name.as_str(), params, self.type_of(f.ret))
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
            format: FORMAT,
            types: self.types.nodes().to_vec(),
            constants: self.constants.clone(),
            globals: self.globals.clone(),
            functions: self.functions.clone(),
            code: self.code.clone(),
            result: self.result,
            memory: self.memory,
        };

        musli::storage::to_vec(&stored).map_err(|e| {
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
        let stored: Stored = musli::storage::from_slice(bytes).map_err(|e| {
            StoreError::new(Message::NotDecoded)
                .with_source(e.to_string())
                .context(Message::DecodingProgram)
        })?;

        if stored.format != FORMAT {
            return Err(StoreError::new(Message::WrongFormat {
                found: stored.format,
                reads: FORMAT,
            })
            .context(Message::DecodingProgram));
        }

        let verifying = |e: StoreError| e.context(Message::VerifyingProgram);
        let types = Types::from_stored(stored.types).map_err(verifying)?;

        let program = Self::verify(
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
        types: Types,
        constants: Vec<Constant>,
        globals: Vec<Global>,
        functions: Vec<HostFunction>,
        code: Vec<Instr>,
        result: TypeId,
    ) -> Result<Self, StoreError> {
        let mut verifier = Verifier {
            types: &types,
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

        let mut names = HashSet::new();
        let mut global_at = Vec::with_capacity(globals.len());
        let mut global_words = 0usize;

        for (at, global) in globals.iter().enumerate() {
            let at_global = |e: StoreError| e.context(Message::AtGlobal { at: count(at) });

            if !is_value_name(&global.name) || !names.insert(global.name.as_str()) {
                return Err(at_global(Invalid::BadGlobal.into()));
            }

            verifier.known(global.ty).map_err(at_global)?;
            global_at.push(words(global_words)?);
            global_words = global_words.saturating_add(types.width(global.ty));
        }

        let mut signatures = HashSet::new();

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

        let global_ids = global_at
            .iter()
            .copied()
            .zip(globals.iter().map(|g| g.ty))
            .collect::<HashMap<_, _>>();

        let memory = verifier.code(&code, &global_ids, &functions, result)?;

        let ids: Vec<TypeId> = globals.iter().map(|g| g.ty).collect();

        // The globals' types are held written out, and building them holds
        // every type they reach too, so both are bounded together, not only
        // one by one.
        let nodes = ids
            .iter()
            .fold(0usize, |n, id| n.saturating_add(types.size(*id)));
        let reached = types
            .reached_size(&ids)
            .ok_or_else(|| StoreError::from(Invalid::BadGlobal))?;

        if nodes > MAX_GLOBAL_TYPE_NODES || reached > MAX_GLOBAL_TYPE_NODES {
            return Err(Invalid::TypeTooLarge.into());
        }

        let global_types = types
            .to_types(&ids)
            .ok_or_else(|| StoreError::from(Invalid::BadGlobal))?;

        Ok(Self {
            memory: Memory {
                constants: words(constant_words)?,
                globals: words(global_words)?,
                ..memory
            },
            types,
            constants,
            globals,
            functions,
            code,
            result,
            global_types,
            global_at,
            constant_at,
        })
    }
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
        globals: &HashMap<u32, TypeId>,
        functions: &[HostFunction],
        result: TypeId,
    ) -> Result<Memory, StoreError> {
        let int = self.find(TypeNode::Int).ok();
        let bool = self.find(TypeNode::Bool).ok();

        let mut stack = Model {
            types: self.types,
            stack: Vec::new(),
            words: 0,
            max: 0,
        };
        let mut heap = 0usize;
        // The stack each pending jump expects where it lands.
        let mut targets = BTreeMap::<usize, Vec<TypeId>>::new();

        for (at, instr) in code.iter().enumerate() {
            let fail = |invalid: Invalid| {
                StoreError::from(invalid).context(Message::AtInstruction { pc: count(at) })
            };

            if let Some(expected) = targets.remove(&at)
                && expected != stack.stack
            {
                return Err(fail(Invalid::TwoStacks));
            }

            let need = |ty: Option<TypeId>, want: Option<TypeId>| match ty {
                None => Err(fail(Invalid::PopsEmpty)),
                Some(ty) if Some(ty) == want => Ok(()),
                Some(_) => Err(fail(Invalid::WrongOperand)),
            };

            let push = match *instr {
                Instr::Int(_) => self.find(TypeNode::Int)?,
                Instr::Bool(_) => self.find(TypeNode::Bool)?,
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
                    self.find(TypeNode::Int)?
                }
                Instr::Not => {
                    need(stack.pop(), bool)?;
                    self.find(TypeNode::Bool)?
                }
                Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Rem => {
                    need(stack.pop(), int)?;
                    need(stack.pop(), int)?;
                    self.find(TypeNode::Int)?
                }
                Instr::Lt | Instr::Le | Instr::Gt | Instr::Ge => {
                    need(stack.pop(), int)?;
                    need(stack.pop(), int)?;
                    self.find(TypeNode::Bool)?
                }
                Instr::Eq | Instr::Ne => {
                    let right = stack.pop();
                    let left = stack.pop();

                    if left.is_none() || left != right || (left != int && left != bool) {
                        return Err(fail(Invalid::MixedCompare));
                    }

                    self.find(TypeNode::Bool)?
                }
                Instr::And(skip) | Instr::Or(skip) => {
                    let top = stack.pop();
                    need(top, bool)?;

                    let target = usize::try_from(skip)
                        .ok()
                        .and_then(|skip| at.checked_add(skip)?.checked_add(1))
                        .filter(|target| *target <= code.len())
                        .ok_or_else(|| fail(Invalid::JumpPastEnd))?;

                    let mut landing = stack.stack.clone();
                    landing.extend(top);

                    if targets.get(&target).is_some_and(|other| *other != landing) {
                        return Err(fail(Invalid::JumpsDisagree));
                    }

                    targets.insert(target, landing);
                    continue;
                }
                Instr::Field { at, ty } => {
                    let record = stack.pop().ok_or_else(|| fail(Invalid::PopsEmpty))?;

                    let Some(TypeNode::Record(fields)) = self.types.get(record) else {
                        return Err(fail(Invalid::FieldOfNonRecord));
                    };

                    let at = usize::try_from(at).unwrap_or(usize::MAX);

                    if !self
                        .types
                        .field_offsets(fields)
                        .any(|(offset, field)| offset == at && field.ty == ty)
                    {
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
                Instr::Len => {
                    stack
                        .pop()
                        .and_then(|l| self.entry(l))
                        .ok_or_else(|| fail(Invalid::LenNonList))?;
                    self.find(TypeNode::Int)?
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
            && expected != stack.stack
        {
            return Err(Invalid::EndStackDisagrees.into());
        }

        if !targets.is_empty() {
            return Err(Invalid::JumpIntoNothing.into());
        }

        if stack.stack != [result] {
            return Err(Invalid::NotOneResult {
                values: u64::try_from(stack.stack.len()).unwrap_or(u64::MAX),
            }
            .into());
        }

        Ok(Memory {
            stack: words(stack.max)?,
            heap: words(heap)?,
            constants: 0,
            globals: 0,
        })
    }

    fn entry(&self, list: TypeId) -> Option<TypeId> {
        match self.types.get(list)? {
            TypeNode::List(entry) => Some(*entry),
            _ => None,
        }
    }
}

/// The verifier's model of the stack: the type of each value on it, and
/// the words they take.
struct Model<'a> {
    types: &'a Types,
    stack: Vec<TypeId>,
    words: usize,
    max: usize,
}

impl Model<'_> {
    fn pop(&mut self) -> Option<TypeId> {
        let ty = self.stack.pop()?;
        self.words = self.words.saturating_sub(self.types.width(ty));
        Some(ty)
    }

    fn push(&mut self, ty: TypeId) {
        self.stack.push(ty);
        self.words = self.words.saturating_add(self.types.width(ty));
        self.max = self.max.max(self.words);
    }
}

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}
