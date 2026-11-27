//! The machine that runs a [`Program`]: a fixed amount of scratch memory
//! reserved when it is made, the values of a [`Globals`] store read in
//! place, and a step limit on how long it runs.
//!
//! A run addresses one space of words: the store's words from 0, then the
//! machine's after them. Reads go to whichever holds the address, and
//! writes only ever to the machine's.

use std::cmp::Ordering;

use crate::globals::Globals;
use crate::memory::{self, Fuel, Words, WordsMut, count, fault};
use crate::message::{Context, Fault, Key, Location, MAX_KEY_SHOWN, Message, Op, Root, RunError};
use crate::program::{Constant, HostFunction, Instr, Program, Scalar};
use crate::types::TypeNode;
use crate::value::Value;
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

    /// Allow `words` words of memory: a program's stack, lists and
    /// constants, and the parts of the values returned by the caller's
    /// functions. The values it reads are in the [`Globals`] store it runs
    /// with, not the machine.
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
    /// use evalite::{Environment, Globals, Host, Machine, Type, Value};
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
    /// let mut env = Environment::new();
    /// env.function("rating", [Type::string()], Type::int())?;
    /// let globals = Globals::new(&env);
    /// let mut machine = Machine::default();
    ///
    /// let program = evalite::compile("rating(\"brawl\") + 1", &env)?;
    /// let outcome = machine.load(&program, &globals)?.run_with(&mut Ratings)?;
    /// assert_eq!(outcome.value().as_int(), Some(4));
    ///
    /// let program = evalite::compile("rating(\"melee\")", &env)?;
    /// let error = machine.load(&program, &globals)?.run_with(&mut Ratings).unwrap_err();
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
/// use evalite::{Environment, Globals, Limits, Machine, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("a", Type::int())?;
/// let mut globals = Globals::new(&env);
/// globals.set("a", &Value::int(20))?;
///
/// let program = evalite::compile("a * 2 + 1", &env)?;
/// let mut machine = Machine::new(Limits::new());
/// let outcome = machine.load(&program, &globals)?.run()?;
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

    /// The most of the machine's memory it used at once, in words; at most
    /// its [`Limits::memory`]. The values it read in the store are not
    /// counted.
    pub fn memory(&self) -> u64 {
        self.memory
    }
}

/// A machine with a fixed scratch memory of 64-bit words, reserved when it
/// is made, which runs programs one at a time over the values of a
/// [`Globals`] store.
///
/// Memory is untyped: the verifier has proved what every word is, so the
/// machine checks only that each access is inside memory and that each
/// word decodes as the kind it reads (the layout is in the `word` module).
///
/// ```
/// use evalite::{Environment, Globals, Limits, Machine, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let program = evalite::compile("strength + bonus.unwrap_or(0)", &env)?;
///
/// let mut globals = Globals::new(&env);
/// let mut machine = Machine::new(Limits::new());
/// globals.set("strength", &Value::int(3))?;
///
/// let error = machine.load(&program, &globals)?.run().unwrap_err();
/// let labels: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
/// assert_eq!(labels[0], "`bonus` is not set, set it to an Option<Int> before running");
///
/// globals.set("bonus", &Value::some(Value::int(2)))?;
/// assert_eq!(machine.load(&program, &globals)?.run()?.value(), &Value::int(5));
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Machine {
    limits: Limits,
    words: Box<[Word]>,
}

impl Machine {
    /// A machine with `limits`. It reserves all of its memory on the first
    /// [`load`](Self::load) that passes the checks, and keeps it for every
    /// later one, so a machine refused before it runs anything never takes
    /// its words.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            words: Box::default(),
        }
    }

    /// The words it has reserved: none until a load passes the checks.
    #[cfg(test)]
    pub(crate) fn reserved(&self) -> usize {
        self.words.len()
    }

    /// What it allows.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Get ready to run `program` over the values of `globals`, copying
    /// its constants into memory. Its values are read where the store
    /// holds them, so loading copies none of them, however large.
    ///
    /// # Errors
    ///
    /// Returns a [`RunError`] when `globals` does not
    /// [`accept`](Globals::accepts) the program (it was compiled against
    /// another layout of values), the program needs more memory than the
    /// machine's [`Limits`] allow, or the store and the machine together
    /// are more words than a run can address (2^32).
    ///
    /// ```
    /// use evalite::{Environment, Globals, Machine, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("wits", Type::int())?;
    /// let program = evalite::compile("wits + 1", &env)?;
    ///
    /// let mut other = Environment::new();
    /// other.value("wits", Type::bool())?;
    ///
    /// let error = Machine::default().load(&program, &Globals::new(&other)).unwrap_err();
    /// let labels: Vec<String> = error.labels(&program, None).map(|l| l.message().to_string()).collect();
    /// assert_eq!(labels, [
    ///     "the store does not hold `wits` where the program reads it, of its type",
    ///     "while loading the program",
    /// ]);
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn load<'a>(
        &'a mut self,
        program: &'a Program,
        globals: &'a Globals,
    ) -> Result<Run<'a>, RunError> {
        if let Some(n) = globals.refused(program) {
            let global = u32::try_from(n).ok();
            return Err(RunError::new(Message::OtherLayout { global }))
                .context(Message::LoadingProgram);
        }

        let memory = program.memory();
        let needed = memory.total();

        if needed > u64::from(self.limits.memory) {
            return Err(RunError::new(Message::TooMuchMemory {
                needed,
                allowed: self.limits.memory,
            }))
            .context(Message::LoadingProgram);
        }

        // Checked against the limit, not the words held: they are only
        // reserved once the checks pass.
        let len = usize::try_from(self.limits.memory).unwrap_or(usize::MAX);
        let base = globals.words().len();
        let end = base
            .checked_add(len)
            .filter(|end| u32::try_from(*end).is_ok());

        let Some(end) = end else {
            return Err(RunError::new(Message::PastAddressSpace {
                store: count(base),
                machine: self.limits.memory,
            }))
            .context(Message::LoadingProgram);
        };

        if self.words.len() != len {
            self.words = vec![Word::int(0); len].into_boxed_slice();
        }

        self.load_checked(program, globals, end)
            .context(Message::LoadingProgram)
    }

    fn load_checked<'a>(
        &'a mut self,
        program: &'a Program,
        globals: &'a Globals,
        end: usize,
    ) -> Result<Run<'a>, RunError> {
        let memory = program.memory();
        let store = globals.words();
        let base = store.len();

        let stack_cap = size(memory.stack)?;
        let stack_base = end
            .checked_sub(stack_cap)
            .filter(|at| *at >= base)
            .ok_or_else(|| fault(Fault::StackLargerThanMachine))?;
        let limit = stack_base
            .checked_sub(size(memory.constants)?)
            .filter(|at| *at >= base)
            .ok_or_else(|| fault(Fault::ConstantsLargerThanMachine))?;

        let mut run = Run {
            store,
            set: globals.set_flags(),
            base,
            words: &mut self.words,
            program,
            fuel: Fuel::new(self.limits.fuel),
            len: base,
            limit,
            heap: size(memory.heap)?,
            stack_base,
            stack_cap,
            sp: 0,
            peak_len: base,
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

/// A program loaded on a [`Machine`] over a [`Globals`] store, ready to
/// run.
#[derive(Debug)]
pub struct Run<'a> {
    /// The store's words, addresses `..base`.
    store: &'a [Word],
    /// Whether the value whose slot starts at each word is set.
    set: &'a [bool],
    base: usize,
    /// The machine's words, addresses `base..`.
    words: &'a mut [Word],
    program: &'a Program,
    fuel: Fuel,
    /// Words `base..len` hold lists and the parts of returned values, up
    /// to `limit`, where the constants are. Every position is an address,
    /// counted from the store's first word.
    len: usize,
    limit: usize,
    /// The words kept for the lists the program has yet to build, which
    /// returned values may not take.
    heap: usize,
    /// The stack is the `stack_cap` words from `stack_base`, `sp` deep.
    stack_base: usize,
    stack_cap: usize,
    sp: usize,
    peak_len: usize,
    peak_sp: usize,
}

impl<'a> Run<'a> {
    /// Run the program, which calls none of the caller's functions.
    ///
    /// # Errors
    ///
    /// Returns a [`RunError`] when a value the program reads is not set in
    /// the store, the program runs out of fuel, an operation overflows or divides by zero, or
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
            steps: self.fuel.spent(),
            memory: count(self.peak_len.saturating_sub(self.base))
                .saturating_add(count(self.peak_sp))
                .saturating_add(u64::from(program.memory().constants)),
        })
    }

    /// The program's value, left on the stack, spending a step per word
    /// of its parts.
    fn result(&mut self) -> Result<Value, RunError> {
        let program = self.program;

        if self.sp != program.types.width(program.result) {
            return Err(fault(Fault::NotOneResult));
        }

        let mut fuel = self.fuel;
        let value = memory::read(
            &*self,
            &program.types,
            self.stack_base,
            program.result,
            &mut fuel,
        )?;
        self.fuel = fuel;
        Ok(value)
    }

    /// Copy the program's constants into memory from `limit`.
    fn load_constants(&mut self) -> Result<(), RunError> {
        let program = self.program;

        for (constant, (at, _)) in program.constants.iter().zip(&program.constant_at) {
            let at = self.limit.checked_add(size(*at)?);
            let at = at.ok_or_else(|| fault(Fault::ConstantOutsideMemory))?;

            match constant {
                Constant::String(s) => memory::put_bytes(self, at, s.as_bytes())?,
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
                let at = size(at)?;

                if !self.set.get(at).copied().unwrap_or(false) {
                    return Err(self.unset(at));
                }

                match types.width(ty) {
                    // Most values are a word, read straight from the store.
                    1 => self
                        .store
                        .get(at)
                        .copied()
                        .ok_or_else(|| fault(Fault::OutsideMemory))?,
                    width => {
                        self.push_from(at, width)?;
                        return Ok(next);
                    }
                }
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
            Instr::Get(entry) => {
                let i = self.pop_int()?;
                let (start, len) = self.pop()?.as_reference()?;
                let at = usize::try_from(i).ok().filter(|at| *at < len);

                // An option of the entry: tagged when the entry has no
                // niche, else the entry's one word or `Word::NONE`.
                let width = types.width(entry);
                let tagged = !types.has_niche(entry);

                match at {
                    Some(at) => {
                        if tagged {
                            self.push(Word::present(true))?;
                        }

                        self.push_entry(start, at, width)?;
                    }
                    None if tagged => self.push_absent(width)?,
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
                self.heap = self
                    .heap
                    .checked_sub(words)
                    .ok_or_else(|| fault(Fault::UndeclaredHeap))?;
                let at = self.alloc(words)?;

                self.copy(from, words, at)?;
                self.sp = top;
                Word::reference(at, n)?
            }
            Instr::Map { keys, map } => {
                let Some(TypeNode::Map { value, .. }) = types.get(map) else {
                    return Err(fault(Fault::NoSuchType));
                };

                // The keys are in key order, as the verifier checks, and the
                // values above each other on the stack in that order.
                let (keys, n) = self.constant(keys)?.as_reference()?;
                self.spend(n)?;

                let width = types.width(*value);
                let stride = width
                    .checked_add(1)
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                let words = n
                    .checked_mul(stride)
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                let values = n
                    .checked_mul(width)
                    .ok_or_else(|| fault(Fault::ListTooLarge))?;
                let top = self
                    .sp
                    .checked_sub(values)
                    .ok_or_else(|| fault(Fault::ListPastStack))?;
                let from = self
                    .stack_base
                    .checked_add(top)
                    .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
                self.heap = self
                    .heap
                    .checked_sub(words)
                    .ok_or_else(|| fault(Fault::UndeclaredHeap))?;
                let at = self.alloc(words)?;

                for n in 0..n {
                    let entry = n.checked_mul(stride).and_then(|e| e.checked_add(at));
                    let key = self.get(keys.checked_add(n))?;
                    self.put(entry, key)?;

                    for w in 0..width {
                        let word = n
                            .checked_mul(width)
                            .and_then(|v| v.checked_add(from)?.checked_add(w));
                        let dest = entry.and_then(|e| e.checked_add(1)?.checked_add(w));
                        let word = self.get(word)?;
                        self.put(dest, word)?;
                    }
                }

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
                if let Some(width) = types.tagged(option) {
                    // The option's tag, its value and the default, each
                    // value `width` words: the one kept moves over the tag.
                    let tag = self.below(width.saturating_mul(2).saturating_add(1))?;
                    let keep = if self.get(Some(tag))?.as_bool()? {
                        tag.saturating_add(1)
                    } else {
                        tag.saturating_add(1).saturating_add(width)
                    };

                    self.move_down(keep, tag, width)?;
                    return Ok(next);
                }

                let default = self.pop()?;
                let value = self.pop()?;
                if value.is_none() { default } else { value }
            }
            Instr::Unwrap(option) => {
                if let Some(width) = types.tagged(option) {
                    let tag = self.below(width.saturating_add(1))?;

                    if !self.get(Some(tag))?.as_bool()? {
                        return Err(RunError::new(Message::UnwrapNone));
                    }

                    self.move_down(tag.saturating_add(1), tag, width)?;
                    return Ok(next);
                }

                Some(self.pop()?)
                    .filter(|value| !value.is_none())
                    .ok_or_else(|| RunError::new(Message::UnwrapNone))?
            }
            Instr::IsSome(option) | Instr::IsNone(option) => {
                let none = if let Some(width) = types.tagged(option) {
                    let tag = self.below(width.saturating_add(1))?;
                    let present = self.get(Some(tag))?.as_bool()?;
                    self.drop_words(width.saturating_add(1))?;
                    !present
                } else {
                    self.pop()?.is_none()
                };

                Word::bool(none == matches!(instr, Instr::IsNone(..)))
            }
            Instr::Some(option) => {
                // A tagged option's tag goes below its value; any other's
                // value is its own present option.
                if let Some(width) = types.tagged(option) {
                    let value = self.below(width)?;
                    self.reserve(1)?;
                    self.copy(value, width, value.saturating_add(1))?;
                    self.put(Some(value), Word::present(true))?;
                }

                return Ok(next);
            }
            Instr::None(option) => {
                match types.tagged(option) {
                    Some(width) => self.push_absent(width)?,
                    None => self.push(Word::NONE)?,
                }

                return Ok(next);
            }
            Instr::Len => {
                let (_, len) = self.pop()?.as_reference()?;
                Word::int(i64::try_from(len).map_err(|_| fault(Fault::ListTooLongToCount))?)
            }
            Instr::Call(f) => {
                self.call(f, host)?;
                return Ok(next);
            }
            Instr::MapIndex(map) | Instr::MapGet(map) => {
                let Some(TypeNode::Map { key, value }) = types.get(map) else {
                    return Err(fault(Fault::NoSuchType));
                };

                let strings = matches!(types.get(*key), Some(TypeNode::String));
                let width = types.width(*value);
                let tagged = !types.has_niche(*value);

                let needle = self.pop()?;
                let (start, len) = self.pop()?.as_reference()?;
                let found = self.find_key(start, len, width, needle, strings)?;

                match (instr, found) {
                    (_, Some(at)) => {
                        if tagged && matches!(instr, Instr::MapGet(..)) {
                            self.push(Word::present(true))?;
                        }

                        let from = at
                            .checked_add(1)
                            .ok_or_else(|| fault(Fault::OutsideMemory))?;
                        self.push_from(from, width)?;
                    }
                    (Instr::MapIndex(..), None) => {
                        let key = if strings {
                            let (at, len) = needle.as_reference()?;
                            // Only as much as the message shows: a key is cut to
                            // `MAX_KEY_SHOWN` characters of at most 4 bytes, and
                            // one more byte tells it was cut.
                            let max = MAX_KEY_SHOWN.saturating_mul(4).saturating_add(4);
                            Key::string(&memory::string_prefix(self, at, len, max)?)
                        } else {
                            Key::Int(needle.as_int())
                        };

                        return Err(RunError::new(Message::NoSuchKey { key }));
                    }
                    (_, None) if tagged => self.push_absent(width)?,
                    (_, None) => self.push(Word::NONE)?,
                }

                return Ok(next);
            }
        };

        self.push(push)?;
        Ok(next)
    }

    /// Where the entry of key `needle` starts in the map of `len` entries
    /// from `start`, whose values are `width` words, found by binary
    /// search; `strings` when its keys are strings, else `Int`s. It spends
    /// a step per probe, and per word of a string key compared.
    fn find_key(
        &mut self,
        start: usize,
        len: usize,
        width: usize,
        needle: Word,
        strings: bool,
    ) -> Result<Option<usize>, RunError> {
        let stride = width
            .checked_add(1)
            .ok_or_else(|| fault(Fault::OutsideMemory))?;
        memory::inside(self, start, len.checked_mul(stride))?;

        let mut low = 0usize;
        let mut high = len;

        while low < high {
            let mid = low.saturating_add(high.saturating_sub(low) / 2);
            let at = mid
                .checked_mul(stride)
                .and_then(|at| at.checked_add(start))
                .ok_or_else(|| fault(Fault::OutsideMemory))?;
            self.spend(1)?;
            let probe = self.get(Some(at))?;

            let order = if strings {
                self.compare_strings(probe, needle)?
            } else {
                probe.as_int().cmp(&needle.as_int())
            };

            match order {
                Ordering::Less => low = mid.saturating_add(1),
                Ordering::Greater => high = mid,
                Ordering::Equal => return Ok(Some(at)),
            }
        }

        Ok(None)
    }

    /// The order of the strings `a` and `b` refer to, by their bytes,
    /// spending a step per word compared.
    fn compare_strings(&mut self, a: Word, b: Word) -> Result<Ordering, RunError> {
        let (a_at, a_len) = a.as_reference()?;
        let (b_at, b_len) = b.as_reference()?;
        let common = a_len.min(b_len);

        for n in 0..common.div_ceil(8) {
            self.spend(1)?;
            let x = self.get(a_at.checked_add(n))?.as_bytes();
            let y = self.get(b_at.checked_add(n))?.as_bytes();
            let take = common.saturating_sub(n.saturating_mul(8)).min(8);
            let order = x.get(..take).cmp(&y.get(..take));

            if order != Ordering::Equal {
                return Ok(order);
            }
        }

        Ok(a_len.cmp(&b_len))
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

    /// Call `function` with its arguments from the stack, spending a step
    /// per word of their parts as they are read.
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

        let mut fuel = self.fuel;

        for ty in &function.params {
            args.push(memory::read(&*self, &program.types, at, *ty, &mut fuel)?);
            at = at.saturating_add(program.types.width(*ty));
        }

        self.fuel = fuel;
        self.sp = top;

        host.call(&function.name, args)
            .map_err(|message| RunError::new(Message::HostFailed).with_source(message))
    }

    /// Check and push what `function`, the program's `f`, returned.
    fn returned(&mut self, f: u32, function: &HostFunction, value: &Value) -> Result<(), RunError> {
        let types = &self.program.types;
        let needed = memory::measure(types, value, function.ret, Root::Returned(f))?;

        self.spend(needed)?;
        let left = self.left();

        if needed > left {
            return Err(RunError::new(Message::TooLarge {
                needed: count(needed),
                left: count(left),
            }));
        }

        let width = types.width(function.ret);
        let dest = self.reserve(width)?;
        memory::write(self, types, value, function.ret, dest)
    }

    /// The error of reading the value whose slot starts at word `at`,
    /// which is not set.
    #[cold]
    #[inline(never)]
    fn unset(&self, at: usize) -> RunError {
        let global = self
            .program
            .globals
            .iter()
            .position(|g| usize::try_from(g.slot).is_ok_and(|slot| slot == at))
            .and_then(|n| u32::try_from(n).ok());

        match global {
            Some(global) => RunError::new(Message::Unset { global }),
            None => fault(Fault::OutsideMemory),
        }
    }

    /// The words left for returned values.
    fn left(&self) -> usize {
        self.limit
            .saturating_sub(self.len)
            .saturating_sub(self.heap)
    }

    fn spend(&mut self, steps: usize) -> Result<(), RunError> {
        self.fuel.spend(steps)
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

    /// Where the word `n` words below the top of the stack is, `n` from 1.
    fn below(&self, n: usize) -> Result<usize, RunError> {
        self.sp
            .checked_sub(n)
            .and_then(|at| self.stack_base.checked_add(at))
            .ok_or_else(|| fault(Fault::EmptyStack))
    }

    /// Drop the top `n` words of the stack.
    fn drop_words(&mut self, n: usize) -> Result<(), RunError> {
        self.sp = self
            .sp
            .checked_sub(n)
            .ok_or_else(|| fault(Fault::EmptyStack))?;
        Ok(())
    }

    /// Move the `n` words at `from` down to `to`, below them on the stack,
    /// and make them its top.
    fn move_down(&mut self, from: usize, to: usize, n: usize) -> Result<(), RunError> {
        if to > from {
            return Err(fault(Fault::OutsideMemory));
        }

        self.copy(from, n, to)?;
        self.sp = to
            .checked_add(n)
            .and_then(|top| top.checked_sub(self.stack_base))
            .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
        Ok(())
    }

    /// Push an absent tagged option whose value is `width` words: its tag,
    /// then zeros where the value would be.
    fn push_absent(&mut self, width: usize) -> Result<(), RunError> {
        self.push(Word::present(false))?;

        for _ in 0..width {
            self.push(Word::int(0))?;
        }

        Ok(())
    }

    /// Push `word`. The stack is in the machine's words, so it is
    /// written there without asking which memory holds it.
    #[inline]
    fn push(&mut self, word: Word) -> Result<(), RunError> {
        let at = self.reserve(1)?;
        *at.checked_sub(self.base)
            .and_then(|at| self.words.get_mut(at))
            .ok_or_else(|| fault(Fault::StackOutsideMemory))? = word;
        Ok(())
    }

    /// Push the `n` words from `from`.
    fn push_from(&mut self, from: usize, n: usize) -> Result<(), RunError> {
        memory::inside(self, from, Some(n))?;
        let at = self.reserve(n)?;
        self.copy(from, n, at)
    }

    /// Copy the `n` words from `from`, all in the store or all in the
    /// machine, to `to` in the machine.
    fn copy(&mut self, from: usize, n: usize, to: usize) -> Result<(), RunError> {
        let outside = || fault(Fault::OutsideMemory);
        let to = to
            .checked_sub(self.base)
            .ok_or_else(|| fault(Fault::WriteToStore))?;
        let to_end = to.checked_add(n).ok_or_else(outside)?;

        match from.checked_sub(self.base) {
            Some(from) => {
                let end = from
                    .checked_add(n)
                    .filter(|end| *end <= self.words.len() && to_end <= self.words.len())
                    .ok_or_else(outside)?;
                self.words.copy_within(from..end, to);
            }
            None => {
                let end = from.checked_add(n).ok_or_else(outside)?;
                let source = self.store.get(from..end).ok_or_else(outside)?;
                let dest = self.words.get_mut(to..to_end).ok_or_else(outside)?;
                dest.copy_from_slice(source);
            }
        }

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
        // The stack is in the machine's words.
        let word = self
            .stack_base
            .checked_add(sp)
            .and_then(|at| at.checked_sub(self.base))
            .and_then(|at| self.words.get(at))
            .copied()
            .ok_or_else(|| fault(Fault::StackOutsideMemory))?;
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
    #[inline]
    fn get(&self, at: Option<usize>) -> Result<Word, RunError> {
        memory::get(self, at)
    }

    #[inline]
    fn put(&mut self, at: Option<usize>, word: Word) -> Result<(), RunError> {
        memory::put(self, at, word)
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
}

impl Words for Run<'_> {
    #[inline]
    fn word(&self, at: usize) -> Option<Word> {
        match at.checked_sub(self.base) {
            Some(at) => self.words.get(at).copied(),
            None => self.store.get(at).copied(),
        }
    }

    fn end(&self) -> usize {
        self.base.saturating_add(self.words.len())
    }
}

impl WordsMut for Run<'_> {
    #[inline]
    fn set_word(&mut self, at: usize, word: Word) -> Option<()> {
        *self.words.get_mut(at.checked_sub(self.base)?)? = word;
        Some(())
    }

    fn alloc(&mut self, n: usize) -> Result<usize, RunError> {
        Run::alloc(self, n)
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

fn index<T>(items: &[T], at: u32) -> Option<&T> {
    items.get(usize::try_from(at).ok()?)
}

fn size(n: u32) -> Result<usize, RunError> {
    usize::try_from(n).map_err(|_| fault(Fault::PastAddressSpace))
}
