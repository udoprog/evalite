//! The machine that runs a [`Program`]: a fixed amount of memory reserved
//! when it is made, values bound and type-checked before it runs, and a
//! step limit on how long it runs.

use std::collections::BTreeMap;
use std::fmt;

use crate::Type;
use crate::program::{Constant, Instr, Program, Scalar};
use crate::types::{TypeId, TypeNode};

/// The memory a [`Machine`] has unless [`Limits::memory`] says otherwise,
/// in slots of one value each.
pub const DEFAULT_MEMORY: u32 = 1024;

/// The steps a run may take unless [`Limits::fuel`] says otherwise.
pub const DEFAULT_FUEL: u64 = 1000;

/// What a [`Machine`] allows a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    memory: u32,
    fuel: u64,
}

impl Limits {
    /// [`DEFAULT_MEMORY`] slots and [`DEFAULT_FUEL`] steps.
    pub const fn new() -> Self {
        Self {
            memory: DEFAULT_MEMORY,
            fuel: DEFAULT_FUEL,
        }
    }

    /// Allow `slots` slots of memory: a program's stack, lists and
    /// constants, one slot per value it reads, and the parts of the values
    /// bound to it and returned by the caller's functions.
    pub const fn memory(mut self, slots: u32) -> Self {
        self.memory = slots;
        self
    }

    /// Allow `steps` steps per run.
    pub const fn fuel(mut self, steps: u64) -> Self {
        self.fuel = steps;
        self
    }

    /// The memory allowed, in slots.
    pub const fn memory_slots(&self) -> u32 {
        self.memory
    }

    /// The steps allowed per run.
    pub const fn fuel_steps(&self) -> u64 {
        self.fuel
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new()
    }
}

/// A value bound to a program or given back by one.
///
/// An `Option<T>` is [`Value::None`] or [`Value::Some`] of a `T`, never a
/// bare `T`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
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
    /// A value of the caller's opaque type `ty`, which the caller's
    /// functions know by `handle`.
    Opaque {
        ty: String,
        handle: u64,
    },
}

impl Value {
    /// A present option.
    pub fn some(value: Value) -> Self {
        Self::Some(Box::new(value))
    }

    /// A record of these fields.
    pub fn record<K>(fields: impl IntoIterator<Item = (K, Value)>) -> Self
    where
        K: Into<String>,
    {
        Self::Record(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// A value of the caller's opaque type.
    pub fn opaque(ty: impl Into<String>, handle: u64) -> Self {
        Self::Opaque {
            ty: ty.into(),
            handle,
        }
    }

    /// What it is, as an error names it.
    fn describe(&self) -> String {
        match self {
            Self::Int(_) => "Int".to_owned(),
            Self::Bool(_) => "Bool".to_owned(),
            Self::String(_) => "String".to_owned(),
            Self::List(_) => "a list".to_owned(),
            Self::Record(_) => "a record".to_owned(),
            Self::None => "none".to_owned(),
            Self::Some(_) => "some(..)".to_owned(),
            Self::Opaque { ty, .. } => ty.clone(),
        }
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

enum Piece<'a> {
    Value(&'a Value),
    Text(&'static str),
    Key(&'a str),
}

impl fmt::Display for Value {
    /// Values as shex writes them: `3`, `true`, `"Brawl"`, `[1, 2]`,
    /// `{athletics: 2, "Brawl (bar)": 1}`, `none`, `some(2)`, and an opaque
    /// value as `<Track 3>`.
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
                Piece::Value(value) => match value {
                    Value::Int(n) => write!(f, "{n}")?,
                    Value::Bool(b) => write!(f, "{b}")?,
                    Value::String(s) => write_string(f, s)?,
                    Value::None => f.write_str("none")?,
                    Value::Opaque { ty, handle } => write!(f, "<{ty} {handle}>")?,
                    Value::Some(value) => {
                        f.write_str("some(")?;
                        stack.push(Piece::Text(")"));
                        stack.push(Piece::Value(value));
                    }
                    Value::List(items) => {
                        f.write_str("[")?;
                        stack.push(Piece::Text("]"));

                        for (n, item) in items.iter().enumerate().rev() {
                            stack.push(Piece::Value(item));

                            if n > 0 {
                                stack.push(Piece::Text(", "));
                            }
                        }
                    }
                    Value::Record(fields) => {
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

fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();

    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why a program could not be loaded, bound or run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// The program needs more memory than the machine has.
    TooMuchMemory { needed: u64, allowed: u32 },
    /// A binding names a value the program does not read.
    UnknownValue { name: String },
    /// A value is bound twice.
    AlreadyBound { name: String },
    /// A bound value is not of its declared type: the part at `path`, such
    /// as `stats.wits` or `health[2]`, is `found` instead.
    Mistyped {
        path: String,
        expected: Type,
        found: String,
    },
    /// A bound value does not fit in the memory left.
    BindingTooLarge {
        name: String,
        needed: u64,
        left: u64,
    },
    /// A value the program reads was not bound before it ran.
    Unbound { name: String, ty: Type },
    /// The run took every step it was allowed.
    OutOfFuel { fuel: u64 },
    /// Arithmetic past the range of an `Int`, such as `9223372036854775807 + 1`.
    Overflow { expr: String },
    /// `a / 0` or `a % 0`.
    DivideByZero { expr: String },
    /// `list[index]` past either end of the list.
    IndexOutOfRange { index: i64, len: usize },
    /// `lookup(index, list)` of an empty list.
    EmptyLookup,
    /// `value.clamp(low, high)` with `low` above `high`.
    ClampBounds { low: i64, high: i64 },
    /// A function of the caller's failed with `message`.
    Host { name: String, message: String },
    /// A function of the caller's returned a value of another type than it
    /// declares; `path` is the mismatched part, empty for the whole value.
    HostResult {
        name: String,
        path: String,
        expected: Type,
        found: String,
    },
    /// A function of the caller's returned a value too large for the
    /// memory left.
    HostResultTooLarge {
        name: String,
        needed: u64,
        left: u64,
    },
    /// The program is not one shex made. Programs from
    /// [`compile`](crate::compile) and [`Program::from_bytes`] never give
    /// it.
    Fault(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMuchMemory { needed, allowed } => write!(
                f,
                "the program needs {needed} slots of memory, more than the {allowed} the machine allows"
            ),
            Self::UnknownValue { name } => write!(f, "the program reads no value `{name}`"),
            Self::AlreadyBound { name } => write!(f, "`{name}` is already bound"),
            Self::Mistyped {
                path,
                expected,
                found,
            } => write!(f, "`{path}` must be {expected}, found {found}"),
            Self::BindingTooLarge { name, needed, left } => write!(
                f,
                "binding `{name}` needs {needed} slots of memory, {left} are left"
            ),
            Self::Unbound { name, ty } => {
                write!(f, "`{name}` is not bound, bind a {ty} before running")
            }
            Self::OutOfFuel { fuel } => write!(
                f,
                "the expression took too long: it used all of its {fuel} steps"
            ),
            Self::Overflow { expr } => write!(f, "`{expr}` overflows an Int"),
            Self::DivideByZero { expr } => write!(f, "`{expr}` divides by zero"),
            Self::IndexOutOfRange { index, len } => {
                let entries = if *len == 1 { "entry" } else { "entries" };
                write!(
                    f,
                    "index {index} is out of range of a list of {len} {entries}"
                )
            }
            Self::EmptyLookup => f.write_str("`lookup` of an empty list has no entry to give"),
            Self::ClampBounds { low, high } => write!(
                f,
                "`clamp` needs its low bound {low} to be at most its high bound {high}"
            ),
            Self::Host { name, message } => write!(f, "`{name}` failed: {message}"),
            Self::HostResult {
                name,
                path,
                expected,
                found,
            } => {
                if path.is_empty() {
                    write!(f, "`{name}` must return {expected}, returned {found}")
                } else {
                    write!(
                        f,
                        "`{name}` returned a value whose `{path}` must be {expected}, found {found}"
                    )
                }
            }
            Self::HostResultTooLarge { name, needed, left } => write!(
                f,
                "the value `{name}` returned needs {needed} slots of memory, {left} are left"
            ),
            Self::Fault(what) => write!(f, "the program is faulty: {what}"),
        }
    }
}

impl std::error::Error for RunError {}

fn fault(what: &str) -> RunError {
    RunError::Fault(what.to_owned())
}

/// The caller's functions, which a program calls by the names and types
/// its environment declared. Each call gets its arguments in order and
/// gives a value of the declared return type, or an error message.
///
/// A closure `FnMut(&str, Vec<Value>) -> Result<Value, String>` is a host.
pub trait Host {
    fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String>;
}

impl<F> Host for F
where
    F: FnMut(&str, Vec<Value>) -> Result<Value, String>,
{
    fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        self(name, args)
    }
}

/// A host with no functions, for programs that call none.
struct NoHost;

impl Host for NoHost {
    fn call(&mut self, _: &str, _: Vec<Value>) -> Result<Value, String> {
        Err("no functions are provided".to_owned())
    }
}

/// What a run gave back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The program's value, of its [`result_type`](Program::result_type).
    pub value: Value,
    /// The steps it took.
    pub steps: u64,
    /// The most memory it used at once, in slots, bindings included.
    pub memory: u64,
}

/// One machine value. Every value takes one slot; strings, lists and
/// records refer to their parts elsewhere in the machine or in the
/// program's constants. An `Option<T>` is `None` or the `T` itself, since
/// options do not nest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Unbound,
    Int(i64),
    Bool(bool),
    None,
    /// A constant string or list of the program's.
    Const(u32),
    /// A string of `len` bytes, packed 8 to a slot from `at`.
    Str {
        at: u32,
        len: u32,
    },
    List {
        at: u32,
        len: u32,
    },
    /// A record's fields in name order.
    Record {
        at: u32,
        len: u32,
    },
    Opaque(u64),
    /// 8 bytes of a string.
    Bytes([u8; 8]),
}

/// A machine with a fixed memory, reserved when it is made, which runs
/// programs one at a time.
///
/// ```
/// use shex::{Environment, Limits, Machine, Type, Value};
///
/// let mut env = Environment::with_builtins();
/// env.value("strength", Type::Int)?;
/// env.value("bonus", Type::option(Type::Int))?;
///
/// let program = shex::compile("strength + bonus.unwrap_or(0)", &env)?;
///
/// let mut machine = Machine::new(Limits::new());
/// let mut run = machine.load(&program)?;
/// run.bind("strength", Value::Int(3))?;
/// run.bind("bonus", Value::some(Value::Int(2)))?;
/// assert_eq!(run.run()?.value, Value::Int(5));
///
/// let mut run = machine.load(&program)?;
/// let error = run.bind("strength", Value::Bool(true)).unwrap_err();
/// assert_eq!(error.to_string(), "`strength` must be Int, found Bool");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Machine {
    limits: Limits,
    slots: Box<[Slot]>,
}

impl Machine {
    /// A machine with `limits`, reserving all of its memory now.
    pub fn new(limits: Limits) -> Self {
        let len = usize::try_from(limits.memory).unwrap_or(usize::MAX);

        Self {
            limits,
            slots: vec![Slot::Unbound; len].into_boxed_slice(),
        }
    }

    /// What it allows.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Get ready to run `program`, refusing it when its memory, with a
    /// slot per value it reads, is more than the machine has.
    pub fn load<'a>(&'a mut self, program: &'a Program) -> Result<Run<'a>, RunError> {
        let memory = program.memory();
        let globals = program.globals.len();

        let needed = memory
            .total()
            .saturating_add(u64::try_from(globals).unwrap_or(u64::MAX));

        if needed > u64::from(self.limits.memory) {
            return Err(RunError::TooMuchMemory {
                needed,
                allowed: self.limits.memory,
            });
        }

        let size = |n: u32| usize::try_from(n).map_err(|_| fault("memory past the address space"));
        let stack_cap = size(memory.stack)?;
        let stack_base = self
            .slots
            .len()
            .checked_sub(stack_cap)
            .ok_or_else(|| fault("a stack larger than the machine"))?;
        let limit = stack_base
            .checked_sub(size(memory.constants)?)
            .ok_or_else(|| fault("constants larger than the machine"))?;

        for slot in self
            .slots
            .get_mut(..globals)
            .ok_or_else(|| fault("more values than the machine has slots"))?
        {
            *slot = Slot::Unbound;
        }

        Ok(Run {
            slots: &mut self.slots,
            program,
            fuel: self.limits.fuel,
            left: self.limits.fuel,
            len: globals,
            limit,
            heap: size(memory.heap)?,
            stack_base,
            stack_cap,
            sp: 0,
            peak_len: globals,
            peak_sp: 0,
        })
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new(Limits::new())
    }
}

/// A program loaded on a [`Machine`], taking its bindings.
#[derive(Debug)]
pub struct Run<'a> {
    slots: &'a mut [Slot],
    program: &'a Program,
    fuel: u64,
    left: u64,
    /// Slots `..len` hold the values read, then the parts of bound values,
    /// lists and returned values, up to `limit`.
    len: usize,
    limit: usize,
    /// The slots kept for the lists the program has yet to build, which
    /// bound and returned values may not take.
    heap: usize,
    /// The stack is the `stack_cap` slots from `stack_base`, `sp` deep.
    stack_base: usize,
    stack_cap: usize,
    sp: usize,
    peak_len: usize,
    peak_sp: usize,
}

#[derive(Clone, Copy)]
enum Seg<'v> {
    Root,
    Field(&'v str),
    Index(usize),
}

enum Read {
    Visit(Slot, TypeId),
    List(usize),
    Record(TypeId),
    Some,
}

/// Why a value does not fit its type.
struct Mismatch {
    path: String,
    expected: Type,
    found: String,
}

impl<'a> Run<'a> {
    /// The values the program reads, with their types, each to be bound.
    pub fn globals(&self) -> Vec<(&'a str, Type)> {
        self.program.globals()
    }

    /// Bind `value` to the value `name` the program reads, checking it is
    /// of its declared type and fits in the memory left.
    pub fn bind(&mut self, name: &str, value: Value) -> Result<(), RunError> {
        let program = self.program;

        let (at, global) = program
            .globals
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == name)
            .ok_or_else(|| RunError::UnknownValue {
                name: name.to_owned(),
            })?;

        if self.slots.get(at) != Some(&Slot::Unbound) {
            return Err(RunError::AlreadyBound {
                name: name.to_owned(),
            });
        }

        let needed = self
            .measure(&value, global.ty, name)?
            .map_err(|m| RunError::Mistyped {
                path: m.path,
                expected: m.expected,
                found: m.found,
            })?;

        let left = self.left();

        if needed > left {
            return Err(RunError::BindingTooLarge {
                name: name.to_owned(),
                needed: count(needed),
                left: count(left),
            });
        }

        let slot = self.write(&value, global.ty)?;
        *self
            .slots
            .get_mut(at)
            .ok_or_else(|| fault("a value outside memory"))? = slot;
        Ok(())
    }

    /// Run the program, which calls none of the caller's functions.
    pub fn run(self) -> Result<Outcome, RunError> {
        self.run_with(&mut NoHost)
    }

    /// Run the program, calling `host` for the caller's functions.
    pub fn run_with(mut self, host: &mut dyn Host) -> Result<Outcome, RunError> {
        let program = self.program;

        for (at, global) in program.globals.iter().enumerate() {
            if self.slots.get(at) == Some(&Slot::Unbound) {
                return Err(RunError::Unbound {
                    name: global.name.clone(),
                    ty: program.type_of(global.ty),
                });
            }
        }

        let code = program.code.as_slice();
        let mut pc = 0usize;

        while let Some(instr) = code.get(pc) {
            self.spend(1)?;
            pc = self.step(*instr, pc, host)?;
        }

        let result = self.pop()?;

        if self.sp != 0 {
            return Err(fault("values are left on the stack"));
        }

        let value = self.read(result, program.result)?;

        Ok(Outcome {
            value,
            steps: self.fuel.saturating_sub(self.left),
            memory: count(self.peak_len)
                .saturating_add(count(self.peak_sp))
                .saturating_add(u64::from(program.memory().constants)),
        })
    }

    /// Run one instruction at `pc`, giving the next.
    fn step(&mut self, instr: Instr, pc: usize, host: &mut dyn Host) -> Result<usize, RunError> {
        let next = pc.checked_add(1).ok_or_else(|| fault("code too long"))?;

        let push = match instr {
            Instr::Int(n) => Slot::Int(n),
            Instr::Bool(b) => Slot::Bool(b),
            Instr::Const(c) => Slot::Const(c),
            Instr::Global(g) => {
                index(&self.program.globals, g).ok_or_else(|| fault("no such value"))?;
                *index(self.slots, g).ok_or_else(|| fault("no such value"))?
            }
            Instr::Neg => {
                let a = self.pop_int()?;
                Slot::Int(a.checked_neg().ok_or_else(|| overflow(format!("-({a})")))?)
            }
            Instr::Abs => {
                let a = self.pop_int()?;
                Slot::Int(
                    a.checked_abs()
                        .ok_or_else(|| overflow(format!("({a}).abs()")))?,
                )
            }
            Instr::Not => Slot::Bool(!self.pop_bool()?),
            Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Rem => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;
                Slot::Int(arithmetic(instr, a, b)?)
            }
            Instr::Lt | Instr::Le | Instr::Gt | Instr::Ge => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;

                Slot::Bool(match instr {
                    Instr::Lt => a < b,
                    Instr::Le => a <= b,
                    Instr::Gt => a > b,
                    _ => a >= b,
                })
            }
            Instr::Eq | Instr::Ne => {
                let b = self.pop()?;
                let a = self.pop()?;

                let equal = match (a, b) {
                    (Slot::Int(a), Slot::Int(b)) => a == b,
                    (Slot::Bool(a), Slot::Bool(b)) => a == b,
                    _ => return Err(fault("compares other than two Ints or two Bools")),
                };

                Slot::Bool(equal == matches!(instr, Instr::Eq))
            }
            Instr::And(skip) | Instr::Or(skip) => {
                let top = self.pop_bool()?;
                let decides = top == matches!(instr, Instr::Or(..));

                if !decides {
                    return Ok(next);
                }

                self.push(Slot::Bool(top))?;

                return usize::try_from(skip)
                    .ok()
                    .and_then(|skip| next.checked_add(skip))
                    .ok_or_else(|| fault("jumps past the end"));
            }
            Instr::Field(n) => {
                let Slot::Record { at, len } = self.pop()? else {
                    return Err(fault("reads a field of other than a record"));
                };

                if n >= len {
                    return Err(fault("reads a field the record does not have"));
                }

                self.slot(at, n)?
            }
            Instr::Index => {
                let i = self.pop_int()?;
                let list = self.pop()?;
                let len = self.len_of(list)?;

                let at = usize::try_from(i)
                    .ok()
                    .filter(|at| *at < len)
                    .ok_or(RunError::IndexOutOfRange { index: i, len })?;

                self.entry(list, at)?
            }
            Instr::Lookup => {
                let list = self.pop()?;
                let i = self.pop_int()?;
                let len = self.len_of(list)?;
                let last = len.checked_sub(1).ok_or(RunError::EmptyLookup)?;
                let at = usize::try_from(i.max(0)).unwrap_or(usize::MAX).min(last);
                self.entry(list, at)?
            }
            Instr::List(n) => {
                let len = size(n)?;
                self.spend(len)?;

                let top = self
                    .sp
                    .checked_sub(len)
                    .ok_or_else(|| fault("makes a list of more entries than the stack has"))?;
                let from = self
                    .stack_base
                    .checked_add(top)
                    .ok_or_else(|| fault("a stack outside memory"))?;
                let end = from
                    .checked_add(len)
                    .filter(|end| *end <= self.slots.len())
                    .ok_or_else(|| fault("a stack outside memory"))?;
                self.heap = self
                    .heap
                    .checked_sub(len)
                    .ok_or_else(|| fault("builds more lists than it declares"))?;
                let at = self.alloc(len)?;

                self.slots.copy_within(from..end, at);
                self.sp = top;

                Slot::List {
                    at: slot_index(at)?,
                    len: n,
                }
            }
            Instr::Min(n) | Instr::Max(n) => {
                self.spend(size(n)?)?;
                let mut best = self.pop_int()?;

                for _ in 1..n {
                    let a = self.pop_int()?;

                    best = if matches!(instr, Instr::Min(..)) {
                        best.min(a)
                    } else {
                        best.max(a)
                    };
                }

                Slot::Int(best)
            }
            Instr::Clamp => {
                let high = self.pop_int()?;
                let low = self.pop_int()?;
                let value = self.pop_int()?;

                if low > high {
                    return Err(RunError::ClampBounds { low, high });
                }

                Slot::Int(value.clamp(low, high))
            }
            Instr::UnwrapOr => {
                let default = self.pop()?;
                let option = self.pop()?;

                if option == Slot::None {
                    default
                } else {
                    option
                }
            }
            Instr::IsSome | Instr::IsNone => {
                let none = self.pop()? == Slot::None;
                Slot::Bool(none == matches!(instr, Instr::IsNone))
            }
            Instr::Len => {
                let list = self.pop()?;
                let len = self.len_of(list)?;
                Slot::Int(i64::try_from(len).map_err(|_| fault("a list too long to count"))?)
            }
            Instr::Call(f) => self.call(f, host)?,
        };

        self.push(push)?;
        Ok(next)
    }

    fn call(&mut self, f: u32, host: &mut dyn Host) -> Result<Slot, RunError> {
        let program = self.program;
        let function = index(&program.functions, f).ok_or_else(|| fault("no such function"))?;

        let top = self
            .sp
            .checked_sub(function.params.len())
            .ok_or_else(|| fault("calls with fewer arguments than the stack has"))?;

        let mut args = Vec::with_capacity(function.params.len());

        for (n, ty) in function.params.iter().enumerate() {
            let at = self
                .stack_base
                .checked_add(top)
                .and_then(|at| at.checked_add(n))
                .ok_or_else(|| fault("a stack outside memory"))?;
            let slot = *self
                .slots
                .get(at)
                .ok_or_else(|| fault("a stack outside memory"))?;
            args.push(self.read(slot, *ty)?);
        }

        self.sp = top;

        let value = host
            .call(&function.name, args)
            .map_err(|message| RunError::Host {
                name: function.name.clone(),
                message,
            })?;

        let needed = self
            .measure(&value, function.ret, "")?
            .map_err(|m| RunError::HostResult {
                name: function.name.clone(),
                path: m.path,
                expected: m.expected,
                found: m.found,
            })?;

        self.spend(needed)?;
        let left = self.left();

        if needed > left {
            return Err(RunError::HostResultTooLarge {
                name: function.name.clone(),
                needed: count(needed),
                left: count(left),
            });
        }

        self.write(&value, function.ret)
    }

    /// The slots left for bound and returned values.
    fn left(&self) -> usize {
        self.limit
            .saturating_sub(self.len)
            .saturating_sub(self.heap)
    }

    fn spend(&mut self, steps: usize) -> Result<(), RunError> {
        self.left = self
            .left
            .checked_sub(count(steps))
            .ok_or(RunError::OutOfFuel { fuel: self.fuel })?;
        Ok(())
    }

    fn push(&mut self, slot: Slot) -> Result<(), RunError> {
        if self.sp >= self.stack_cap {
            return Err(fault("the stack grows past the depth it declares"));
        }

        let at = self
            .stack_base
            .checked_add(self.sp)
            .ok_or_else(|| fault("a stack outside memory"))?;
        *self
            .slots
            .get_mut(at)
            .ok_or_else(|| fault("a stack outside memory"))? = slot;
        self.sp = self.sp.saturating_add(1);
        self.peak_sp = self.peak_sp.max(self.sp);
        Ok(())
    }

    fn pop(&mut self) -> Result<Slot, RunError> {
        let sp = self
            .sp
            .checked_sub(1)
            .ok_or_else(|| fault("pops an empty stack"))?;
        let at = self
            .stack_base
            .checked_add(sp)
            .ok_or_else(|| fault("a stack outside memory"))?;
        let slot = *self
            .slots
            .get(at)
            .ok_or_else(|| fault("a stack outside memory"))?;
        self.sp = sp;
        Ok(slot)
    }

    fn pop_int(&mut self) -> Result<i64, RunError> {
        match self.pop()? {
            Slot::Int(n) => Ok(n),
            _ => Err(fault("finds other than an Int")),
        }
    }

    fn pop_bool(&mut self) -> Result<bool, RunError> {
        match self.pop()? {
            Slot::Bool(b) => Ok(b),
            _ => Err(fault("finds other than a Bool")),
        }
    }

    /// Take `n` slots of memory.
    fn alloc(&mut self, n: usize) -> Result<usize, RunError> {
        let at = self.len;
        let len = at
            .checked_add(n)
            .filter(|len| *len <= self.limit)
            .ok_or_else(|| fault("uses more memory than it declares"))?;
        self.len = len;
        self.peak_len = self.peak_len.max(len);
        Ok(at)
    }

    fn slot(&self, at: u32, n: u32) -> Result<Slot, RunError> {
        at.checked_add(n)
            .and_then(|at| index(self.slots, at))
            .copied()
            .ok_or_else(|| fault("a value outside memory"))
    }

    fn constant(&self, c: u32) -> Result<&'a Constant, RunError> {
        let program = self.program;
        index(&program.constants, c).ok_or_else(|| fault("no such constant"))
    }

    fn len_of(&self, list: Slot) -> Result<usize, RunError> {
        match list {
            Slot::List { len, .. } => size(len),
            Slot::Const(c) => match self.constant(c)? {
                Constant::List { items, .. } => Ok(items.len()),
                Constant::String(_) => Err(fault("a string used as a list")),
            },
            _ => Err(fault("finds other than a list")),
        }
    }

    fn entry(&self, list: Slot, at: usize) -> Result<Slot, RunError> {
        match list {
            Slot::List { at: start, .. } => self.slot(start, slot_index(at)?),
            Slot::Const(c) => match self.constant(c)? {
                Constant::List { items, .. } => items
                    .get(at)
                    .map(scalar)
                    .ok_or_else(|| fault("an entry past the end")),
                Constant::String(_) => Err(fault("a string used as a list")),
            },
            _ => Err(fault("finds other than a list")),
        }
    }

    /// Check `value` is of type `ty`, giving the slots its parts take.
    fn measure(
        &self,
        value: &Value,
        ty: TypeId,
        root: &str,
    ) -> Result<Result<usize, Mismatch>, RunError> {
        let program = self.program;
        let mut nodes = vec![(0usize, Seg::Root)];
        let mut work = vec![(value, ty, 0usize, None)];
        let mut total = 0usize;

        while let Some((value, ty, node, option)) = work.pop() {
            let ty_node = program.types.get(ty).ok_or_else(|| fault("no such type"))?;

            let found = match (ty_node, value) {
                (TypeNode::Option(_), Value::None)
                | (TypeNode::Int, Value::Int(_))
                | (TypeNode::Bool, Value::Bool(_)) => continue,
                (TypeNode::Option(inner), Value::Some(value)) => {
                    work.push((value, *inner, node, Some(ty)));
                    continue;
                }
                (TypeNode::Opaque(name), Value::Opaque { ty, .. }) if name == ty => continue,
                (TypeNode::String, Value::String(s)) => {
                    total = total.saturating_add(s.len().div_ceil(8));
                    continue;
                }
                (TypeNode::List(entry), Value::List(items)) => {
                    total = total.saturating_add(items.len());

                    for (i, item) in items.iter().enumerate() {
                        work.push((item, *entry, nodes.len(), None));
                        nodes.push((node, Seg::Index(i)));
                    }

                    continue;
                }
                (TypeNode::Record(fields), Value::Record(map)) => {
                    let missing = fields.iter().find(|f| !map.contains_key(&f.name));
                    let extra = map.keys().find(|k| !fields.iter().any(|f| f.name == **k));

                    match (missing, extra) {
                        (Some(field), _) => format!("a record without `{}`", field.name),
                        (None, Some(key)) => format!("a record with an extra field `{key}`"),
                        (None, None) => {
                            total = total.saturating_add(fields.len());

                            for field in fields {
                                if let Some(value) = map.get(&field.name) {
                                    work.push((value, field.ty, nodes.len(), None));
                                    nodes.push((node, Seg::Field(&field.name)));
                                }
                            }

                            continue;
                        }
                    }
                }
                _ => value.describe(),
            };

            return Ok(Err(Mismatch {
                path: path(root, &nodes, node),
                expected: program.type_of(option.unwrap_or(ty)),
                found: match option {
                    Some(_) => format!("some({found})"),
                    None => found,
                },
            }));
        }

        Ok(Ok(total))
    }

    /// Write `value`, already measured against `ty`, into memory, giving
    /// its slot.
    fn write(&mut self, value: &Value, ty: TypeId) -> Result<Slot, RunError> {
        let mut work = Vec::new();
        let root = self.place(value, ty, &mut work)?;

        while let Some((value, ty, dest)) = work.pop() {
            let slot = self.place(value, ty, &mut work)?;
            *self
                .slots
                .get_mut(dest)
                .ok_or_else(|| fault("a value outside memory"))? = slot;
        }

        Ok(root)
    }

    fn place<'v>(
        &mut self,
        mut value: &'v Value,
        mut ty: TypeId,
        work: &mut Vec<(&'v Value, TypeId, usize)>,
    ) -> Result<Slot, RunError> {
        let program = self.program;

        loop {
            let ty_node = program.types.get(ty).ok_or_else(|| fault("no such type"))?;

            return Ok(match (ty_node, value) {
                (TypeNode::Option(_), Value::None) => Slot::None,
                (TypeNode::Option(inner), Value::Some(inner_value)) => {
                    value = inner_value;
                    ty = *inner;
                    continue;
                }
                (TypeNode::Int, Value::Int(n)) => Slot::Int(*n),
                (TypeNode::Bool, Value::Bool(b)) => Slot::Bool(*b),
                (TypeNode::Opaque(_), Value::Opaque { handle, .. }) => Slot::Opaque(*handle),
                (TypeNode::String, Value::String(s)) => {
                    let at = self.alloc(s.len().div_ceil(8))?;

                    for (n, chunk) in s.as_bytes().chunks(8).enumerate() {
                        let mut bytes = [0u8; 8];

                        for (b, c) in bytes.iter_mut().zip(chunk) {
                            *b = *c;
                        }

                        *at.checked_add(n)
                            .and_then(|at| self.slots.get_mut(at))
                            .ok_or_else(|| fault("a string outside memory"))? = Slot::Bytes(bytes);
                    }

                    Slot::Str {
                        at: slot_index(at)?,
                        len: u32::try_from(s.len()).map_err(|_| fault("a string too long"))?,
                    }
                }
                (TypeNode::List(entry), Value::List(items)) => {
                    let at = self.alloc(items.len())?;

                    for (n, item) in items.iter().enumerate() {
                        work.push((item, *entry, at.saturating_add(n)));
                    }

                    Slot::List {
                        at: slot_index(at)?,
                        len: slot_index(items.len())?,
                    }
                }
                (TypeNode::Record(fields), Value::Record(map)) => {
                    let at = self.alloc(fields.len())?;

                    for (n, field) in fields.iter().enumerate() {
                        let value = map
                            .get(&field.name)
                            .ok_or_else(|| fault("a record lost a field"))?;
                        work.push((value, field.ty, at.saturating_add(n)));
                    }

                    Slot::Record {
                        at: slot_index(at)?,
                        len: slot_index(fields.len())?,
                    }
                }
                _ => return Err(fault("a value changed type")),
            });
        }
    }

    /// The value in `slot`, of type `ty`.
    fn read(&self, slot: Slot, ty: TypeId) -> Result<Value, RunError> {
        let program = self.program;
        let mut steps = vec![Read::Visit(slot, ty)];
        let mut out = Vec::<Value>::new();

        while let Some(step) = steps.pop() {
            match step {
                Read::Visit(slot, ty) => {
                    let ty_node = program.types.get(ty).ok_or_else(|| fault("no such type"))?;

                    match (ty_node, slot) {
                        (TypeNode::Option(_), Slot::None) => out.push(Value::None),
                        (TypeNode::Option(inner), slot) => {
                            steps.push(Read::Some);
                            steps.push(Read::Visit(slot, *inner));
                        }
                        (TypeNode::Int, Slot::Int(n)) => out.push(Value::Int(n)),
                        (TypeNode::Bool, Slot::Bool(b)) => out.push(Value::Bool(b)),
                        (TypeNode::Opaque(name), Slot::Opaque(handle)) => out.push(Value::Opaque {
                            ty: name.clone(),
                            handle,
                        }),
                        (TypeNode::String, Slot::Const(c)) => match self.constant(c)? {
                            Constant::String(s) => out.push(Value::String(s.clone())),
                            Constant::List { .. } => return Err(fault("a list used as a string")),
                        },
                        (TypeNode::String, Slot::Str { at, len }) => {
                            out.push(Value::String(self.string(at, len)?));
                        }
                        (TypeNode::List(entry), Slot::Const(c)) => match self.constant(c)? {
                            Constant::List { items, .. } => {
                                steps.push(Read::List(items.len()));
                                steps.extend(
                                    items.iter().rev().map(|i| Read::Visit(scalar(i), *entry)),
                                );
                            }
                            Constant::String(_) => return Err(fault("a string used as a list")),
                        },
                        (TypeNode::List(entry), Slot::List { at, len }) => {
                            steps.push(Read::List(size(len)?));

                            for n in (0..len).rev() {
                                steps.push(Read::Visit(self.slot(at, n)?, *entry));
                            }
                        }
                        (TypeNode::Record(fields), Slot::Record { at, len }) => {
                            if size(len)? != fields.len() {
                                return Err(fault("a record of the wrong size"));
                            }

                            steps.push(Read::Record(ty));

                            for (n, field) in (0..len).zip(fields).rev() {
                                steps.push(Read::Visit(self.slot(at, n)?, field.ty));
                            }
                        }
                        _ => return Err(fault("a value is not of its type")),
                    }
                }
                Read::List(n) => {
                    let at = out
                        .len()
                        .checked_sub(n)
                        .ok_or_else(|| fault("a list lost entries"))?;
                    let items = out.split_off(at);
                    out.push(Value::List(items));
                }
                Read::Record(ty) => {
                    let Some(TypeNode::Record(fields)) = program.types.get(ty) else {
                        return Err(fault("a record of other than a record type"));
                    };

                    let at = out
                        .len()
                        .checked_sub(fields.len())
                        .ok_or_else(|| fault("a record lost fields"))?;
                    let values = out.split_off(at);

                    out.push(Value::Record(
                        fields.iter().map(|f| f.name.clone()).zip(values).collect(),
                    ));
                }
                Read::Some => {
                    let value = out.pop().ok_or_else(|| fault("an option lost its value"))?;
                    out.push(Value::some(value));
                }
            }
        }

        match (out.pop(), out.is_empty()) {
            (Some(value), true) => Ok(value),
            _ => Err(fault("a value read as several")),
        }
    }

    fn string(&self, at: u32, len: u32) -> Result<String, RunError> {
        let len = size(len)?;
        let mut bytes = Vec::with_capacity(len);

        for n in 0..len.div_ceil(8) {
            match self.slot(at, slot_index(n)?)? {
                Slot::Bytes(chunk) => bytes.extend_from_slice(&chunk),
                _ => return Err(fault("a string's bytes are missing")),
            }
        }

        bytes.truncate(len);
        String::from_utf8(bytes).map_err(|_| fault("a string is not UTF-8"))
    }
}

fn arithmetic(instr: Instr, a: i64, b: i64) -> Result<i64, RunError> {
    let (op, value) = match instr {
        Instr::Add => ("+", a.checked_add(b)),
        Instr::Sub => ("-", a.checked_sub(b)),
        Instr::Mul => ("*", a.checked_mul(b)),
        Instr::Div => ("/", a.checked_div(b)),
        _ => ("%", a.checked_rem(b)),
    };

    let expr = || format!("{a} {op} {b}");

    match value {
        Some(value) => Ok(value),
        None if b == 0 && matches!(op, "/" | "%") => Err(RunError::DivideByZero { expr: expr() }),
        None => Err(overflow(expr())),
    }
}

fn overflow(expr: String) -> RunError {
    RunError::Overflow { expr }
}

fn scalar(item: &Scalar) -> Slot {
    match *item {
        Scalar::Int(n) => Slot::Int(n),
        Scalar::Bool(b) => Slot::Bool(b),
        Scalar::Const(c) => Slot::Const(c),
    }
}

fn path(root: &str, nodes: &[(usize, Seg<'_>)], mut node: usize) -> String {
    let mut segs = Vec::new();

    while let Some((parent, seg)) = nodes.get(node) {
        if let Seg::Root = seg {
            break;
        }

        segs.push(*seg);
        node = *parent;
    }

    let mut path = root.to_owned();

    for seg in segs.iter().rev() {
        match seg {
            Seg::Root => {}
            Seg::Field(name) if is_field_name(name) => {
                if !path.is_empty() {
                    path.push('.');
                }

                path.push_str(name);
            }
            Seg::Field(name) => path.push_str(&format!("[{name:?}]")),
            Seg::Index(i) => path.push_str(&format!("[{i}]")),
        }
    }

    path
}

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}

fn size(n: u32) -> Result<usize, RunError> {
    usize::try_from(n).map_err(|_| fault("a size past the address space"))
}

fn slot_index(n: usize) -> Result<u32, RunError> {
    u32::try_from(n).map_err(|_| fault("memory past what a slot can address"))
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
