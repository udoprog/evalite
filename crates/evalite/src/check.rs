//! The type checker and compiler: one iterative walk over a parse tree
//! that gives every expression its type and emits its code.

use std::collections::HashMap;

use syntree::{FlavorDefault, Node, Span};

use crate::Type;
use crate::env::{Builtin, Environment, Overload};
use crate::error::{Arity, Callee, Error, ErrorKind};
use crate::methods::{self, MethodSig, Receiver, Slot};
use crate::parser::{Options, Tree, parse_with};
use crate::program::{Constant, Global, HostFunction, Instr, Program, Scalar};
use crate::syntax::{Syntax, unescape};
use crate::types::{TypeId, TypeNode, Types};

type N<'a> = Node<'a, Syntax, FlavorDefault>;

/// Parse `source` and compile it against `env`, with the default
/// [`Options`].
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::with_builtins();
/// env.value("wits", Type::Int)?;
/// env.value("bonus", Type::option(Type::Int))?;
///
/// let program = evalite::compile("min(wits, 3) + bonus.unwrap_or(0)", &env)?;
/// assert_eq!(program.result_type(), Type::Int);
///
/// let error = evalite::compile("wits + bonus", &env).unwrap_err();
/// assert_eq!(
///     error.to_string(),
///     "`+` takes Int and Int, found Int and Option<Int>, unwrap the optional value first with `.unwrap_or(...)`"
/// );
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
pub fn compile(source: &str, env: &Environment) -> Result<Program, Error> {
    compile_with(source, env, &Options::new())
}

/// Parse `source` with `options` and compile it against `env`.
pub fn compile_with(source: &str, env: &Environment, options: &Options) -> Result<Program, Error> {
    let tree = parse_with(source, options)?;
    compile_tree(source, &tree, env)
}

/// Compile a tree [`parse`](crate::parse) made of `source`.
pub(crate) fn compile_tree(source: &str, tree: &Tree, env: &Environment) -> Result<Program, Error> {
    let mut compiler = Compiler::new(source, env)?;

    let root = tree
        .first()
        .and_then(|root| exprs(root).next())
        .ok_or_else(|| internal("a tree without an expression"))?;

    compiler.run(root)?;
    compiler.finish()
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
    /// After a call's arguments.
    Call(N<'a>, &'a [Overload], String),
    /// After a method's receiver.
    Method(N<'a>),
    /// After a method's arguments.
    /// After a method's arguments, with its receiver's `T`.
    MethodExit(N<'a>, &'static MethodSig, TypeId, Callee),
    Field(N<'a>),
    /// After an index's receiver.
    Index(N<'a>),
    /// After a list index's index.
    IndexExit(N<'a>),
    /// After a list's entries, whose code starts at `start`.
    List {
        node: N<'a>,
        start: usize,
    },
}

/// A value the code so far leaves on the stack.
#[derive(Debug, Clone, Copy)]
struct Value {
    ty: TypeId,
    span: Span<u32>,
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
    global_ids: HashMap<String, u32>,
    functions: Vec<HostFunction>,
    function_ids: HashMap<(String, Vec<TypeId>), u32>,
    code: Vec<Instr>,
    /// The words the globals so far take.
    global_words: usize,
    values: Vec<Value>,
    /// `&&` and `||` jumps waiting for their right operands.
    jumps: Vec<usize>,
}

impl<'a> Compiler<'a> {
    fn new(source: &'a str, env: &'a Environment) -> Result<Self, Error> {
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
            global_words: 0,
            values: Vec::new(),
            jumps: Vec::new(),
        })
    }

    fn text(&self, node: N<'_>) -> &'a str {
        self.source.get(node.range()).unwrap_or_default()
    }

    fn ty(&self, id: TypeId) -> Type {
        self.types.to_type(id).unwrap_or(Type::Int)
    }

    fn node(&mut self, node: TypeNode, at: Span<u32>) -> Result<TypeId, Error> {
        self.types
            .add(node)
            .ok_or_else(|| Error::new(at, ErrorKind::TypeTooDeep))
    }

    fn intern(&mut self, ty: &Type, at: Span<u32>) -> Result<TypeId, Error> {
        self.types
            .intern(ty)
            .ok_or_else(|| Error::new(at, ErrorKind::TypeTooDeep))
    }

    /// Emit `instr`. The memory the code uses is the verifier's to count
    /// when the program is finished.
    fn emit(&mut self, instr: Instr) {
        self.code.push(instr);
    }

    fn push(&mut self, ty: TypeId, span: Span<u32>) {
        self.values.push(Value { ty, span });
    }

    fn pop(&mut self) -> Result<Value, Error> {
        self.values
            .pop()
            .ok_or_else(|| internal("no value where one was expected"))
    }

    /// The last `n` values, in order.
    fn pop_n(&mut self, n: usize) -> Result<Vec<Value>, Error> {
        let at = self
            .values
            .len()
            .checked_sub(n)
            .ok_or_else(|| internal("fewer values than expected"))?;
        Ok(self.values.split_off(at))
    }

    fn run(&mut self, root: N<'a>) -> Result<(), Error> {
        let mut work = vec![Work::Enter(root)];

        while let Some(item) = work.pop() {
            match item {
                Work::Enter(node) => self.enter(node, &mut work)?,
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
                Work::Call(node, overloads, name) => self.call(node, overloads, name)?,
                Work::Method(node) => self.method(node, &mut work)?,
                Work::MethodExit(node, method, inner, callee) => {
                    self.method_exit(node, method, inner, callee)?
                }
                Work::Field(node) => self.field(node)?,
                Work::Index(node) => self.index(node, &mut work)?,
                Work::IndexExit(node) => self.index_exit(node)?,
                Work::List { node, start } => self.list(node, start)?,
            }
        }

        Ok(())
    }

    fn enter(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Error> {
        let span = *node.span();
        let mut parts = exprs(node);

        match node.value() {
            Syntax::Integer => {
                let value = self
                    .text(node)
                    .parse::<i64>()
                    .map_err(|_| Error::new(span, ErrorKind::IntegerTooLarge))?;
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
                self.push(ty, span);
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
                let name = parts
                    .next()
                    .ok_or_else(|| internal("a call without a name"))?;
                let text = self.text(name);
                let overloads = self.function(text, *name.span())?;

                work.push(Work::Call(node, overloads, text.to_owned()));
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
            Syntax::List => {
                work.push(Work::List {
                    node,
                    start: self.code.len(),
                });
                let entries = parts.collect::<Vec<_>>();
                work.extend(entries.into_iter().rev().map(Work::Enter));
            }
            kind => return Err(internal(format!("{kind:?} is not an expression"))),
        }

        Ok(())
    }

    fn string(&mut self, text: String) -> Result<u32, Error> {
        if let Some(c) = self.strings.get(&text) {
            return Ok(*c);
        }

        let c = count(self.constants.len())?;
        self.constants.push(Constant::String(text.clone()));
        self.strings.insert(text, c);
        Ok(c)
    }

    /// Where the global `name` is held, in words, and its type.
    fn global(&mut self, name: &str, at: Span<u32>) -> Result<(u32, TypeId), Error> {
        let Some(ty) = self.env.values.get(name) else {
            let kind = if self.env.functions.contains_key(name) {
                ErrorKind::FunctionAsValue(name.to_owned())
            } else {
                ErrorKind::UnknownName {
                    name: name.to_owned(),
                    suggestion: suggest(name, self.env.values.keys()),
                }
            };

            return Err(Error::new(at, kind));
        };

        let ty = self.intern(ty, at)?;

        if let Some(g) = self.global_ids.get(name) {
            return Ok((*g, ty));
        }

        let g = count(self.global_words)?;
        self.global_words = self.global_words.saturating_add(self.types.width(ty));
        self.globals.push(Global {
            name: name.to_owned(),
            ty,
        });
        self.global_ids.insert(name.to_owned(), g);
        Ok((g, ty))
    }

    fn function(&self, name: &str, at: Span<u32>) -> Result<&'a [Overload], Error> {
        let env = self.env;

        match env.functions.get(name) {
            Some(overloads) if !overloads.is_empty() => Ok(overloads),
            _ if env.values.contains_key(name) => {
                Err(Error::new(at, ErrorKind::NotAFunction(name.to_owned())))
            }
            _ => Err(Error::new(
                at,
                ErrorKind::UnknownFunction {
                    name: name.to_owned(),
                    suggestion: suggest(name, env.functions.keys()),
                },
            )),
        }
    }

    fn unary(&mut self, node: N<'a>, start: usize) -> Result<(), Error> {
        let op = operator(node).ok_or_else(|| internal("a unary without an operator"))?;
        let value = self.pop()?;
        let span = *node.span();

        let (want, instr) = match op {
            Syntax::Minus => (self.int, Instr::Neg),
            _ => (self.bool, Instr::Not),
        };

        if value.ty != want {
            return Err(Error::new(
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
                self.code.truncate(start);
                self.code.push(Instr::Int(n));
            }
            None => self.emit(instr),
        }

        self.push(want, span);
        Ok(())
    }

    fn binary(&mut self, node: N<'a>) -> Result<(), Error> {
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

        let fits = match op {
            Syntax::EqEq | Syntax::NotEq => {
                left.ty == right.ty && (left.ty == self.int || left.ty == self.bool)
            }
            _ => left.ty == self.int && right.ty == self.int,
        };

        if !fits {
            return Err(Error::new(
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

    fn logic(&mut self, node: N<'a>) -> Result<(), Error> {
        let op = operator(node).ok_or_else(|| internal("a binary without an operator"))?;
        let right = self.pop()?;
        let left = self.pop()?;
        let span = *node.span();

        if left.ty != self.bool || right.ty != self.bool {
            return Err(Error::new(
                span,
                ErrorKind::Operator {
                    op,
                    operands: vec![self.ty(left.ty), self.ty(right.ty)],
                },
            ));
        }

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

        self.push(self.bool, span);
        Ok(())
    }

    fn call(&mut self, node: N<'a>, overloads: &'a [Overload], name: String) -> Result<(), Error> {
        let args = args_of(node);
        let values = self.pop_n(args.len())?;
        let found = values.iter().map(|v| v.ty).collect::<Vec<_>>();
        let span = *node.span();

        let mut first = None;

        for overload in overloads {
            match self.overload(overload, &found, span)? {
                Ok(resolved) => return self.emit_call(&name, overload, resolved, &found, span),
                Err(mismatch) => {
                    first.get_or_insert(mismatch);
                }
            }
        }

        let callee = Callee::Function(name.clone());

        match (overloads, first) {
            ([_], Some(mismatch)) => Err(self.mismatch(mismatch, callee, &values, &args, node)),
            _ => Err(Error::new(
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
    ) -> Result<Result<TypeId, Mismatch>, Error> {
        let int = self.int;

        let ints = |from: usize| {
            found
                .iter()
                .enumerate()
                .skip(from)
                .find(|(_, t)| **t != int)
                .map(|(n, _)| Mismatch::Argument(n, "Int".to_owned()))
        };

        Ok(match overload {
            Overload::Builtin(Builtin::Min | Builtin::Max) => {
                if found.is_empty() {
                    Err(Mismatch::Count(Arity::AtLeast(1)))
                } else if let Some(mismatch) = ints(0) {
                    Err(mismatch)
                } else {
                    Ok(int)
                }
            }
            Overload::Builtin(Builtin::Lookup) => match found {
                [index, list] => {
                    if *index != int {
                        Err(Mismatch::Argument(0, "Int".to_owned()))
                    } else if let Some(TypeNode::List(entry)) = self.types.get(*list) {
                        Ok(*entry)
                    } else {
                        Err(Mismatch::Argument(1, "List<T>".to_owned()))
                    }
                }
                _ => Err(Mismatch::Count(Arity::Exactly(2))),
            },
            Overload::Host { params, ret } => {
                if params.len() != found.len() {
                    Err(Mismatch::Count(Arity::Exactly(params.len())))
                } else {
                    let mut result = Ok(self.intern(ret, at)?);

                    for (n, (param, found)) in params.iter().zip(found).enumerate() {
                        if self.intern(param, at)? != *found {
                            result = Err(Mismatch::Argument(n, param.to_string()));
                            break;
                        }
                    }

                    result
                }
            }
        })
    }

    fn emit_call(
        &mut self,
        name: &str,
        overload: &Overload,
        ret: TypeId,
        found: &[TypeId],
        span: Span<u32>,
    ) -> Result<(), Error> {
        let n = found.len();

        let instr = match overload {
            Overload::Builtin(Builtin::Min) => Instr::Min(count(n)?),
            Overload::Builtin(Builtin::Max) => Instr::Max(count(n)?),
            Overload::Builtin(Builtin::Lookup) => Instr::Lookup(ret),
            Overload::Host { .. } => {
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

                Instr::Call(f)
            }
        };

        self.emit(instr);
        self.push(ret, span);
        Ok(())
    }

    fn mismatch(
        &self,
        mismatch: Mismatch,
        callee: Callee,
        values: &[Value],
        args: &[N<'_>],
        node: N<'_>,
    ) -> Error {
        match mismatch {
            Mismatch::Count(expected) => {
                let at = node
                    .children()
                    .find(|n| n.value() == Syntax::Args)
                    .map_or(*node.span(), |n| *n.span());

                Error::new(
                    at,
                    ErrorKind::ArgumentCount {
                        callee,
                        expected,
                        found: args.len(),
                    },
                )
            }
            Mismatch::Argument(n, expected) => match values.get(n) {
                Some(value) => Error::new(
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

    fn method(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Error> {
        let receiver = *self
            .values
            .last()
            .ok_or_else(|| internal("a method without a receiver"))?;
        let name_node = name_of(node).ok_or_else(|| internal("a method without a name"))?;
        let name = self.text(name_node);

        let kind = match self.types.get(receiver.ty) {
            Some(TypeNode::Int) => Some((Receiver::Int, self.int)),
            Some(TypeNode::Option(t)) => Some((Receiver::Option, *t)),
            Some(TypeNode::List(t)) => Some((Receiver::List, *t)),
            _ => None,
        };

        let method = kind.and_then(|(kind, inner)| Some((methods::find(kind, name)?, inner)));

        let receiver_type = self.ty(receiver.ty);

        let Some((method, inner)) = method else {
            return Err(Error::new(
                *name_node.span(),
                ErrorKind::UnknownMethod {
                    available: methods::method_names(&receiver_type),
                    receiver: receiver_type,
                    method: name.to_owned(),
                },
            ));
        };

        let callee = Callee::Method {
            receiver: receiver_type,
            name: name.to_owned(),
        };

        work.push(Work::MethodExit(node, method, inner, callee));
        push_args(work, node);
        Ok(())
    }

    fn method_exit(
        &mut self,
        node: N<'a>,
        method: &MethodSig,
        inner: TypeId,
        callee: Callee,
    ) -> Result<(), Error> {
        let args = args_of(node);
        let values = self.pop_n(args.len())?;
        let receiver = self.pop()?;
        let span = *node.span();

        let resolve = |slot: Slot| slot.resolve(&self.int, &self.bool, &inner);
        let params = method
            .params
            .iter()
            .map(|p| resolve(*p))
            .collect::<Vec<_>>();
        let (instr, ret) = ((method.instr)(receiver.ty), resolve(method.ret));

        if params.len() != values.len() {
            let mismatch = Mismatch::Count(Arity::Exactly(params.len()));
            return Err(self.mismatch(mismatch, callee, &values, &args, node));
        }

        for (n, (param, value)) in params.iter().zip(&values).enumerate() {
            if *param != value.ty {
                let mismatch = Mismatch::Argument(n, self.ty(*param).to_string());
                return Err(self.mismatch(mismatch, callee, &values, &args, node));
            }
        }

        self.emit(instr);
        self.push(ret, span);
        Ok(())
    }

    fn field(&mut self, node: N<'a>) -> Result<(), Error> {
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
    ) -> Result<(), Error> {
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

            return Err(Error::new(at, kind));
        };

        let Some((at_word, field)) = self
            .types
            .field_offsets(fields)
            .find(|(_, f)| f.name == name)
        else {
            return Err(Error::new(
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
        self.push(ty, span);
        Ok(())
    }

    fn index(&mut self, node: N<'a>, work: &mut Vec<Work<'a>>) -> Result<(), Error> {
        let receiver = *self
            .values
            .last()
            .ok_or_else(|| internal("an index without a receiver"))?;
        let index = exprs(node)
            .nth(1)
            .ok_or_else(|| internal("an index without an index"))?;

        match self.types.get(receiver.ty) {
            Some(TypeNode::List(_)) => {
                work.push(Work::IndexExit(node));
                work.push(Work::Enter(index));
                Ok(())
            }
            Some(TypeNode::Record(_)) => {
                let name = match index.value() {
                    Syntax::String => unescape(self.text(index)),
                    _ => None,
                };

                let Some(name) = name else {
                    return Err(Error::new(*index.span(), ErrorKind::RecordIndex));
                };

                self.pop()?;
                self.read_field(receiver, &name, *index.span(), *node.span(), false)
            }
            _ => Err(Error::new(
                receiver.span,
                ErrorKind::NotIndexable(self.ty(receiver.ty)),
            )),
        }
    }

    fn index_exit(&mut self, node: N<'a>) -> Result<(), Error> {
        let index = self.pop()?;
        let list = self.pop()?;

        let Some(TypeNode::List(entry)) = self.types.get(list.ty) else {
            return Err(internal("indexing other than a list"));
        };

        let entry = *entry;

        if index.ty != self.int {
            return Err(Error::new(
                index.span,
                ErrorKind::IndexType {
                    list: self.ty(list.ty),
                    found: self.ty(index.ty),
                },
            ));
        }

        self.emit(Instr::Index(entry));
        self.push(entry, *node.span());
        Ok(())
    }

    fn list(&mut self, node: N<'a>, start: usize) -> Result<(), Error> {
        let n = exprs(node).count();
        let values = self.pop_n(n)?;
        let span = *node.span();

        let Some(first) = values.first() else {
            return Err(Error::new(span, ErrorKind::EmptyList));
        };

        let entry = first.ty;

        for (at, value) in values.iter().enumerate().skip(1) {
            if value.ty != entry {
                return Err(Error::new(
                    value.span,
                    ErrorKind::ListEntry {
                        index: at.saturating_add(1),
                        expected: self.ty(entry),
                        found: self.ty(value.ty),
                    },
                ));
            }
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
                self.code.truncate(start);

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

    fn finish(self) -> Result<Program, Error> {
        let [value] = self.values.as_slice() else {
            return Err(internal("not one value at the end"));
        };

        let program = Program::verify(
            self.types,
            self.constants,
            self.globals,
            self.functions,
            self.code,
            value.ty,
        )
        .map_err(|e| internal(format!("the compiled program is invalid: {e}")))?;

        Ok(program)
    }
}

/// Why an overload does not take a call's arguments.
enum Mismatch {
    Count(Arity),
    /// The argument at this index, from 0, is not of the type described.
    Argument(usize, String),
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
            | Syntax::List
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

fn count(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::new(Span::new(0, 0), ErrorKind::TooLarge))
}

fn internal(message: impl std::fmt::Display) -> Error {
    Error::new(Span::new(0, 0), ErrorKind::Internal(message.to_string()))
}

/// A declared name `name` may be a misspelling of: one equal but for
/// case, else the nearest within two edits, if it is unique.
fn suggest<'n>(name: &str, names: impl Iterator<Item = &'n String> + Clone) -> Option<String> {
    if let Some(same) = names.clone().find(|n| n.eq_ignore_ascii_case(name)) {
        return Some(same.clone());
    }

    let limit = (name.chars().count() / 3).clamp(1, 2);
    let mut best: Option<(usize, &String)> = None;
    let mut tied = false;

    for candidate in names {
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
