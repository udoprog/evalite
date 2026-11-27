//! The type checker and compiler: one iterative walk over a parse tree
//! that gives every expression its type and emits its code.

use std::collections::HashMap;

use syntree::{Node, Span};

use crate::Type;
use crate::debug::DebugInfo;
use crate::env::{Environment, Overload};
use crate::error::{Arity, Callee, Diagnostic, Diagnostics, ErrorKind, Reporter};
use crate::lexer::parse_int;
use crate::message::Key;
use crate::methods::{self, Associated, MethodSig, PathError};
use crate::parser::{Options, Parsed, parse_with};
use crate::program::{Constant, Global, HostFunction, Instr, Program, Scalar};
use crate::references::{Literal, Recorder, Reference, ReferenceKind};
use crate::sig::{Mismatch, Shape};
use crate::syntax::{Syntax, callee, unescape};
use crate::types::{TypeId, TypeNode, Types};

type N<'a> = Node<'a, Syntax, crate::parser::Flavor>;

/// Parse `source` and compile it against `env`, with the default
/// [`Options`].
///
/// # Errors
///
/// Returns [`Diagnostics`] when `source` does not parse, nests deeper than
/// `max_depth`, uses a name `env` does not declare, or does not type check:
/// every syntax, name and type error, in source order, each reported once.
/// A syntax error is recovered from, and what is around it still checked.
/// The [`labels`](crate::Diagnostic::labels) of each point into `source`.
///
/// # Examples
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("wits", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
///
/// let program = evalite::compile("Int::min(wits, 3) + bonus.unwrap_or(0)", &env)?;
/// assert_eq!(program.result_type(), Type::int());
///
/// let errors = evalite::compile("wits + bonus", &env).unwrap_err();
/// assert_eq!(
///     errors.to_string(),
///     "`+` takes Int and Int, found Int and Option<Int>, unwrap the optional value first with `.unwrap_or(...)`"
/// );
///
/// let errors = evalite::compile("Int::min(wits, true) + nothing", &env).unwrap_err();
/// assert_eq!(
///     errors.to_string(),
///     "argument 2 of `Int::min` must be Int, found Bool\nunknown name `nothing`"
/// );
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
pub fn compile(source: &str, env: &Environment) -> Result<Program, Diagnostics> {
    compile_with(source, env, &Options::new())
}

/// Parse `source` with `options` and compile it against `env`.
///
/// # Errors
///
/// As [`compile`]: [`Diagnostics`] when `source` does not parse, nests
/// deeper than `options` allows, or does not type check against `env`.
pub fn compile_with(
    source: &str,
    env: &Environment,
    options: &Options,
) -> Result<Program, Diagnostics> {
    Ok(compile_parsed(source, parse_with(source, options)?, env)?.0)
}

/// Parse `source` with `options` and compile it against `env`, also giving
/// the [`DebugInfo`] that says where each instruction came from, for a
/// [`RunError`](crate::RunError)'s labels.
///
/// # Examples
///
/// ```
/// use evalite::{Environment, Machine, Options, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("bonus", Type::int())?;
/// let source = "1 + 9223372036854775807 + bonus";
/// let (program, debug) = evalite::compile_debug(source, &env, &Options::new())?;
///
/// let mut machine = Machine::default();
/// let mut run = machine.load(&program)?;
/// run.bind("bonus", &Value::int(0))?;
/// let error = run.run().unwrap_err();
///
/// let span = error.labels(&program, Some(&debug)).next().and_then(|l| l.span());
/// assert_eq!(span.map(|s| &source[s.range()]), Some("1 + 9223372036854775807"));
///
/// assert!(evalite::compile_debug("1 +", &env, &Options::new()).is_err());
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// The same as [`compile_with`].
pub fn compile_debug(
    source: &str,
    env: &Environment,
    options: &Options,
) -> Result<(Program, DebugInfo), Diagnostics> {
    compile_parsed(source, parse_with(source, options)?, env)
}

/// Compile what [`parse_with`] made of `source`, with its debug
/// information. Its syntax errors are reported with the type errors, in
/// source order, and its `Error` nodes check as anything.
pub(crate) fn compile_parsed(
    source: &str,
    parsed: Parsed,
    env: &Environment,
) -> Result<(Program, DebugInfo), Diagnostics> {
    let Parsed { tree, reporter } = parsed;
    let mut compiler = Compiler::new(source, env, reporter).map_err(Diagnostics::one)?;

    let root = tree
        .first()
        .and_then(|root| exprs(root).next())
        .ok_or_else(|| Diagnostics::one(internal("a tree without an expression")))?;

    if let Err(fatal) = compiler.run(root) {
        compiler.reporter.report(fatal);
    }

    compiler.finish()
}

/// The references of what [`parse_with`] made of `source`, recorded as it
/// compiles, once it compiles.
pub(crate) fn references_parsed(
    source: &str,
    parsed: Parsed,
    env: &Environment,
) -> Result<Vec<Reference>, Diagnostics> {
    let Parsed { tree, reporter } = parsed;
    let mut compiler = Compiler::new(source, env, reporter).map_err(Diagnostics::one)?;
    compiler.recorder = Some(Recorder::default());

    let root = tree
        .first()
        .and_then(|root| exprs(root).next())
        .ok_or_else(|| Diagnostics::one(internal("a tree without an expression")))?;

    if let Err(fatal) = compiler.run(root) {
        compiler.reporter.report(fatal);
    }

    let recorder = compiler.recorder.take();
    compiler.finish()?;
    Ok(recorder.map(Recorder::finish).unwrap_or_default())
}

enum Work<'a> {
    Enter(N<'a>),
    /// After a unary operator's operand, whose code starts at `usize`.
    Unary(N<'a>, usize),
    Binary(N<'a>),
    /// After `&&` or `||`'s left operand.
    Logic(N<'a>),
    /// After `&&` or `||`'s right operand.
    LogicExit(N<'a>),
    Paren(N<'a>),
    /// After a call's arguments, with the reference it starts, if
    /// references are recorded.
    Call(N<'a>, &'a [Overload], String, Option<u32>),
    /// After the arguments of a call of a path, such as `Int::min(...)`.
    Path(N<'a>, Associated, String),
    /// After a method's receiver.
    Method(N<'a>),
    /// After a method's arguments.
    MethodExit(N<'a>, &'static MethodSig, Callee),
    Field(N<'a>),
    /// After an unwrap's operand.
    Unwrap(N<'a>),
    /// After an index's receiver.
    Index(N<'a>),
    /// After a list's index or a map's key.
    IndexExit(N<'a>),
    /// After a list's entries, whose code starts at `start`.
    List {
        node: N<'a>,
        start: usize,
    },
    /// After a map's values, in key order: `order[n]` is where the `n`th
    /// of them is written, counting from 0. Its keys are the constant list
    /// `keys` of type `key`.
    Map {
        node: N<'a>,
        order: Vec<usize>,
        keys: u32,
        key: TypeId,
    },
    /// After the parts of an expression already found to be in error: drop
    /// the `usize` values they left and give the expression the error
    /// type.
    Discard(N<'a>, usize),
}

impl<'a> Work<'a> {
    /// The node it is about.
    fn node(&self) -> N<'a> {
        match self {
            Work::Enter(node)
            | Work::Unary(node, _)
            | Work::Binary(node)
            | Work::Logic(node)
            | Work::LogicExit(node)
            | Work::Paren(node)
            | Work::Call(node, ..)
            | Work::Path(node, ..)
            | Work::Method(node)
            | Work::MethodExit(node, ..)
            | Work::Field(node)
            | Work::Index(node)
            | Work::IndexExit(node)
            | Work::Unwrap(node)
            | Work::List { node, .. }
            | Work::Map { node, .. }
            | Work::Discard(node, _) => *node,
        }
    }
}

/// A value the code so far leaves on the stack.
#[derive(Debug, Clone, Copy)]
struct Value {
    ty: TypeId,
    span: Span<u32>,
    /// The reference of the [`Recorder`] it is a part of, which what
    /// reads it extends.
    path: Option<u32>,
}

struct Compiler<'a> {
    source: &'a str,
    env: &'a Environment,
    types: Types,
    int: TypeId,
    bool: TypeId,
    constants: Vec<Constant>,
    strings: HashMap<String, u32>,
    globals: Vec<Global>,
    /// Where each global used so far is held, in words, and its type.
    global_ids: HashMap<String, (u32, TypeId)>,
    functions: Vec<HostFunction>,
    function_ids: HashMap<(String, Vec<TypeId>), u32>,
    code: Vec<Instr>,
    /// The span of each instruction of `code`.
    spans: Vec<Span<u32>>,
    /// The span of the expression whose code is being emitted.
    span: Span<u32>,
    /// The words the globals so far take.
    global_words: usize,
    values: Vec<Value>,
    /// `&&` and `||` jumps waiting for their right operands.
    jumps: Vec<usize>,
    /// The type of an expression in error, which fits everywhere and is
    /// never reported again. It is no type of `types`, so it never reaches
    /// a program.
    error: TypeId,
    reporter: Reporter,
    /// The references so far, when they are asked for.
    recorder: Option<Recorder>,
}

impl<'a> Compiler<'a> {
    fn new(source: &'a str, env: &'a Environment, reporter: Reporter) -> Result<Self, Diagnostic> {
        let mut types = Types::new();
        let int = types.add(TypeNode::Int).ok_or_else(|| internal("no Int"))?;
        let bool = types
            .add(TypeNode::Bool)
            .ok_or_else(|| internal("no Bool"))?;

        Ok(Self {
            source,
            env,
            types,
            int,
            bool,
            constants: Vec::new(),
            strings: HashMap::new(),
            globals: Vec::new(),
            global_ids: HashMap::new(),
            functions: Vec::new(),
            function_ids: HashMap::new(),
            code: Vec::new(),
            spans: Vec::new(),
            span: Span::point(0),
            global_words: 0,
            values: Vec::new(),
            jumps: Vec::new(),
            error: TypeId(u32::MAX),
            reporter,
            recorder: None,
        })
    }

    fn text(&self, node: N<'_>) -> &'a str {
        self.source.get(node.range()).unwrap_or_default()
    }

    fn ty(&self, id: TypeId) -> Type {
        self.types.to_type(id).unwrap_or(Type::int())
    }

    fn node(&mut self, node: TypeNode, at: Span<u32>) -> Result<TypeId, Diagnostic> {
        self.types
            .add(node)
            .ok_or_else(|| Diagnostic::new(at, ErrorKind::TypeTooDeep))
    }

    fn intern(&mut self, ty: &Type, at: Span<u32>) -> Result<TypeId, Diagnostic> {
        self.types
            .intern(ty)
            .ok_or_else(|| Diagnostic::new(at, ErrorKind::TypeTooDeep))
    }

    /// Emit `instr`. The memory the code uses is the verifier's to count
    /// when the program is finished.
    fn emit(&mut self, instr: Instr) {
        self.code.push(instr);
        self.spans.push(self.span);
    }

    /// Drop the code from `start`.
    fn truncate(&mut self, start: usize) {
        self.code.truncate(start);
        self.spans.truncate(start);
    }

    fn push(&mut self, ty: TypeId, span: Span<u32>) {
        self.push_path(ty, span, None);
    }

    /// Push a value that is part of the reference `path`.
    fn push_path(&mut self, ty: TypeId, span: Span<u32>, path: Option<u32>) {
        self.values.push(Value { ty, span, path });
    }

    /// Start a reference at `name`, when references are recorded.
    fn root(
        &mut self,
        kind: ReferenceKind,
        name: &str,
        span: Span<u32>,
        at: Span<u32>,
    ) -> Option<u32> {
        self.recorder.as_mut()?.root(kind, name, span, at)
    }

    /// Extend the reference `path` of a receiver by indexing it with the
    /// expression `index`, a map's key when `map`.
    fn record_index(&mut self, path: Option<u32>, map: bool, index: N<'_>) {
        let (Some(path), Some(recorder)) = (path, self.recorder.as_mut()) else {
            return;
        };

        let (literal, span) = literal(index, self.source);
        recorder.index(path, map, literal, span);
    }

    fn pop(&mut self) -> Result<Value, Diagnostic> {
        self.values
            .pop()
            .ok_or_else(|| internal("no value where one was expected"))
    }

    /// The last `n` values, in order.
    fn pop_n(&mut self, n: usize) -> Result<Vec<Value>, Diagnostic> {
        let at = self
            .values
            .len()
            .checked_sub(n)
            .ok_or_else(|| internal("fewer values than expected"))?;
        Ok(self.values.split_off(at))
    }

    /// Whether any of `values` is in error.
    fn any_error(&self, values: &[Value]) -> bool {
        values.iter().any(|v| v.ty == self.error)
    }

    /// Report `diagnostic` and continue past it.
    fn report(&mut self, diagnostic: Diagnostic) {
        self.reporter.report(diagnostic);
    }

    /// Check and compile the expression at `root`, reporting each error and
    /// continuing past it. Only an error in evalite itself, or in the size
    /// of the program, stops it.
    fn run(&mut self, root: N<'a>) -> Result<(), Diagnostic> {
        let mut work = vec![Work::Enter(root)];

        while let Some(item) = work.pop() {
            self.span = *item.node().span();

            // Every step that fails has taken the values of its parts and
            // left none of its own, so the expression it is about gets the
            // error type.
            if let Err(error) = self.step(item, &mut work) {
                if error.is_fatal() {
                    return Err(error);
                }

                self.report(error);
                self.push(self.error, self.span);
            }
        }

        Ok(())
    }

    fn step(&mut self, item: Work<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Diagnostic> {
        {
            match item {
                Work::Enter(node) => self.enter(node, work)?,
                Work::Unary(node, start) => self.unary(node, start)?,
                Work::Binary(node) => self.binary(node)?,
                Work::Logic(node) => {
                    let instr = match operator(node) {
                        Some(Syntax::AndAnd) => Instr::And(0),
                        _ => Instr::Or(0),
                    };

                    self.jumps.push(self.code.len());
                    self.emit(instr);
                }
                Work::LogicExit(node) => self.logic(node)?,
                Work::Paren(node) => {
                    let value = self
                        .values
                        .last_mut()
                        .ok_or_else(|| internal("an empty parenthesis"))?;
                    value.span = *node.span();
                }
                Work::Call(node, overloads, name, path) => {
                    self.call(node, overloads, name, path)?;
                }
                Work::Path(node, associated, path) => self.path(node, associated, path)?,
                Work::Method(node) => self.method(node, work)?,
                Work::MethodExit(node, method, callee) => self.method_exit(node, method, callee)?,
                Work::Field(node) => self.field(node)?,
                Work::Index(node) => self.index(node, work)?,
                Work::IndexExit(node) => self.index_exit(node)?,
                Work::Unwrap(node) => self.unwrap(node)?,
                Work::List { node, start } => self.list(node, start)?,
                Work::Map {
                    node,
                    order,
                    keys,
                    key,
                } => self.map_exit(node, &order, keys, key)?,
                Work::Discard(node, n) => {
                    self.pop_n(n)?;
                    self.push(self.error, *node.span());
                }
            }
        }

        Ok(())
    }

    fn enter(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Diagnostic> {
        let span = *node.span();
        let mut parts = exprs(node);

        match node.value() {
            Syntax::Integer => {
                let (value, _) = parse_int(self.text(node), false)
                    .ok_or_else(|| Diagnostic::new(span, ErrorKind::IntegerTooLarge))?;
                self.emit(Instr::Int(value));
                self.push(self.int, span);
            }
            Syntax::Bool => {
                let value = self.text(node) == "true";
                self.emit(Instr::Bool(value));
                self.push(self.bool, span);
            }
            Syntax::String => {
                let text = unescape(self.text(node))
                    .ok_or_else(|| internal("a malformed string literal"))?;
                let c = self.string(text)?;
                let ty = self.node(TypeNode::String, span)?;
                self.emit(Instr::Const(c));
                self.push(ty, span);
            }
            Syntax::Name => {
                let name = self.text(node);
                let (at, ty) = self.global(name, span)?;
                self.emit(Instr::Global { at, ty });
                let path = self.root(ReferenceKind::Value, name, span, span);
                self.push_path(ty, span, path);
            }
            Syntax::Paren => {
                let inner = parts.next().ok_or_else(|| internal("an empty `(`"))?;
                work.push(Work::Paren(node));
                work.push(Work::Enter(inner));
            }
            Syntax::Unary => {
                let operand = parts.next().ok_or_else(|| internal("a lone operator"))?;
                work.push(Work::Unary(node, self.code.len()));
                work.push(Work::Enter(operand));
            }
            Syntax::Binary => {
                let (Some(left), Some(right)) = (parts.next(), parts.next()) else {
                    return Err(internal("a binary operator without two operands"));
                };

                if matches!(operator(node), Some(Syntax::AndAnd | Syntax::OrOr)) {
                    work.push(Work::LogicExit(node));
                    work.push(Work::Enter(right));
                    work.push(Work::Logic(node));
                } else {
                    work.push(Work::Binary(node));
                    work.push(Work::Enter(right));
                }

                work.push(Work::Enter(left));
            }
            Syntax::Call => {
                let (name, text) =
                    callee(node, self.source).ok_or_else(|| internal("a call without a name"))?;

                let found = if name.value() == Syntax::Path {
                    self.find_path(name)
                        .map(|associated| Work::Path(node, associated, text))
                } else {
                    match self.function(&text, *name.span()) {
                        Ok(overloads) => {
                            let path =
                                self.root(ReferenceKind::Function, &text, *name.span(), span);
                            Ok(Work::Call(node, overloads, text, path))
                        }
                        Err(error) => Err(error),
                    }
                };

                match found {
                    Ok(found) => work.push(found),
                    Err(error) if error.is_fatal() => return Err(error),
                    Err(error) => {
                        self.report(error);
                        work.push(Work::Discard(node, args_of(node).len()));
                    }
                }

                push_args(work, node);
            }
            Syntax::Method | Syntax::Field | Syntax::Index => {
                let receiver = parts.next().ok_or_else(|| internal("no receiver"))?;

                work.push(match node.value() {
                    Syntax::Method => Work::Method(node),
                    Syntax::Field => Work::Field(node),
                    _ => Work::Index(node),
                });
                work.push(Work::Enter(receiver));
            }
            Syntax::Unwrap => {
                let operand = parts.next().ok_or_else(|| internal("no operand"))?;
                work.push(Work::Unwrap(node));
                work.push(Work::Enter(operand));
            }
            Syntax::Error => {
                // Its error is reported; what it holds is checked, but
                // nothing about it.
                let parts = parts.collect::<Vec<_>>();
                work.push(Work::Discard(node, parts.len()));
                work.extend(parts.into_iter().rev().map(Work::Enter));
            }
            Syntax::List => {
                work.push(Work::List {
                    node,
                    start: self.code.len(),
                });
                let entries = parts.collect::<Vec<_>>();
                work.extend(entries.into_iter().rev().map(Work::Enter));
            }
            Syntax::Map => self.map(node, work)?,
            kind => return Err(internal(format!("{kind:?} is not an expression"))),
        }

        Ok(())
    }

    fn string(&mut self, text: String) -> Result<u32, Diagnostic> {
        if let Some(c) = self.strings.get(&text) {
            return Ok(*c);
        }

        let c = count(self.constants.len())?;
        self.constants.push(Constant::String(text.clone()));
        self.strings.insert(text, c);
        Ok(c)
    }

    /// Where the global `name` is held, in words, and its type. A global
    /// is interned the first time it is used.
    fn global(&mut self, name: &str, at: Span<u32>) -> Result<(u32, TypeId), Diagnostic> {
        if let Some(found) = self.global_ids.get(name) {
            return Ok(*found);
        }

        let Some(ty) = self.env.values.get(name) else {
            let kind = if self.env.functions.contains_key(name) {
                ErrorKind::FunctionAsValue(name.to_owned())
            } else {
                ErrorKind::UnknownName {
                    name: name.to_owned(),
                    suggestion: suggest(name, self.env.values.keys()),
                }
            };

            return Err(Diagnostic::new(at, kind));
        };

        let ty = self.intern(ty, at)?;
        let g = count(self.global_words)?;
        self.global_words = self.global_words.saturating_add(self.types.width(ty));
        self.globals.push(Global {
            name: name.to_owned(),
            ty,
        });
        self.global_ids.insert(name.to_owned(), (g, ty));
        Ok((g, ty))
    }

    fn function(&self, name: &str, at: Span<u32>) -> Result<&'a [Overload], Diagnostic> {
        let env = self.env;

        match env.functions.get(name) {
            Some(overloads) if !overloads.is_empty() => Ok(overloads),
            _ if env.values.contains_key(name) => Err(Diagnostic::new(
                at,
                ErrorKind::NotAFunction(name.to_owned()),
            )),
            _ => Err(Diagnostic::new(
                at,
                ErrorKind::UnknownFunction {
                    name: name.to_owned(),
                    suggestion: suggest(name, env.functions.keys()),
                },
            )),
        }
    }

    fn find_path(&self, path: N<'_>) -> Result<Associated, Diagnostic> {
        let mut names = path.children().filter(|n| n.value() == Syntax::NameToken);

        let (Some(ty), Some(name)) = (names.next(), names.next()) else {
            return Err(internal("a path without two names"));
        };

        methods::find_path(self.text(ty), self.text(name)).map_err(|error| match error {
            PathError::UnknownType => Diagnostic::new(
                *ty.span(),
                ErrorKind::UnknownPathType(self.text(ty).to_owned()),
            ),
            PathError::Unknown(ty, available) => Diagnostic::new(
                *name.span(),
                ErrorKind::UnknownAssociated {
                    ty,
                    name: self.text(name).to_owned(),
                    available,
                },
            ),
        })
    }

    fn unary(&mut self, node: N<'a>, start: usize) -> Result<(), Diagnostic> {
        let op = operator(node).ok_or_else(|| internal("a unary without an operator"))?;
        let value = self.pop()?;
        let span = *node.span();

        let (want, instr) = match op {
            Syntax::Minus => (self.int, Instr::Neg),
            _ => (self.bool, Instr::Not),
        };

        if value.ty == self.error {
            self.push(self.error, span);
            return Ok(());
        }

        if value.ty != want {
            return Err(Diagnostic::new(
                span,
                ErrorKind::Operator {
                    op,
                    operands: vec![self.ty(value.ty)],
                },
            ));
        }

        // `-12` is a constant, so lists of negative numbers fold too.
        let folded = match self.code.get(start..) {
            Some([Instr::Int(n)]) if op == Syntax::Minus => n.checked_neg(),
            _ => None,
        };

        match folded {
            Some(n) => {
                self.truncate(start);
                self.emit(Instr::Int(n));
            }
            None => self.emit(instr),
        }

        self.push(want, span);
        Ok(())
    }

    fn binary(&mut self, node: N<'a>) -> Result<(), Diagnostic> {
        let op = operator(node).ok_or_else(|| internal("a binary without an operator"))?;
        let right = self.pop()?;
        let left = self.pop()?;
        let span = *node.span();

        let (instr, result) = match op {
            Syntax::Plus => (Instr::Add, self.int),
            Syntax::Minus => (Instr::Sub, self.int),
            Syntax::Star => (Instr::Mul, self.int),
            Syntax::Slash => (Instr::Div, self.int),
            Syntax::Percent => (Instr::Rem, self.int),
            Syntax::Lt => (Instr::Lt, self.bool),
            Syntax::Le => (Instr::Le, self.bool),
            Syntax::Gt => (Instr::Gt, self.bool),
            Syntax::Ge => (Instr::Ge, self.bool),
            Syntax::EqEq => (Instr::Eq, self.bool),
            Syntax::NotEq => (Instr::Ne, self.bool),
            op => return Err(internal(format!("{op:?} is not a binary operator"))),
        };

        if self.any_error(&[left, right]) {
            self.push(self.error, span);
            return Ok(());
        }

        let fits = match op {
            Syntax::EqEq | Syntax::NotEq => {
                left.ty == right.ty && (left.ty == self.int || left.ty == self.bool)
            }
            _ => left.ty == self.int && right.ty == self.int,
        };

        if !fits {
            return Err(Diagnostic::new(
                span,
                ErrorKind::Operator {
                    op,
                    operands: vec![self.ty(left.ty), self.ty(right.ty)],
                },
            ));
        }

        self.emit(instr);
        self.push(result, span);
        Ok(())
    }

    fn logic(&mut self, node: N<'a>) -> Result<(), Diagnostic> {
        let op = operator(node).ok_or_else(|| internal("a binary without an operator"))?;
        let right = self.pop()?;
        let left = self.pop()?;
        let span = *node.span();

        let jump = self
            .jumps
            .pop()
            .ok_or_else(|| internal("no jump to patch"))?;

        let skip = self
            .code
            .len()
            .checked_sub(jump)
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| internal("a jump after its target"))?;
        let skip = count(skip)?;

        match self.code.get_mut(jump) {
            Some(Instr::And(n) | Instr::Or(n)) => *n = skip,
            _ => return Err(internal("the jump to patch is gone")),
        }

        if self.any_error(&[left, right]) {
            self.push(self.error, span);
            return Ok(());
        }

        if left.ty != self.bool || right.ty != self.bool {
            return Err(Diagnostic::new(
                span,
                ErrorKind::Operator {
                    op,
                    operands: vec![self.ty(left.ty), self.ty(right.ty)],
                },
            ));
        }

        self.push(self.bool, span);
        Ok(())
    }

    fn call(
        &mut self,
        node: N<'a>,
        overloads: &'a [Overload],
        name: String,
        path: Option<u32>,
    ) -> Result<(), Diagnostic> {
        let args = args_of(node);
        let values = self.pop_n(args.len())?;
        let found = values.iter().map(|v| v.ty).collect::<Vec<_>>();
        let span = *node.span();

        if self.any_error(&values) {
            self.push(self.error, span);
            return Ok(());
        }

        let mut first = None;

        for overload in overloads {
            match self.overload(overload, &found, span)? {
                Ok(resolved) => {
                    self.emit_call(&name, resolved, &found, span)?;

                    if let Some(value) = self.values.last_mut() {
                        value.path = path;
                    }

                    return Ok(());
                }
                Err(mismatch) => {
                    first.get_or_insert(mismatch);
                }
            }
        }

        let callee = Callee::Function(name.clone());

        match (overloads, first) {
            ([_], Some(mismatch)) => Err(self.mismatch(mismatch, callee, &values, &args, node)),
            _ => Err(Diagnostic::new(
                span,
                ErrorKind::NoOverload {
                    function: name,
                    found: found.iter().map(|t| self.ty(*t)).collect(),
                    candidates: overloads.iter().map(Overload::describe).collect(),
                },
            )),
        }
    }

    /// The return type of `overload` called with `found`, or why it does
    /// not take them.
    fn overload(
        &mut self,
        overload: &Overload,
        found: &[TypeId],
        at: Span<u32>,
    ) -> Result<Result<TypeId, Mismatch>, Diagnostic> {
        if overload.params.len() != found.len() {
            return Ok(Err(Mismatch::Count(Arity::Exactly(overload.params.len()))));
        }

        for (n, (param, found)) in overload.params.iter().zip(found).enumerate() {
            if self.intern(param, at)? != *found {
                return Ok(Err(Mismatch::Argument(n, param.to_string())));
            }
        }

        Ok(Ok(self.intern(&overload.ret, at)?))
    }

    fn emit_call(
        &mut self,
        name: &str,
        ret: TypeId,
        found: &[TypeId],
        span: Span<u32>,
    ) -> Result<(), Diagnostic> {
        let key = (name.to_owned(), found.to_vec());

        let f = match self.function_ids.get(&key) {
            Some(f) => *f,
            None => {
                let f = count(self.functions.len())?;
                self.functions.push(HostFunction {
                    name: name.to_owned(),
                    params: found.to_vec(),
                    ret,
                });
                self.function_ids.insert(key, f);
                f
            }
        };

        self.emit(Instr::Call(f));
        self.push(ret, span);
        Ok(())
    }

    /// After the arguments of a call of the path `path`.
    fn path(
        &mut self,
        node: N<'a>,
        associated: Associated,
        path: String,
    ) -> Result<(), Diagnostic> {
        let args = args_of(node);
        let values = self.pop_n(args.len())?;
        let span = *node.span();

        if self.any_error(&values) {
            self.push(self.error, span);
            return Ok(());
        }

        let found = values.iter().map(|v| self.ty(v.ty)).collect::<Vec<_>>();
        let associated = associated.for_receiver(found.first());

        let ret = match associated.call(&found) {
            Ok(ret) => self.intern(&ret, span)?,
            Err(mismatch) => {
                let callee = Callee::Function(path);
                return Err(self.mismatch(mismatch, callee, &values, &args, node));
            }
        };

        let mut reference = None;

        let instr = match associated {
            Associated::Function(function) => Some((function.instr)(count(values.len())?)),
            Associated::Method(method) => {
                let receiver = values
                    .first()
                    .ok_or_else(|| internal("a method's path without a receiver"))?;
                reference = self.continue_path(method, receiver.path, args.get(1).copied());
                let held = match self.types.get(receiver.ty) {
                    Some(TypeNode::List(held) | TypeNode::Option(held)) => Some(*held),
                    _ => None,
                };
                (method.instr)(receiver.ty, held)
            }
        };

        let instr =
            instr.ok_or_else(|| internal(format_args!("`{path}` without an instruction")))?;
        self.emit(instr);
        self.push_path(ret, span, reference);
        Ok(())
    }

    /// The reference that calling `method` on a receiver that is part of
    /// `path` is still part of, with `arg` its first argument: through
    /// `unwrap_or`, and a list's or map's `get`, which is a step.
    fn continue_path(
        &mut self,
        method: &MethodSig,
        path: Option<u32>,
        arg: Option<N<'_>>,
    ) -> Option<u32> {
        match (method.receiver, method.name, arg) {
            (Shape::Option(_), "unwrap_or", _) => path,
            (Shape::List(_), "get", Some(arg)) => {
                self.record_index(path, false, arg);
                path
            }
            (Shape::Map(..), "get", Some(arg)) => {
                self.record_index(path, true, arg);
                path
            }
            _ => None,
        }
    }

    fn mismatch(
        &self,
        mismatch: Mismatch,
        callee: Callee,
        values: &[Value],
        args: &[N<'_>],
        node: N<'_>,
    ) -> Diagnostic {
        match mismatch {
            Mismatch::Count(expected) => {
                let at = node
                    .children()
                    .find(|n| n.value() == Syntax::Args)
                    .map_or(*node.span(), |n| *n.span());

                Diagnostic::new(
                    at,
                    ErrorKind::ArgumentCount {
                        callee,
                        expected,
                        found: args.len(),
                    },
                )
            }
            Mismatch::Argument(n, expected) => match values.get(n) {
                Some(value) => Diagnostic::new(
                    value.span,
                    ErrorKind::ArgumentType {
                        callee,
                        index: n.saturating_add(1),
                        expected,
                        found: self.ty(value.ty),
                    },
                ),
                None => internal("a mismatch past the arguments"),
            },
        }
    }

    fn method(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Diagnostic> {
        let receiver = *self
            .values
            .last()
            .ok_or_else(|| internal("a method without a receiver"))?;
        let name_node = name_of(node).ok_or_else(|| internal("a method without a name"))?;
        let name = self.text(name_node);

        if receiver.ty == self.error {
            work.push(Work::Discard(node, args_of(node).len().saturating_add(1)));
            push_args(work, node);
            return Ok(());
        }

        let receiver_type = self.ty(receiver.ty);

        let Some(method) = methods::find(&receiver_type, name) else {
            self.report(Diagnostic::new(
                *name_node.span(),
                ErrorKind::UnknownMethod {
                    available: methods::method_names(&receiver_type),
                    receiver: receiver_type,
                    method: name.to_owned(),
                },
            ));
            work.push(Work::Discard(node, args_of(node).len().saturating_add(1)));
            push_args(work, node);
            return Ok(());
        };

        let callee = Callee::Method {
            receiver: receiver_type,
            name: name.to_owned(),
        };

        work.push(Work::MethodExit(node, method, callee));
        push_args(work, node);
        Ok(())
    }

    fn method_exit(
        &mut self,
        node: N<'a>,
        method: &MethodSig,
        callee: Callee,
    ) -> Result<(), Diagnostic> {
        let args = args_of(node);
        let values = self.pop_n(args.len())?;
        let receiver = self.pop()?;
        let span = *node.span();

        if self.any_error(&values) {
            self.push(self.error, span);
            return Ok(());
        }

        let found = values.iter().map(|v| self.ty(v.ty)).collect::<Vec<_>>();

        let ret = match method.call(&self.ty(receiver.ty), &found) {
            Ok(ret) => self.intern(&ret, span)?,
            Err(mismatch) => return Err(self.mismatch(mismatch, callee, &values, &args, node)),
        };

        let held = match self.types.get(receiver.ty) {
            Some(TypeNode::List(held) | TypeNode::Option(held)) => Some(*held),
            _ => None,
        };
        let instr = (method.instr)(receiver.ty, held).ok_or_else(|| {
            internal(format_args!(
                "`{}` of a {}",
                method.name,
                self.ty(receiver.ty)
            ))
        })?;
        self.emit(instr);
        let path = self.continue_path(method, receiver.path, args.first().copied());
        self.push_path(ret, span, path);
        Ok(())
    }

    fn field(&mut self, node: N<'a>) -> Result<(), Diagnostic> {
        let receiver = self.pop()?;
        let name_node = name_of(node).ok_or_else(|| internal("a field without a name"))?;
        let name = self.text(name_node);
        self.read_field(receiver, name, *name_node.span(), *node.span(), true)
    }

    /// Read the field `name` of `receiver`, `at` naming it.
    fn read_field(
        &mut self,
        receiver: Value,
        name: &str,
        at: Span<u32>,
        span: Span<u32>,
        dotted: bool,
    ) -> Result<(), Diagnostic> {
        if receiver.ty == self.error {
            self.push(self.error, span);
            return Ok(());
        }

        let Some(TypeNode::Record(fields)) = self.types.get(receiver.ty) else {
            let receiver = self.ty(receiver.ty);

            let kind = if dotted {
                ErrorKind::NoFields {
                    receiver,
                    field: name.to_owned(),
                }
            } else {
                ErrorKind::NotIndexable(receiver)
            };

            return Err(Diagnostic::new(at, kind));
        };

        let Some((at_word, field)) = self
            .types
            .field_offsets(fields)
            .find(|(_, f)| f.name == name)
        else {
            return Err(Diagnostic::new(
                at,
                ErrorKind::UnknownField {
                    field: name.to_owned(),
                    fields: fields.iter().map(|f| f.name.clone()).collect(),
                },
            ));
        };

        let ty = field.ty;
        self.emit(Instr::Field {
            at: count(at_word)?,
            ty,
        });

        if let (Some(path), Some(recorder)) = (receiver.path, self.recorder.as_mut()) {
            // A dotted field is part of its node; a quoted one is its
            // string.
            recorder.field(path, name, at, if dotted { span } else { at });
        }

        self.push_path(ty, span, receiver.path);
        Ok(())
    }

    /// `operand!`: the value of an option, which fails the run when absent.
    fn unwrap(&mut self, node: N<'a>) -> Result<(), Diagnostic> {
        let operand = self.pop()?;

        if operand.ty == self.error {
            self.push(self.error, *node.span());
            return Ok(());
        }

        let Some(TypeNode::Option(value)) = self.types.get(operand.ty) else {
            return Err(Diagnostic::new(
                *node.span(),
                ErrorKind::UnwrapNonOption(self.ty(operand.ty)),
            ));
        };

        let value = *value;
        self.emit(Instr::Unwrap(operand.ty));
        self.push_path(value, *node.span(), operand.path);
        Ok(())
    }

    fn index(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Diagnostic> {
        let receiver = *self
            .values
            .last()
            .ok_or_else(|| internal("an index without a receiver"))?;
        let index = exprs(node)
            .nth(1)
            .ok_or_else(|| internal("an index without an index"))?;

        let error = match self.types.get(receiver.ty) {
            _ if receiver.ty == self.error => None,
            Some(TypeNode::List(_) | TypeNode::Map { .. }) => {
                work.push(Work::IndexExit(node));
                work.push(Work::Enter(index));
                return Ok(());
            }
            Some(TypeNode::Record(_)) => {
                let name = match index.value() {
                    Syntax::String => unescape(self.text(index)),
                    _ => None,
                };

                match name {
                    Some(name) => {
                        self.pop()?;
                        return self.read_field(
                            receiver,
                            &name,
                            *index.span(),
                            *node.span(),
                            false,
                        );
                    }
                    None => Some(Diagnostic::new(*index.span(), ErrorKind::RecordIndex)),
                }
            }
            _ => Some(Diagnostic::new(
                receiver.span,
                ErrorKind::NotIndexable(self.ty(receiver.ty)),
            )),
        };

        if let Some(error) = error {
            self.report(error);
        }

        // The index is still checked, then dropped with the receiver.
        work.push(Work::Discard(node, 2));
        work.push(Work::Enter(index));
        Ok(())
    }

    fn index_exit(&mut self, node: N<'a>) -> Result<(), Diagnostic> {
        let index = self.pop()?;
        let list = self.pop()?;
        let index_node = exprs(node)
            .nth(1)
            .ok_or_else(|| internal("an index without an index"))?;

        if self.any_error(&[index, list]) {
            self.push(self.error, *node.span());
            return Ok(());
        }

        let entry = match self.types.get(list.ty) {
            Some(TypeNode::List(entry)) => *entry,
            Some(TypeNode::Map { key, value }) => {
                let (key, value) = (*key, *value);

                if index.ty != key {
                    return Err(Diagnostic::new(
                        index.span,
                        ErrorKind::KeyType {
                            map: self.ty(list.ty),
                            key: self.ty(key),
                            found: self.ty(index.ty),
                        },
                    ));
                }

                self.emit(Instr::MapIndex(list.ty));
                self.record_index(list.path, true, index_node);
                self.push_path(value, *node.span(), list.path);
                return Ok(());
            }
            _ => return Err(internal("indexing other than a list or a map")),
        };

        if index.ty != self.int {
            return Err(Diagnostic::new(
                index.span,
                ErrorKind::IndexType {
                    list: self.ty(list.ty),
                    found: self.ty(index.ty),
                },
            ));
        }

        self.emit(Instr::Index(entry));
        self.record_index(list.path, false, index_node);
        self.push_path(entry, *node.span(), list.path);
        Ok(())
    }

    fn list(&mut self, node: N<'a>, start: usize) -> Result<(), Diagnostic> {
        let n = exprs(node).count();
        let values = self.pop_n(n)?;
        let span = *node.span();

        let Some(first) = values.first() else {
            return Err(Diagnostic::new(span, ErrorKind::EmptyList));
        };

        let entry = first.ty;

        // Each entry of another type than the first is its own mistake,
        // unless either is already in error.
        let mut fits = entry != self.error;

        for (at, value) in values.iter().enumerate().skip(1) {
            if value.ty == self.error {
                fits = false;
            } else if entry != self.error && value.ty != entry {
                fits = false;
                self.report(Diagnostic::new(
                    value.span,
                    ErrorKind::ListEntry {
                        index: at.saturating_add(1),
                        expected: self.ty(entry),
                        found: self.ty(value.ty),
                    },
                ));
            }
        }

        if !fits {
            self.push(self.error, span);
            return Ok(());
        }

        let ty = self.node(TypeNode::List(entry), span)?;

        // A list of constants is a constant.
        let items = self.code.get(start..).and_then(|code| {
            code.iter()
                .map(|instr| match *instr {
                    Instr::Int(v) => Some(Scalar::Int(v)),
                    Instr::Bool(v) => Some(Scalar::Bool(v)),
                    Instr::Const(c) => Some(Scalar::Const(c)),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
        });

        match items {
            Some(items) if items.len() == n => {
                self.truncate(start);

                let c = count(self.constants.len())?;
                self.constants.push(Constant::List { entry, items });
                self.emit(Instr::Const(c));
            }
            _ => {
                self.emit(Instr::List {
                    len: count(n)?,
                    entry,
                });
            }
        }

        self.push(ty, span);
        Ok(())
    }

    /// A map literal: its keys are checked here, as literals of one type
    /// given once each, then its entries are compiled in key order.
    fn map(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Diagnostic> {
        // An `Error` part is an entry without its `:`, already reported.
        let parts = node
            .children()
            .filter(|n| matches!(n.value(), Syntax::Entry | Syntax::Error))
            .collect::<Vec<_>>();

        if parts.is_empty() {
            return Err(Diagnostic::new(*node.span(), ErrorKind::EmptyMap));
        }

        let mut entries = Vec::with_capacity(parts.len());
        let mut keys = Vec::with_capacity(parts.len());
        let mut fits = true;

        for part in &parts {
            if part.value() != Syntax::Entry {
                fits = false;
                continue;
            }

            let mut exprs = exprs(*part);

            let (Some(key), Some(value)) = (exprs.next(), exprs.next()) else {
                return Err(internal("a map entry without a key and a value"));
            };

            match self.key(key) {
                MapKey::Literal(literal) => keys.push((literal, *key.span())),
                MapKey::Broken => fits = false,
                MapKey::Other => {
                    self.report(Diagnostic::new(*key.span(), ErrorKind::MapKeyLiteral));
                    fits = false;
                }
            }

            entries.push((key, value));
        }

        let order = if fits { self.key_order(&keys) } else { None };

        let Some(order) = order else {
            // Its values are still checked, then dropped. Its keys are
            // literals or already reported.
            let values = parts.iter().filter_map(|part| match part.value() {
                Syntax::Entry => exprs(*part).nth(1),
                _ => Some(*part),
            });
            let values = values.collect::<Vec<_>>();
            work.push(Work::Discard(node, values.len()));
            work.extend(values.into_iter().rev().map(Work::Enter));
            return Ok(());
        };

        // The keys, sorted, are a constant list.
        let mut items = Vec::with_capacity(order.len());
        let mut values = Vec::with_capacity(order.len());

        for at in &order {
            let (Some((literal, _)), Some((_, value))) = (keys.get(*at), entries.get(*at)) else {
                return Err(internal("a map entry past the entries"));
            };

            items.push(match literal {
                KeyLit::Int(n) => Scalar::Int(*n),
                KeyLit::String(s) => Scalar::Const(self.string(s.clone())?),
            });
            values.push(*value);
        }

        let span = *node.span();

        let key = match keys.first() {
            Some((KeyLit::Int(_), _)) => self.int,
            _ => self.node(TypeNode::String, span)?,
        };

        self.node(TypeNode::List(key), span)?;
        let c = count(self.constants.len())?;
        self.constants.push(Constant::List { entry: key, items });

        work.push(Work::Map {
            node,
            order,
            keys: c,
            key,
        });
        work.extend(values.into_iter().rev().map(Work::Enter));
        Ok(())
    }

    /// Where each of `keys`, in the order they are written, goes in key
    /// order, if they are of one type and each given once; each that is not
    /// is reported.
    fn key_order(&mut self, keys: &[(KeyLit, Span<u32>)]) -> Option<Vec<usize>> {
        let mut fits = true;

        if let Some((first, _)) = keys.first() {
            for (at, (key, span)) in keys.iter().enumerate().skip(1) {
                if key.is_int() != first.is_int() {
                    fits = false;
                    self.report(Diagnostic::new(
                        *span,
                        ErrorKind::MapKeyMixed {
                            index: at.saturating_add(1),
                            expected: first.ty(),
                            found: key.ty(),
                        },
                    ));
                }
            }
        }

        if !fits {
            return None;
        }

        // The sort is stable, so a key given again follows where it is
        // first given, and each repeat is found by the one sort.
        let mut order = (0..keys.len()).collect::<Vec<_>>();
        order.sort_by(|a, b| keys.get(*a).map(|k| &k.0).cmp(&keys.get(*b).map(|k| &k.0)));

        let mut first: Option<&(KeyLit, Span<u32>)> = None;
        let mut repeats = Vec::new();

        for entry in order.iter().filter_map(|at| keys.get(*at)) {
            match first {
                Some((key, at)) if *key == entry.0 => {
                    let key = match key {
                        KeyLit::Int(n) => Key::Int(*n),
                        KeyLit::String(s) => Key::string(s),
                    };

                    repeats.push(Diagnostic::new(
                        entry.1,
                        ErrorKind::RepeatedKey {
                            key: key.to_string(),
                            first: *at,
                        },
                    ));
                }
                _ => first = Some(entry),
            }
        }

        if repeats.is_empty() {
            return Some(order);
        }

        for repeat in repeats {
            self.report(repeat);
        }

        None
    }

    /// The value of the map key `node`.
    fn key(&self, node: N<'_>) -> MapKey {
        let (node, negative) = match node.value() {
            Syntax::Unary if operator(node) == Some(Syntax::Minus) => match exprs(node).next() {
                Some(operand) => (operand, true),
                None => return MapKey::Broken,
            },
            _ => (node, false),
        };

        match node.value() {
            Syntax::Integer => match parse_int(self.text(node), negative) {
                Some((n, _)) => MapKey::Literal(KeyLit::Int(n)),
                None => MapKey::Broken,
            },
            Syntax::String if !negative => match unescape(self.text(node)) {
                Some(s) => MapKey::Literal(KeyLit::String(s)),
                None => MapKey::Broken,
            },
            Syntax::Error => MapKey::Broken,
            _ => MapKey::Other,
        }
    }

    /// After a map's values, in key order.
    fn map_exit(
        &mut self,
        node: N<'a>,
        order: &[usize],
        keys: u32,
        key: TypeId,
    ) -> Result<(), Diagnostic> {
        let values = self.pop_n(order.len())?;
        let span = *node.span();

        // The values in the order they are written.
        let mut written = vec![None; order.len()];

        for (value, at) in values.iter().zip(order) {
            if let Some(slot) = written.get_mut(*at) {
                *slot = Some(*value);
            }
        }

        let written = written
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| internal("a map value missing"))?;

        let Some(first) = written.first() else {
            return Err(internal("a map without entries"));
        };

        let entry = first.ty;
        let mut fits = entry != self.error;

        for (at, value) in written.iter().enumerate().skip(1) {
            if value.ty == self.error {
                fits = false;
            } else if entry != self.error && value.ty != entry {
                fits = false;
                self.report(Diagnostic::new(
                    value.span,
                    ErrorKind::MapValue {
                        index: at.saturating_add(1),
                        expected: self.ty(entry),
                        found: self.ty(value.ty),
                    },
                ));
            }
        }

        if !fits {
            self.push(self.error, span);
            return Ok(());
        }

        let ty = self.node(TypeNode::Map { key, value: entry }, span)?;
        self.emit(Instr::Map { keys, map: ty });
        self.push(ty, span);
        Ok(())
    }

    /// The program, unless any diagnostic was reported.
    fn finish(self) -> Result<(Program, DebugInfo), Diagnostics> {
        if let Some(diagnostics) = self.reporter.finish() {
            return Err(diagnostics);
        }

        let [value] = self.values.as_slice() else {
            return Err(Diagnostics::one(internal("not one value at the end")));
        };

        let program = Program::verify(
            self.types,
            self.constants,
            self.globals,
            self.functions,
            self.code,
            value.ty,
        )
        .map_err(|e| {
            Diagnostics::one(internal(format!("the compiled program is invalid: {e:#}")))
        })?;

        Ok((program, DebugInfo::new(self.spans)))
    }
}

/// The literal an index or a key is written as, with its span inside any
/// parentheses, or `None` and the span of the expression that computes it.
fn literal(node: N<'_>, source: &str) -> (Option<Literal>, Span<u32>) {
    let mut node = node;

    // Each parenthesis is looked into once, so this is bounded by the
    // expression.
    while node.value() == Syntax::Paren {
        match exprs(node).next() {
            Some(inner) => node = inner,
            None => break,
        }
    }

    let text = |n: N<'_>| source.get(n.range()).unwrap_or_default();

    let literal = match node.value() {
        Syntax::Integer => parse_int(text(node), false).map(|(n, _)| Literal::Int(n)),
        Syntax::String => unescape(text(node)).map(Literal::String),
        Syntax::Unary if operator(node) == Some(Syntax::Minus) => exprs(node)
            .next()
            .filter(|n| n.value() == Syntax::Integer)
            .and_then(|n| parse_int(text(n), true))
            .map(|(n, _)| Literal::Int(n)),
        _ => None,
    };

    (literal, *node.span())
}

/// What a map's key is.
enum MapKey {
    Literal(KeyLit),
    /// In error, already reported.
    Broken,
    /// Other than a literal.
    Other,
}

/// A map key's value. Their order is the machine's: `Int`s by value,
/// `String`s by their bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum KeyLit {
    Int(i64),
    String(String),
}

impl KeyLit {
    fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    fn ty(&self) -> Type {
        match self {
            Self::Int(_) => Type::int(),
            Self::String(_) => Type::string(),
        }
    }
}

fn is_expr(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::Integer
            | Syntax::String
            | Syntax::Bool
            | Syntax::Name
            | Syntax::Paren
            | Syntax::Unary
            | Syntax::Binary
            | Syntax::Call
            | Syntax::Method
            | Syntax::Field
            | Syntax::Index
            | Syntax::Unwrap
            | Syntax::List
            | Syntax::Map
            | Syntax::Error
    )
}

/// The expressions directly inside `node`.
fn exprs<'a>(node: N<'a>) -> impl Iterator<Item = N<'a>> {
    node.children().filter(|n| is_expr(n.value()))
}

/// The operator token of a unary or binary node.
fn operator(node: N<'_>) -> Option<Syntax> {
    node.children()
        .map(|n| n.value())
        .find(|kind| crate::syntax::is_operator(*kind))
}

/// The name of a method or field: the `Name` after its `.`.
fn name_of(node: N<'_>) -> Option<N<'_>> {
    node.children()
        .skip_while(|n| n.value() != Syntax::Dot)
        .find(|n| n.value() == Syntax::Name)
}

/// The arguments of a call or method.
fn args_of(node: N<'_>) -> Vec<N<'_>> {
    node.children()
        .find(|n| n.value() == Syntax::Args)
        .map(|args| exprs(args).collect())
        .unwrap_or_default()
}

fn push_args<'a>(work: &mut Vec<Work<'a>>, node: N<'a>) {
    work.extend(args_of(node).into_iter().rev().map(Work::Enter));
}

fn count(n: usize) -> Result<u32, Diagnostic> {
    u32::try_from(n).map_err(|_| Diagnostic::new(Span::new(0, 0), ErrorKind::TooLarge))
}

fn internal(message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(Span::new(0, 0), ErrorKind::Internal(message.to_string()))
}

/// A declared name `name` may be a misspelling of: one equal but for
/// case, else the nearest within two edits, if it is unique.
fn suggest<'n>(name: &str, names: impl Iterator<Item = &'n String> + Clone) -> Option<String> {
    if let Some(same) = names.clone().find(|n| n.eq_ignore_ascii_case(name)) {
        return Some(same.clone());
    }

    let len = name.chars().count();
    let limit = (len / 3).clamp(1, 2);
    let mut best: Option<(usize, &String)> = None;
    let mut tied = false;

    for candidate in names {
        // The distance is at least the difference in length, so a name far
        // longer than every candidate costs nothing to rule out: the work
        // is bounded by the declared names, never by the input.
        if len.abs_diff(candidate.chars().count()) > limit {
            continue;
        }

        let d = distance(name, candidate);

        if d > limit || d >= name.len() || d >= candidate.len() {
            continue;
        }

        match best {
            Some((b, _)) if d > b => {}
            Some((b, _)) if d == b => tied = true,
            _ => {
                best = Some((d, candidate));
                tied = false;
            }
        }
    }

    best.filter(|_| !tied).map(|(_, n)| n.clone())
}

/// The Levenshtein distance between `a` and `b`.
fn distance(a: &str, b: &str) -> usize {
    let b = b.chars().collect::<Vec<_>>();
    let mut row = (0..=b.len()).collect::<Vec<_>>();

    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = i;
        let mut next = vec![i.saturating_add(1)];

        for (j, cb) in b.iter().enumerate() {
            let above = row.get(j.saturating_add(1)).copied().unwrap_or(usize::MAX);
            let left = next.last().copied().unwrap_or(usize::MAX);
            let cost = usize::from(ca != *cb);

            next.push(
                diagonal
                    .saturating_add(cost)
                    .min(above.saturating_add(1))
                    .min(left.saturating_add(1)),
            );
            diagonal = above;
        }

        row = next;
    }

    row.last().copied().unwrap_or(0)
}
