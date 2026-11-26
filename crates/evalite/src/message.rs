//! The error core every evalite error is built on: one [`Message`] enum
//! for every error and every context, frames of context with optional
//! locations, the `.context` pattern that adds them, and the public
//! [`Label`]s an error is inspected through.
//!
//! A message holds only plain data: numbers, and ids into the
//! [`Program`] that names are resolved against when it is shown. Nothing
//! here allocates or formats until an error is made or shown.

use std::fmt;
use std::iter::FusedIterator;

use syntree::Span;

use crate::debug::DebugInfo;
use crate::error::{ParseError, ParseText};
use crate::program::Program;
use crate::types::is_field_name;
use crate::types::{TypeId, TypeNode};

/// What went wrong, or what was being done when it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    /// The program needs more memory than the machine has.
    TooMuchMemory { needed: u64, allowed: u32 },
    /// A binding names a value the program does not read.
    UnknownValue,
    /// A value is bound twice.
    AlreadyBound { global: u32 },
    /// A value is not of its declared type: it is `found` instead, inside
    /// a `some(..)` when `in_some`.
    Mistyped {
        expected: TypeId,
        found: Found,
        in_some: bool,
    },
    /// A bound or returned value does not fit in the memory left.
    TooLarge { needed: u64, left: u64 },
    /// A value the program reads was not bound before it ran.
    Unbound { global: u32 },
    /// The run took every step it was allowed.
    OutOfFuel { fuel: u64 },
    /// Arithmetic past the range of an `Int`; `b` is unused for `Neg` and
    /// `Abs`.
    Overflow { op: Op, a: i64, b: i64 },
    /// `a / 0` or `a % 0`.
    DivideByZero { op: Op, a: i64 },
    /// `list[index]` past either end of the list.
    IndexOutOfRange { index: i64, len: u64 },
    /// `lookup(index, list)` of an empty list.
    EmptyLookup,
    /// `value.clamp(low, high)` with `low` above `high`.
    ClampBounds { low: i64, high: i64 },
    /// A function of the caller's failed; its own error is the source.
    HostFailed,
    /// The program is not one evalite made.
    Fault(Fault),
    /// Context: loading a program onto a machine.
    LoadingProgram,
    /// Context: binding a value the program reads.
    Binding { global: u32 },
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
}

/// What a mistyped value was found to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Found {
    Int,
    Bool,
    String,
    List,
    Record,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seg {
    /// Field `field` of the record type `record`.
    Field {
        record: TypeId,
        field: u32,
    },
    Index(u64),
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
    RecordLostFields,
    OptionLostValue,
    ReadAsSeveral,
    NotUtf8,
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
            Self::RecordLostFields => "a record lost fields",
            Self::OptionLostValue => "an option lost its value",
            Self::ReadAsSeveral => "a value read as several",
            Self::NotUtf8 => "a string is not UTF-8",
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

/// A message shown with the names `program` gives its ids, or without
/// names when there is no program.
pub(crate) struct Shown<'a> {
    message: &'a Message,
    program: Option<&'a Program>,
}

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.program;

        match self.message {
            Message::TooMuchMemory { needed, allowed } => write!(
                f,
                "the program needs {needed} words of memory, more than the {allowed} the machine allows"
            ),
            Message::UnknownValue => f.write_str("the program reads no value of that name"),
            Message::AlreadyBound { global } => match global_name(p, *global) {
                Some(name) => write!(f, "`{name}` is already bound"),
                None => f.write_str("the value is already bound"),
            },
            Message::Mistyped {
                expected,
                found,
                in_some,
            } => {
                match p.and_then(|p| p.types.to_type(*expected)) {
                    Some(ty) => write!(f, "expected {ty}, found ")?,
                    None => f.write_str("expected another type, found ")?,
                }

                if *in_some {
                    f.write_str("some(")?;
                }

                match found {
                    Found::Int => f.write_str("Int")?,
                    Found::Bool => f.write_str("Bool")?,
                    Found::String => f.write_str("String")?,
                    Found::List => f.write_str("a list")?,
                    Found::Record => f.write_str("a record")?,
                    Found::None => f.write_str("none")?,
                    Found::Some => f.write_str("some(..)")?,
                    Found::RecordWithout { record, field } => {
                        match field_name(p, *record, *field) {
                            Some(name) => write!(f, "a record without `{name}`")?,
                            None => f.write_str("a record without one of its fields")?,
                        }
                    }
                    Found::RecordWithExtra => f.write_str("a record with an extra field")?,
                }

                if *in_some {
                    f.write_str(")")?;
                }

                Ok(())
            }
            Message::TooLarge { needed, left } => write!(
                f,
                "the value needs {needed} words of memory, {left} are left"
            ),
            Message::Unbound { global } => match (global_name(p, *global), global_type(p, *global))
            {
                (Some(name), Some(ty)) => {
                    write!(f, "`{name}` is not bound, bind a {ty} before running")
                }
                _ => f.write_str("a value the program reads is not bound"),
            },
            Message::OutOfFuel { fuel } => write!(
                f,
                "the expression took too long: it used all of its {fuel} steps"
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
                let entries = if *len == 1 { "entry" } else { "entries" };
                write!(
                    f,
                    "index {index} is out of range of a list of {len} {entries}"
                )
            }
            Message::EmptyLookup => f.write_str("`lookup` of an empty list has no entry to give"),
            Message::ClampBounds { low, high } => write!(
                f,
                "`clamp` needs its low bound {low} to be at most its high bound {high}"
            ),
            Message::HostFailed => f.write_str("the function failed"),
            Message::Fault(fault) => write!(f, "the program is faulty: {}", fault.what()),
            Message::LoadingProgram => f.write_str("while loading the program"),
            Message::Binding { global } => match global_name(p, *global) {
                Some(name) => write!(f, "while binding `{name}`"),
                None => f.write_str("while binding a value"),
            },
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

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}

fn global_name(program: Option<&Program>, global: u32) -> Option<&str> {
    Some(index(&program?.globals, global)?.name.as_str())
}

fn global_type(program: Option<&Program>, global: u32) -> Option<crate::Type> {
    let program = program?;
    program.types.to_type(index(&program.globals, global)?.ty)
}

fn function_name(program: Option<&Program>, function: u32) -> Option<&str> {
    Some(index(&program?.functions, function)?.name.as_str())
}

fn field_name(program: Option<&Program>, record: TypeId, field: u32) -> Option<&str> {
    match program?.types.get(record)? {
        TypeNode::Record(fields) => Some(index(fields, field)?.name.as_str()),
        _ => None,
    }
}

/// Why a program could not be loaded, bound or run.
///
/// It is opaque: [`Display`](fmt::Display) gives its message, `{:#}` the
/// whole chain of what was being done, and [`labels`](Self::labels) each
/// part with the names of the program's values and functions and, with
/// [`DebugInfo`], where in the expression it is.
///
/// ```
/// use evalite::{Environment, Machine, Type, Value};
///
/// let mut env = Environment::with_builtins();
/// env.value("strength", Type::int())?;
/// let program = evalite::compile("strength", &env)?;
///
/// let mut machine = Machine::default();
/// let mut run = machine.load(&program)?;
/// let error = run.bind("strength", Value::bool(true)).unwrap_err();
///
/// assert_eq!(error.to_string(), "expected another type, found Bool");
/// assert_eq!(format!("{error:#}"), "expected another type, found Bool: while binding a value");
///
/// let labels: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
/// assert_eq!(labels, ["expected Int, found Bool", "while binding `strength`"]);
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

    /// Whether the program is faulty, one evalite did not make.
    #[cfg(test)]
    pub(crate) fn is_fault(&self) -> bool {
        matches!(self.message(), Message::Fault(..))
    }

    /// Every frame, primary first.
    #[cfg(test)]
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
    /// use evalite::{Environment, Machine, Type, Value};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.value("a", Type::int())?;
    /// let source = "1 + a / 0";
    /// let (program, debug) = evalite::compile_debug(source, &env, &evalite::Options::new())?;
    ///
    /// let mut machine = Machine::default();
    /// let mut run = machine.load(&program)?;
    /// run.bind("a", Value::int(7))?;
    /// let error = run.run().unwrap_err();
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
        let frames = self.inner.frames.as_slice();
        let shown = if f.alternate() {
            frames
        } else {
            frames.get(..1).unwrap_or_default()
        };

        for (n, frame) in shown.iter().enumerate() {
            if n > 0 {
                f.write_str(": ")?;
            }

            Shown {
                message: &frame.message,
                program: None,
            }
            .fmt(f)?;
        }

        Ok(())
    }
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
/// use evalite::{Machine, Limits};
///
/// let env = evalite::Environment::with_builtins();
/// let program = evalite::compile("[1, 2, 3][1]", &env)?;
/// let error = Machine::new(Limits::new().memory(1)).load(&program).unwrap_err();
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
        error: &'a ParseError,
        /// The index of the next label.
        next: usize,
    },
}

impl<'a> Labels<'a> {
    /// The labels of a parse or type error.
    pub(crate) fn parse(error: &'a ParseError) -> Self {
        Self {
            inner: LabelsInner::Parse { error, next: 0 },
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
                        program: *program,
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
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.inner {
            LabelsInner::Run { frames, .. } => frames.size_hint(),
            LabelsInner::Parse { error, next } => {
                let n = error.label_count().saturating_sub(*next);
                (n, Some(n))
            }
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
    /// use evalite::{Environment, Machine, Options};
    ///
    /// let env = Environment::with_builtins();
    /// let (program, debug) = evalite::compile_debug("2 + -(9223372036854775807 * 2)", &env, &Options::new())?;
    /// let error = Machine::default().load(&program)?.run().unwrap_err();
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
    /// use evalite::{Environment, Machine, Limits};
    ///
    /// let env = Environment::with_builtins();
    /// let program = evalite::compile("1 + 2", &env)?;
    /// let error = Machine::new(Limits::new().fuel(1)).load(&program)?.run().unwrap_err();
    ///
    /// let messages: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
    /// assert_eq!(messages, [
    ///     "the expression took too long: it used all of its 1 steps",
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
    /// use evalite::{Environment, Machine, Limits};
    ///
    /// let env = Environment::with_builtins();
    /// let program = evalite::compile("1 + 2", &env)?;
    /// let error = Machine::new(Limits::new().fuel(1)).load(&program)?.run().unwrap_err();
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
}

impl fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(shown) => shown.fmt(f),
            Self::Parse(text) => text.fmt(f),
        }
    }
}

/// Show `message` against `program`, for tests.
#[cfg(test)]
pub(crate) fn show(message: &Message, program: Option<&Program>) -> String {
    Shown { message, program }.to_string()
}
