//! Rewrites: one path the references of an expression read replaced by
//! another, as edits of exactly the parts that change.

use std::fmt;
use std::mem;

use syntree::Span;

use crate::check::compile_with;
use crate::env::Environment;
use crate::error::Diagnostics;
use crate::parser::Options;
use crate::references::{Path, Reference, ReferenceKind, Step, StepRepr, quoted, references};
use crate::structure::{EditError, Node, NodeId, Repr, Structure, StructureErrors, is_reserved};
use crate::types::{is_field_name, is_value_name};

/// Replace the path `from` with `to` wherever an expression reads it, or a
/// path past it, for [`rewrite`] and [`Structure::rewrite`].
///
/// The two paths start at the same kind of thing, a value or a function,
/// and have the same kinds of steps, so each part is replaced in place:
/// renaming a value or a function (`rank` to `level`), a field
/// (`stats.rank` to `stats.level`), or re-keying (`m[0x1]` to `m[0x2]`).
/// With [`check`](Self::check), the result must compile against that
/// environment, else nothing is rewritten.
///
/// ```
/// use evalite::{Environment, Options, Path, Rewrite, Type};
///
/// let mut env = Environment::new();
/// env.value("stats", Type::record([("rank", Type::int())]))?;
///
/// let mut renamed = Environment::new();
/// renamed.value("stats", Type::record([("level", Type::int())]))?;
///
/// let rename = Rewrite::new(Path::value("stats").field("rank"), Path::value("stats").field("level"))
///     .check(&renamed);
///
/// let (text, n) = evalite::rewrite("stats.rank*2 + stats . rank", &env, &rename, &Options::new())?;
/// assert_eq!((text.as_str(), n), ("stats.level*2 + stats . level", 2));
///
/// // `stats.rank` does not compile against `renamed`.
/// let error = evalite::rewrite("stats.rank", &renamed, &rename, &Options::new()).unwrap_err();
/// assert_eq!(error.to_string(), "the expression does not compile");
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Rewrite<'a> {
    from: Path,
    to: Path,
    check: Option<&'a Environment>,
}

impl<'a> Rewrite<'a> {
    /// Rewrite `from` to `to`, unchecked.
    pub fn new(from: Path, to: Path) -> Self {
        Self {
            from,
            to,
            check: None,
        }
    }

    /// Refuse the rewrite unless its result compiles against `env`, such as
    /// the environment that declares the new names.
    pub fn check(mut self, env: &'a Environment) -> Self {
        self.check = Some(env);
        self
    }

    /// The new root name and new steps, each only where it changes, or
    /// why the paths cannot be rewritten into one another.
    fn plan(&self) -> Result<Plan<'_>, RewriteError> {
        let (from, to) = (&self.from, &self.to);

        let same = from.kind == to.kind
            && from.steps.len() == to.steps.len()
            && from.steps.iter().zip(&to.steps).all(|(a, b)| match (a, b) {
                // A map is keyed by Ints or by strings, never both.
                (StepRepr::Key(a), StepRepr::Key(b)) => {
                    a.as_int().is_some() == b.as_int().is_some()
                }
                _ => mem::discriminant(a) == mem::discriminant(b),
            });

        if !same {
            return Err(RewriteKind::Shape(from.clone(), to.clone()).into());
        }

        let root = (from.name != to.name).then_some(to.name.as_str());

        if root.is_some_and(|name| !is_value_name(name) || is_reserved(name)) {
            return Err(RewriteKind::InvalidName(to.name.clone()).into());
        }

        let steps = from
            .steps
            .iter()
            .zip(&to.steps)
            .map(|(a, b)| (a != b).then_some(b))
            .collect::<Vec<_>>();

        let unchanged = root.is_none() && steps.iter().all(Option::is_none);
        Ok(Plan {
            root,
            steps,
            unchanged,
        })
    }
}

struct Plan<'p> {
    root: Option<&'p str>,
    /// For each step of `from`, what it becomes, if it changes.
    steps: Vec<Option<&'p StepRepr>>,
    unchanged: bool,
}

impl Plan<'_> {
    /// The references `rewrite` changes.
    fn matching<'r>(
        &self,
        rewrite: &Rewrite<'_>,
        refs: &'r [Reference],
    ) -> impl Iterator<Item = &'r Reference> {
        let unchanged = self.unchanged;
        let from = &rewrite.from;
        refs.iter()
            .filter(move |r| !unchanged && r.starts_with(from))
    }
}

/// Rewrite `source` as `rewrite` says, giving the new text and how many
/// references were rewritten. Only the bytes of the parts that change are
/// replaced, so the rest keeps its spacing and parentheses. A field whose
/// new name is not a plain name is written `["Brawl (bar)"]`, and a key or
/// an index written in hexadecimal stays hexadecimal.
///
/// `source` must compile against `env`, as for [`references`], which finds
/// what to rewrite. It is one walk over the expression, plus the edits.
///
/// ```
/// use evalite::{Environment, Options, Path, Rewrite, Type};
///
/// let mut env = Environment::new();
/// let specialty = Type::record([("dice", Type::int())]);
/// env.value("specialties", Type::map(Type::int(), specialty))?;
///
/// let rekey = Rewrite::new(
///     Path::value("specialties").int_key(0x1A).field("dice"),
///     Path::value("specialties").int_key(0x2B).field("Brawl (bar)"),
/// );
///
/// let source = "specialties[0x1A].dice + specialties.get(26)!.dice";
/// let (text, n) = evalite::rewrite(source, &env, &rekey, &Options::new())?;
/// assert_eq!(text, r#"specialties[0x2B]["Brawl (bar)"] + specialties.get(43)!["Brawl (bar)"]"#);
/// assert_eq!(n, 2);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Returns a [`RewriteError`] when the two paths are not the same kind of
/// path, the new name of a value or a function is not a valid name,
/// `source` does not compile against `env` with `options`, or the result
/// does not compile against the [`check`](Rewrite::check) environment.
pub fn rewrite(
    source: &str,
    env: &Environment,
    rewrite: &Rewrite<'_>,
    options: &Options,
) -> Result<(String, usize), RewriteError> {
    let plan = rewrite.plan()?;
    let refs = references(source, env, options).map_err(|e| RewriteKind::Source(e.into()))?;

    let mut edits = Vec::new();
    let mut count = 0usize;

    for reference in plan.matching(rewrite, &refs) {
        count = count.saturating_add(1);

        if let Some(name) = plan.root {
            edits.push((reference.span(), name.to_owned()));
        }

        for (step, new) in reference.steps().iter().zip(&plan.steps) {
            if let Some(new) = new {
                edits.push(text_edit(source, step, new)?);
            }
        }
    }

    let text = apply(source, edits)?;

    if let Some(check) = rewrite.check {
        compile_with(&text, check, options).map_err(|e| RewriteKind::Check(e.into()))?;
    }

    Ok((text, count))
}

/// The bytes to replace to make `step` read `new`, and what with.
fn text_edit(
    source: &str,
    step: &Step,
    new: &StepRepr,
) -> Result<(Span<u32>, String), RewriteError> {
    let span = step.span();
    let old = source.get(span.range()).ok_or(RewriteKind::Internal)?;

    let text = match new {
        StepRepr::Field(name) if old.starts_with('"') => quoted(name),
        StepRepr::Field(name) if is_plain(name) => name.clone(),
        StepRepr::Field(name) => {
            // `.x` becomes `["x y"]`, from the `.` before the name.
            let before = source
                .get(..span.range().start)
                .ok_or(RewriteKind::Internal)?
                .trim_end();
            let dot = before.strip_suffix('.').ok_or(RewriteKind::Internal)?.len();
            let dot = u32::try_from(dot).map_err(|_| RewriteKind::Internal)?;
            return Ok((Span::new(dot, span.end), format!("[{}]", quoted(name))));
        }
        StepRepr::Key(key) => match (key.as_int(), key.as_str()) {
            (Some(n), _) => int_text(old, n),
            (_, Some(s)) => quoted(s),
            _ => return Err(RewriteKind::Internal.into()),
        },
        StepRepr::Index(n) => int_text(old, *n),
        StepRepr::Dynamic => return Err(RewriteKind::Internal.into()),
    };

    Ok((span, text))
}

/// Whether a field named `name` is written `.name`.
fn is_plain(name: &str) -> bool {
    is_field_name(name) && !is_reserved(name)
}

/// `n` written as the integer literal `old` is: in hexadecimal, with the
/// same `0x` and the same case of digits, when `old` is.
fn int_text(old: &str, n: i64) -> String {
    let digits = old.trim_start_matches('-').trim_start();

    let Some(prefix) = digits.get(..2).filter(|p| p.eq_ignore_ascii_case("0x")) else {
        return n.to_string();
    };

    let sign = if n < 0 { "-" } else { "" };
    let abs = n.unsigned_abs();

    if digits.chars().skip(2).any(|c| c.is_ascii_lowercase()) {
        format!("{sign}{prefix}{abs:x}")
    } else {
        format!("{sign}{prefix}{abs:X}")
    }
}

/// `source` with each of `edits` made.
fn apply(source: &str, mut edits: Vec<(Span<u32>, String)>) -> Result<String, RewriteError> {
    edits.sort_by_key(|(span, _)| span.start);

    let extra = edits
        .iter()
        .fold(0usize, |n, (_, text)| n.saturating_add(text.len()));
    let mut out = String::with_capacity(source.len().saturating_add(extra));
    let mut at = 0usize;

    for (span, text) in edits {
        let range = span.range();

        let kept = source.get(at..range.start).ok_or(RewriteKind::Internal)?;

        out.push_str(kept);
        out.push_str(&text);
        at = range.end;
    }

    out.push_str(source.get(at..).ok_or(RewriteKind::Internal)?);
    Ok(out)
}

impl Structure {
    /// Rewrite the structure as `rewrite` says, as [`rewrite`] does its
    /// text, editing the nodes of the parts that change, and give how many
    /// references were rewritten. A field that cannot be written `.name`
    /// becomes an index by a string, and a key or an index written in
    /// hexadecimal stays hexadecimal. On an error the structure is left as
    /// it was.
    ///
    /// ```
    /// use evalite::{Environment, Node, Options, Path, Rewrite, Structure, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("tags", Type::map(Type::int(), Type::int()))?;
    /// env.function("roll", [Type::int()], Type::int())?;
    ///
    /// let mut s = Structure::from_text("roll(tags[0x1F])")?;
    /// let rekey = Rewrite::new(Path::value("tags").int_key(0x1F), Path::value("tags").int_key(-2));
    /// assert_eq!(s.rewrite(&env, &rekey, &Options::new())?, 1);
    /// assert_eq!(s.to_text(), "roll(tags[-0x2])");
    ///
    /// let rename = Rewrite::new(Path::function("roll"), Path::function("dice"));
    /// assert!(s.rewrite(&env, &rename.clone().check(&env), &Options::new()).is_err());
    /// assert_eq!(s.to_text(), "roll(tags[-0x2])");
    /// assert_eq!(s.rewrite(&env, &rename, &Options::new())?, 1);
    /// assert_eq!(s.to_text(), "dice(tags[-0x2])");
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`rewrite`]: a [`RewriteError`] when the two paths are not the
    /// same kind of path, a new name is not valid, the structure does not
    /// compile against `env` with `options`, or the result does not compile
    /// against the [`check`](Rewrite::check) environment.
    pub fn rewrite(
        &mut self,
        env: &Environment,
        rewrite: &Rewrite<'_>,
        options: &Options,
    ) -> Result<usize, RewriteError> {
        let plan = rewrite.plan()?;
        let refs = self
            .references(env, options)
            .map_err(|e| RewriteKind::Source(e.into()))?;

        let mut new = self.clone();
        let mut count = 0usize;

        for reference in plan.matching(rewrite, &refs) {
            count = count.saturating_add(1);

            if let Some(name) = plan.root {
                rename(&mut new, reference, name)?;
            }

            for (step, to) in reference.steps().iter().zip(&plan.steps) {
                if let Some(to) = to {
                    let id = step.node().ok_or(RewriteKind::Internal)?;
                    edit_step(&mut new, id, to)?;
                }
            }
        }

        if let Some(check) = rewrite.check {
            new.compile_with(check, options)
                .map_err(|e| RewriteKind::Check(e.into()))?;
        }

        *self = new;
        Ok(count)
    }
}

/// Rename the value or the function `reference` starts at to `name`.
fn rename(s: &mut Structure, reference: &Reference, name: &str) -> Result<(), RewriteError> {
    let id = reference.node().ok_or(RewriteKind::Internal)?;
    let node = s.get(id).ok_or(RewriteKind::Internal)?;

    let node = match reference.kind() {
        ReferenceKind::Function => {
            let (_, args) = node.as_call().ok_or(RewriteKind::Internal)?;
            Node::call(name, args.to_vec())
        }
        _ => {
            node.as_name().ok_or(RewriteKind::Internal)?;
            Node::name(name)
        }
    };

    s.replace(id, node)?;
    Ok(())
}

/// Make the step at node `id` read `to`.
fn edit_step(s: &mut Structure, id: NodeId, to: &StepRepr) -> Result<(), RewriteError> {
    let old = s.get(id).ok_or(RewriteKind::Internal)?;
    let hex = matches!(old.0, Repr::Hex(_));

    let node = match to {
        StepRepr::Field(name) => match old.as_field() {
            Some((receiver, _)) if is_plain(name) => Node::field(receiver, Some(name)),
            Some((receiver, _)) => {
                let key = s.add(Node::string(name))?;
                Node::index(receiver, key)
            }
            None if old.as_string().is_some() => Node::string(name),
            None => return Err(RewriteKind::Internal.into()),
        },
        // A key or an index is a literal.
        _ if old.as_int().is_none() && old.as_string().is_none() => {
            return Err(RewriteKind::Internal.into());
        }
        StepRepr::Key(key) => match (key.as_int(), key.as_str()) {
            (Some(n), _) => int_node(hex, n),
            (_, Some(k)) => Node::string(k),
            _ => return Err(RewriteKind::Internal.into()),
        },
        StepRepr::Index(n) => int_node(hex, *n),
        StepRepr::Dynamic => return Err(RewriteKind::Internal.into()),
    };

    s.replace(id, node)?;
    Ok(())
}

fn int_node(hex: bool, n: i64) -> Node {
    if hex {
        Node(Repr::Hex(n))
    } else {
        Node::int(n)
    }
}

/// Why an expression or a structure could not be rewritten, by
/// [`rewrite`] or [`Structure::rewrite`]. `{:#}` adds the compile errors
/// of an expression that does not compile, which are also its
/// [`source`](std::error::Error::source).
///
/// ```
/// use evalite::{Environment, Options, Path, Rewrite};
///
/// let to_call = Rewrite::new(Path::value("rank"), Path::function("rank"));
/// let error = evalite::rewrite("1", &Environment::new(), &to_call, &Options::new()).unwrap_err();
/// assert_eq!(error.to_string(), "cannot rewrite `rank` to `rank()`, which starts at another kind of name or has other kinds of steps");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteError {
    kind: RewriteKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RewriteKind {
    Shape(Path, Path),
    InvalidName(String),
    /// The expression does not compile against its environment.
    Source(Errors),
    /// The rewritten expression does not compile against the check
    /// environment.
    Check(Errors),
    Edit(EditError),
    /// The references found do not fit the expression, a bug in evalite.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Errors {
    Text(Diagnostics),
    Structure(StructureErrors),
}

impl Errors {
    fn as_error(&self) -> &(dyn std::error::Error + 'static) {
        match self {
            Self::Text(e) => e,
            Self::Structure(e) => e,
        }
    }
}

impl From<Diagnostics> for Errors {
    fn from(e: Diagnostics) -> Self {
        Self::Text(e)
    }
}

impl From<StructureErrors> for Errors {
    fn from(e: StructureErrors) -> Self {
        Self::Structure(e)
    }
}

impl From<RewriteKind> for RewriteError {
    #[inline]
    fn from(kind: RewriteKind) -> Self {
        Self { kind }
    }
}

impl From<EditError> for RewriteError {
    #[inline]
    fn from(e: EditError) -> Self {
        RewriteKind::Edit(e).into()
    }
}

impl fmt::Display for RewriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            RewriteKind::Shape(from, to) => write!(
                f,
                "cannot rewrite `{from}` to `{to}`, which starts at another kind of name or has other kinds of steps"
            ),
            RewriteKind::InvalidName(name) => write!(
                f,
                "`{name}` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`"
            ),
            RewriteKind::Source(e) => {
                f.write_str("the expression does not compile")?;

                if f.alternate() {
                    write!(f, ": {}", e.as_error())?;
                }

                Ok(())
            }
            RewriteKind::Check(e) => {
                f.write_str("the rewritten expression does not compile")?;

                if f.alternate() {
                    write!(f, ": {}", e.as_error())?;
                }

                Ok(())
            }
            RewriteKind::Edit(e) => e.fmt(f),
            RewriteKind::Internal => f.write_str("evalite could not place an edit of the rewrite"),
        }
    }
}

impl std::error::Error for RewriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            RewriteKind::Source(e) | RewriteKind::Check(e) => Some(e.as_error()),
            RewriteKind::Edit(e) => Some(e),
            _ => None,
        }
    }
}
