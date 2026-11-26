//! The machine that runs a [`Program`]: a fixed amount of memory reserved
//! when it is made, values bound and type-checked before it runs, and a
//! step limit on how long it runs.

use crate::Type;
use crate::message::{Context, Fault, Found, Location, Message, Op, Root, RunError, Seg};
use crate::program::{Constant, HostFunction, Instr, Program, Scalar};
use crate::types::{TypeId, TypeNode};
use crate::value::{Repr, Value};
use crate::word::Word;

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

#[cold]
#[inline(never)]
fn fault(fault: Fault) -> RunError {
    RunError::from(fault)
}

/// The caller's functions, which a program calls by the names and types
/// its environment declared. Each call gets its arguments in order and
/// gives a value of the declared return type, or an error message.
///
/// The message is the caller's own, so it is not one of evalite's: a
/// failed call's [`RunError`] says which function failed, and has the
/// message as its [`source`](std::error::Error::source).
///
/// A closure `FnMut(&str, Vec<Value>) -> Result<Value, String>` is a host.
pub trait Host {
    /// Call the function `name` with `args`, which are of the types the
    /// environment declared for it.
    ///
    /// # Errors
    ///
    /// The caller's own message, which fails the run with a [`RunError`]
    /// whose [`source`](std::error::Error::source) it is.
    ///
    /// ```
    /// use std::error::Error;
    ///
    /// use evalite::{Environment, Host, Machine, Type, Value};
    ///
    /// struct Ratings;
    ///
    /// impl Host for Ratings {
    ///     fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
    ///         match (name, args.first().and_then(Value::as_str)) {
    ///             ("rating", Some("brawl")) => Ok(Value::int(3)),
    ///             _ => Err(format!("no rating for {args:?}")),
    ///         }
    ///     }
    /// }
    ///
    /// let mut env = Environment::with_builtins();
    /// env.function("rating", [Type::string()], Type::int())?;
    /// let mut machine = Machine::default();
    ///
    /// let program = evalite::compile("rating(\"brawl\") + 1", &env)?;
    /// let outcome = machine.load(&program)?.run_with(&mut Ratings)?;
    /// assert_eq!(outcome.value().as_int(), Some(4));
    ///
    /// let program = evalite::compile("rating(\"melee\")", &env)?;
    /// let error = machine.load(&program)?.run_with(&mut Ratings).unwrap_err();
    /// assert!(error.source().is_some_and(|e| e.to_string().starts_with("no rating")));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
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

/// What a run gave back: the program's value, the steps it took and the
/// most memory it used.
///
/// ```
/// use evalite::{Environment, Limits, Machine, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("a", Type::int())?;
///
/// let program = evalite::compile("a * 2 + 1", &env)?;
/// let mut machine = Machine::new(Limits::new());
/// let mut run = machine.load(&program)?;
/// run.bind("a", Value::int(20))?;
///
/// let outcome = run.run()?;
/// assert_eq!(outcome.value(), &Value::int(41));
/// assert_eq!(outcome.steps(), 5);
/// assert!(outcome.memory() <= u64::from(machine.limits().memory_words()));
/// assert_eq!(outcome.into_value().as_int(), Some(41));
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub(crate) value: Value,
    pub(crate) steps: u64,
    pub(crate) memory: u64,
}

impl Outcome {
    /// The program's value, of its [`result_type`](Program::result_type).
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The program's value, taken out of the outcome.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// The steps it took, at most the machine's [`Limits::fuel`].
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The most memory it used at once, in words, bindings included; at
    /// most the machine's [`Limits::memory`].
    pub fn memory(&self) -> u64 {
        self.memory
    }
}

/// A machine with a fixed memory of 64-bit words, reserved when it is
/// made, which runs programs one at a time.
///
/// Memory is untyped: the verifier has proved what every word is, so the
/// machine checks only that each access is inside memory and that each
/// word decodes as the kind it reads (the layout is in the `word` module).
///
/// ```
/// use evalite::{Environment, Limits, Machine, Type, Value};
///
/// let mut env = Environment::with_builtins();
/// env.value("strength", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let program = evalite::compile("strength + bonus.unwrap_or(0)", &env)?;
///
/// let mut machine = Machine::new(Limits::new());
/// let mut run = machine.load(&program)?;
/// run.bind("strength", Value::int(3))?;
/// run.bind("bonus", Value::some(Value::int(2)))?;
/// assert_eq!(run.run()?.value(), &Value::int(5));
///
/// let mut run = machine.load(&program)?;
/// let error = run.bind("strength", Value::bool(true)).unwrap_err();
/// let labels: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
/// assert_eq!(labels, ["expected Int, found Bool", "while binding `strength`"]);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Machine {
    limits: Limits,
    words: Box<[Word]>,
    /// Which of a loaded program's globals are bound.
    bound: Vec<bool>,
}

impl Machine {
    /// A machine with `limits`, reserving all of its memory now.
    pub fn new(limits: Limits) -> Self {
        let len = usize::try_from(limits.memory).unwrap_or(usize::MAX);

        Self {
            limits,
            words: vec![Word::int(0); len].into_boxed_slice(),
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
    ///
    /// # Errors
    ///
    /// Returns a [`RunError`] when the program needs more memory, the values it
    /// reads included, than the machine's [`Limits`] allow.
    pub fn load<'a>(&'a mut self, program: &'a Program) -> Result<Run<'a>, RunError> {
        let memory = program.memory();
        let needed = memory.total();

        if needed > u64::from(self.limits.memory) {
            return Err(RunError::new(Message::TooMuchMemory {
                needed,
                allowed: self.limits.memory,
            }))
            .context(Message::LoadingProgram);
        }

        self.load_checked(program).context(Message::LoadingProgram)
    }

    fn load_checked<'a>(&'a mut self, program: &'a Program) -> Result<Run<'a>, RunError> {
        let memory = program.memory();

        let stack_cap = size(memory.stack)?;
        let stack_base = self
            .words
            .len()
            .checked_sub(stack_cap)
            .ok_or_else(|| fault(Fault::StackLargerThanMachine))?;
        let limit = stack_base
            .checked_sub(size(memory.constants)?)
            .ok_or_else(|| fault(Fault::ConstantsLargerThanMachine))?;

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
    words: &'a mut [Word],
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

enum Read {
    Visit(usize, TypeId),
    List(usize),
    Record(TypeId),
    Some,
}

impl<'a> Run<'a> {
    /// The values the program reads, with their types, each to be bound.
    pub fn globals(&self) -> Vec<(&'a str, Type)> {
        self.program.globals()
    }

    /// Bind `value` to the value `name` the program reads, checking it is
    /// of its declared type and fits in the memory left.
    ///
    /// # Errors
    ///
    /// Returns a [`RunError`] when the program reads no value `name`, it is
    /// already bound, `value` is not of its declared type (the error's labels
    /// give the path into `value`), or `value` does not fit in the memory left.
    pub fn bind(&mut self, name: &str, value: Value) -> Result<(), RunError> {
        let program = self.program;

        let (n, global) = program
            .globals
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == name)
            .ok_or_else(|| RunError::new(Message::UnknownValue))?;

        let id = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))?;
        self.bind_global(n, global.ty, &value)
            .context(Message::Binding { global: id })
    }

    fn bind_global(&mut self, n: usize, ty: TypeId, value: &Value) -> Result<(), RunError> {
        let program = self.program;
        let id = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))?;

        if self.bound.get(n).copied().unwrap_or(true) {
            return Err(RunError::new(Message::AlreadyBound { global: id }));
        }

        let needed = self.measure(value, ty, Root::Global(id))?;
        let left = self.left();

        if needed > left {
            return Err(RunError::new(Message::TooLarge {
                needed: count(needed),
                left: count(left),
            }));
        }

        let at = program
            .global_at
            .get(n)
            .copied()
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        self.write(value, ty, size(at)?)?;

        if let Some(bound) = self.bound.get_mut(n) {
            *bound = true;
        }

        Ok(())
    }

    /// Run the program, which calls none of the caller's functions.
    ///
    /// # Errors
    ///
    /// Returns a [`RunError`] when a value the program reads is unbound, the
    /// program runs out of fuel, an operation overflows or divides by zero, or
    /// an index is out of range.
    pub fn run(self) -> Result<Outcome, RunError> {
        self.run_with(&mut NoHost)
    }

    /// Run the program, calling `host` for the caller's functions.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run), and when `host` fails a call or gives a value of
    /// another type than declared; a failed call's message is the error's
    /// [`source`](std::error::Error::source).
    pub fn run_with(mut self, host: &mut dyn Host) -> Result<Outcome, RunError> {
        let program = self.program;

        if let Some(n) = self.bound.iter().position(|bound| !bound) {
            let global = u32::try_from(n).map_err(|_| fault(Fault::PastAddressSpace));
            return Err(RunError::new(Message::Unbound { global: global? }))
                .context(Message::Running);
        }

        let code = program.code.as_slice();
        let mut pc = 0usize;

        while let Some(instr) = code.get(pc) {
            let at = u32::try_from(pc).unwrap_or(u32::MAX);

            pc = self
                .spend(1)
                .and_then(|()| self.step(*instr, pc, host))
                .with_context(|| Message::Instruction { pc: at })
                .at(Location::Instr(at))
                .context(Message::Running)?;
        }

        let value = self.result().context(Message::ReadingResult)?;

        Ok(Outcome {
            value,
            steps: self.fuel.saturating_sub(self.left),
            memory: count(self.peak_len)
                .saturating_add(count(self.peak_sp))
                .saturating_add(u64::from(program.memory().constants)),
        })
    }

    /// The program's value, left on the stack.
    fn result(&self) -> Result<Value, RunError> {
        let program = self.program;

        if self.sp != program.types.width(program.result) {
            return Err(fault(Fault::NotOneResult));
        }

        self.read(self.stack_base, program.result)
    }

    /// Copy the program's constants into memory from `limit`.
    fn load_constants(&mut self) -> Result<(), RunError> {
        let program = self.program;

        for (constant, (at, _)) in program.constants.iter().zip(&program.constant_at) {
            let at = self.limit.checked_add(size(*at)?);
            let at = at.ok_or_else(|| fault(Fault::ConstantOutsideMemory))?;

            match constant {
                Constant::String(s) => self.put_bytes(at, s.as_bytes())?,
                Constant::List { items, .. } => {
                    for (n, item) in items.iter().enumerate() {
                        let word = match *item {
                            Scalar::Int(n) => Word::int(n),
                            Scalar::Bool(b) => Word::bool(b),
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
    fn constant(&self, c: u32) -> Result<Word, RunError> {
        let (at, len) = index(&self.program.constant_at, c)
            .copied()
            .ok_or_else(|| fault(Fault::NoSuchConstant))?;
        let at = self
            .limit
            .checked_add(size(at)?)
            .ok_or_else(|| fault(Fault::ConstantOutsideMemory))?;
        Ok(Word::reference(at, size(len)?)?)
    }

    /// Run one instruction at `pc`, giving the next.
    fn step(&mut self, instr: Instr, pc: usize, host: &mut dyn Host) -> Result<usize, RunError> {
        let next = pc.checked_add(1).ok_or_else(|| fault(Fault::CodeTooLong))?;
        let types = &self.program.types;

        let push = match instr {
            Instr::Int(n) => Word::int(n),
            Instr::Bool(b) => Word::bool(b),
            Instr::Const(c) => self.constant(c)?,
            Instr::Global { at, ty } => {
                self.push_from(size(at)?, types.width(ty))?;
                return Ok(next);
            }
            Instr::Neg => {
                let a = self.pop_int()?;
                Word::int(a.checked_neg().ok_or_else(|| overflow(Op::Neg, a, 0))?)
            }
            Instr::Abs => {
                let a = self.pop_int()?;
                Word::int(a.checked_abs().ok_or_else(|| overflow(Op::Abs, a, 0))?)
            }
            Instr::Not => Word::bool(!self.pop_bool()?),
            Instr::Add | Instr::Sub | Instr::Mul | Instr::Div | Instr::Rem => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;
                Word::int(arithmetic(instr, a, b)?)
            }
            Instr::Lt | Instr::Le | Instr::Gt | Instr::Ge => {
                let b = self.pop_int()?;
                let a = self.pop_int()?;

                Word::bool(match instr {
                    Instr::Lt => a < b,
                    Instr::Le => a <= b,
                    Instr::Gt => a > b,
                    _ => a >= b,
                })
            }
            Instr::Eq | Instr::Ne => {
                let b = self.pop()?;
                let a = self.pop()?;
                Word::bool((a == b) == matches!(instr, Instr::Eq))
            }
            Instr::And(skip) | Instr::Or(skip) => {
                let top = self.pop_bool()?;
                let decides = top == matches!(instr, Instr::Or(..));

                if !decides {
                    return Ok(next);
                }

                self.push(Word::bool(top))?;

                return usize::try_from(skip)
                    .ok()
                    .and_then(|skip| next.checked_add(skip))
                    .ok_or_else(|| fault(Fault::JumpPastEnd));
            }
            Instr::Field { at, ty } => {
                let (start, len) = self.pop()?.as_reference()?;
                let width = types.width(ty);
                let at = size(at)?;

                if at.checked_add(width).is_none_or(|end| end > len) {
                    return Err(fault(Fault::FieldPastRecord));
                }

                let from = start
                    .checked_add(at)
                    .ok_or_else(|| fault(Fault::OutsideMemory))?;
                self.push_from(from, width)?;
                return Ok(next);
            }
            Instr::Index(entry) => {
                let i = self.pop_int()?;
                let (start, len) = self.pop()?.as_reference()?;

                let at = usize::try_from(i)
                    .ok()
                    .filter(|at| *at < len)
                    .ok_or_else(|| {
                        RunError::new(Message::IndexOutOfRange {
                            index: i,
                            len: count(len),
                        })
                    })?;

                self.push_entry(start, at, types.width(entry))?;
                return Ok(next);
            }
            Instr::Lookup(entry) => {
                let (start, len) = self.pop()?.as_reference()?;
                let i = self.pop_int()?;
                let at = usize::try_from(i).ok().filter(|at| *at < len);

                // An `Int` entry is given as an `Option<Int>`, two words; an
                // `Option<Int>` one as itself; and any other as one word,
                // `Word::NONE` when out of range.
                let int = types.get(entry) == Some(&TypeNode::Int);

                match at {
                    Some(at) if int => {
                        self.push(Word::present(true))?;
                        self.push_entry(start, at, 1)?;
                    }
                    Some(at) => self.push_entry(start, at, types.width(entry))?,
                    None if int || types.is_option_int(entry) => {
                        self.push(Word::present(false))?;
                        self.push(Word::int(0))?;
                    }
                    None => self.push(Word::NONE)?,
                }

                return Ok(next);
            }
            Instr::List { len, entry } => {
                let n = size(len)?;
                self.spend(n)?;

                let words = n
                    .checked_mul(types.width(entry))
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                let top = self
                    .sp
                    .checked_sub(words)
                    .ok_or_else(|| fault(Fault::ListPastStack))?;
                let from = self
                    .stack_base
                    .checked_add(top)
                    .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
                let end = from
                    .checked_add(words)
                    .filter(|end| *end <= self.words.len())
                    .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
                self.heap = self
                    .heap
                    .checked_sub(words)
                    .ok_or_else(|| fault(Fault::UndeclaredHeap))?;
                let at = self.alloc(words)?;

                self.words.copy_within(from..end, at);
                self.sp = top;
                Word::reference(at, n)?
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

                Word::int(best)
            }
            Instr::Clamp => {
                let high = self.pop_int()?;
                let low = self.pop_int()?;
                let value = self.pop_int()?;

                if low > high {
                    return Err(RunError::new(Message::ClampBounds { low, high }));
                }

                Word::int(value.clamp(low, high))
            }
            Instr::UnwrapOr(option) => {
                let default = self.pop()?;

                if types.is_option_int(option) {
                    let value = self.pop()?;
                    let present = self.pop()?.as_bool()?;
                    if present { value } else { default }
                } else {
                    let value = self.pop()?;
                    if value.is_none() { default } else { value }
                }
            }
            Instr::IsSome(option) | Instr::IsNone(option) => {
                let none = if types.is_option_int(option) {
                    self.pop()?;
                    !self.pop()?.as_bool()?
                } else {
                    self.pop()?.is_none()
                };

                Word::bool(none == matches!(instr, Instr::IsNone(..)))
            }
            Instr::Len => {
                let (_, len) = self.pop()?.as_reference()?;
                Word::int(i64::try_from(len).map_err(|_| fault(Fault::ListTooLongToCount))?)
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
        let function = index(&program.functions, f).ok_or_else(|| fault(Fault::NoSuchFunction))?;

        let value = self
            .call_host(function, host)
            .context(Message::CallingHost { function: f })?;

        self.returned(f, function, &value)
            .context(Message::ReadingHostResult { function: f })
    }

    /// Call `function` with its arguments from the stack.
    fn call_host(
        &mut self,
        function: &HostFunction,
        host: &mut dyn Host,
    ) -> Result<Value, RunError> {
        let program = self.program;
        let words = function
            .params
            .iter()
            .fold(0usize, |n, ty| n.saturating_add(program.types.width(*ty)));
        let top = self
            .sp
            .checked_sub(words)
            .ok_or_else(|| fault(Fault::ArgumentsPastStack))?;

        let mut args = Vec::with_capacity(function.params.len());
        let mut at = self
            .stack_base
            .checked_add(top)
            .ok_or_else(|| fault(Fault::StackOutsideMemory))?;

        for ty in &function.params {
            args.push(self.read(at, *ty)?);
            at = at.saturating_add(program.types.width(*ty));
        }

        self.sp = top;

        host.call(&function.name, args)
            .map_err(|message| RunError::new(Message::HostFailed).with_source(message))
    }

    /// Check and push what `function`, the program's `f`, returned.
    fn returned(&mut self, f: u32, function: &HostFunction, value: &Value) -> Result<(), RunError> {
        let needed = self.measure(value, function.ret, Root::Returned(f))?;

        self.spend(needed)?;
        let left = self.left();

        if needed > left {
            return Err(RunError::new(Message::TooLarge {
                needed: count(needed),
                left: count(left),
            }));
        }

        let width = self.program.types.width(function.ret);
        let dest = self.reserve(width)?;
        self.write(value, function.ret, dest)
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
            .ok_or_else(|| RunError::new(Message::OutOfFuel { fuel: self.fuel }))?;
        Ok(())
    }

    /// Take `n` words on top of the stack, giving where they start.
    fn reserve(&mut self, n: usize) -> Result<usize, RunError> {
        let sp = self
            .sp
            .checked_add(n)
            .filter(|sp| *sp <= self.stack_cap)
            .ok_or_else(|| fault(Fault::UndeclaredStack))?;
        let at = self
            .stack_base
            .checked_add(self.sp)
            .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
        self.sp = sp;
        self.peak_sp = self.peak_sp.max(sp);
        Ok(at)
    }

    fn push(&mut self, word: Word) -> Result<(), RunError> {
        let at = self.reserve(1)?;
        self.put(Some(at), word)
    }

    /// Push the `n` words from `from`.
    fn push_from(&mut self, from: usize, n: usize) -> Result<(), RunError> {
        let end = from
            .checked_add(n)
            .filter(|end| *end <= self.words.len())
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
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
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        self.push_from(from, width)
    }

    fn pop(&mut self) -> Result<Word, RunError> {
        let sp = self
            .sp
            .checked_sub(1)
            .ok_or_else(|| fault(Fault::EmptyStack))?;
        let word = self.get(self.stack_base.checked_add(sp))?;
        self.sp = sp;
        Ok(word)
    }

    fn pop_int(&mut self) -> Result<i64, RunError> {
        Ok(self.pop()?.as_int())
    }

    fn pop_bool(&mut self) -> Result<bool, RunError> {
        Ok(self.pop()?.as_bool()?)
    }

    /// The word at `at`.
    fn get(&self, at: Option<usize>) -> Result<Word, RunError> {
        at.and_then(|at| self.words.get(at))
            .copied()
            .ok_or_else(|| fault(Fault::OutsideMemory))
    }

    fn put(&mut self, at: Option<usize>, word: Word) -> Result<(), RunError> {
        *at.and_then(|at| self.words.get_mut(at))
            .ok_or_else(|| fault(Fault::OutsideMemory))? = word;
        Ok(())
    }

    /// Pack `bytes` 8 to a word from `at`.
    fn put_bytes(&mut self, at: usize, bytes: &[u8]) -> Result<(), RunError> {
        for (n, chunk) in bytes.chunks(8).enumerate() {
            let mut word = [0u8; 8];

            for (b, c) in word.iter_mut().zip(chunk) {
                *b = *c;
            }

            self.put(at.checked_add(n), Word::bytes(word))?;
        }

        Ok(())
    }

    /// Take `n` words of memory.
    fn alloc(&mut self, n: usize) -> Result<usize, RunError> {
        let at = self.len;
        let len = at
            .checked_add(n)
            .filter(|len| *len <= self.limit)
            .ok_or_else(|| fault(Fault::UndeclaredMemory))?;
        self.len = len;
        self.peak_len = self.peak_len.max(len);
        Ok(at)
    }

    /// Check `value` is of type `ty`, giving the words its parts take;
    /// a mismatch names its path from `root`.
    fn measure(&self, value: &Value, ty: TypeId, root: Root) -> Result<usize, RunError> {
        let program = self.program;
        let types = &program.types;
        // Each node's parent and the step to it; the root has none.
        let mut nodes = vec![(0usize, None::<Seg>)];
        let mut work = vec![(value, ty, 0usize, None)];
        let mut total = 0usize;

        while let Some((value, ty, node, option)) = work.pop() {
            let ty_node = types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))?;

            let found = match (ty_node, &value.0) {
                (TypeNode::Option(_), Repr::None)
                | (TypeNode::Int, Repr::Int(_))
                | (TypeNode::Bool, Repr::Bool(_)) => continue,
                (TypeNode::Option(inner), Repr::Some(value)) => {
                    work.push((value, *inner, node, Some(ty)));
                    continue;
                }
                (TypeNode::String, Repr::String(s)) => {
                    total = total.saturating_add(s.len().div_ceil(8));
                    continue;
                }
                (TypeNode::List(entry), Repr::List(items)) => {
                    let width = types.width(*entry);
                    total = total.saturating_add(items.len().saturating_mul(width));

                    for (i, item) in items.iter().enumerate() {
                        work.push((item, *entry, nodes.len(), None));
                        nodes.push((node, Some(Seg::Index(count(i)))));
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
                            total = total.saturating_add(types.record_words(fields));

                            for (n, field) in fields.iter().enumerate() {
                                if let Some(value) = map.get(&field.name) {
                                    let field_id = u32::try_from(n).unwrap_or(u32::MAX);
                                    work.push((value, field.ty, nodes.len(), None));
                                    nodes.push((
                                        node,
                                        Some(Seg::Field {
                                            record: ty,
                                            field: field_id,
                                        }),
                                    ));
                                }
                            }

                            continue;
                        }
                    }
                }
                _ => value.describe(),
            };

            return Err(mistyped(
                option.unwrap_or(ty),
                found,
                option.is_some(),
                root,
                &nodes,
                node,
            ));
        }

        Ok(total)
    }

    /// Write `value`, already measured against `ty`, at `dest`, taking
    /// memory for its parts.
    fn write(&mut self, value: &Value, ty: TypeId, dest: usize) -> Result<(), RunError> {
        let program = self.program;
        let types = &program.types;
        let mut work = vec![(value, ty, dest)];

        while let Some((value, ty, dest)) = work.pop() {
            let ty_node = types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))?;

            let word = match (ty_node, &value.0) {
                (TypeNode::Option(_), Repr::None) if types.is_option_int(ty) => {
                    self.put(Some(dest), Word::present(false))?;
                    self.put(dest.checked_add(1), Word::int(0))?;
                    continue;
                }
                (TypeNode::Option(_), Repr::None) => Word::NONE,
                (TypeNode::Option(inner), Repr::Some(value)) => {
                    let dest = if types.is_option_int(ty) {
                        self.put(Some(dest), Word::present(true))?;
                        dest.checked_add(1)
                            .ok_or_else(|| fault(Fault::OutsideMemory))?
                    } else {
                        dest
                    };

                    work.push((value, *inner, dest));
                    continue;
                }
                (TypeNode::Int, Repr::Int(n)) => Word::int(*n),
                (TypeNode::Bool, Repr::Bool(b)) => Word::bool(*b),
                (TypeNode::String, Repr::String(s)) => {
                    let at = self.alloc(s.len().div_ceil(8))?;
                    self.put_bytes(at, s.as_bytes())?;
                    Word::reference(at, s.len())?
                }
                (TypeNode::List(entry), Repr::List(items)) => {
                    let width = types.width(*entry);
                    let words = items
                        .len()
                        .checked_mul(width)
                        .ok_or_else(|| fault(Fault::ListTooLarge))?;
                    let at = self.alloc(words)?;

                    for (n, item) in items.iter().enumerate() {
                        let dest = n
                            .checked_mul(width)
                            .and_then(|n| n.checked_add(at))
                            .ok_or_else(|| fault(Fault::OutsideMemory))?;
                        work.push((item, *entry, dest));
                    }

                    Word::reference(at, items.len())?
                }
                (TypeNode::Record(fields), Repr::Record(map)) => {
                    let words = types.record_words(fields);
                    let at = self.alloc(words)?;

                    for (offset, field) in types.field_offsets(fields) {
                        let value = map
                            .get(&field.name)
                            .ok_or_else(|| fault(Fault::RecordLostField))?;
                        let dest = at
                            .checked_add(offset)
                            .ok_or_else(|| fault(Fault::OutsideMemory))?;
                        work.push((value, field.ty, dest));
                    }

                    Word::reference(at, words)?
                }
                _ => return Err(fault(Fault::ValueChangedType)),
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

                    match types.get(ty).ok_or_else(|| fault(Fault::NoSuchType))? {
                        TypeNode::Option(inner) => {
                            let at = if types.is_option_int(ty) {
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
                            out.push(Value::string(self.string(at, len)?));
                        }
                        TypeNode::List(entry) => {
                            let (start, len) = word.as_reference()?;
                            let width = types.width(*entry);
                            self.inside(start, len.checked_mul(width))?;
                            steps.push(Read::List(len));

                            for n in (0..len).rev() {
                                let at = n.saturating_mul(width).saturating_add(start);
                                steps.push(Read::Visit(at, *entry));
                            }
                        }
                        TypeNode::Record(fields) => {
                            let (start, len) = word.as_reference()?;

                            if len != types.record_words(fields) {
                                return Err(fault(Fault::RecordWrongSize));
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
                        .ok_or_else(|| fault(Fault::ListLostEntries))?;
                    let items = out.split_off(at);
                    out.push(Value(Repr::List(items)));
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

    /// Check the `len` words from `at` are inside memory.
    fn inside(&self, at: usize, len: Option<usize>) -> Result<(), RunError> {
        len.and_then(|len| at.checked_add(len))
            .filter(|end| *end <= self.words.len())
            .map(|_| ())
            .ok_or_else(|| fault(Fault::OutsideMemory))
    }

    /// The string of `len` bytes packed from `at`.
    fn string(&self, at: usize, len: usize) -> Result<String, RunError> {
        self.inside(at, Some(len.div_ceil(8)))?;
        let mut bytes = Vec::with_capacity(len);

        for n in 0..len.div_ceil(8) {
            bytes.extend_from_slice(&self.get(at.checked_add(n))?.as_bytes());
        }

        bytes.truncate(len);
        String::from_utf8(bytes).map_err(|_| fault(Fault::NotUtf8))
    }
}

fn arithmetic(instr: Instr, a: i64, b: i64) -> Result<i64, RunError> {
    let (op, value) = match instr {
        Instr::Add => (Op::Add, a.checked_add(b)),
        Instr::Sub => (Op::Sub, a.checked_sub(b)),
        Instr::Mul => (Op::Mul, a.checked_mul(b)),
        Instr::Div => (Op::Div, a.checked_div(b)),
        _ => (Op::Rem, a.checked_rem(b)),
    };

    match value {
        Some(value) => Ok(value),
        None if b == 0 && matches!(op, Op::Div | Op::Rem) => {
            Err(RunError::new(Message::DivideByZero { op, a }))
        }
        None => Err(overflow(op, a, b)),
    }
}

#[cold]
#[inline(never)]
fn overflow(op: Op, a: i64, b: i64) -> RunError {
    RunError::new(Message::Overflow { op, a, b })
}

/// The error of a value found to be `found` where `expected` was, at
/// `node` of the path from `root`.
#[cold]
#[inline(never)]
fn mistyped(
    expected: TypeId,
    found: Found,
    in_some: bool,
    root: Root,
    nodes: &[(usize, Option<Seg>)],
    mut node: usize,
) -> RunError {
    let mut path = Vec::new();

    while let Some((parent, Some(seg))) = nodes.get(node) {
        path.push(*seg);
        node = *parent;
    }

    path.reverse();

    let error = RunError::new(Message::Mistyped {
        expected,
        found,
        in_some,
    });

    // A whole value is named by the binding's or the function's context.
    if path.is_empty() {
        return error;
    }

    error.context(Message::AtPath {
        root,
        path: path.into_boxed_slice(),
    })
}

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}

fn size(n: u32) -> Result<usize, RunError> {
    usize::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
