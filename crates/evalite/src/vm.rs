//! The machine that runs a [`Program`]: a fixed amount of memory reserved
//! when it is made, values bound and type-checked before it runs, and a
//! step limit on how long it runs.

use std::collections::BTreeMap;
use std::fmt;

use crate::Type;
use crate::program::{Constant, Instr, Program, Scalar};
use crate::types::{NONE, TypeId, TypeNode};

/// The memory a [`Machine`] has unless [`Limits::memory`] says otherwise,
/// in 64-bit words.
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
    /// [`DEFAULT_MEMORY`] words and [`DEFAULT_FUEL`] steps.
    pub const fn new() -> Self {
        Self {
            memory: DEFAULT_MEMORY,
            fuel: DEFAULT_FUEL,
        }
    }

    /// Allow `words` words of memory: a program's stack, lists, constants
    /// and the values it reads, and the parts of the values bound to it
    /// and returned by the caller's functions.
    pub const fn memory(mut self, words: u32) -> Self {
        self.memory = words;
        self
    }

    /// Allow `steps` steps per run.
    pub const fn fuel(mut self, steps: u64) -> Self {
        self.fuel = steps;
        self
    }

    /// The memory allowed, in words.
    pub const fn memory_words(&self) -> u32 {
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
    /// Values as evalite writes them: `3`, `true`, `"Brawl"`, `[1, 2]`,
    /// `{athletics: 2, "Brawl (bar)": 1}`, `none` and `some(2)`.
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
    /// The program is not one evalite made. Programs from
    /// [`compile`](crate::compile) and [`Program::from_bytes`] never give
    /// it.
    Fault(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMuchMemory { needed, allowed } => write!(
                f,
                "the program needs {needed} words of memory, more than the {allowed} the machine allows"
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
                "binding `{name}` needs {needed} words of memory, {left} are left"
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
                "the value `{name}` returned needs {needed} words of memory, {left} are left"
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
    /// The most memory it used at once, in words, bindings included.
    pub memory: u64,
}

/// A machine with a fixed memory of 64-bit words, reserved when it is
/// made, which runs programs one at a time.
///
/// Memory is untyped: the verifier has proved what every word is, so the
/// machine checks only that each access is inside memory. A word is an
/// `Int` as `i64`, a `Bool` as 0 or 1, or a reference packing where a
/// string's, list's or record's parts start (the low 32 bits) and how many
/// there are (the high 32: a string's bytes, a list's entries or a
/// record's words). An `Option<Int>` is two words, 1 or 0 for present or
/// absent then the `Int`; every other option is one word, [`NONE`] when
/// absent. A string's bytes are packed 8 to a word.
///
/// ```
/// use evalite::{Environment, Limits, Machine, Type, Value};
///
/// let mut env = Environment::with_builtins();
/// env.value("strength", Type::Int)?;
/// env.value("bonus", Type::option(Type::Int))?;
///
/// let program = evalite::compile("strength + bonus.unwrap_or(0)", &env)?;
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
    words: Box<[u64]>,
    /// Which of a loaded program's globals are bound.
    bound: Vec<bool>,
}

impl Machine {
    /// A machine with `limits`, reserving all of its memory now.
    pub fn new(limits: Limits) -> Self {
        let len = usize::try_from(limits.memory).unwrap_or(usize::MAX);

        Self {
            limits,
            words: vec![0; len].into_boxed_slice(),
            bound: Vec::new(),
        }
    }

    /// What it allows.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Get ready to run `program`, refusing it when its memory, the words
    /// of the values it reads included, is more than the machine has, and
    /// copying its constants into memory.
    pub fn load<'a>(&'a mut self, program: &'a Program) -> Result<Run<'a>, RunError> {
        let memory = program.memory();
        let needed = memory.total();

        if needed > u64::from(self.limits.memory) {
            return Err(RunError::TooMuchMemory {
                needed,
                allowed: self.limits.memory,
            });
        }

        let stack_cap = size(memory.stack)?;
        let stack_base = self
            .words
            .len()
            .checked_sub(stack_cap)
            .ok_or_else(|| fault("a stack larger than the machine"))?;
        let limit = stack_base
            .checked_sub(size(memory.constants)?)
            .ok_or_else(|| fault("constants larger than the machine"))?;

        self.bound.clear();
        self.bound.resize(program.globals.len(), false);

        let globals = size(memory.globals)?;

        let mut run = Run {
            words: &mut self.words,
            bound: &mut self.bound,
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
        };

        run.load_constants()?;
        Ok(run)
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
    words: &'a mut [u64],
    bound: &'a mut [bool],
    program: &'a Program,
    fuel: u64,
    left: u64,
    /// Words `..len` hold the values read, then the parts of bound values,
    /// lists and returned values, up to `limit`, where the constants are.
    len: usize,
    limit: usize,
    /// The words kept for the lists the program has yet to build, which
    /// bound and returned values may not take.
    heap: usize,
    /// The stack is the `stack_cap` words from `stack_base`, `sp` deep.
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
    Visit(usize, TypeId),
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

        let (n, global) = program
            .globals
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == name)
            .ok_or_else(|| RunError::UnknownValue {
                name: name.to_owned(),
            })?;

        if self.bound.get(n).copied().unwrap_or(true) {
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

        let at = program
            .global_at
            .get(n)
            .copied()
            .ok_or_else(|| fault("a value outside memory"))?;
        self.write(&value, global.ty, size(at)?)?;

        if let Some(bound) = self.bound.get_mut(n) {
            *bound = true;
        }

        Ok(())
    }

    /// Run the program, which calls none of the caller's functions.
    pub fn run(self) -> Result<Outcome, RunError> {
        self.run_with(&mut NoHost)
    }

    /// Run the program, calling `host` for the caller's functions.
    pub fn run_with(mut self, host: &mut dyn Host) -> Result<Outcome, RunError> {
        let program = self.program;

        for (global, bound) in program.globals.iter().zip(self.bound.iter()) {
            if !bound {
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

        if self.sp != program.types.width(program.result) {
            return Err(fault("not one value is left on the stack"));
        }

        let value = self.read(self.stack_base, program.result)?;

        Ok(Outcome {
            value,
            steps: self.fuel.saturating_sub(self.left),
            memory: count(self.peak_len)
                .saturating_add(count(self.peak_sp))
                .saturating_add(u64::from(program.memory().constants)),
        })
    }

    /// Copy the program's constants into memory from `limit`.
    fn load_constants(&mut self) -> Result<(), RunError> {
        let program = self.program;

        for (constant, (at, _)) in program.constants.iter().zip(&program.constant_at) {
            let at = self.limit.checked_add(size(*at)?);
            let at = at.ok_or_else(|| fault("a constant outside memory"))?;

            match constant {
                Constant::String(s) => self.put_bytes(at, s.as_bytes())?,
                Constant::List { items, .. } => {
                    for (n, item) in items.iter().enumerate() {
                        let word = match *item {
                            Scalar::Int(n) => n as u64,
                            Scalar::Bool(b) => u64::from(b),
                            Scalar::Const(c) => self.constant(c)?,
                        };

                        self.put(at.checked_add(n), word)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// The reference to the constant `c`.
    fn constant(&self, c: u32) -> Result<u64, RunError> {
        let (at, len) = index(&self.program.constant_at, c)
            .copied()
            .ok_or_else(|| fault("no such constant"))?;
        let at = self
            .limit
            .checked_add(size(at)?)
            .ok_or_else(|| fault("a constant outside memory"))?;
        reference(at, size(len)?)
    }

    /// Run one instruction at `pc`, giving the next.
    fn step(&mut self, instr: Instr, pc: usize, host: &mut dyn Host) -> Result<usize, RunError> {
        let next = pc.checked_add(1).ok_or_else(|| fault("code too long"))?;
        let types = &self.program.types;

        let push = match instr {
            Instr::Int(n) => n as u64,
            Instr::Bool(b) => u64::from(b),
            Instr::Const(c) => self.constant(c)?,
            Instr::Global { at, ty } => {
                self.push_from(size(at)?, types.width(ty))?;
                return Ok(next);
            }
            Instr::Neg => {
                let a = self.pop_int()?;
                int_word(a.checked_neg().ok_or_else(|| overflow(format!("-({a})")))?)
            }
            Instr::Abs => {
                let a = self.pop_int()?;
                int_word(
                    a.checked_abs()
                        .ok_or_else(|| overflow(format!("({a}).abs()")))?,
                )
            }
            Instr::Not => u64::from(!self.pop_bool()?),
            Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Rem => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;
                int_word(arithmetic(instr, a, b)?)
            }
            Instr::Lt | Instr::Le | Instr::Gt | Instr::Ge => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;

                u64::from(match instr {
                    Instr::Lt => a < b,
                    Instr::Le => a <= b,
                    Instr::Gt => a > b,
                    _ => a >= b,
                })
            }
            Instr::Eq | Instr::Ne => {
                let b = self.pop()?;
                let a = self.pop()?;
                u64::from((a == b) == matches!(instr, Instr::Eq))
            }
            Instr::And(skip) | Instr::Or(skip) => {
                let top = self.pop_bool()?;
                let decides = top == matches!(instr, Instr::Or(..));

                if !decides {
                    return Ok(next);
                }

                self.push(u64::from(top))?;

                return usize::try_from(skip)
                    .ok()
                    .and_then(|skip| next.checked_add(skip))
                    .ok_or_else(|| fault("jumps past the end"));
            }
            Instr::Field { at, ty } => {
                let (start, len) = unpack(self.pop()?)?;
                let width = types.width(ty);
                let at = size(at)?;

                if at.checked_add(width).is_none_or(|end| end > len) {
                    return Err(fault("reads a field the record does not have"));
                }

                let from = start
                    .checked_add(at)
                    .ok_or_else(|| fault("a value outside memory"))?;
                self.push_from(from, width)?;
                return Ok(next);
            }
            Instr::Index(entry) => {
                let i = self.pop_int()?;
                let (start, len) = unpack(self.pop()?)?;

                let at = usize::try_from(i)
                    .ok()
                    .filter(|at| *at < len)
                    .ok_or(RunError::IndexOutOfRange { index: i, len })?;

                self.push_entry(start, at, types.width(entry))?;
                return Ok(next);
            }
            Instr::Lookup(entry) => {
                let (start, len) = unpack(self.pop()?)?;
                let i = self.pop_int()?;
                let last = len.checked_sub(1).ok_or(RunError::EmptyLookup)?;
                let at = usize::try_from(i.max(0)).unwrap_or(usize::MAX).min(last);
                self.push_entry(start, at, types.width(entry))?;
                return Ok(next);
            }
            Instr::List { len, entry } => {
                let n = size(len)?;
                self.spend(n)?;

                let words = n
                    .checked_mul(types.width(entry))
                    .ok_or_else(|| fault("a list too large"))?;
                let top = self
                    .sp
                    .checked_sub(words)
                    .ok_or_else(|| fault("makes a list of more entries than the stack has"))?;
                let from = self
                    .stack_base
                    .checked_add(top)
                    .ok_or_else(|| fault("a stack outside memory"))?;
                let end = from
                    .checked_add(words)
                    .filter(|end| *end <= self.words.len())
                    .ok_or_else(|| fault("a stack outside memory"))?;
                self.heap = self
                    .heap
                    .checked_sub(words)
                    .ok_or_else(|| fault("builds more lists than it declares"))?;
                let at = self.alloc(words)?;

                self.words.copy_within(from..end, at);
                self.sp = top;
                reference(at, n)?
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

                int_word(best)
            }
            Instr::Clamp => {
                let high = self.pop_int()?;
                let low = self.pop_int()?;
                let value = self.pop_int()?;

                if low > high {
                    return Err(RunError::ClampBounds { low, high });
                }

                int_word(value.clamp(low, high))
            }
            Instr::UnwrapOr(option) => {
                let default = self.pop()?;

                if types.is_option_int(option) {
                    let value = self.pop()?;
                    let present = self.pop()?;
                    if present != 0 { value } else { default }
                } else {
                    let value = self.pop()?;
                    if value != NONE { value } else { default }
                }
            }
            Instr::IsSome(option) | Instr::IsNone(option) => {
                let none = if types.is_option_int(option) {
                    self.pop()?;
                    self.pop()? == 0
                } else {
                    self.pop()? == NONE
                };

                u64::from(none == matches!(instr, Instr::IsNone(..)))
            }
            Instr::Len => {
                let (_, len) = unpack(self.pop()?)?;
                i64::try_from(len).map_err(|_| fault("a list too long to count"))? as u64
            }
            Instr::Call(f) => {
                self.call(f, host)?;
                return Ok(next);
            }
        };

        self.push(push)?;
        Ok(next)
    }

    fn call(&mut self, f: u32, host: &mut dyn Host) -> Result<(), RunError> {
        let program = self.program;
        let function = index(&program.functions, f).ok_or_else(|| fault("no such function"))?;

        let words = function
            .params
            .iter()
            .fold(0usize, |n, ty| n.saturating_add(program.types.width(*ty)));
        let top = self
            .sp
            .checked_sub(words)
            .ok_or_else(|| fault("calls with more arguments than the stack has"))?;

        let mut args = Vec::with_capacity(function.params.len());
        let mut at = self
            .stack_base
            .checked_add(top)
            .ok_or_else(|| fault("a stack outside memory"))?;

        for ty in &function.params {
            args.push(self.read(at, *ty)?);
            at = at.saturating_add(program.types.width(*ty));
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

        let width = program.types.width(function.ret);
        let dest = self.reserve(width)?;
        self.write(&value, function.ret, dest)
    }

    /// The words left for bound and returned values.
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

    /// Take `n` words on top of the stack, giving where they start.
    fn reserve(&mut self, n: usize) -> Result<usize, RunError> {
        let sp = self
            .sp
            .checked_add(n)
            .filter(|sp| *sp <= self.stack_cap)
            .ok_or_else(|| fault("the stack grows past the depth it declares"))?;
        let at = self
            .stack_base
            .checked_add(self.sp)
            .ok_or_else(|| fault("a stack outside memory"))?;
        self.sp = sp;
        self.peak_sp = self.peak_sp.max(sp);
        Ok(at)
    }

    fn push(&mut self, word: u64) -> Result<(), RunError> {
        let at = self.reserve(1)?;
        self.put(Some(at), word)
    }

    /// Push the `n` words from `from`.
    fn push_from(&mut self, from: usize, n: usize) -> Result<(), RunError> {
        let end = from
            .checked_add(n)
            .filter(|end| *end <= self.words.len())
            .ok_or_else(|| fault("a value outside memory"))?;
        let at = self.reserve(n)?;
        self.words.copy_within(from..end, at);
        Ok(())
    }

    /// Push entry `at` of the list from `start` whose entries are `width`
    /// words.
    fn push_entry(&mut self, start: usize, at: usize, width: usize) -> Result<(), RunError> {
        let from = at
            .checked_mul(width)
            .and_then(|at| at.checked_add(start))
            .ok_or_else(|| fault("a value outside memory"))?;
        self.push_from(from, width)
    }

    fn pop(&mut self) -> Result<u64, RunError> {
        let sp = self
            .sp
            .checked_sub(1)
            .ok_or_else(|| fault("pops an empty stack"))?;
        let word = self.get(self.stack_base.checked_add(sp))?;
        self.sp = sp;
        Ok(word)
    }

    fn pop_int(&mut self) -> Result<i64, RunError> {
        Ok(self.pop()? as i64)
    }

    fn pop_bool(&mut self) -> Result<bool, RunError> {
        Ok(self.pop()? != 0)
    }

    /// The word at `at`.
    fn get(&self, at: Option<usize>) -> Result<u64, RunError> {
        at.and_then(|at| self.words.get(at))
            .copied()
            .ok_or_else(|| fault("a value outside memory"))
    }

    fn put(&mut self, at: Option<usize>, word: u64) -> Result<(), RunError> {
        *at.and_then(|at| self.words.get_mut(at))
            .ok_or_else(|| fault("a value outside memory"))? = word;
        Ok(())
    }

    /// Pack `bytes` 8 to a word from `at`.
    fn put_bytes(&mut self, at: usize, bytes: &[u8]) -> Result<(), RunError> {
        for (n, chunk) in bytes.chunks(8).enumerate() {
            let mut word = [0u8; 8];

            for (b, c) in word.iter_mut().zip(chunk) {
                *b = *c;
            }

            self.put(at.checked_add(n), u64::from_le_bytes(word))?;
        }

        Ok(())
    }

    /// Take `n` words of memory.
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

    /// Check `value` is of type `ty`, giving the words its parts take.
    fn measure(
        &self,
        value: &Value,
        ty: TypeId,
        root: &str,
    ) -> Result<Result<usize, Mismatch>, RunError> {
        let program = self.program;
        let types = &program.types;
        let mut nodes = vec![(0usize, Seg::Root)];
        let mut work = vec![(value, ty, 0usize, None)];
        let mut total = 0usize;

        while let Some((value, ty, node, option)) = work.pop() {
            let ty_node = types.get(ty).ok_or_else(|| fault("no such type"))?;

            let found = match (ty_node, value) {
                (TypeNode::Option(_), Value::None)
                | (TypeNode::Int, Value::Int(_))
                | (TypeNode::Bool, Value::Bool(_)) => continue,
                (TypeNode::Option(inner), Value::Some(value)) => {
                    work.push((value, *inner, node, Some(ty)));
                    continue;
                }
                (TypeNode::String, Value::String(s)) => {
                    total = total.saturating_add(s.len().div_ceil(8));
                    continue;
                }
                (TypeNode::List(entry), Value::List(items)) => {
                    let width = types.width(*entry);
                    total = total.saturating_add(items.len().saturating_mul(width));

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
                            total = total.saturating_add(types.record_words(fields));

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

    /// Write `value`, already measured against `ty`, at `dest`, taking
    /// memory for its parts.
    fn write(&mut self, value: &Value, ty: TypeId, dest: usize) -> Result<(), RunError> {
        let program = self.program;
        let types = &program.types;
        let mut work = vec![(value, ty, dest)];

        while let Some((value, ty, dest)) = work.pop() {
            let ty_node = types.get(ty).ok_or_else(|| fault("no such type"))?;

            let word = match (ty_node, value) {
                (TypeNode::Option(_), Value::None) if types.is_option_int(ty) => {
                    self.put(Some(dest), 0)?;
                    self.put(dest.checked_add(1), 0)?;
                    continue;
                }
                (TypeNode::Option(_), Value::None) => NONE,
                (TypeNode::Option(inner), Value::Some(value)) => {
                    let dest = if types.is_option_int(ty) {
                        self.put(Some(dest), 1)?;
                        dest.checked_add(1)
                            .ok_or_else(|| fault("a value outside memory"))?
                    } else {
                        dest
                    };

                    work.push((value, *inner, dest));
                    continue;
                }
                (TypeNode::Int, Value::Int(n)) => *n as u64,
                (TypeNode::Bool, Value::Bool(b)) => u64::from(*b),
                (TypeNode::String, Value::String(s)) => {
                    let at = self.alloc(s.len().div_ceil(8))?;
                    self.put_bytes(at, s.as_bytes())?;
                    reference(at, s.len())?
                }
                (TypeNode::List(entry), Value::List(items)) => {
                    let width = types.width(*entry);
                    let words = items
                        .len()
                        .checked_mul(width)
                        .ok_or_else(|| fault("a list too large"))?;
                    let at = self.alloc(words)?;

                    for (n, item) in items.iter().enumerate() {
                        let dest = n
                            .checked_mul(width)
                            .and_then(|n| n.checked_add(at))
                            .ok_or_else(|| fault("a value outside memory"))?;
                        work.push((item, *entry, dest));
                    }

                    reference(at, items.len())?
                }
                (TypeNode::Record(fields), Value::Record(map)) => {
                    let words = types.record_words(fields);
                    let at = self.alloc(words)?;

                    for (offset, field) in types.field_offsets(fields) {
                        let value = map
                            .get(&field.name)
                            .ok_or_else(|| fault("a record lost a field"))?;
                        let dest = at
                            .checked_add(offset)
                            .ok_or_else(|| fault("a value outside memory"))?;
                        work.push((value, field.ty, dest));
                    }

                    reference(at, words)?
                }
                _ => return Err(fault("a value changed type")),
            };

            self.put(Some(dest), word)?;
        }

        Ok(())
    }

    /// The value of type `ty` held at `at`.
    fn read(&self, at: usize, ty: TypeId) -> Result<Value, RunError> {
        let program = self.program;
        let types = &program.types;
        let mut steps = vec![Read::Visit(at, ty)];
        let mut out = Vec::<Value>::new();

        while let Some(step) = steps.pop() {
            match step {
                Read::Visit(at, ty) => {
                    let word = self.get(Some(at))?;

                    match types.get(ty).ok_or_else(|| fault("no such type"))? {
                        TypeNode::Option(inner) => {
                            let at = if types.is_option_int(ty) {
                                if word == 0 {
                                    out.push(Value::None);
                                    continue;
                                }

                                at.checked_add(1)
                                    .ok_or_else(|| fault("a value outside memory"))?
                            } else if word == NONE {
                                out.push(Value::None);
                                continue;
                            } else {
                                at
                            };

                            steps.push(Read::Some);
                            steps.push(Read::Visit(at, *inner));
                        }
                        TypeNode::Int => out.push(Value::Int(word as i64)),
                        TypeNode::Bool => out.push(Value::Bool(word != 0)),
                        TypeNode::String => {
                            let (at, len) = unpack(word)?;
                            out.push(Value::String(self.string(at, len)?));
                        }
                        TypeNode::List(entry) => {
                            let (start, len) = unpack(word)?;
                            let width = types.width(*entry);
                            self.inside(start, len.checked_mul(width))?;
                            steps.push(Read::List(len));

                            for n in (0..len).rev() {
                                let at = n.saturating_mul(width).saturating_add(start);
                                steps.push(Read::Visit(at, *entry));
                            }
                        }
                        TypeNode::Record(fields) => {
                            let (start, len) = unpack(word)?;

                            if len != types.record_words(fields) {
                                return Err(fault("a record of the wrong size"));
                            }

                            self.inside(start, Some(len))?;
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
                        .ok_or_else(|| fault("a list lost entries"))?;
                    let items = out.split_off(at);
                    out.push(Value::List(items));
                }
                Read::Record(ty) => {
                    let Some(TypeNode::Record(fields)) = types.get(ty) else {
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

    /// Check the `len` words from `at` are inside memory.
    fn inside(&self, at: usize, len: Option<usize>) -> Result<(), RunError> {
        len.and_then(|len| at.checked_add(len))
            .filter(|end| *end <= self.words.len())
            .map(|_| ())
            .ok_or_else(|| fault("a value outside memory"))
    }

    /// The string of `len` bytes packed from `at`.
    fn string(&self, at: usize, len: usize) -> Result<String, RunError> {
        self.inside(at, Some(len.div_ceil(8)))?;
        let mut bytes = Vec::with_capacity(len);

        for n in 0..len.div_ceil(8) {
            bytes.extend_from_slice(&self.get(at.checked_add(n))?.to_le_bytes());
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

fn int_word(n: i64) -> u64 {
    n as u64
}

/// A reference to the `len` parts from `at`: `at` in the low 32 bits,
/// `len` in the high.
fn reference(at: usize, len: usize) -> Result<u64, RunError> {
    let at = u32::try_from(at).map_err(|_| fault("memory past what a word can address"))?;
    let len = u32::try_from(len).map_err(|_| fault("a value too long for a word"))?;
    Ok(u64::from(at) | u64::from(len).wrapping_shl(32))
}

/// Where a reference's parts start, and how many there are.
fn unpack(word: u64) -> Result<(usize, usize), RunError> {
    let at = u32::try_from(word & u64::from(u32::MAX)).unwrap_or(u32::MAX);
    let len = u32::try_from(word.wrapping_shr(32)).unwrap_or(u32::MAX);
    Ok((size(at)?, size(len)?))
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

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
