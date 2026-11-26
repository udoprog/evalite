use syntree::{Builder, FlavorDefault, Span};

use crate::error::{Delimiter, Diagnostic, ErrorKind, Found, Nesting};
use crate::lexer::{Lexer, Token, span};
use crate::syntax::Syntax;

/// A parsed expression: a `Root` node over every byte of the input.
pub(crate) type Tree = syntree::Tree<Syntax, FlavorDefault>;

type Checkpoint = syntree::Checkpoint<<FlavorDefault as syntree::Flavor>::Pointer>;

/// How many bytes an expression may be unless [`Options::max_len`] says
/// otherwise (64 KiB).
pub const DEFAULT_MAX_LEN: usize = 64 * 1024;

/// How deeply expressions nest unless [`Options::max_depth`] says
/// otherwise.
pub const DEFAULT_MAX_DEPTH: usize = 32;

/// How to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    max_depth: usize,
    max_len: usize,
}

impl Options {
    /// The default options: at most [`DEFAULT_MAX_DEPTH`] levels of
    /// nesting and [`DEFAULT_MAX_LEN`] bytes of input.
    ///
    /// ```
    /// use evalite::{Environment, Options};
    ///
    /// let env = Environment::with_builtins();
    /// let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
    ///
    /// assert!(evalite::compile_with(&deep, &env, &Options::new()).is_err());
    /// assert!(evalite::compile_with(&deep, &env, &Options::new().max_depth(64)).is_ok());
    /// assert_eq!(Options::new(), Options::default());
    /// ```
    pub const fn new() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_len: DEFAULT_MAX_LEN,
        }
    }

    /// Allow at most `max_depth` levels of nesting.
    pub const fn max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Refuse an expression longer than `max_len` bytes, before lexing it.
    ///
    /// ```
    /// use evalite::{Environment, Options};
    ///
    /// let env = Environment::with_builtins();
    ///
    /// assert!(evalite::compile_with("1 + 2", &env, &Options::new().max_len(5)).is_ok());
    /// assert!(evalite::compile_with("1 + 2", &env, &Options::new().max_len(4)).is_err());
    /// ```
    pub const fn max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `source` as an expression with the default [`Options`].
#[cfg(test)]
pub(crate) fn parse(source: &str) -> Result<Tree, Diagnostic> {
    parse_with(source, &Options::new())
}

/// Parse `source` as an expression.
pub(crate) fn parse_with(source: &str, options: &Options) -> Result<Tree, Diagnostic> {
    if u32::try_from(source.len()).is_err() {
        return Err(Diagnostic::new(
            span(0, 0),
            ErrorKind::TooLong(source.len()),
        ));
    }

    if source.len() > options.max_len {
        return Err(Diagnostic::new(
            span(0, 0),
            ErrorKind::OverLimit {
                len: source.len(),
                max: options.max_len,
            },
        ));
    }

    let mut parser = Parser {
        lexer: Lexer::new(source),
        tree: Builder::new(),
        peeked: None,
        trivia: None,
        frames: Vec::new(),
        depth: 0,
        max_depth: options.max_depth,
        start: None,
        primary: None,
    };

    parser.run()?;
    parser.tree.build().map_err(internal)
}

/// Binding powers, loosest first.
const OR: u8 = 1;
const AND: u8 = 2;
const COMPARE: u8 = 3;
const SUM: u8 = 4;
const PRODUCT: u8 = 5;

fn binary(kind: Syntax) -> Option<u8> {
    Some(match kind {
        Syntax::OrOr => OR,
        Syntax::AndAnd => AND,
        Syntax::EqEq | Syntax::NotEq | Syntax::Lt | Syntax::Le | Syntax::Gt | Syntax::Ge => COMPARE,
        Syntax::Plus | Syntax::Minus => SUM,
        Syntax::Star | Syntax::Slash | Syntax::Percent => PRODUCT,
        _ => return None,
    })
}

/// Work deferred until the operand being parsed is complete.
enum Frame {
    /// A prefix operator whose node starts at `start`.
    Prefix { start: Checkpoint },
    /// A binary operator whose left operand starts at `start`.
    Binary { start: Checkpoint, power: u8 },
    /// An open group.
    Group(Group),
}

struct Group {
    delimiter: Delimiter,
    /// Where the node the group closes into starts.
    start: Checkpoint,
    /// For `Args`, where the `Args` node starts, and whether it is a `Call`
    /// or `Method`.
    args: Option<(Checkpoint, Syntax)>,
    open: Span<u32>,
    /// The start of the operand the group is part of.
    outer: Option<Checkpoint>,
}

enum State {
    Operand,
    Operator,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    tree: Builder<Syntax, FlavorDefault>,
    peeked: Option<Token>,
    /// Whitespace before `peeked`, added to the tree only when the token
    /// after it is, so nodes never end in whitespace.
    trivia: Option<Token>,
    frames: Vec<Frame>,
    /// The prefix and group frames on the stack.
    depth: usize,
    max_depth: usize,
    /// Where the operand being parsed starts, prefix operators included.
    start: Option<Checkpoint>,
    /// Where the latest primary starts, which postfixes wrap.
    primary: Option<Checkpoint>,
}

impl Parser<'_> {
    fn run(&mut self) -> Result<(), Diagnostic> {
        self.tree.open(Syntax::Root).map_err(internal)?;
        let mut state = State::Operand;

        loop {
            state = match state {
                State::Operand => self.operand()?,
                State::Operator => match self.operator()? {
                    Some(state) => state,
                    None => break,
                },
            };
        }

        self.flush()?;
        self.tree.close().map_err(internal)?;
        Ok(())
    }

    fn peek(&mut self) -> Result<Option<Token>, Diagnostic> {
        if self.peeked.is_none() {
            while let Some(token) = self.lexer.next()? {
                if token.kind == Syntax::Whitespace {
                    self.trivia = Some(token);
                } else {
                    self.peeked = Some(token);
                    break;
                }
            }
        }

        Ok(self.peeked)
    }

    fn peek_kind(&mut self) -> Result<Option<Syntax>, Diagnostic> {
        Ok(self.peek()?.map(|token| token.kind))
    }

    /// Add pending whitespace to the tree.
    fn flush(&mut self) -> Result<(), Diagnostic> {
        if let Some(token) = self.trivia.take() {
            self.tree
                .token(Syntax::Whitespace, token.len())
                .map_err(internal)?;
        }

        Ok(())
    }

    /// Add the peeked token to the tree.
    fn bump(&mut self) -> Result<(), Diagnostic> {
        self.flush()?;

        if let Some(token) = self.peeked.take() {
            self.tree.token(token.kind, token.len()).map_err(internal)?;
        }

        Ok(())
    }

    fn checkpoint(&mut self) -> Result<Checkpoint, Diagnostic> {
        self.flush()?;
        self.tree.checkpoint().map_err(internal)
    }

    fn close_at(&mut self, c: &Checkpoint, kind: Syntax) -> Result<(), Diagnostic> {
        self.tree.close_at(c, kind).map_err(internal)?;
        Ok(())
    }

    fn end_span(&self) -> Span<u32> {
        span(self.lexer.len(), self.lexer.len())
    }

    fn found(&self, token: Option<Token>) -> (Found, Span<u32>) {
        match token {
            Some(token) => (Found::Token(token.kind), token.span()),
            None => (Found::End, self.end_span()),
        }
    }

    fn nest(&mut self, nesting: Nesting, at: Span<u32>) -> Result<(), Diagnostic> {
        if self.depth >= self.max_depth {
            return Err(Diagnostic::new(
                at,
                ErrorKind::TooDeep {
                    nesting,
                    max: self.max_depth,
                },
            ));
        }

        self.depth = self.depth.saturating_add(1);
        Ok(())
    }

    /// Parse what starts an operand: a prefix operator, which leaves the
    /// operand to come, or a primary.
    fn operand(&mut self) -> Result<State, Diagnostic> {
        let token = self.peek()?;

        let Some(token) = token else {
            let (found, at) = self.found(None);
            return Err(Diagnostic::new(at, ErrorKind::ExpectedExpression(found)));
        };

        let c = self.checkpoint()?;

        if self.start.is_none() {
            self.start = Some(c.clone());
        }

        match token.kind {
            Syntax::Minus | Syntax::Bang => {
                self.nest(Nesting::Unary(token.kind), token.span())?;
                self.bump()?;
                self.frames.push(Frame::Prefix { start: c });
                Ok(State::Operand)
            }
            Syntax::OpenParen => self.open(Delimiter::Paren, c, None, token),
            Syntax::OpenBracket => self.open(Delimiter::List, c, None, token),
            Syntax::IntegerToken => {
                let text = self.lexer_text(token);

                if text.parse::<i64>().is_err() {
                    return Err(Diagnostic::new(token.span(), ErrorKind::IntegerTooLarge));
                }

                self.literal(c, Syntax::Integer)
            }
            Syntax::StringToken => self.literal(c, Syntax::String),
            Syntax::True | Syntax::False => self.literal(c, Syntax::Bool),
            Syntax::NameToken => {
                self.bump()?;
                self.close_at(&c, Syntax::Name)?;

                if let Some(open) = self.peek()?.filter(|t| t.kind == Syntax::OpenParen) {
                    let args = self.checkpoint()?;
                    return self.open(Delimiter::Args, c, Some((args, Syntax::Call)), open);
                }

                self.primary = Some(c);
                Ok(State::Operator)
            }
            _ => {
                let (found, at) = self.found(Some(token));
                Err(Diagnostic::new(at, ErrorKind::ExpectedExpression(found)))
            }
        }
    }

    fn lexer_text(&self, token: Token) -> &str {
        self.lexer.text(token.start, token.end)
    }

    fn literal(&mut self, c: Checkpoint, kind: Syntax) -> Result<State, Diagnostic> {
        self.bump()?;
        self.close_at(&c, kind)?;
        self.primary = Some(c);
        Ok(State::Operator)
    }

    /// Open a group at the peeked token `open`.
    fn open(
        &mut self,
        delimiter: Delimiter,
        start: Checkpoint,
        args: Option<(Checkpoint, Syntax)>,
        open: Token,
    ) -> Result<State, Diagnostic> {
        let nesting = match delimiter {
            Delimiter::Paren => Nesting::Paren,
            Delimiter::Args => Nesting::Args,
            Delimiter::List => Nesting::List,
            Delimiter::Index => Nesting::Index,
        };

        self.nest(nesting, open.span())?;
        self.bump()?;

        self.frames.push(Frame::Group(Group {
            delimiter,
            start,
            args,
            open: open.span(),
            outer: self.start.take(),
        }));

        self.primary = None;

        if matches!(delimiter, Delimiter::Args | Delimiter::List)
            && self.peek_kind()? == Some(delimiter.close())
        {
            return self.close_group();
        }

        Ok(State::Operand)
    }

    /// Close the group on top of the stack at the peeked token, its closer.
    fn close_group(&mut self) -> Result<State, Diagnostic> {
        let Some(Frame::Group(group)) = self.frames.pop() else {
            return Err(internal("no group to close"));
        };

        self.depth = self.depth.saturating_sub(1);
        self.bump()?;

        match (group.delimiter, &group.args) {
            (Delimiter::Args, Some((args, kind))) => {
                self.close_at(args, Syntax::Args)?;
                self.close_at(&group.start, *kind)?;
            }
            (Delimiter::Paren, _) => self.close_at(&group.start, Syntax::Paren)?,
            (Delimiter::List, _) => self.close_at(&group.start, Syntax::List)?,
            (Delimiter::Index, _) => self.close_at(&group.start, Syntax::Index)?,
            (Delimiter::Args, None) => return Err(internal("arguments without a node")),
        }

        self.start = group.outer;
        self.primary = Some(group.start);
        Ok(State::Operator)
    }

    /// Reduce the frames binding at least as tightly as `power`, giving
    /// where the operand they make up starts.
    fn reduce(&mut self, power: u8, at: Span<u32>) -> Result<Checkpoint, Diagnostic> {
        let mut left = self.start.clone();

        loop {
            let (start, kind) = match self.frames.last() {
                Some(Frame::Prefix { start }) => (start.clone(), Syntax::Unary),
                Some(Frame::Binary { start, power: p }) if *p >= power => {
                    if *p == COMPARE && power == COMPARE {
                        return Err(Diagnostic::new(at, ErrorKind::ChainedComparison));
                    }

                    (start.clone(), Syntax::Binary)
                }
                _ => break,
            };

            if kind == Syntax::Unary {
                self.depth = self.depth.saturating_sub(1);
            }

            self.frames.pop();
            self.close_at(&start, kind)?;
            left = Some(start);
        }

        left.ok_or_else(|| internal("an operator without an operand"))
    }

    fn innermost(&self) -> Option<&Group> {
        self.frames.iter().rev().find_map(|frame| match frame {
            Frame::Group(group) => Some(group),
            _ => None,
        })
    }

    /// Parse what follows a complete primary: postfixes, an operator, a
    /// separator or closer, or the end, which gives `None`.
    fn operator(&mut self) -> Result<Option<State>, Diagnostic> {
        let token = self.peek()?;

        let Some(token) = token else {
            self.reduce(0, self.end_span())?;

            if let Some(group) = self.innermost() {
                return Err(Diagnostic::new(
                    self.end_span(),
                    ErrorKind::Unclosed {
                        open: group.delimiter,
                        open_span: group.open,
                        found: Found::End,
                    },
                ));
            }

            return Ok(None);
        };

        let Some(primary) = self.primary.clone() else {
            return Err(internal("an operator without a primary"));
        };

        if let Some(power) = binary(token.kind) {
            let left = self.reduce(power, token.span())?;
            self.bump()?;
            self.frames.push(Frame::Binary { start: left, power });
            self.start = None;
            self.primary = None;
            return Ok(Some(State::Operand));
        }

        match token.kind {
            Syntax::Dot => {
                self.bump()?;

                let name = self.peek()?;

                if name.is_none_or(|t| t.kind != Syntax::NameToken) {
                    let (found, at) = self.found(name);
                    return Err(Diagnostic::new(at, ErrorKind::ExpectedName(found)));
                }

                let c = self.checkpoint()?;
                self.bump()?;
                self.close_at(&c, Syntax::Name)?;

                if let Some(open) = self.peek()?.filter(|t| t.kind == Syntax::OpenParen) {
                    let args = self.checkpoint()?;
                    let args = Some((args, Syntax::Method));
                    return self.open(Delimiter::Args, primary, args, open).map(Some);
                }

                self.close_at(&primary, Syntax::Field)?;
                Ok(Some(State::Operator))
            }
            Syntax::OpenBracket => self.open(Delimiter::Index, primary, None, token).map(Some),
            Syntax::Bang => {
                self.bump()?;
                self.close_at(&primary, Syntax::Unwrap)?;
                Ok(Some(State::Operator))
            }
            Syntax::Comma | Syntax::CloseParen | Syntax::CloseBracket => {
                self.reduce(0, token.span())?;

                let Some(group) = self.innermost() else {
                    let (found, at) = self.found(Some(token));

                    let kind = if token.kind == Syntax::Comma {
                        ErrorKind::ExpectedOperator { found, open: None }
                    } else {
                        ErrorKind::Unmatched(found)
                    };

                    return Err(Diagnostic::new(at, kind));
                };

                let delimiter = group.delimiter;
                let open_span = group.open;

                if token.kind == Syntax::Comma
                    && matches!(delimiter, Delimiter::Args | Delimiter::List)
                {
                    self.bump()?;
                    self.start = None;
                    self.primary = None;
                    return Ok(Some(State::Operand));
                }

                if token.kind != delimiter.close() {
                    return Err(Diagnostic::new(
                        token.span(),
                        ErrorKind::Unclosed {
                            open: delimiter,
                            open_span,
                            found: Found::Token(token.kind),
                        },
                    ));
                }

                self.close_group().map(Some)
            }
            _ => {
                let open = self.innermost().map(|group| group.delimiter);
                let (found, at) = self.found(Some(token));
                Err(Diagnostic::new(
                    at,
                    ErrorKind::ExpectedOperator { found, open },
                ))
            }
        }
    }
}

fn internal(error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(span(0, 0), ErrorKind::Internal(error.to_string()))
}
