//! The error core every evalite error is built on: one [`Message`] enum
//! for every error and every context, frames of context with optional
//! locations, the `.context` pattern that adds them, and the public
//! [`Label`]s an error is inspected through.
//!
//! A message holds only plain data: numbers, and ids into the
//! [`Program`] (or the [`Globals`](crate::Globals) store) that names are
//! resolved against when it is shown, through [`Names`]. Nothing here
//! allocates or formats until an error is made or shown.

use std::fmt;
use std::iter::FusedIterator;

use syntree::Span;

use crate::debug::DebugInfo;
use crate::error::{Diagnostic, ParseText};
use crate::program::Program;
use crate::types::is_field_name;
use crate::types::{TypeId, TypeNode, Types};

/// What went wrong, or what was being done when it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    /// The program needs more memory than the machine has.
    TooMuchMemory { needed: u64, allowed: u32 },
    /// A value set in a store that its environment does not declare.
    NotDeclared { name: Box<str> },
    /// A path from what a function gives, which no store holds.
    FunctionPath { name: Box<str> },
    /// A path into a value of the store that is not set.
    NotSet { global: u32 },
    /// A path's `step` from a value of type `ty`, which has no such part.
    NoStep { ty: TypeId, step: PathStep },
    /// A path through an absent option.
    NoneOnPath,
    /// A path to insert or remove at that does not end with a list's
    /// index or a map's key.
    NotEntryPath,
    /// A path to push onto that is to a value of type `ty`, not a list.
    NotList { ty: TypeId },
    /// A path's map key of another type than the map's keys, `expected`.
    KeyMistyped { expected: TypeId, found: Found },
    /// A program that reads `global` (or, when `None`, some value) where
    /// the store does not hold it with its type.
    OtherLayout { global: Option<u32> },
    /// A store and a machine whose words together are past what a
    /// reference can address.
    PastAddressSpace { store: u64, machine: u32 },
    /// A value is not of its declared type: it is `found` instead, inside
    /// a `some(..)` when `in_some`.
    Mistyped {
        expected: TypeId,
        found: Found,
        in_some: bool,
    },
    /// A set or returned value does not fit in the memory left.
    TooLarge { needed: u64, left: u64 },
    /// A set or returned value holds a string of `len` bytes, or a list or
    /// map of `len` entries, past the `max` a reference word can say.
    TooLong { len: u64, max: u64 },
    /// A value the program reads is not set in the store it runs with.
    Unset { global: u32 },
    /// The run took every step it was allowed.
    OutOfFuel { fuel: u64 },
    /// Arithmetic past the range of an `Int`; `b` is unused for `Neg` and
    /// `Abs`.
    Overflow { op: Op, a: i64, b: i64 },
    /// `a / 0` or `a % 0`.
    DivideByZero { op: Op, a: i64 },
    /// `list[index]` past either end of the list.
    IndexOutOfRange { index: i64, len: u64 },
    /// `map[key]` of a key the map does not have.
    NoSuchKey { key: Key },
    /// `option!` of an absent option.
    UnwrapNone,
    /// `value.clamp(low, high)` with `low` above `high`.
    ClampBounds { low: i64, high: i64 },
    /// A function of the caller's failed; its own error is the source.
    HostFailed,
    /// The program is not one evalite made.
    Fault(Fault),
    /// Context: loading a program onto a machine.
    LoadingProgram,
    /// Context: setting a value of a store.
    Setting { global: u32 },
    /// Context: setting the part of a value of a store at `path`, as the
    /// caller wrote it.
    SettingAt { path: Box<str> },
    /// Context: inserting into a value of a store at `path`.
    InsertingAt { path: Box<str> },
    /// Context: removing the part of a value of a store at `path`.
    RemovingAt { path: Box<str> },
    /// Context: pushing onto the list of a store at `path`.
    PushingOnto { path: Box<str> },
    /// Context: the part of a value at `path` from `root`.
    AtPath { root: Root, path: Box<[Seg]> },
    /// Context: calling a function of the caller's.
    CallingHost { function: u32 },
    /// Context: checking what a function of the caller's returned.
    ReadingHostResult { function: u32 },
    /// Context: running one instruction.
    Instruction { pc: u32 },
    /// Context: running the program.
    Running,
    /// Context: reading the program's value back.
    ReadingResult,
    /// Something could not be encoded; the encoder's error is the source.
    CannotEncode,
    /// The bytes do not decode; the decoder's error is the source.
    NotDecoded,
    /// Stored in format `found`, where this evalite reads `reads`.
    WrongFormat { found: u32, reads: u32 },
    /// Stored bytes decode, but to something evalite would not make.
    Invalid(Invalid),
    /// Context: storing a program.
    StoringProgram,
    /// Context: reading a stored program.
    DecodingProgram,
    /// Context: checking a program is one evalite would compile.
    VerifyingProgram,
    /// Context: storing debug info.
    StoringDebugInfo,
    /// Context: reading stored debug info.
    DecodingDebugInfo,
    /// Context: storing a structure.
    StoringStructure,
    /// Context: reading a stored structure.
    DecodingStructure,
    /// Context: storing a store's values.
    StoringValues,
    /// Context: reading a store's stored values.
    DecodingValues,
    /// Context: one entry of a stored type table.
    AtType { at: u32 },
    /// Context: one stored constant.
    AtConstant { at: u32 },
    /// Context: one value a stored program reads.
    AtGlobal { at: u32 },
    /// Context: one function of the caller's a stored program calls.
    AtFunction { at: u32 },
    /// Context: one instruction of stored code.
    AtInstruction { pc: u32 },
    /// Context: one node of a stored structure.
    AtNode { node: u32 },
    /// Context: one span of stored debug info.
    AtSpan { at: u32 },
}

/// A map key, as an error names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Key {
    Int(i64),
    /// A `String` key, cut to [`MAX_KEY_SHOWN`] characters.
    String(Box<str>),
}

/// The most characters of a `String` key, or of a field's name, an error
/// shows.
pub(crate) const MAX_KEY_SHOWN: usize = 64;

impl Key {
    /// The key `s`, cut short when it is long.
    pub(crate) fn string(s: &str) -> Self {
        Self::String(cut(s))
    }
}

/// `s`, cut to [`MAX_KEY_SHOWN`] characters and an ellipsis when it is
/// longer.
pub(crate) fn cut(s: &str) -> Box<str> {
    match s.char_indices().nth(MAX_KEY_SHOWN) {
        Some((at, _)) => {
            let mut cut = s.get(..at).unwrap_or_default().to_owned();
            cut.push('…');
            cut.into()
        }
        None => s.into(),
    }
}

/// A step of a path a caller wrote, as an error names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PathStep {
    /// A field's name, [`cut`] when it is long.
    Field(Box<str>),
    Key(Key),
    Index(i64),
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{n}"),
            Self::String(s) => write!(f, "{s:?}"),
        }
    }
}

/// Why stored bytes are not something evalite would make, by what is
/// wrong with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invalid {
    TypeRefersLater,
    FieldsUnsorted,
    TypeRepeated,
    TypeTooDeep,
    TypeTooLarge,
    MapKeyType,
    UnknownType,
    MissingType,
    EmptyConstantList,
    ConstantRefersLater,
    ConstantEntryMistyped,
    BadGlobal,
    BadFunction,
    TooMuchToCount,
    TwoStacks,
    PopsEmpty,
    WrongOperand,
    NoSuchConstant,
    NoSuchGlobal,
    GlobalsOverlap,
    MixedCompare,
    JumpPastEnd,
    JumpsDisagree,
    FieldOfNonRecord,
    NoSuchField,
    IndexNonList,
    GetNonList,
    EmptyList,
    ListEntriesMissing,
    NoOperands,
    UnwrapNonOption,
    TestNonOption,
    LenNonList,
    IndexNonMap,
    GetNonMap,
    MakeNonMap,
    EmptyMap,
    MapKeysMistyped,
    MapKeysUnsorted,
    MapEntriesMissing,
    NoSuchFunction,
    EndStackDisagrees,
    JumpIntoNothing,
    NotOneResult { values: u64 },
    MemoryMismatch { declared: u64, used: u64 },
    TooManyNodes,
    PartPastEnd,
    PartTwice,
    RootPastEnd,
    UnusedNodes,
    NotOneTree,
    SpanBackwards,
    ValueEndsEarly,
    ValuePastEnd,
    ValueTooDeep,
    ValueFieldsUnsorted,
    ValueKeysUnsorted,
    ValueTwice,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            Self::TypeRefersLater => "the type refers to a later type",
            Self::FieldsUnsorted => "the type has fields that are not sorted and unique",
            Self::TypeRepeated => "the type repeats an earlier type",
            Self::TypeTooDeep => "the type nests too deeply",
            Self::TypeTooLarge => "the type is too large",
            Self::MapKeyType => "the type is a map keyed by other than Int or String",
            Self::UnknownType => "a type is not in the type table",
            Self::MissingType => "a type it needs is not in the type table",
            Self::EmptyConstantList => "the constant is an empty list",
            Self::ConstantRefersLater => "the constant refers to a later constant",
            Self::ConstantEntryMistyped => "the constant has an entry of another type",
            Self::BadGlobal => "the value's name is invalid or repeated",
            Self::BadFunction => "the function's name is invalid or its signature repeated",
            Self::TooMuchToCount => "it uses too much memory to count",
            Self::TwoStacks => "the instruction is reached with two different stacks",
            Self::PopsEmpty => "the instruction pops an empty stack",
            Self::WrongOperand => "the instruction finds an operand of the wrong type",
            Self::NoSuchConstant => "the instruction names a constant there is not",
            Self::NoSuchGlobal => "the instruction names a value there is not",
            Self::GlobalsOverlap => "the value is held in words another value is held in",
            Self::MixedCompare => "the instruction compares other than two Ints or two Bools",
            Self::JumpPastEnd => "the instruction jumps past the end",
            Self::JumpsDisagree => {
                "the instruction jumps where another jump lands with another stack"
            }
            Self::FieldOfNonRecord => "the instruction reads a field of other than a record",
            Self::NoSuchField => "the instruction reads a field the record does not have",
            Self::IndexNonList => "the instruction indexes other than a list of its entry type",
            Self::GetNonList => "the instruction gets from other than a list of its entry type",
            Self::EmptyList => "the instruction makes a list of no entries",
            Self::ListEntriesMissing => "the instruction makes a list of entries it does not find",
            Self::NoOperands => "the instruction takes no operands",
            Self::UnwrapNonOption => "the instruction unwraps other than an option type",
            Self::TestNonOption => "the instruction tests other than an option type",
            Self::LenNonList => "the instruction takes the length of other than a list or a map",
            Self::IndexNonMap => "the instruction looks up a key in other than a map of its type",
            Self::GetNonMap => "the instruction gets a key from other than a map of its type",
            Self::MakeNonMap => "the instruction makes a map of other than a map type",
            Self::EmptyMap => "the instruction makes a map of no entries",
            Self::MapKeysMistyped => {
                "the instruction makes a map of keys that are no constant list of its key type"
            }
            Self::MapKeysUnsorted => {
                "the instruction makes a map of keys that are not sorted and unique"
            }
            Self::MapEntriesMissing => "the instruction makes a map of entries it does not find",
            Self::NoSuchFunction => "the instruction calls a function there is not",
            Self::EndStackDisagrees => "a jump lands at the end with another stack",
            Self::JumpIntoNothing => "a jump lands inside nothing",
            Self::NotOneResult { values } => {
                return write!(
                    f,
                    "it ends with {} on the stack, not one of its result type",
                    plural(*values, "value", "values")
                );
            }
            Self::MemoryMismatch { declared, used } if declared == used => {
                return write!(
                    f,
                    "it declares {} of memory, but uses them otherwise",
                    plural(*declared, "word", "words")
                );
            }
            Self::MemoryMismatch { declared, used } => {
                return write!(
                    f,
                    "it declares {} of memory but uses {used}",
                    plural(*declared, "word", "words")
                );
            }
            Self::TooManyNodes => "there are too many nodes to number",
            Self::PartPastEnd => "the node has a part past the end",
            Self::PartTwice => "the node is a part twice",
            Self::RootPastEnd => "the root is past the end",
            Self::UnusedNodes => "some nodes are not part of the expression",
            Self::NotOneTree => "the structure's nodes are not one tree",
            Self::SpanBackwards => "the span ends before it starts",
            Self::ValueEndsEarly => "the value's parts end before the value does",
            Self::ValuePastEnd => "the value has parts past its end",
            Self::ValueTooDeep => "the value nests too deeply",
            Self::ValueFieldsUnsorted => "the value has a record of fields not sorted and unique",
            Self::ValueKeysUnsorted => "the value has a map of keys not sorted and unique",
            Self::ValueTwice => "the value is stored twice",
        };

        f.write_str(what)
    }
}

/// What a mistyped value was found to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Found {
    Int,
    Bool,
    String,
    List,
    Record,
    Map,
    None,
    Some,
    /// A record without field `field` of the record type `record`.
    RecordWithout {
        record: TypeId,
        field: u32,
    },
    /// A record with a field its type does not have.
    RecordWithExtra,
}

/// An arithmetic operator, as an overflow names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Neg,
    Abs,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

/// Where a value path starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Root {
    /// A value the program reads.
    Global(u32),
    /// What a function of the caller's returned.
    Returned(u32),
}

/// One step of a value path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seg {
    /// Field `field` of the record type `record`.
    Field {
        record: TypeId,
        field: u32,
    },
    Index(u64),
    /// The value of a map's key.
    Key(Key),
    /// The key of a map's entry, from 0 in key order.
    KeyOf(u64),
}

/// A program that is not one evalite made, by what it did wrong. Programs
/// from [`compile`](crate::compile) and [`Program::from_bytes`] never
/// fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    WideAddress,
    WideLength,
    NoneReference,
    NotABool,
    PastAddressSpace,
    StackLargerThanMachine,
    ConstantsLargerThanMachine,
    OutsideMemory,
    StackOutsideMemory,
    ConstantOutsideMemory,
    NotOneResult,
    NoSuchConstant,
    NoSuchFunction,
    NoSuchType,
    CodeTooLong,
    JumpPastEnd,
    FieldPastRecord,
    ListTooLarge,
    ListPastStack,
    ArgumentsPastStack,
    UndeclaredHeap,
    UndeclaredMemory,
    UndeclaredStack,
    ListTooLongToCount,
    EmptyStack,
    ValueChangedType,
    RecordLostField,
    RecordWrongSize,
    RecordNotRecordType,
    ListLostEntries,
    MapLostEntries,
    MapUnsorted,
    RecordLostFields,
    OptionLostValue,
    ReadAsSeveral,
    NotUtf8,
    StorePastLimit,
    BadCapacity,
    WriteToStore,
    DynamicStep,
}

impl Fault {
    const fn what(self) -> &'static str {
        match self {
            Self::WideAddress => "memory past what a word can address",
            Self::WideLength => "a value too long for a word",
            Self::NoneReference => "a reference past the end of memory",
            Self::NotABool => "a Bool other than 0 or 1",
            Self::PastAddressSpace => "a size past the address space",
            Self::StackLargerThanMachine => "a stack larger than the machine",
            Self::ConstantsLargerThanMachine => "constants larger than the machine",
            Self::OutsideMemory => "a value outside memory",
            Self::StackOutsideMemory => "a stack outside memory",
            Self::ConstantOutsideMemory => "a constant outside memory",
            Self::NotOneResult => "not one value is left on the stack",
            Self::NoSuchConstant => "no such constant",
            Self::NoSuchFunction => "no such function",
            Self::NoSuchType => "no such type",
            Self::CodeTooLong => "code too long",
            Self::JumpPastEnd => "jumps past the end",
            Self::FieldPastRecord => "reads a field the record does not have",
            Self::ListTooLarge => "a list too large",
            Self::ListPastStack => "makes a list of more entries than the stack has",
            Self::ArgumentsPastStack => "calls with more arguments than the stack has",
            Self::UndeclaredHeap => "builds more lists than it declares",
            Self::UndeclaredMemory => "uses more memory than it declares",
            Self::UndeclaredStack => "the stack grows past the depth it declares",
            Self::ListTooLongToCount => "a list too long to count",
            Self::EmptyStack => "pops an empty stack",
            Self::ValueChangedType => "a value changed type",
            Self::RecordLostField => "a record lost a field",
            Self::RecordWrongSize => "a record of the wrong size",
            Self::RecordNotRecordType => "a record of other than a record type",
            Self::ListLostEntries => "a list lost entries",
            Self::MapLostEntries => "a map lost entries",
            Self::MapUnsorted => "a map whose keys are not sorted and unique",
            Self::RecordLostFields => "a record lost fields",
            Self::OptionLostValue => "an option lost its value",
            Self::ReadAsSeveral => "a value read as several",
            Self::NotUtf8 => "a string is not UTF-8",
            Self::StorePastLimit => "a store past its limit",
            Self::BadCapacity => "a list or map with a capacity below its length",
            Self::WriteToStore => "writes to the store it reads",
            Self::DynamicStep => "a path with a computed step",
        }
    }
}

/// Where in a program a frame is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Location {
    /// The instruction at this index of the code.
    Instr(u32),
}

/// One message of an error, with where it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) message: Message,
    pub(crate) location: Option<Location>,
}

impl Frame {
    fn span(&self, debug: Option<&DebugInfo>) -> Option<Span<u32>> {
        match self.location? {
            Location::Instr(pc) => debug?.span(pc),
        }
    }
}

/// What gives a message's ids their names: a [`Program`] for its runs,
/// or a store for setting its values.
pub(crate) trait Names {
    /// The table the message's type ids are in.
    fn types(&self) -> &Types;

    /// The name and type of the value `global`.
    fn global(&self, global: u32) -> Option<(&str, TypeId)>;

    /// The name of the caller's function `function`.
    fn function(&self, function: u32) -> Option<&str>;
}

impl Names for Program {
    fn types(&self) -> &Types {
        &self.types
    }

    fn global(&self, global: u32) -> Option<(&str, TypeId)> {
        let global = index(&self.globals, global)?;
        Some((global.name.as_str(), global.ty))
    }

    fn function(&self, function: u32) -> Option<&str> {
        Some(index(&self.functions, function)?.name.as_str())
    }
}

/// A message shown with the names `names` gives its ids, or without
/// names when there are none.
pub(crate) struct Shown<'a> {
    pub(crate) message: &'a Message,
    pub(crate) names: Option<&'a dyn Names>,
}

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.names;

        match self.message {
            Message::TooMuchMemory { needed, allowed } => write!(
                f,
                "the program needs {} of memory, more than the {allowed} the machine allows",
                plural(*needed, "word", "words")
            ),
            Message::NotDeclared { name } => write!(f, "no value `{name}` is declared"),
            Message::OtherLayout { global } => match global.and_then(|g| global_name(p, g)) {
                Some(name) => write!(
                    f,
                    "the store does not hold `{name}` where the program reads it, of its type"
                ),
                None => {
                    f.write_str("the store does not hold the program's values where it reads them")
                }
            },
            Message::PastAddressSpace { store, machine } => write!(
                f,
                "the store's {} and the machine's {} are more than a run can address",
                plural(*store, "word", "words"),
                plural(*machine, "word", "words")
            ),
            Message::Mistyped {
                expected,
                found,
                in_some,
            } => {
                match p.and_then(|p| p.types().to_type(*expected)) {
                    Some(ty) => write!(f, "expected {ty}, found ")?,
                    None => f.write_str("expected another type, found ")?,
                }

                if *in_some {
                    f.write_str("some(")?;
                }

                write_found(f, p, *found)?;

                if *in_some {
                    f.write_str(")")?;
                }

                Ok(())
            }
            Message::FunctionPath { name } => write!(
                f,
                "`{name}()` is what a function gives, not a value a store holds"
            ),
            Message::NotSet { global } => match global_name(p, *global) {
                Some(name) => write!(f, "`{name}` is not set"),
                None => f.write_str("the value is not set"),
            },
            Message::NoStep { ty, step } => {
                match p.and_then(|p| p.types().to_type(*ty)) {
                    Some(ty) => write!(f, "{ty} has no ")?,
                    None => f.write_str("the value has no ")?,
                }

                match step {
                    PathStep::Field(name) => write!(f, "field {name:?}"),
                    PathStep::Key(key) => write!(f, "key {key}"),
                    PathStep::Index(index) => write!(f, "index {index}"),
                }
            }
            Message::NoneOnPath => f.write_str("the optional value is none"),
            Message::NotEntryPath => {
                f.write_str("the path does not end with a list's index or a map's key")
            }
            Message::NotList { ty } => match p.and_then(|p| p.types().to_type(*ty)) {
                Some(ty) => write!(f, "{ty} is not a list"),
                None => f.write_str("the value is not a list"),
            },
            Message::KeyMistyped { expected, found } => {
                match p.and_then(|p| p.types().to_type(*expected)) {
                    Some(ty) => {
                        let ty = ty.to_string();
                        let a = crate::error::article(&ty);
                        write!(f, "expected {a} {ty} key, found ")?;
                    }
                    None => f.write_str("expected a key of another type, found ")?,
                }

                write_found(f, p, *found)
            }
            Message::TooLarge { needed, left } => write!(
                f,
                "the value needs {} of memory, {left} are left",
                plural(*needed, "word", "words")
            ),
            Message::TooLong { len, max } => write!(
                f,
                "the value holds a string, list or map {len} long, longer than the {max} a value may hold"
            ),
            Message::Unset { global } => match (global_name(p, *global), global_type(p, *global)) {
                (Some(name), Some(ty)) => {
                    let ty = ty.to_string();
                    let a = crate::error::article(&ty);
                    write!(f, "`{name}` is not set, set it to {a} {ty} before running")
                }
                _ => f.write_str("a value the program reads is not set"),
            },
            Message::OutOfFuel { fuel } => write!(
                f,
                "the expression took too long: it used all of its {}",
                plural(*fuel, "step", "steps")
            ),
            Message::Overflow { op, a, b } => {
                f.write_str("`")?;
                write_op(f, *op, *a, *b)?;
                f.write_str("` overflows an Int")
            }
            Message::DivideByZero { op, a } => {
                f.write_str("`")?;
                write_op(f, *op, *a, 0)?;
                f.write_str("` divides by zero")
            }
            Message::IndexOutOfRange { index, len } => {
                write!(
                    f,
                    "index {index} is out of range of a list of {}",
                    plural(*len, "entry", "entries")
                )
            }
            Message::NoSuchKey { key } => write!(f, "the map has no key {key}"),
            Message::UnwrapNone => f.write_str("`!` found no value: the optional value is none"),
            Message::ClampBounds { low, high } => write!(
                f,
                "`clamp` needs its low bound {low} to be at most its high bound {high}"
            ),
            Message::HostFailed => f.write_str("the function failed"),
            Message::Fault(fault) => write!(f, "the program is faulty: {}", fault.what()),
            Message::LoadingProgram => f.write_str("while loading the program"),
            Message::Setting { global } => match global_name(p, *global) {
                Some(name) => write!(f, "while setting `{name}`"),
                None => f.write_str("while setting a value"),
            },
            Message::SettingAt { path } => write!(f, "while setting `{path}`"),
            Message::InsertingAt { path } => write!(f, "while inserting at `{path}`"),
            Message::RemovingAt { path } => write!(f, "while removing `{path}`"),
            Message::PushingOnto { path } => write!(f, "while pushing onto `{path}`"),
            Message::AtPath { root, path } => {
                f.write_str("at `")?;

                match root {
                    Root::Global(global) => match global_name(p, *global) {
                        Some(name) => f.write_str(name)?,
                        None => f.write_str("_")?,
                    },
                    Root::Returned(function) => match function_name(p, *function) {
                        Some(name) => write!(f, "{name}(..)")?,
                        None => f.write_str("_(..)")?,
                    },
                }

                for seg in path {
                    match seg {
                        Seg::Field { record, field } => match field_name(p, *record, *field) {
                            Some(name) if is_field_name(name) => write!(f, ".{name}")?,
                            Some(name) => write!(f, "[{name:?}]")?,
                            None => f.write_str("._")?,
                        },
                        Seg::Index(i) => write!(f, "[{i}]")?,
                        Seg::Key(key) => write!(f, "[{key}]")?,
                        Seg::KeyOf(n) => write!(f, "[key #{n}]")?,
                    }
                }

                f.write_str("`")
            }
            Message::CallingHost { function } => match function_name(p, *function) {
                Some(name) => write!(f, "while calling `{name}`"),
                None => f.write_str("while calling a function"),
            },
            Message::ReadingHostResult { function } => match function_name(p, *function) {
                Some(name) => write!(f, "while reading what `{name}` returned"),
                None => f.write_str("while reading what a function returned"),
            },
            Message::Instruction { pc } => write!(f, "while running instruction {pc}"),
            Message::Running => f.write_str("while running the program"),
            Message::ReadingResult => f.write_str("while reading the program's value"),
            Message::CannotEncode => f.write_str("it could not be encoded"),
            Message::NotDecoded => f.write_str("the bytes do not decode"),
            Message::WrongFormat { found, reads } => write!(
                f,
                "it was stored in format {found}, this evalite reads format {reads}"
            ),
            Message::Invalid(invalid) => invalid.fmt(f),
            Message::StoringProgram => f.write_str("while storing the program"),
            Message::DecodingProgram => f.write_str("while reading a stored program"),
            Message::VerifyingProgram => f.write_str("while verifying the program"),
            Message::StoringDebugInfo => f.write_str("while storing debug info"),
            Message::DecodingDebugInfo => f.write_str("while reading stored debug info"),
            Message::StoringStructure => f.write_str("while storing the structure"),
            Message::DecodingStructure => f.write_str("while reading a stored structure"),
            Message::StoringValues => f.write_str("while storing the values"),
            Message::DecodingValues => f.write_str("while reading stored values"),
            Message::AtType { at } => write!(f, "at type {at}"),
            Message::AtConstant { at } => write!(f, "at constant {at}"),
            Message::AtGlobal { at } => write!(f, "at value {at}"),
            Message::AtFunction { at } => write!(f, "at function {at}"),
            Message::AtInstruction { pc } => write!(f, "at instruction {pc}"),
            Message::AtNode { node } => write!(f, "at node #{node}"),
            Message::AtSpan { at } => write!(f, "at span {at}"),
        }
    }
}

fn write_op(f: &mut fmt::Formatter<'_>, op: Op, a: i64, b: i64) -> fmt::Result {
    let op = match op {
        Op::Neg => return write!(f, "-({a})"),
        Op::Abs => return write!(f, "({a}).abs()"),
        Op::Add => "+",
        Op::Sub => "-",
        Op::Mul => "*",
        Op::Div => "/",
        Op::Rem => "%",
    };

    write!(f, "{a} {op} {b}")
}

/// Displays a count with the singular or plural noun to match.
pub(crate) struct Plural<T> {
    n: T,
    one: &'static str,
    many: &'static str,
}

/// Shows `n` followed by `one` when it is 1 and `many` otherwise.
pub(crate) fn plural<T>(n: T, one: &'static str, many: &'static str) -> Plural<T> {
    Plural { n, one, many }
}

impl<T: fmt::Display + Copy + PartialEq + From<u8>> fmt::Display for Plural<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let noun = if self.n == T::from(1) {
            self.one
        } else {
            self.many
        };
        write!(f, "{} {noun}", self.n)
    }
}

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}

/// What a mistyped value was `found` to be.
fn write_found(f: &mut fmt::Formatter<'_>, p: Option<&dyn Names>, found: Found) -> fmt::Result {
    match found {
        Found::Int => f.write_str("Int"),
        Found::Bool => f.write_str("Bool"),
        Found::String => f.write_str("String"),
        Found::List => f.write_str("a list"),
        Found::Record => f.write_str("a record"),
        Found::Map => f.write_str("a map"),
        Found::None => f.write_str("none"),
        Found::Some => f.write_str("some(..)"),
        Found::RecordWithout { record, field } => match field_name(p, record, field) {
            Some(name) => write!(f, "a record without `{name}`"),
            None => f.write_str("a record without one of its fields"),
        },
        Found::RecordWithExtra => f.write_str("a record with an extra field"),
    }
}

fn global_name(names: Option<&dyn Names>, global: u32) -> Option<&str> {
    Some(names?.global(global)?.0)
}

fn global_type(names: Option<&dyn Names>, global: u32) -> Option<crate::Type> {
    let names = names?;
    names.types().to_type(names.global(global)?.1)
}

fn function_name(names: Option<&dyn Names>, function: u32) -> Option<&str> {
    names?.function(function)
}

fn field_name(names: Option<&dyn Names>, record: TypeId, field: u32) -> Option<&str> {
    match names?.types().get(record)? {
        TypeNode::Record(fields) => Some(index(fields, field)?.name.as_str()),
        _ => None,
    }
}

/// Why a program could not be loaded or run.
///
/// It is opaque: [`Display`](fmt::Display) gives its message, `{:#}` the
/// whole chain of what was being done, and [`labels`](Self::labels) each
/// part with the names of the program's values and functions and, with
/// [`DebugInfo`], where in the expression it is.
///
/// ```
/// use evalite::{Environment, Globals, Machine, Type};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
/// let program = evalite::compile("strength", &env)?;
///
/// let globals = Globals::new(&env);
/// let mut machine = Machine::default();
/// let error = machine.load(&program, &globals)?.run().unwrap_err();
///
/// assert_eq!(error.to_string(), "a value the program reads is not set");
/// assert_eq!(
///     format!("{error:#}"),
///     "a value the program reads is not set: while running instruction 0: while running the program",
/// );
///
/// let labels: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
/// assert_eq!(labels[0], "`strength` is not set, set it to an Int before running");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
pub struct RunError {
    inner: Box<Inner>,
}

struct Inner {
    /// The primary message, then context frames, innermost first.
    frames: Vec<Frame>,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl RunError {
    #[cold]
    #[inline(never)]
    pub(crate) fn new(message: Message) -> Self {
        let mut frames = Vec::with_capacity(4);
        frames.push(Frame {
            message,
            location: None,
        });

        Self {
            inner: Box::new(Inner {
                frames,
                source: None,
            }),
        }
    }

    /// With `source` as the cause, the caller's own error.
    #[cold]
    #[inline(never)]
    pub(crate) fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    /// The primary message.
    #[cfg(test)]
    pub(crate) fn message(&self) -> &Message {
        match self.inner.frames.first() {
            Some(frame) => &frame.message,
            None => &Message::Running,
        }
    }

    /// Whether the store refused for want of room to write in.
    pub(crate) fn is_too_large(&self) -> bool {
        matches!(
            self.inner.frames.first(),
            Some(Frame {
                message: Message::TooLarge { .. },
                ..
            })
        )
    }

    /// Whether the program is faulty, one evalite did not make.
    #[cfg(test)]
    pub(crate) fn is_fault(&self) -> bool {
        matches!(self.message(), Message::Fault(..))
    }

    /// Every frame, primary first.
    pub(crate) fn frames(&self) -> &[Frame] {
        &self.inner.frames
    }

    /// With `message` as the next frame out.
    #[cold]
    #[inline(never)]
    pub(crate) fn context(mut self, message: Message) -> Self {
        self.inner.frames.push(Frame {
            message,
            location: None,
        });
        self
    }

    /// With the outermost frame so far about `location`.
    #[cold]
    #[inline(never)]
    pub(crate) fn at(mut self, location: Location) -> Self {
        if let Some(frame) = self.inner.frames.last_mut() {
            frame.location = Some(location);
        }

        self
    }

    /// The primary message, then each context outward, as [`Label`]s
    /// naming the values and functions of `program`. With `debug`, the
    /// [`DebugInfo`] `program` was compiled with, a label about an
    /// instruction has the span of the expression it ran, and the primary
    /// label that of the innermost one.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Type, Value};
    ///
    /// let mut env = Environment::new();
    /// env.value("a", Type::int())?;
    /// let source = "1 + a / 0";
    /// let (program, debug) = evalite::compile_debug(source, &env, &evalite::Options::new())?;
    ///
    /// let mut globals = Globals::new(&env);
    /// globals.set("a", &Value::int(7))?;
    /// let mut machine = Machine::default();
    /// let error = machine.load(&program, &globals)?.run().unwrap_err();
    ///
    /// let labels = error.labels(&program, Some(&debug)).collect::<Vec<_>>();
    /// assert_eq!(labels.len(), 3);
    /// assert_eq!(labels[0].message().to_string(), "`7 / 0` divides by zero");
    /// assert_eq!(labels[0].span().map(|s| s.range()), Some(4..9));
    /// assert_eq!(labels[1].message().to_string(), "while running instruction 3");
    /// assert_eq!(labels[2].message().to_string(), "while running the program");
    /// assert_eq!(labels[2].span(), None);
    ///
    /// // Without debug info there are no spans.
    /// assert!(error.labels(&program, None).all(|l| l.span().is_none()));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn labels<'a>(&'a self, program: &'a Program, debug: Option<&'a DebugInfo>) -> Labels<'a> {
        let frames = self.inner.frames.as_slice();
        let innermost = frames
            .iter()
            .find(|frame| frame.location.is_some())
            .and_then(|frame| frame.span(debug));

        Labels {
            inner: LabelsInner::Run {
                frames: frames.iter(),
                program: Some(program),
                debug,
                innermost,
                first: true,
            },
        }
    }
}

impl fmt::Debug for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunError")
            .field("frames", &self.inner.frames)
            .field("source", &self.inner.source)
            .finish()
    }
}

impl fmt::Display for RunError {
    /// The primary message; with `{:#}`, every frame, joined by `: `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_frames(&self.inner.frames, f)
    }
}

/// The primary message of `frames`; with `{:#}`, every frame, joined by
/// `: `.
fn write_frames(frames: &[Frame], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let shown = if f.alternate() {
        frames
    } else {
        frames.get(..1).unwrap_or_default()
    };

    for (n, frame) in shown.iter().enumerate() {
        if n > 0 {
            f.write_str(": ")?;
        }

        fmt::Display::fmt(
            &Shown {
                message: &frame.message,
                names: None,
            },
            f,
        )?;
    }

    Ok(())
}

impl std::error::Error for RunError {
    /// The caller's own error, when one of its functions failed.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let source = self.inner.source.as_deref()?;
        Some(source)
    }
}

impl From<Fault> for RunError {
    #[cold]
    #[inline(never)]
    fn from(fault: Fault) -> Self {
        Self::new(Message::Fault(fault))
    }
}

impl From<Message> for RunError {
    #[cold]
    #[inline(never)]
    fn from(message: Message) -> Self {
        Self::new(message)
    }
}

/// Why a program, its [`DebugInfo`] or a [`Structure`](crate::Structure)
/// could not be stored or read back.
///
/// It is opaque like [`RunError`]: [`Display`](fmt::Display) gives its
/// message, `{:#}` the whole chain of what was being done, and
/// [`labels`](Self::labels) each part. When the bytes do not decode, the
/// decoder's own error is its [`source`](std::error::Error::source).
///
/// ```
/// use evalite::Program;
///
/// let error = Program::from_bytes(b"").unwrap_err();
/// assert_eq!(error.to_string(), "the bytes do not decode");
/// assert_eq!(format!("{error:#}"), "the bytes do not decode: while reading a stored program");
/// assert!(std::error::Error::source(&error).is_some());
/// ```
pub struct StoreError {
    inner: Box<Inner>,
}

impl StoreError {
    #[cold]
    #[inline(never)]
    pub(crate) fn new(message: Message) -> Self {
        let mut frames = Vec::with_capacity(4);
        frames.push(Frame {
            message,
            location: None,
        });

        Self {
            inner: Box::new(Inner {
                frames,
                source: None,
            }),
        }
    }

    /// With `source` as the cause, the encoder's or decoder's own error.
    #[cold]
    #[inline(never)]
    pub(crate) fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    /// With `message` as the next frame out.
    #[cold]
    #[inline(never)]
    pub(crate) fn context(mut self, message: Message) -> Self {
        self.inner.frames.push(Frame {
            message,
            location: None,
        });
        self
    }

    /// The primary message.
    #[cfg(test)]
    pub(crate) fn message(&self) -> &Message {
        match self.inner.frames.first() {
            Some(frame) => &frame.message,
            None => &Message::NotDecoded,
        }
    }

    /// The primary message, then each context outward, as [`Label`]s. They
    /// are about bytes rather than source text, so none has a span.
    ///
    /// ```
    /// use evalite::{Environment, Program};
    ///
    /// let program = evalite::compile("1 + 2", &Environment::new())?;
    /// let bytes = program.to_bytes()?;
    ///
    /// // Cut short, the bytes no longer decode.
    /// let error = Program::from_bytes(&bytes[..bytes.len() - 1]).unwrap_err();
    /// let labels: Vec<String> = error.labels().map(|l| l.message().to_string()).collect();
    /// assert_eq!(labels, ["the bytes do not decode", "while reading a stored program"]);
    /// assert!(error.labels().all(|l| l.span().is_none()));
    /// assert_eq!(error.labels().filter(|l| l.is_primary()).count(), 1);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn labels(&self) -> Labels<'_> {
        Labels {
            inner: LabelsInner::Run {
                frames: self.inner.frames.iter(),
                program: None,
                debug: None,
                innermost: None,
                first: true,
            },
        }
    }
}

impl fmt::Debug for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreError")
            .field("frames", &self.inner.frames)
            .field("source", &self.inner.source)
            .finish()
    }
}

impl fmt::Display for StoreError {
    /// The primary message; with `{:#}`, every frame, joined by `: `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_frames(&self.inner.frames, f)
    }
}

impl std::error::Error for StoreError {
    /// The encoder's or decoder's own error, when it failed.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let source = self.inner.source.as_deref()?;
        Some(source)
    }
}

impl From<Invalid> for StoreError {
    #[cold]
    #[inline(never)]
    fn from(invalid: Invalid) -> Self {
        Self::new(Message::Invalid(invalid))
    }
}

/// The `.context` pattern: decorate an error, on its way out of a layer,
/// with what that layer was doing.
pub(crate) trait Context<T> {
    /// Add `message` as the next frame out.
    fn context(self, message: Message) -> Result<T, RunError>;

    /// Add the message `f` gives as the next frame out, building it only
    /// on error.
    fn with_context(self, f: impl FnOnce() -> Message) -> Result<T, RunError>;

    /// Set where the outermost frame so far is about.
    fn at(self, location: Location) -> Result<T, RunError>;
}

impl<T, E> Context<T> for Result<T, E>
where
    E: Into<RunError>,
{
    #[inline]
    fn context(self, message: Message) -> Result<T, RunError> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(error.into().context(message)),
        }
    }

    #[inline]
    fn with_context(self, f: impl FnOnce() -> Message) -> Result<T, RunError> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(error.into().context(f())),
        }
    }

    #[inline]
    fn at(self, location: Location) -> Result<T, RunError> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(error.into().at(location)),
        }
    }
}

/// The [`Label`]s of an error, primary first.
///
/// ```
/// use evalite::{Globals, Machine, Limits};
///
/// let env = evalite::Environment::new();
/// let program = evalite::compile("[1, 2, 3][1]", &env)?;
/// let error = Machine::new(Limits::new().memory(1)).load(&program, &Globals::new(&env)).unwrap_err();
///
/// let labels = error.labels(&program, None);
/// assert_eq!(labels.len(), 2);
/// let primary = labels.filter(|l| l.is_primary()).count();
/// assert_eq!(primary, 1);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
pub struct Labels<'a> {
    inner: LabelsInner<'a>,
}

enum LabelsInner<'a> {
    Run {
        frames: std::slice::Iter<'a, Frame>,
        program: Option<&'a Program>,
        debug: Option<&'a DebugInfo>,
        /// The span of the innermost frame with one, which the primary
        /// label takes when it has none of its own.
        innermost: Option<Span<u32>>,
        first: bool,
    },
    Parse {
        error: &'a Diagnostic,
        /// The index of the next label.
        next: usize,
    },
    /// One label without a span, the error's own text.
    One(Option<&'a (dyn fmt::Display + 'a)>),
    /// Labels without spans, each a message shown already.
    Shown {
        frames: std::slice::Iter<'a, Box<str>>,
        first: bool,
    },
}

impl<'a> Labels<'a> {
    /// The labels of a parse or type error.
    pub(crate) fn parse(error: &'a Diagnostic) -> Self {
        Self {
            inner: LabelsInner::Parse { error, next: 0 },
        }
    }

    /// One primary label of `error`'s own text, without a span.
    pub(crate) fn one(error: &'a (dyn fmt::Display + 'a)) -> Self {
        Self {
            inner: LabelsInner::One(Some(error)),
        }
    }

    /// Labels of messages shown already, the primary first, without
    /// spans.
    pub(crate) fn shown(frames: &'a [Box<str>]) -> Self {
        Self {
            inner: LabelsInner::Shown {
                frames: frames.iter(),
                first: true,
            },
        }
    }
}

impl<'a> Iterator for Labels<'a> {
    type Item = Label<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            LabelsInner::Run {
                frames,
                program,
                debug,
                innermost,
                first,
            } => {
                let frame = frames.next()?;
                let primary = std::mem::replace(first, false);
                let mut span = frame.span(*debug);

                if primary {
                    span = span.or(*innermost);
                }

                Some(Label {
                    message: Text::Run(Shown {
                        message: &frame.message,
                        names: program.map(|p| p as &dyn Names),
                    }),
                    span,
                    primary,
                })
            }
            LabelsInner::Parse { error, next } => {
                let (message, span) = error.label(*next)?;
                let primary = *next == 0;
                *next = next.saturating_add(1);

                Some(Label {
                    message: Text::Parse(message),
                    span: Some(span),
                    primary,
                })
            }
            LabelsInner::One(error) => Some(Label {
                message: Text::Other(error.take()?),
                span: None,
                primary: true,
            }),
            LabelsInner::Shown { frames, first } => Some(Label {
                message: Text::Other(frames.next()?),
                span: None,
                primary: std::mem::replace(first, false),
            }),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.inner {
            LabelsInner::Run { frames, .. } => frames.size_hint(),
            LabelsInner::Parse { error, next } => {
                let n = error.label_count().saturating_sub(*next);
                (n, Some(n))
            }
            LabelsInner::One(error) => {
                let n = usize::from(error.is_some());
                (n, Some(n))
            }
            LabelsInner::Shown { frames, .. } => frames.size_hint(),
        }
    }
}

impl ExactSizeIterator for Labels<'_> {}

impl FusedIterator for Labels<'_> {}

/// One part of an error: a message, and where in the source it is about
/// when that is known.
pub struct Label<'a> {
    message: Text<'a>,
    span: Option<Span<u32>>,
    primary: bool,
}

impl<'a> Label<'a> {
    /// Where in the source this label is about, when known.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Options};
    ///
    /// let env = Environment::new();
    /// let (program, debug) = evalite::compile_debug("2 + -(9223372036854775807 * 2)", &env, &Options::new())?;
    /// let error = Machine::default().load(&program, &Globals::new(&env))?.run().unwrap_err();
    ///
    /// let primary = error.labels(&program, Some(&debug)).next().unwrap();
    /// assert_eq!(primary.span().map(|s| s.range()), Some(6..29));
    /// assert_eq!(error.labels(&program, None).next().unwrap().span(), None);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn span(&self) -> Option<Span<u32>> {
        self.span
    }

    /// What it says, formatted when shown.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Limits};
    ///
    /// let env = Environment::new();
    /// let program = evalite::compile("1 + 2", &env)?;
    /// let globals = Globals::new(&env);
    /// let error = Machine::new(Limits::new().fuel(1)).load(&program, &globals)?.run().unwrap_err();
    ///
    /// let messages: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
    /// assert_eq!(messages, [
    ///     "the expression took too long: it used all of its 1 step",
    ///     "while running instruction 1",
    ///     "while running the program",
    /// ]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn message(&self) -> impl fmt::Display + '_ {
        &self.message
    }

    /// Whether this is the error itself rather than a context of it: true
    /// of the first label only.
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Limits};
    ///
    /// let env = Environment::new();
    /// let program = evalite::compile("1 + 2", &env)?;
    /// let globals = Globals::new(&env);
    /// let error = Machine::new(Limits::new().fuel(1)).load(&program, &globals)?.run().unwrap_err();
    ///
    /// let primary: Vec<bool> = error.labels(&program, None).map(|l| l.is_primary()).collect();
    /// assert_eq!(primary, [true, false, false]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn is_primary(&self) -> bool {
        self.primary
    }
}

impl fmt::Debug for Label<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Label")
            .field("message", &self.message.to_string())
            .field("span", &self.span)
            .field("primary", &self.primary)
            .finish()
    }
}

/// What a label says.
enum Text<'a> {
    Run(Shown<'a>),
    Parse(ParseText<'a>),
    Other(&'a (dyn fmt::Display + 'a)),
}

impl fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(shown) => shown.fmt(f),
            Self::Parse(text) => text.fmt(f),
            Self::Other(error) => error.fmt(f),
        }
    }
}

/// Show `message` against `program`, for tests.
#[cfg(test)]
pub(crate) fn show(message: &Message, program: Option<&Program>) -> String {
    Shown {
        message,
        names: program.map(|p| p as &dyn Names),
    }
    .to_string()
}
