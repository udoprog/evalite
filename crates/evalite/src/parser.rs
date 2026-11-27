use syntree::{Builder, EmptyVec, Span, TreeIndex};

use crate::error::{Delimiter, Diagnostic, Diagnostics, ErrorKind, Found, Nesting, Reporter};
use crate::lexer::{Lexer, Token, parse_int, span};
use crate::syntax::Syntax;

syntree::flavor! {
    /// The tree flavor: `u32` node pointers (source length is already limited
    /// to `u32`) and no token index, which evalite never queries.
    pub(crate) struct Flavor {
        type Index = u32;
        type Width = u32;
        type Indexes = EmptyVec<TreeIndex<Self>>;
    }
}

/// A parsed expression: a `Root` node over every byte of the input.
pub(crate) type Tree = syntree::Tree<Syntax, Flavor>;

type Checkpoint = syntree::Checkpoint<<Flavor as syntree::Flavor>::Pointer>;

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
    /// let env = Environment::new();
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
    /// let env = Environment::new();
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

/// An expression parsed: its tree, and the syntax errors recovered from in
/// making it, each of which the tree holds as an `Error` node.
pub(crate) struct Parsed {
    pub(crate) tree: Tree,
    pub(crate) reporter: Reporter,
}

impl Parsed {
    /// The tree, unless it has syntax errors.
    pub(crate) fn complete(self) -> Result<Tree, Diagnostics> {
        match self.reporter.finish() {
            Some(errors) => Err(errors),
            None => Ok(self.tree),
        }
    }
}

/// Parse `source` as an expression.
///
/// A syntax error is reported and recovered from, so the tree always covers
/// all of `source` and every error in it is reported. Recovery consumes each
/// token once, so it is linear in the input.
///
/// # Errors
///
/// Only input too long to parse at all, or a fault in evalite itself.
pub(crate) fn parse_with(source: &str, options: &Options) -> Result<Parsed, Diagnostics> {
    if u32::try_from(source.len()).is_err() {
        return Err(Diagnostics::one(Diagnostic::new(
            span(0, 0),
            ErrorKind::TooLong(source.len()),
        )));
    }

    if source.len() > options.max_len {
        return Err(Diagnostics::one(Diagnostic::new(
            span(0, 0),
            ErrorKind::OverLimit {
                len: source.len(),
                max: options.max_len,
            },
        )));
    }

    let mut parser = Parser {
        lexer: Lexer::new(source),
        tree: Builder::new_with(),
        peeked: None,
        rejected: false,
        reported: false,
        trivia: None,
        frames: Vec::new(),
        groups: Vec::new(),
        parens: 0,
        brackets: 0,
        braces: 0,
        depth: 0,
        max_depth: options.max_depth,
        too_deep: false,
        start: None,
        primary: None,
        reporter: Reporter::default(),
    };

    parser.run().map_err(Diagnostics::one)?;

    let tree = parser
        .tree
        .build()
        .map_err(internal)
        .map_err(Diagnostics::one)?;

    Ok(Parsed {
        tree,
        reporter: parser.reporter,
    })
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

/// Whether `kind` can start an operand.
fn starts_operand(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::Minus
            | Syntax::Bang
            | Syntax::OpenParen
            | Syntax::OpenBracket
            | Syntax::OpenBrace
            | Syntax::IntegerToken
            | Syntax::StringToken
            | Syntax::True
            | Syntax::False
            | Syntax::NameToken
    )
}

/// Work deferred until the operand being parsed is complete.
enum Frame {
    /// A prefix operator whose node starts at `start`.
    Prefix { start: Checkpoint },
    /// A binary operator whose left operand starts at `start`. With
    /// `error`, the operator is in error, a chained comparison or a
    /// rejected `=`, `&` or `|`, so it closes into an `Error` node.
    Binary {
        start: Checkpoint,
        power: u8,
        error: bool,
    },
    /// An open group, the last of `groups`.
    Group,
    /// A map's entry, which starts at `start`: its key, then once `colon`
    /// is passed its value. It binds more loosely than any operator, so
    /// only a `,`, a closer or the end closes it, into an `Entry` node, or
    /// an `Error` node without its `:`. Its missing `:` is reported once,
    /// when `reported`.
    Entry {
        start: Checkpoint,
        colon: bool,
        reported: bool,
    },
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

impl Group {
    /// The error that the group is not closed at `at`, where `found` is.
    /// It is reported at the group's opener.
    fn unclosed(&self, at: Span<u32>, found: Found) -> Diagnostic {
        let kind = ErrorKind::Unclosed {
            open: self.delimiter,
            close_span: at,
            found,
        };

        Diagnostic::new(self.open, kind)
    }
}

enum State {
    Operand,
    Operator,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    tree: Builder<Syntax, Flavor>,
    peeked: Option<Token>,
    /// Whether the lexer rejected `peeked`, which it has reported.
    rejected: bool,
    /// Whether an error at `peeked` has been reported, by the lexer or the
    /// parser, so no other is.
    reported: bool,
    /// Whitespace before `peeked`, added to the tree only when the token
    /// after it is, so nodes never end in whitespace.
    trivia: Option<Token>,
    frames: Vec<Frame>,
    /// The open groups, innermost last.
    groups: Vec<Group>,
    /// How many of `groups` a `)` closes, how many a `]` does and how
    /// many a `}` does.
    parens: usize,
    brackets: usize,
    braces: usize,
    /// The prefix and group frames on the stack.
    depth: usize,
    max_depth: usize,
    /// Whether nesting past `max_depth` has been reported, which is done
    /// once.
    too_deep: bool,
    /// Where the operand being parsed starts, prefix operators included.
    start: Option<Checkpoint>,
    /// Where the latest primary starts, which postfixes wrap.
    primary: Option<Checkpoint>,
    reporter: Reporter,
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

    fn peek(&mut self) -> Option<Token> {
        while self.peeked.is_none() {
            match self.lexer.next() {
                Ok(Some(token)) if token.kind == Syntax::Whitespace => self.trivia = Some(token),
                Ok(Some(token)) => self.peeked = Some(token),
                Ok(None) => break,
                Err(rejected) => {
                    self.reporter.report(rejected.diagnostic);
                    self.peeked = Some(rejected.token);
                    self.rejected = true;
                    self.reported = true;
                }
            }
        }

        self.peeked
    }

    fn peek_kind(&mut self) -> Option<Syntax> {
        self.peek().map(|token| token.kind)
    }

    /// Report an error at `at`.
    fn report(&mut self, at: Span<u32>, kind: ErrorKind) {
        self.report_about(at, Diagnostic::new(at, kind));
    }

    /// Report `diagnostic`, an error about what is at `at`. One about the
    /// peeked token is reported only if none has been, so a token is
    /// reported once.
    fn report_about(&mut self, at: Span<u32>, diagnostic: Diagnostic) {
        if self.peeked.is_some_and(|token| token.span() == at) {
            if self.reported {
                return;
            }

            self.reported = true;
        }

        self.reporter.report(diagnostic);
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
            self.rejected = false;
            self.reported = false;
            self.tree.token(token.kind, token.len()).map_err(internal)?;
        }

        Ok(())
    }

    /// A checkpoint at the next token, after any whitespace before it.
    fn checkpoint(&mut self) -> Result<Checkpoint, Diagnostic> {
        self.peek();
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

    /// Enter a level of nesting, unless it would be past `max_depth`, which
    /// is reported the first time.
    fn nest(&mut self, nesting: Nesting, at: Span<u32>) -> bool {
        if self.depth >= self.max_depth {
            if !self.too_deep {
                self.too_deep = true;
                self.report(
                    at,
                    ErrorKind::TooDeep {
                        nesting,
                        max: self.max_depth,
                    },
                );
            }

            return false;
        }

        self.depth = self.depth.saturating_add(1);
        true
    }

    /// Skip tokens to where the expression can go on: a `,` that fits the
    /// innermost group, a `:` after a map's key, a closer, a binary
    /// operator or the end, outside any group the skipped tokens open.
    fn skip(&mut self) -> Result<(), Diagnostic> {
        let separated = self
            .innermost()
            .is_some_and(|group| group.delimiter.separated());
        let key = self.entry() == Some(false);
        let mut depth = 0usize;

        while let Some(token) = self.peek() {
            match token.kind {
                kind if opens(kind) => depth = depth.saturating_add(1),
                kind if closes(kind) => match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => break,
                },
                Syntax::Comma if depth == 0 && separated => break,
                Syntax::Colon if depth == 0 && key => break,
                kind if depth == 0 && binary(kind).is_some() => break,
                _ => {}
            }

            self.bump()?;
        }

        Ok(())
    }

    /// Skip what a group holds, up to the closer closing it or the end.
    fn skip_group(&mut self) -> Result<(), Diagnostic> {
        let mut depth = 0usize;

        while let Some(token) = self.peek() {
            match token.kind {
                kind if opens(kind) => depth = depth.saturating_add(1),
                kind if closes(kind) => match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => break,
                },
                _ => {}
            }

            self.bump()?;
        }

        Ok(())
    }

    /// Wrap what follows `start` in an `Error` node, which is the operand
    /// being parsed from now on.
    fn recovered(&mut self, start: Checkpoint) -> Result<(), Diagnostic> {
        self.close_at(&start, Syntax::Error)?;
        self.start = Some(start.clone());
        self.primary = Some(start);
        Ok(())
    }

    /// Parse what starts an operand: a prefix operator, which leaves the
    /// operand to come, or a primary.
    fn operand(&mut self) -> Result<State, Diagnostic> {
        let token = self.peek();

        let Some(token) = token.filter(|token| starts_operand(token.kind)) else {
            return self.missing(token);
        };

        let c = self.checkpoint()?;

        if self.start.is_none() {
            self.start = Some(c.clone());
        }

        match token.kind {
            Syntax::Minus | Syntax::Bang => {
                if self.nest(Nesting::Unary(token.kind), token.span()) {
                    self.bump()?;
                    self.frames.push(Frame::Prefix { start: c });
                    return Ok(State::Operand);
                }

                // Too deep: its operand is skipped as an error.
                self.bump()?;
                let operand = self.checkpoint()?;

                while self
                    .peek_kind()
                    .is_some_and(|kind| matches!(kind, Syntax::Minus | Syntax::Bang))
                {
                    self.bump()?;
                }

                self.skip()?;
                self.close_at(&operand, Syntax::Error)?;
                self.close_at(&c, Syntax::Unary)?;
                self.primary = Some(c);
                Ok(State::Operator)
            }
            Syntax::OpenParen => self.open(Delimiter::Paren, c, None, token),
            Syntax::OpenBracket => self.open(Delimiter::List, c, None, token),
            Syntax::OpenBrace => self.open(Delimiter::Map, c, None, token),
            Syntax::IntegerToken => {
                if parse_int(self.lexer_text(token), false).is_none() {
                    self.report(token.span(), ErrorKind::IntegerTooLarge);
                    return self.literal(c, Syntax::Error);
                }

                self.literal(c, Syntax::Integer)
            }
            Syntax::StringToken => self.literal(c, Syntax::String),
            Syntax::True | Syntax::False => self.literal(c, Syntax::Bool),
            Syntax::NameToken => {
                self.bump()?;

                let mut path = None;

                if self.peek_kind() == Some(Syntax::ColonColon) {
                    self.bump()?;
                    let name = self.peek();

                    let Some(name) = name.filter(|t| t.kind == Syntax::NameToken) else {
                        let (found, at) = self.found(name);
                        self.report(at, ErrorKind::ExpectedPathName(found));
                        self.skip()?;
                        self.close_at(&c, Syntax::Error)?;
                        self.primary = Some(c);
                        return Ok(State::Operator);
                    };

                    path = Some(name);
                    self.bump()?;
                    self.close_at(&c, Syntax::Path)?;
                } else {
                    self.close_at(&c, Syntax::Name)?;
                }

                if let Some(open) = self.peek().filter(|t| t.kind == Syntax::OpenParen) {
                    let args = self.checkpoint()?;
                    return self.open(Delimiter::Args, c, Some((args, Syntax::Call)), open);
                }

                if let Some(name) = path {
                    let at = span(token.start, name.end);
                    let path = format!("{}::{}", self.lexer_text(token), self.lexer_text(name));
                    self.report(at, ErrorKind::PathAsValue(path));
                    self.close_at(&c, Syntax::Error)?;
                }

                self.primary = Some(c);
                Ok(State::Operator)
            }
            _ => Err(internal("an operand that starts no operand")),
        }
    }

    /// Where an operand is expected, `token`, which starts none: report it,
    /// and take an `Error` node as the operand. Before what can follow an
    /// operand, it is empty, otherwise it holds what is skipped to there.
    fn missing(&mut self, token: Option<Token>) -> Result<State, Diagnostic> {
        let (found, at) = self.found(token);
        self.report(at, ErrorKind::ExpectedExpression(found));

        let skip = token.is_some_and(|token| {
            !matches!(token.kind, Syntax::Comma | Syntax::Colon)
                && !closes(token.kind)
                && binary(token.kind).is_none()
        });

        // An empty node goes before any whitespace, so the node it is part
        // of does not end in whitespace.
        let c = if skip {
            self.checkpoint()?
        } else {
            self.tree.checkpoint().map_err(internal)?
        };

        if self.start.is_none() {
            self.start = Some(c.clone());
        }

        if skip {
            self.skip()?;
        }

        self.close_at(&c, Syntax::Error)?;
        self.primary = Some(c);
        Ok(State::Operator)
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

    /// Open a group at the peeked token `open`. Past `max_depth`, what it
    /// holds is skipped as an error, without nesting.
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
            Delimiter::Map => Nesting::Map,
        };

        let fits = self.nest(nesting, open.span());

        if !fits {
            // Closing the group leaves the level it did not enter.
            self.depth = self.depth.saturating_add(1);
        }

        self.bump()?;

        let count = self.count(delimiter.close());
        *count = count.saturating_add(1);
        self.frames.push(Frame::Group);
        self.groups.push(Group {
            delimiter,
            start,
            args,
            open: open.span(),
            outer: self.start.take(),
        });

        self.primary = None;

        if !fits {
            let c = self.checkpoint()?;
            self.skip_group()?;
            self.close_at(&c, Syntax::Error)?;
            let closed = self.peek().is_some();
            return self.close_group(closed);
        }

        if matches!(
            delimiter,
            Delimiter::Args | Delimiter::List | Delimiter::Map
        ) && self.peek_kind() == Some(delimiter.close())
        {
            return self.close_group(true);
        }

        if delimiter == Delimiter::Map {
            self.open_entry()?;
        }

        Ok(State::Operand)
    }

    /// Start a map's entry at the next token.
    fn open_entry(&mut self) -> Result<(), Diagnostic> {
        let start = self.checkpoint()?;
        self.frames.push(Frame::Entry {
            start,
            colon: false,
            reported: false,
        });
        Ok(())
    }

    /// The map entry being parsed, if the innermost group is a map: whether
    /// its `:` is passed.
    fn entry(&self) -> Option<bool> {
        self.frames.iter().rev().find_map(|frame| match frame {
            Frame::Entry { colon, .. } => Some(Some(*colon)),
            Frame::Group => Some(None),
            _ => None,
        })?
    }

    /// Whether the map entry being parsed lacks its `:`, which is not yet
    /// reported.
    fn key_unreported(&self) -> bool {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| match frame {
                Frame::Entry {
                    colon, reported, ..
                } => Some(!*colon && !*reported),
                Frame::Group => Some(false),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Note that the map entry being parsed has its missing `:` reported.
    fn report_key_end(&mut self) {
        let entry = self.frames.iter_mut().rev().find_map(|frame| match frame {
            Frame::Entry { reported, .. } => Some(Some(reported)),
            Frame::Group => Some(None),
            _ => None,
        });

        if let Some(Some(reported)) = entry {
            *reported = true;
        }
    }

    /// Close the group on top of the stack, at the peeked token if
    /// `closer`, otherwise where it is, without one.
    fn close_group(&mut self, closer: bool) -> Result<State, Diagnostic> {
        let (Some(Frame::Group), Some(group)) = (self.frames.pop(), self.groups.pop()) else {
            return Err(internal("no group to close"));
        };

        let count = self.count(group.delimiter.close());
        *count = count.saturating_sub(1);

        self.depth = self.depth.saturating_sub(1);

        if closer {
            self.bump()?;
        }

        match (group.delimiter, &group.args) {
            (Delimiter::Args, Some((args, kind))) => {
                self.close_at(args, Syntax::Args)?;
                self.close_at(&group.start, *kind)?;
            }
            (Delimiter::Paren, _) => self.close_at(&group.start, Syntax::Paren)?,
            (Delimiter::List, _) => self.close_at(&group.start, Syntax::List)?,
            (Delimiter::Index, _) => self.close_at(&group.start, Syntax::Index)?,
            (Delimiter::Map, _) => self.close_at(&group.start, Syntax::Map)?,
            (Delimiter::Args, None) => return Err(internal("arguments without a node")),
        }

        self.start = group.outer;
        self.primary = Some(group.start);
        Ok(State::Operator)
    }

    /// Reduce the frames binding at least as tightly as `power`, giving
    /// where the operand they make up starts, and whether a comparison was
    /// reduced for another comparison, which do not chain.
    fn reduce(&mut self, power: u8) -> Result<(Checkpoint, bool), Diagnostic> {
        let mut left = self.start.clone();
        let mut chained = false;

        loop {
            let (start, kind) = match self.frames.last() {
                Some(Frame::Prefix { start }) => (start.clone(), Syntax::Unary),
                Some(Frame::Binary {
                    start,
                    power: p,
                    error,
                }) if *p >= power => {
                    // A chain already reported is not reported again.
                    chained |= *p == COMPARE && power == COMPARE && !*error;
                    let kind = if *error {
                        Syntax::Error
                    } else {
                        Syntax::Binary
                    };
                    (start.clone(), kind)
                }
                Some(Frame::Entry { start, colon, .. }) if power == 0 => {
                    let kind = if *colon { Syntax::Entry } else { Syntax::Error };
                    (start.clone(), kind)
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

        let left = left.ok_or_else(|| internal("an operator without an operand"))?;
        Ok((left, chained))
    }

    fn innermost(&self) -> Option<&Group> {
        self.groups.last()
    }

    /// How many open groups the closer `close` closes.
    fn count(&mut self, close: Syntax) -> &mut usize {
        match close {
            Syntax::CloseParen => &mut self.parens,
            Syntax::CloseBrace => &mut self.braces,
            _ => &mut self.brackets,
        }
    }

    /// Parse what follows a complete primary: postfixes, an operator, a
    /// separator or closer, or the end, which gives `None`.
    fn operator(&mut self) -> Result<Option<State>, Diagnostic> {
        let Some(token) = self.peek() else {
            self.reduce(0)?;

            // Each group still open is closed at the end.
            while let Some(group) = self.innermost() {
                let at = self.end_span();
                let diagnostic = group.unclosed(at, Found::End);
                self.report_about(at, diagnostic);
                self.close_group(false)?;
                self.reduce(0)?;
            }

            return Ok(None);
        };

        let Some(primary) = self.primary.clone() else {
            return Err(internal("an operator without a primary"));
        };

        if let Some(power) = binary(token.kind) {
            let (left, chained) = self.reduce(power)?;

            if chained {
                self.report(token.span(), ErrorKind::ChainedComparison);
            }

            let error = chained || self.rejected;
            self.bump()?;
            self.frames.push(Frame::Binary {
                start: left,
                power,
                error,
            });
            self.start = None;
            self.primary = None;
            return Ok(Some(State::Operand));
        }

        match token.kind {
            Syntax::Dot => {
                self.bump()?;

                let name = self.peek();

                if name.is_none_or(|t| t.kind != Syntax::NameToken) {
                    let (found, at) = self.found(name);
                    self.report(at, ErrorKind::ExpectedName(found));
                    self.skip()?;
                    self.close_at(&primary, Syntax::Error)?;
                    return Ok(Some(State::Operator));
                }

                let c = self.checkpoint()?;
                self.bump()?;
                self.close_at(&c, Syntax::Name)?;

                if let Some(open) = self.peek().filter(|t| t.kind == Syntax::OpenParen) {
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
            Syntax::Comma => self.comma(token).map(Some),
            Syntax::Colon if self.entry() == Some(false) => {
                // The key is complete; the entry stays open for its value.
                self.reduce(OR)?;

                if let Some(Frame::Entry { colon, .. }) = self.frames.last_mut() {
                    *colon = true;
                }

                self.bump()?;
                self.start = None;
                self.primary = None;
                Ok(Some(State::Operand))
            }
            kind if closes(kind) => self.closer(token).map(Some),
            _ => {
                let open = self.innermost().map(|group| group.delimiter);
                let (found, at) = self.found(Some(token));

                let kind = if self.entry() == Some(false) {
                    // The entry's one missing `:` is reported here, not
                    // again at its `,` or closer.
                    self.report_key_end();
                    ErrorKind::ExpectedKeyEnd(found)
                } else {
                    ErrorKind::ExpectedOperator { found, open }
                };

                self.report(at, kind);
                self.skip()?;
                self.close_at(&primary, Syntax::Error)?;
                Ok(Some(State::Operator))
            }
        }
    }

    /// A `,` after an operand: the next entry of a list or arguments,
    /// otherwise an error, skipped with what follows it.
    fn comma(&mut self, token: Token) -> Result<State, Diagnostic> {
        let key = self.key_unreported();
        let (left, _) = self.reduce(0)?;
        let (found, at) = self.found(Some(token));

        let open = self.innermost().map(|group| group.delimiter);

        if open.is_some_and(Delimiter::separated) {
            if key {
                self.report(at, ErrorKind::ExpectedKeyEnd(found));
            }

            self.bump()?;
            self.start = None;
            self.primary = None;

            if open == Some(Delimiter::Map) {
                self.open_entry()?;
            }

            return Ok(State::Operand);
        }

        self.report(at, ErrorKind::ExpectedOperator { found, open });
        self.bump()?;
        self.skip()?;
        self.recovered(left)?;
        Ok(State::Operator)
    }

    /// A `)` or `]` after an operand. It closes the innermost group it
    /// matches, and the groups inside that one, which are reported as not
    /// closed. Matching none, it closes the innermost group, reported as
    /// closed by the wrong token, or with no group open it is reported
    /// and skipped.
    fn closer(&mut self, token: Token) -> Result<State, Diagnostic> {
        // A map's key closed without its `:`.
        let key = self.key_unreported() && token.kind == Syntax::CloseBrace;
        let (left, _) = self.reduce(0)?;
        let (found, at) = self.found(Some(token));

        if key {
            self.report(at, ErrorKind::ExpectedKeyEnd(found));
        }

        let matches = *self.count(token.kind) > 0;

        let Some(group) = self.innermost() else {
            self.report(at, ErrorKind::Unmatched(found));
            self.bump()?;
            self.recovered(left)?;
            return Ok(State::Operator);
        };

        if !matches {
            let diagnostic = group.unclosed(at, found);
            self.report_about(at, diagnostic);
            return self.close_group(true);
        }

        // Each group is closed once, so this is linear over the parse.
        while let Some(group) = self
            .innermost()
            .filter(|group| group.delimiter.close() != token.kind)
        {
            let diagnostic = group.unclosed(at, found);
            self.report_about(at, diagnostic);
            self.close_group(false)?;
            self.reduce(0)?;
        }

        self.close_group(true)
    }
}

/// Whether `kind` opens a group.
fn opens(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::OpenParen | Syntax::OpenBracket | Syntax::OpenBrace
    )
}

/// Whether `kind` closes a group.
fn closes(kind: Syntax) -> bool {
    matches!(
        kind,
        Syntax::CloseParen | Syntax::CloseBracket | Syntax::CloseBrace
    )
}

fn internal(error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(span(0, 0), ErrorKind::Internal(error.to_string()))
}
