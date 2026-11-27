//! References: the names, fields, keys and indexes an expression uses, as
//! paths from the value or function they start at, recorded by the
//! checker's own walk so every step is typed as compiling types it.

use std::collections::HashMap;

use syntree::Span;

use crate::check::references_parsed;
use crate::env::Environment;
use crate::error::Diagnostics;
use crate::parser::{Options, parse_with};
use crate::structure::NodeId;
use crate::value::Value;

/// What a [`Reference`] starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReferenceKind {
    /// A value of the environment, such as `rank`.
    Value,
    /// A call of the environment's function, such as `roll(2)`.
    Function,
}

/// What a [`Step`] of a [`Reference`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StepKind {
    /// A record's field, written `.x` or `["x y"]`. Its name is
    /// [`Step::name`].
    Field,
    /// A map's key, written as a literal: `m[0x1A]`, `m["brawl"]` or
    /// `m.get(7)`. Its key is [`Step::key`].
    Key,
    /// A list's index, written as a literal: `list[0]` or `list.get(2)`.
    /// Its index is [`Step::index`].
    Index,
    /// A list's index or a map's key that is computed, such as `list[n]`
    /// or `m[k + 1]`: the path past it is only partly known.
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StepRepr {
    Field(String),
    Key(Value),
    Index(i64),
    Dynamic,
}

/// One step along a [`Reference`]'s path, from what the step before it
/// gives.
///
/// Its [`span`](Self::span) is exactly the bytes that name it: a field's
/// name, `"x y"` with its quotes, a key's or index's literal, or the
/// expression that computes a dynamic one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    repr: StepRepr,
    span: Span<u32>,
    /// The span of the syntax node that holds it: a dotted field's whole
    /// `x.name`, else the same as `span`. A structure's node is found by it.
    at: Span<u32>,
    node: Option<NodeId>,
}

impl Step {
    /// What kind of step it is.
    pub fn kind(&self) -> StepKind {
        match self.repr {
            StepRepr::Field(_) => StepKind::Field,
            StepRepr::Key(_) => StepKind::Key,
            StepRepr::Index(_) => StepKind::Index,
            StepRepr::Dynamic => StepKind::Dynamic,
        }
    }

    /// The field's name, of a [`StepKind::Field`] step, however it is
    /// written.
    pub fn name(&self) -> Option<&str> {
        match &self.repr {
            StepRepr::Field(name) => Some(name),
            _ => None,
        }
    }

    /// The key, an `Int` or a `String`, of a [`StepKind::Key`] step.
    pub fn key(&self) -> Option<&Value> {
        match &self.repr {
            StepRepr::Key(key) => Some(key),
            _ => None,
        }
    }

    /// The index of a [`StepKind::Index`] step.
    pub fn index(&self) -> Option<i64> {
        match self.repr {
            StepRepr::Index(index) => Some(index),
            _ => None,
        }
    }

    /// Where it is written: in the source, or in a structure's
    /// [`to_text`](crate::Structure::to_text).
    pub fn span(&self) -> Span<u32> {
        self.span
    }

    /// The node it is part of, for references of a
    /// [`Structure`](crate::Structure): a dotted field's
    /// [`Node::field`](crate::Node::field), else the literal or
    /// expression its [`span`](Self::span) covers. `None` for references of
    /// text.
    pub fn node(&self) -> Option<NodeId> {
        self.node
    }
}

/// A name an expression uses, and the path it reads from it, as
/// [`references`] reports it.
///
/// A path continues through `x!`, `x.unwrap_or(..)`, `list.get(i)` and
/// `map.get(k)`, and their paths such as `Map::get(m, k)`, since what they
/// give is still part of `x`; it ends at anything else, such as
/// `x.len()` or `x + 1`. So `a.unwrap_or(b).c` reports `a` then `.c`,
/// and `b` alone.
///
/// ```
/// use evalite::{Environment, Options, ReferenceKind, StepKind, Type};
///
/// let mut env = Environment::new();
/// let specialty = Type::record([("dice", Type::int())]);
/// env.value("specialties", Type::map(Type::int(), specialty))?;
/// env.function("roll", [Type::int()], Type::int())?;
///
/// let source = "roll(specialties.get(0x1A)!.dice)";
/// let refs = evalite::references(source, &env, &Options::new())?;
///
/// let [roll, specialties] = refs.as_slice() else { panic!() };
/// assert_eq!((roll.kind(), roll.name()), (ReferenceKind::Function, "roll"));
/// assert!(roll.steps().is_empty());
///
/// assert_eq!(specialties.name(), "specialties");
/// let kinds = specialties.steps().iter().map(|s| s.kind()).collect::<Vec<_>>();
/// assert_eq!(kinds, [StepKind::Key, StepKind::Field]);
/// assert_eq!(&source[specialties.steps()[0].span().range()], "0x1A");
/// assert_eq!(specialties.steps()[1].name(), Some("dice"));
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    kind: ReferenceKind,
    name: String,
    span: Span<u32>,
    /// The span of the syntax node that holds it: a call's whole text,
    /// else the same as `span`.
    at: Span<u32>,
    node: Option<NodeId>,
    steps: Vec<Step>,
}

impl Reference {
    /// What it starts at: a value or a function call.
    pub fn kind(&self) -> ReferenceKind {
        self.kind
    }

    /// The value's or the function's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the name is written: in the source, or in a structure's
    /// [`to_text`](crate::Structure::to_text).
    pub fn span(&self) -> Span<u32> {
        self.span
    }

    /// The node of the name, for references of a
    /// [`Structure`](crate::Structure): a [`Node::name`](crate::Node::name)
    /// or a [`Node::call`](crate::Node::call). `None` for references of
    /// text.
    pub fn node(&self) -> Option<NodeId> {
        self.node
    }

    /// The path read from it, in order. Empty when the name is used as it
    /// is.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }
}

/// Every name `source` uses, as a [`Reference`] with the path it reads,
/// in the order they are written. `source` must compile against `env`,
/// since what a step is, such as a map's key or a list's index, is known
/// only from the types.
///
/// It is one walk over the expression, the checker's, so it takes time in
/// proportion to the expression and never recurses.
///
/// ```
/// use evalite::{Environment, Options, StepKind, Type, Value};
///
/// let mut env = Environment::new();
/// env.value("health", Type::record([("boxes", Type::list(Type::int()))]))?;
/// env.value("tags", Type::map(Type::string(), Type::int()))?;
/// env.value("n", Type::int())?;
///
/// let source = r#"health["boxes"][n] + tags["brawl"]"#;
/// let refs = evalite::references(source, &env, &Options::new())?;
/// let names = refs.iter().map(|r| r.name()).collect::<Vec<_>>();
/// assert_eq!(names, ["health", "n", "tags"]);
///
/// let health = refs[0].steps();
/// assert_eq!(health[0].name(), Some("boxes"));
/// assert_eq!(health[1].kind(), StepKind::Dynamic);
/// assert_eq!(&source[health[1].span().range()], "n");
/// assert_eq!(refs[2].steps()[0].key(), Some(&Value::string("brawl")));
///
/// let errors = evalite::references("health.count", &env, &Options::new()).unwrap_err();
/// assert_eq!(errors.to_string(), r#"this record has no field "count", its fields are "boxes""#);
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// As [`compile_with`](crate::compile_with): [`Diagnostics`] when `source`
/// does not parse, nests deeper than `options` allows, or does not type
/// check against `env`.
pub fn references(
    source: &str,
    env: &Environment,
    options: &Options,
) -> Result<Vec<Reference>, Diagnostics> {
    references_parsed(source, parse_with(source, options)?, env)
}

/// Give each reference and step its node: the one whose span in the
/// rendered text is that of the syntax node holding it.
pub(crate) fn locate_nodes(
    references: &mut [Reference],
    spans: &[(NodeId, Span<u32>)],
    text: &str,
) {
    let mut by_span = HashMap::with_capacity(spans.len());

    for (id, span) in spans {
        by_span.entry(*span).or_insert(*id);

        // A parenthesised node's span has its parentheses: the expression
        // inside them is the node too.
        if text.get(span.range()).is_some_and(|t| t.starts_with('(')) {
            let inner = Span::new(span.start.saturating_add(1), span.end.saturating_sub(1));
            by_span.entry(inner).or_insert(*id);
        }
    }

    for reference in references {
        reference.node = by_span.get(&reference.at).copied();

        for step in &mut reference.steps {
            step.node = by_span.get(&step.at).copied();
        }
    }
}

/// What the checker records as it walks: each reference so far, which
/// values on its stack extend by their index.
#[derive(Default)]
pub(crate) struct Recorder {
    references: Vec<Reference>,
}

/// A literal a key or an index is written as.
pub(crate) enum Literal {
    Int(i64),
    String(String),
}

impl Recorder {
    /// Start a reference at `name`, written at `span` in a node at `at`,
    /// and give its index.
    pub(crate) fn root(
        &mut self,
        kind: ReferenceKind,
        name: &str,
        span: Span<u32>,
        at: Span<u32>,
    ) -> Option<u32> {
        let n = u32::try_from(self.references.len()).ok()?;

        self.references.push(Reference {
            kind,
            name: name.to_owned(),
            span,
            at,
            node: None,
            steps: Vec::new(),
        });

        Some(n)
    }

    /// Read the field `name`, written at `span` in a node at `at`, along
    /// reference `path`.
    pub(crate) fn field(&mut self, path: u32, name: &str, span: Span<u32>, at: Span<u32>) {
        self.push(path, StepRepr::Field(name.to_owned()), span, at);
    }

    /// Index reference `path` with the expression at `span`, a map's key
    /// when `map`, else a list's index, which is a literal when `literal`
    /// is.
    pub(crate) fn index(
        &mut self,
        path: u32,
        map: bool,
        literal: Option<Literal>,
        span: Span<u32>,
    ) {
        let repr = match literal {
            Some(Literal::Int(n)) if map => StepRepr::Key(Value::int(n)),
            Some(Literal::String(s)) if map => StepRepr::Key(Value::string(s)),
            Some(Literal::Int(n)) => StepRepr::Index(n),
            _ => StepRepr::Dynamic,
        };

        self.push(path, repr, span, span);
    }

    fn push(&mut self, path: u32, repr: StepRepr, span: Span<u32>, at: Span<u32>) {
        let reference = usize::try_from(path)
            .ok()
            .and_then(|n| self.references.get_mut(n));

        if let Some(reference) = reference {
            reference.steps.push(Step {
                repr,
                span,
                at,
                node: None,
            });
        }
    }

    pub(crate) fn finish(self) -> Vec<Reference> {
        self.references
    }
}
