use crate::message::{Invalid, Message, StoreError};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

use musli::{Decode, Encode};

/// How deeply a type may nest, such as `List<List<Int>>` at 2. Declared
/// types past it are refused, and so are stored programs.
pub const MAX_TYPE_DEPTH: usize = 64;

/// The type of a value: `Int`, `Bool`, `String`, `List<T>`, `Option<T>`
/// or a record.
///
/// It is opaque. Build one with [`Type::int`], [`Type::bool`],
/// [`Type::string`], [`Type::list`], [`Type::option`] and
/// [`Type::record`], and look into one with the `is_*` and `as_*`
/// try-accessors, which give `false` or `None` for any other type.
///
/// Options nest: an `Option<Option<Int>>` is `none`, `some(none)` or
/// `some(some(n))`, as a lookup in a list of optional values gives.
///
/// ```
/// use evalite::Type;
///
/// let health = Type::record([("boxes", Type::int()), ("count", Type::list(Type::int()))]);
/// assert_eq!(health.to_string(), "{boxes: Int, count: List<Int>}");
/// assert_eq!(Type::option(Type::string()).to_string(), "Option<String>");
/// assert_ne!(Type::int(), Type::bool());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Type(pub(crate) Kind);

/// What a [`Type`] is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
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
}

impl Type {
    /// `Int`, a 64-bit signed integer.
    ///
    /// ```
    /// assert_eq!(evalite::Type::int().to_string(), "Int");
    /// ```
    pub const fn int() -> Self {
        Self(Kind::Int)
    }

    /// `Bool`, `true` or `false`.
    ///
    /// ```
    /// assert_eq!(evalite::Type::bool().to_string(), "Bool");
    /// ```
    pub const fn bool() -> Self {
        Self(Kind::Bool)
    }

    /// `String`, a string literal such as a library table's name.
    ///
    /// ```
    /// assert_eq!(evalite::Type::string().to_string(), "String");
    /// ```
    pub const fn string() -> Self {
        Self(Kind::String)
    }

    /// `List<T>`, entries of one type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert_eq!(Type::list(Type::list(Type::int())).to_string(), "List<List<Int>>");
    /// ```
    pub fn list(entry: Type) -> Self {
        Self(Kind::List(Box::new(entry)))
    }

    /// `Option<T>`, a value that may be absent.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert_eq!(Type::option(Type::int()).to_string(), "Option<Int>");
    /// ```
    pub fn option(value: Type) -> Self {
        Self(Kind::Option(Box::new(value)))
    }

    /// A record of these fields, read with `.name` or `["name"]`. A name
    /// given twice keeps its last type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// let ty = Type::record([("boxes", Type::int()), ("Brawl (bar)", Type::option(Type::int()))]);
    /// assert_eq!(ty.to_string(), r#"{"Brawl (bar)": Option<Int>, boxes: Int}"#);
    /// ```
    pub fn record<K>(fields: impl IntoIterator<Item = (K, Type)>) -> Self
    where
        K: Into<String>,
    {
        Self(Kind::Record(
            fields.into_iter().map(|(k, t)| (k.into(), t)).collect(),
        ))
    }

    /// Whether it is `Int`.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert!(Type::int().is_int());
    /// assert!(!Type::bool().is_int());
    /// assert!(!Type::option(Type::int()).is_int());
    /// assert!(!Type::list(Type::int()).is_int());
    /// ```
    pub fn is_int(&self) -> bool {
        matches!(self.0, Kind::Int)
    }

    /// Whether it is `Bool`.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert!(Type::bool().is_bool());
    /// assert!(!Type::int().is_bool());
    /// assert!(!Type::option(Type::bool()).is_bool());
    /// ```
    pub fn is_bool(&self) -> bool {
        matches!(self.0, Kind::Bool)
    }

    /// Whether it is `String`.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert!(Type::string().is_string());
    /// assert!(!Type::int().is_string());
    /// assert!(!Type::list(Type::string()).is_string());
    /// ```
    pub fn is_string(&self) -> bool {
        matches!(self.0, Kind::String)
    }

    /// The entry type of the `List<T>` it is, or `None` if it is another
    /// type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// let ty = Type::list(Type::option(Type::int()));
    /// assert_eq!(ty.as_list(), Some(&Type::option(Type::int())));
    /// assert_eq!(ty.as_list().and_then(Type::as_option), Some(&Type::int()));
    ///
    /// assert_eq!(Type::int().as_list(), None);
    /// assert_eq!(Type::option(Type::list(Type::int())).as_list(), None);
    /// assert_eq!(Type::record([("a", Type::list(Type::int()))]).as_list(), None);
    /// ```
    pub fn as_list(&self) -> Option<&Type> {
        match &self.0 {
            Kind::List(entry) => Some(entry),
            _ => None,
        }
    }

    /// The value type of the `Option<T>` it is, or `None` if it is another
    /// type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// assert_eq!(Type::option(Type::bool()).as_option(), Some(&Type::bool()));
    ///
    /// assert_eq!(Type::bool().as_option(), None);
    /// assert_eq!(Type::list(Type::option(Type::int())).as_option(), None);
    /// ```
    pub fn as_option(&self) -> Option<&Type> {
        match &self.0 {
            Kind::Option(value) => Some(value),
            _ => None,
        }
    }

    /// The fields of the record it is, by name in order, or `None` if it is
    /// another type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// let ty = Type::record([("count", Type::list(Type::int())), ("boxes", Type::int())]);
    /// let fields: Vec<(&str, String)> = ty
    ///     .as_record()
    ///     .map(|fields| fields.map(|(name, ty)| (name, ty.to_string())).collect())
    ///     .unwrap_or_default();
    /// assert_eq!(fields, [("boxes", "Int".to_owned()), ("count", "List<Int>".to_owned())]);
    ///
    /// assert!(Type::int().as_record().is_none());
    /// assert!(Type::option(Type::record([("a", Type::int())])).as_record().is_none());
    /// ```
    pub fn as_record(&self) -> Option<FieldTypes<'_>> {
        match &self.0 {
            Kind::Record(fields) => Some(FieldTypes {
                iter: fields.iter(),
            }),
            _ => None,
        }
    }

    pub(crate) fn is_option(&self) -> bool {
        matches!(self.0, Kind::Option(..))
    }
}

/// The fields of a record [`Type`], by name in order, from
/// [`Type::as_record`].
///
/// ```
/// use evalite::Type;
///
/// let ty = Type::record([("b", Type::bool()), ("a", Type::int())]);
/// let mut fields = ty.as_record().into_iter().flatten();
/// assert_eq!(fields.next(), Some(("a", &Type::int())));
/// assert_eq!(fields.next(), Some(("b", &Type::bool())));
/// assert_eq!(fields.next(), None);
/// ```
#[derive(Debug, Clone)]
pub struct FieldTypes<'a> {
    iter: std::collections::btree_map::Iter<'a, String, Type>,
}

impl<'a> Iterator for FieldTypes<'a> {
    type Item = (&'a str, &'a Type);

    fn next(&mut self) -> Option<Self::Item> {
        let (name, ty) = self.iter.next()?;
        Some((name, ty))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl DoubleEndedIterator for FieldTypes<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let (name, ty) = self.iter.next_back()?;
        Some((name, ty))
    }
}

impl ExactSizeIterator for FieldTypes<'_> {}

impl std::iter::FusedIterator for FieldTypes<'_> {}

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
pub(crate) fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();

    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why a type cannot be declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeFault {
    TooDeep,
}

/// Check that `ty` is a type evalite has: at most [`MAX_TYPE_DEPTH`] deep.
pub(crate) fn check_type(ty: &Type) -> Result<(), TypeFault> {
    let mut stack = vec![(ty, 1usize)];

    while let Some((ty, depth)) = stack.pop() {
        if depth > MAX_TYPE_DEPTH {
            return Err(TypeFault::TooDeep);
        }

        let next = depth.saturating_add(1);

        match ty {
            Type(Kind::Int) | Type(Kind::Bool) | Type(Kind::String) => {}
            Type(Kind::List(entry)) | Type(Kind::Option(entry)) => stack.push((entry, next)),
            Type(Kind::Record(fields)) => stack.extend(fields.values().map(|t| (t, next))),
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
    /// Types as written in evalite: `Int`, `List<Int>`, `Option<Int>`, and
    /// records as `{athletics: Int, "Brawl (bar)": Option<Int>}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut stack = vec![Piece::Type(self)];

        while let Some(piece) = stack.pop() {
            match piece {
                Piece::Text(text) => f.write_str(text)?,
                Piece::Field(name) if is_field_name(name) => write!(f, "{name}: ")?,
                Piece::Field(name) => {
                    crate::value::write_string(f, name)?;
                    f.write_str(": ")?;
                }
                Piece::Type(ty) => match ty {
                    Type(Kind::Int) => f.write_str("Int")?,
                    Type(Kind::Bool) => f.write_str("Bool")?,
                    Type(Kind::String) => f.write_str("String")?,
                    Type(Kind::List(entry)) => {
                        f.write_str("List<")?;
                        stack.push(Piece::Text(">"));
                        stack.push(Piece::Type(entry));
                    }
                    Type(Kind::Option(value)) => {
                        f.write_str("Option<")?;
                        stack.push(Piece::Text(">"));
                        stack.push(Piece::Type(value));
                    }
                    Type(Kind::Record(fields)) => {
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
}

/// The types a program uses, each once, every part before what holds it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Types {
    nodes: Vec<TypeNode>,
    ids: HashMap<TypeNode, TypeId>,
    /// How deeply each type nests.
    depths: Vec<usize>,
    /// The words a value of each type takes where it is held.
    widths: Vec<usize>,
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

    /// Whether a value of type `id` is one word that is never
    /// [`Word::NONE`](crate::word::Word::NONE): a `Bool`, or a string, list
    /// or record, which are a word referring to their parts. An option of
    /// it can use `Word::NONE` for absent.
    pub(crate) fn has_niche(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Some(TypeNode::Bool | TypeNode::String | TypeNode::List(_) | TypeNode::Record(_))
        )
    }

    /// The words of the value inside the option `id`, when the option is
    /// tagged: a word saying whether it is present, then its value, always
    /// held, so an absent one takes the same room. An option is tagged when
    /// its value has no niche: an `Option<Int>`, since every word is an
    /// `Int`, and an option of an option, since the inner one's absence is
    /// a value of the outer. `None` for any other type.
    pub(crate) fn tagged(&self, id: TypeId) -> Option<usize> {
        match self.get(id) {
            Some(TypeNode::Option(inner)) if !self.has_niche(*inner) => Some(self.width(*inner)),
            _ => None,
        }
    }

    /// The words a value of type `id` takes where it is held: one for an
    /// `Int`, a `Bool`, a string, list or record, one for an option of a
    /// value with a niche (with [`Word::NONE`](crate::word::Word::NONE) for
    /// absent), and one more than its value for a [`tagged`](Self::tagged)
    /// option, so `Option<Int>` is two words and `Option<Option<Int>>`
    /// three. It is worked out once, when the type is added.
    pub(crate) fn width(&self, id: TypeId) -> usize {
        self.widths.get(id.index()).copied().unwrap_or(1)
    }

    /// The word offset of each field of the record `fields` from its
    /// start, in name order, and the words they take together.
    pub(crate) fn field_offsets<'f>(
        &'f self,
        fields: &'f [FieldNode],
    ) -> impl Iterator<Item = (usize, &'f FieldNode)> + 'f {
        fields.iter().scan(0usize, move |at, field| {
            let here = *at;
            *at = at.saturating_add(self.width(field.ty));
            Some((here, field))
        })
    }

    /// The words the fields of a record take.
    pub(crate) fn record_words(&self, fields: &[FieldNode]) -> usize {
        fields
            .iter()
            .fold(0usize, |n, f| n.saturating_add(self.width(f.ty)))
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
            TypeNode::Int | TypeNode::Bool | TypeNode::String => 0,
            TypeNode::List(id) | TypeNode::Option(id) => self.depth(*id),
            TypeNode::Record(fields) => fields.iter().map(|f| self.depth(f.ty)).max().unwrap_or(0),
        }
        .saturating_add(1);

        if depth > MAX_TYPE_DEPTH {
            return None;
        }

        let width = match &node {
            TypeNode::Option(inner) if !self.has_niche(*inner) => {
                self.width(*inner).saturating_add(1)
            }
            _ => 1,
        };

        let id = TypeId(u32::try_from(self.nodes.len()).ok()?);
        self.nodes.push(node.clone());
        self.depths.push(depth);
        self.widths.push(width);
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
                        Type(Kind::List(t)) | Type(Kind::Option(t)) => steps.push(Step::Visit(t)),
                        Type(Kind::Record(fields)) => {
                            steps.extend(fields.values().rev().map(Step::Visit));
                        }
                        Type(Kind::Int) | Type(Kind::Bool) | Type(Kind::String) => {}
                    }
                }
                Step::Build(ty) => {
                    let node = match ty {
                        Type(Kind::Int) => TypeNode::Int,
                        Type(Kind::Bool) => TypeNode::Bool,
                        Type(Kind::String) => TypeNode::String,
                        Type(Kind::List(_)) => TypeNode::List(done.pop()?),
                        Type(Kind::Option(_)) => TypeNode::Option(done.pop()?),
                        Type(Kind::Record(fields)) => {
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
                TypeNode::Int => Type::int(),
                TypeNode::Bool => Type::bool(),
                TypeNode::String => Type::string(),
                TypeNode::List(t) => Type::list(built.get(&t.index())?.clone()),
                TypeNode::Option(t) => Type::option(built.get(&t.index())?.clone()),
                TypeNode::Record(fields) => {
                    let mut record = BTreeMap::new();

                    for field in fields {
                        record.insert(field.name.clone(), built.get(&field.ty.index())?.clone());
                    }

                    Type::record(record)
                }
            };

            built.insert(at, ty);
        }

        built.remove(&id.index())
    }

    /// Check a stored table: each part before what holds it, each node
    /// once, records sorted, and nothing nested past [`MAX_TYPE_DEPTH`].
    pub(crate) fn from_stored(nodes: Vec<TypeNode>) -> Result<Self, StoreError> {
        let mut types = Self::new();

        for (at, node) in nodes.into_iter().enumerate() {
            let fail = |invalid: Invalid| {
                StoreError::from(invalid).context(Message::AtType {
                    at: u32::try_from(at).unwrap_or(u32::MAX),
                })
            };

            for part in parts(&node) {
                if part.index() >= at {
                    return Err(fail(Invalid::TypeRefersLater));
                }
            }

            match &node {
                TypeNode::Record(fields)
                    if !fields.windows(2).all(|w| match w {
                        [a, b] => a.name < b.name,
                        _ => true,
                    }) =>
                {
                    return Err(fail(Invalid::FieldsUnsorted));
                }
                _ => {}
            }

            if types.find(&node).is_some() {
                return Err(fail(Invalid::TypeRepeated));
            }

            if types.add(node).is_none() {
                return Err(fail(Invalid::TypeTooDeep));
            }
        }

        Ok(types)
    }
}

/// The ids a node refers to.
fn parts(node: &TypeNode) -> Vec<TypeId> {
    match node {
        TypeNode::Int | TypeNode::Bool | TypeNode::String => Vec::new(),
        TypeNode::List(t) | TypeNode::Option(t) => vec![*t],
        TypeNode::Record(fields) => fields.iter().map(|f| f.ty).collect(),
    }
}
