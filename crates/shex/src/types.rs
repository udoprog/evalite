use std::collections::{BTreeMap, HashMap};
use std::fmt;

use musli::{Decode, Encode};

/// How deeply a type may nest, such as `List<List<Int>>` at 2. Declared
/// types past it are refused, and so are stored programs.
pub const MAX_TYPE_DEPTH: usize = 64;

/// The type of a value.
///
/// `Option<Option<T>>` is not a type: an optional value is either absent or
/// a `T`, so [`Environment`](crate::Environment) refuses declaring one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Type {
    /// A 64-bit signed integer.
    Int,
    /// `true` or `false`.
    Bool,
    /// A string literal, such as a library table's name.
    String,
    /// Entries of one type.
    List(Box<Type>),
    /// A value that may be absent, unwrapped with `unwrap_or`.
    Option(Box<Type>),
    /// Named fields, read with `.name` or `["name"]`.
    Record(BTreeMap<String, Type>),
    /// A value of the caller's that only its functions use, such as a
    /// `Track` that `count` reads. Its name starts with an uppercase letter.
    Opaque(String),
}

impl Type {
    /// `List<T>`.
    pub fn list(entry: Type) -> Self {
        Self::List(Box::new(entry))
    }

    /// `Option<T>`.
    pub fn option(value: Type) -> Self {
        Self::Option(Box::new(value))
    }

    /// A record of these fields.
    pub fn record<K>(fields: impl IntoIterator<Item = (K, Type)>) -> Self
    where
        K: Into<String>,
    {
        Self::Record(fields.into_iter().map(|(k, t)| (k.into(), t)).collect())
    }

    /// An opaque type of the caller's.
    pub fn opaque(name: impl Into<String>) -> Self {
        Self::Opaque(name.into())
    }

    pub(crate) fn is_option(&self) -> bool {
        matches!(self, Self::Option(..))
    }
}

/// Type names a caller's opaque type may not take.
const RESERVED: &[&str] = &["Int", "Bool", "String", "List", "Option"];

/// Whether `name` can name an opaque type: an uppercase letter, then
/// letters, digits and `_`, and not a built-in type's name.
pub(crate) fn is_type_name(name: &str) -> bool {
    let mut chars = name.chars();

    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !RESERVED.contains(&name)
}

/// Whether `name` is written as a name in an expression: a lowercase
/// letter or `_`, then lowercase letters, digits and `_`.
pub(crate) fn is_value_name(name: &str) -> bool {
    let mut chars = name.chars();

    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a record field can be read as `.name`.
fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();

    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why a type cannot be declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeFault {
    NestedOption,
    TooDeep,
    TypeName(String),
}

/// Check that `ty` is a type shex has: no `Option<Option<T>>`, opaque
/// names well formed, at most [`MAX_TYPE_DEPTH`] deep.
pub(crate) fn check_type(ty: &Type) -> Result<(), TypeFault> {
    let mut stack = vec![(ty, 1usize)];

    while let Some((ty, depth)) = stack.pop() {
        if depth > MAX_TYPE_DEPTH {
            return Err(TypeFault::TooDeep);
        }

        let next = depth.saturating_add(1);

        match ty {
            Type::Int | Type::Bool | Type::String => {}
            Type::List(entry) => stack.push((entry, next)),
            Type::Option(value) => {
                if value.is_option() {
                    return Err(TypeFault::NestedOption);
                }

                stack.push((value, next));
            }
            Type::Record(fields) => stack.extend(fields.values().map(|t| (t, next))),
            Type::Opaque(name) => {
                if !is_type_name(name) {
                    return Err(TypeFault::TypeName(name.clone()));
                }
            }
        }
    }

    Ok(())
}

enum Piece<'a> {
    Type(&'a Type),
    Text(&'static str),
    Field(&'a str),
}

impl fmt::Display for Type {
    /// Types as written in shex: `Int`, `List<Int>`, `Option<Track>`, and
    /// records as `{athletics: Int, "Brawl (bar)": Option<Int>}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut stack = vec![Piece::Type(self)];

        while let Some(piece) = stack.pop() {
            match piece {
                Piece::Text(text) => f.write_str(text)?,
                Piece::Field(name) if is_field_name(name) => write!(f, "{name}: ")?,
                Piece::Field(name) => write!(f, "{name:?}: ")?,
                Piece::Type(ty) => match ty {
                    Type::Int => f.write_str("Int")?,
                    Type::Bool => f.write_str("Bool")?,
                    Type::String => f.write_str("String")?,
                    Type::Opaque(name) => f.write_str(name)?,
                    Type::List(entry) => {
                        f.write_str("List<")?;
                        stack.push(Piece::Text(">"));
                        stack.push(Piece::Type(entry));
                    }
                    Type::Option(value) => {
                        f.write_str("Option<")?;
                        stack.push(Piece::Text(">"));
                        stack.push(Piece::Type(value));
                    }
                    Type::Record(fields) => {
                        f.write_str("{")?;
                        stack.push(Piece::Text("}"));

                        for (n, (name, ty)) in fields.iter().enumerate().rev() {
                            stack.push(Piece::Type(ty));
                            stack.push(Piece::Field(name));

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

/// A type in a program's type table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Encode, Decode)]
#[musli(transparent)]
pub(crate) struct TypeId(pub(crate) u32);

impl TypeId {
    pub(crate) fn index(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// A record's field in a [`TypeNode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode)]
pub(crate) struct FieldNode {
    pub(crate) name: String,
    pub(crate) ty: TypeId,
}

/// A type whose parts are earlier entries of the same table, so tables
/// are never recursive data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode)]
pub(crate) enum TypeNode {
    Int,
    Bool,
    String,
    List(TypeId),
    Option(TypeId),
    /// Fields sorted by name, each name once.
    Record(Vec<FieldNode>),
    Opaque(String),
}

/// The types a program uses, each once, every part before what holds it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Types {
    nodes: Vec<TypeNode>,
    ids: HashMap<TypeNode, TypeId>,
    /// How deeply each type nests.
    depths: Vec<usize>,
}

impl Types {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn nodes(&self) -> &[TypeNode] {
        &self.nodes
    }

    pub(crate) fn get(&self, id: TypeId) -> Option<&TypeNode> {
        self.nodes.get(id.index())
    }

    pub(crate) fn find(&self, node: &TypeNode) -> Option<TypeId> {
        self.ids.get(node).copied()
    }

    fn depth(&self, id: TypeId) -> usize {
        self.depths.get(id.index()).copied().unwrap_or(usize::MAX)
    }

    /// The id of `node`, adding it when new. `None` when the table is
    /// full or the type nests past [`MAX_TYPE_DEPTH`].
    pub(crate) fn add(&mut self, node: TypeNode) -> Option<TypeId> {
        if let Some(id) = self.ids.get(&node) {
            return Some(*id);
        }

        let depth = match &node {
            TypeNode::Int | TypeNode::Bool | TypeNode::String | TypeNode::Opaque(..) => 0,
            TypeNode::List(id) | TypeNode::Option(id) => self.depth(*id),
            TypeNode::Record(fields) => fields.iter().map(|f| self.depth(f.ty)).max().unwrap_or(0),
        }
        .saturating_add(1);

        if depth > MAX_TYPE_DEPTH {
            return None;
        }

        let id = TypeId(u32::try_from(self.nodes.len()).ok()?);
        self.nodes.push(node.clone());
        self.depths.push(depth);
        self.ids.insert(node, id);
        Some(id)
    }

    /// The id of `ty`, adding it and its parts. Iterative: the parts are
    /// added before what holds them.
    pub(crate) fn intern(&mut self, ty: &Type) -> Option<TypeId> {
        enum Step<'a> {
            Visit(&'a Type),
            Build(&'a Type),
        }

        let mut steps = vec![Step::Visit(ty)];
        let mut done = Vec::<TypeId>::new();

        while let Some(step) = steps.pop() {
            match step {
                Step::Visit(ty) => {
                    steps.push(Step::Build(ty));

                    match ty {
                        Type::List(t) | Type::Option(t) => steps.push(Step::Visit(t)),
                        Type::Record(fields) => {
                            steps.extend(fields.values().rev().map(Step::Visit));
                        }
                        Type::Int | Type::Bool | Type::String | Type::Opaque(..) => {}
                    }
                }
                Step::Build(ty) => {
                    let node = match ty {
                        Type::Int => TypeNode::Int,
                        Type::Bool => TypeNode::Bool,
                        Type::String => TypeNode::String,
                        Type::Opaque(name) => TypeNode::Opaque(name.clone()),
                        Type::List(_) => TypeNode::List(done.pop()?),
                        Type::Option(_) => TypeNode::Option(done.pop()?),
                        Type::Record(fields) => {
                            let at = done.len().checked_sub(fields.len())?;
                            let ids = done.split_off(at);

                            TypeNode::Record(
                                fields
                                    .keys()
                                    .zip(ids)
                                    .map(|(name, ty)| FieldNode {
                                        name: name.clone(),
                                        ty,
                                    })
                                    .collect(),
                            )
                        }
                    };

                    done.push(self.add(node)?);
                }
            }
        }

        done.pop()
    }

    /// The [`Type`] of `id`, built from the leaves up so nothing recurses.
    pub(crate) fn to_type(&self, id: TypeId) -> Option<Type> {
        // The ids `id` reaches, which are all at or below it.
        let mut reach = vec![false; id.index().checked_add(1)?];
        *reach.get_mut(id.index())? = true;

        for at in (0..reach.len()).rev() {
            if !reach.get(at).copied().unwrap_or(false) {
                continue;
            }

            for part in parts(self.nodes.get(at)?) {
                *reach.get_mut(part.index())? = true;
            }
        }

        let mut built = HashMap::<usize, Type>::new();

        for (at, reached) in reach.iter().enumerate() {
            if !reached {
                continue;
            }

            let ty = match self.nodes.get(at)? {
                TypeNode::Int => Type::Int,
                TypeNode::Bool => Type::Bool,
                TypeNode::String => Type::String,
                TypeNode::Opaque(name) => Type::Opaque(name.clone()),
                TypeNode::List(t) => Type::list(built.get(&t.index())?.clone()),
                TypeNode::Option(t) => Type::option(built.get(&t.index())?.clone()),
                TypeNode::Record(fields) => {
                    let mut record = BTreeMap::new();

                    for field in fields {
                        record.insert(field.name.clone(), built.get(&field.ty.index())?.clone());
                    }

                    Type::Record(record)
                }
            };

            built.insert(at, ty);
        }

        built.remove(&id.index())
    }

    /// Check a stored table: each part before what holds it, each node
    /// once, records sorted, no `Option<Option<T>>`, opaque names well
    /// formed and nothing nested past [`MAX_TYPE_DEPTH`].
    pub(crate) fn from_stored(nodes: Vec<TypeNode>) -> Result<Self, String> {
        let mut types = Self::new();

        for (at, node) in nodes.into_iter().enumerate() {
            for part in parts(&node) {
                if part.index() >= at {
                    return Err(format!("type {at} refers to a later type {}", part.0));
                }
            }

            match &node {
                TypeNode::Option(t) if matches!(types.get(*t), Some(TypeNode::Option(..))) => {
                    return Err(format!("type {at} is an option of an option"));
                }
                TypeNode::Record(fields) => {
                    if !fields.windows(2).all(|w| match w {
                        [a, b] => a.name < b.name,
                        _ => true,
                    }) {
                        return Err(format!(
                            "type {at} has fields that are not sorted and unique"
                        ));
                    }
                }
                TypeNode::Opaque(name) if !is_type_name(name) => {
                    return Err(format!("type {at} has an invalid name {name:?}"));
                }
                _ => {}
            }

            if types.find(&node).is_some() {
                return Err(format!("type {at} is a repeat of an earlier type"));
            }

            if types.add(node).is_none() {
                return Err(format!("type {at} nests too deeply"));
            }
        }

        Ok(types)
    }
}

/// The ids a node refers to.
fn parts(node: &TypeNode) -> Vec<TypeId> {
    match node {
        TypeNode::Int | TypeNode::Bool | TypeNode::String | TypeNode::Opaque(..) => Vec::new(),
        TypeNode::List(t) | TypeNode::Option(t) => vec![*t],
        TypeNode::Record(fields) => fields.iter().map(|f| f.ty).collect(),
    }
}
