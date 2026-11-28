use crate::message::{Invalid, Message, StoreError};
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

use musli::{Decode, Encode};

/// How deeply a type may nest, such as `List<List<Int>>` at 2. Declared
/// types past it are refused, and so are stored programs.
pub const MAX_TYPE_DEPTH: usize = 64;

/// How many nodes a type may have once its shared parts are written out,
/// so `List<Int>` has 2. Declared types past it are refused, and so are
/// stored programs. A type is a tree, but a stored table shares parts, so a
/// few nodes can stand for more than a program could hold.
pub const MAX_TYPE_NODES: usize = 1 << 16;

/// The type of a value: `Int`, `Bool`, `String`, `List<T>`, `Option<T>`,
/// `Map<K, V>` or a record.
///
/// It is opaque. Build one with [`Type::int`], [`Type::bool`],
/// [`Type::string`], [`Type::list`], [`Type::option`], [`Type::map`] and
/// [`Type::record`], and look into one with the `is_*` and `as_*`
/// try-accessors, which give `false` or `None` for any other type.
///
/// Options nest: an `Option<Option<Int>>` is `None`, `Some(None)` or
/// `Some(Some(n))`, as `get` on a list of optional values gives.
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
    /// Values by key, read with `[key]`: the key type, then the value type.
    Map(Box<Type>, Box<Type>),
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

    /// `Map<K, V>`, values of type `value` by keys of type `key`, read with
    /// `m[key]` and `m.get(key)`. A key is an `Int` or a `String`: a map of
    /// any other key type is refused where it is declared, as
    /// [`Environment::value`](crate::Environment::value) does.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let ty = Type::map(Type::int(), Type::record([("dice", Type::int())]));
    /// assert_eq!(ty.to_string(), "Map<Int, {dice: Int}>");
    ///
    /// let mut env = Environment::new();
    /// env.value("specialties", ty)?;
    ///
    /// let error = env.value("bad", Type::map(Type::bool(), Type::int())).unwrap_err();
    /// assert_eq!(
    ///     error.to_string(),
    ///     "the type of `bad` has a map keyed by other than Int or String, a map's keys are Int or String"
    /// );
    /// # Ok::<_, evalite::DeclareError>(())
    /// ```
    pub fn map(key: Type, value: Type) -> Self {
        Self(Kind::Map(Box::new(key), Box::new(value)))
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

    /// The key and value types of the `Map<K, V>` it is, or `None` if it is
    /// another type.
    ///
    /// ```
    /// use evalite::Type;
    ///
    /// let ty = Type::map(Type::string(), Type::list(Type::int()));
    /// assert_eq!(ty.as_map(), Some((&Type::string(), &Type::list(Type::int()))));
    ///
    /// assert_eq!(Type::int().as_map(), None);
    /// assert_eq!(Type::list(ty).as_map(), None);
    /// ```
    pub fn as_map(&self) -> Option<(&Type, &Type)> {
        match &self.0 {
            Kind::Map(key, value) => Some((key, value)),
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
    /// More than [`MAX_TYPE_NODES`] nodes.
    TooLarge,
    /// A map whose key type is neither `Int` nor `String`.
    MapKey,
}

/// Check that `ty` is a type evalite has: at most [`MAX_TYPE_DEPTH`] deep
/// and [`MAX_TYPE_NODES`] nodes, and each map keyed by `Int` or `String`.
/// It stops at the first fault, so it visits at most [`MAX_TYPE_NODES`] and
/// one nodes.
pub(crate) fn check_type(ty: &Type) -> Result<(), TypeFault> {
    let mut stack = vec![(ty, 1usize)];
    let mut nodes = 0usize;

    while let Some((ty, depth)) = stack.pop() {
        if depth > MAX_TYPE_DEPTH {
            return Err(TypeFault::TooDeep);
        }

        nodes = nodes.saturating_add(1);

        if nodes > MAX_TYPE_NODES {
            return Err(TypeFault::TooLarge);
        }

        let next = depth.saturating_add(1);

        match ty {
            Type(Kind::Int) | Type(Kind::Bool) | Type(Kind::String) => {}
            Type(Kind::List(entry)) | Type(Kind::Option(entry)) => stack.push((entry, next)),
            Type(Kind::Record(fields)) => stack.extend(fields.values().map(|t| (t, next))),
            Type(Kind::Map(key, value)) => {
                if !matches!(key.0, Kind::Int | Kind::String) {
                    return Err(TypeFault::MapKey);
                }

                stack.push((key, next));
                stack.push((value, next));
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
    /// Types as written in evalite: `Int`, `List<Int>`, `Option<Int>`,
    /// `Map<Int, String>`, and records as
    /// `{athletics: Int, "Brawl (bar)": Option<Int>}`.
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
                    Type(Kind::Map(key, value)) => {
                        f.write_str("Map<")?;
                        stack.push(Piece::Text(">"));
                        stack.push(Piece::Type(value));
                        stack.push(Piece::Text(", "));
                        stack.push(Piece::Type(key));
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
    /// Values by key, the key an `Int` or a `String`.
    Map {
        key: TypeId,
        value: TypeId,
    },
}

/// The types a program uses, each once, every part before what holds it.
#[derive(Debug, Clone)]
pub(crate) struct Types {
    nodes: Vec<TypeNode>,
    /// The latest type of each hash of its node, with the earlier ones of
    /// the same hash chained through `next`. The leaves are not here.
    heads: HashMap<u64, TypeId, BuildHasherDefault<Passthrough>>,
    /// For each type, the earlier one of the same hash.
    next: Vec<Option<TypeId>>,
    /// How deeply each type nests.
    depths: Vec<usize>,
    /// How many nodes each type has with its shared parts written out,
    /// saturating.
    sizes: Vec<usize>,
    /// The words a value of each type takes where it is held.
    widths: Vec<usize>,
    /// The word offset of each field of each record, in name order, so a
    /// field is found by offset or name in logarithmic time. Empty for
    /// other types.
    offsets: Vec<Vec<usize>>,
    /// The ids of `Int`, `Bool` and `String`, found without hashing.
    leaves: [Option<TypeId>; 3],
    /// Where each list, option and map node's hash starts, random for
    /// each table.
    seed: u64,
    /// The keys of the SipHash that hashes records, whose field names a
    /// stored table chooses, random for each table.
    keys: RandomState,
}

impl Default for Types {
    fn default() -> Self {
        let keys = RandomState::new();

        Self {
            nodes: Vec::new(),
            heads: HashMap::default(),
            next: Vec::new(),
            depths: Vec::new(),
            sizes: Vec::new(),
            widths: Vec::new(),
            offsets: Vec::new(),
            leaves: [None; 3],
            seed: keys.hash_one(0u8),
            keys,
        }
    }
}

/// Hashes that are already hashes.
#[derive(Debug, Clone, Copy, Default)]
struct Passthrough(u64);

impl Hasher for Passthrough {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// A small, fast, non-cryptographic hasher (the FxHash scheme) for integer
/// keys: type ids, slots and tags, from a random seed per table.
///
/// It is not for keys an untrusted writer chooses freely, such as names:
/// its multiplier is odd and `1 mod 4`, so two words that differ only in
/// bit 62 give states that differ only in the top two bits, whatever the
/// seed, and the next word can cancel that. Such keys use SipHash
/// ([`RandomState`]) instead.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FxHasher(u64);

/// A [`BuildHasher`] for [`FxHasher`] seeded per table or verification pass.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FxBuildHasher(u64);

impl FxBuildHasher {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl BuildHasher for FxBuildHasher {
    type Hasher = FxHasher;

    fn build_hasher(&self) -> Self::Hasher {
        FxHasher(self.0)
    }
}

impl FxHasher {
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        // Fold high bits down so low-bit bucket selection depends on the whole
        // key, preventing high-bit flooding on untrusted stored inputs.
        let h = (self.0 ^ (self.0 >> 32)).wrapping_mul(Self::SEED);
        h ^ (h >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        let (words, tail) = bytes.as_chunks::<8>();
        self.add(bytes.len() as u64);

        for word in words {
            self.add(u64::from_le_bytes(*word));
        }

        let mut rest = [0u8; 8];

        if let Some(at) = rest.get_mut(..tail.len()) {
            at.copy_from_slice(tail);
        }

        self.add(u64::from_le_bytes(rest));
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }
}

fn leaf(node: &TypeNode) -> Option<usize> {
    match node {
        TypeNode::Int => Some(0),
        TypeNode::Bool => Some(1),
        TypeNode::String => Some(2),
        _ => None,
    }
}

impl Types {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_capacity(nodes: usize) -> Self {
        let non_leaves = nodes.saturating_sub(3);
        let keys = RandomState::new();

        Self {
            nodes: Vec::with_capacity(nodes),
            heads: HashMap::with_capacity_and_hasher(non_leaves, BuildHasherDefault::default()),
            next: Vec::with_capacity(nodes),
            depths: Vec::with_capacity(nodes),
            sizes: Vec::with_capacity(nodes),
            widths: Vec::with_capacity(nodes),
            offsets: Vec::with_capacity(nodes),
            leaves: [None; 3],
            seed: keys.hash_one(0u8),
            keys,
        }
    }

    pub(crate) fn hasher(&self) -> FxBuildHasher {
        FxBuildHasher::new(self.seed)
    }

    pub(crate) fn nodes(&self) -> &[TypeNode] {
        &self.nodes
    }

    pub(crate) fn get(&self, id: TypeId) -> Option<&TypeNode> {
        self.nodes.get(id.index())
    }

    pub(crate) fn find(&self, node: &TypeNode) -> Option<TypeId> {
        match leaf(node) {
            Some(n) => self.leaves.get(n).copied().flatten(),
            None => self.lookup(self.hash_node(node), |n| n == node),
        }
    }

    /// The hash of a list, option or map node with these parts, tagged by
    /// which it is. Its parts are ids, so the fast hasher serves.
    fn hash_ids(&self, tag: u8, ids: &[TypeId]) -> u64 {
        let mut hasher = FxHasher(self.seed);
        tag.hash(&mut hasher);

        for ty in ids {
            ty.hash(&mut hasher);
        }

        hasher.finish()
    }

    /// The hash of a record with these fields. A stored table chooses the
    /// names, so records are hashed with keyed SipHash, which they cannot
    /// be chosen to collide in. Equal records have equal hashes, without
    /// building the node.
    fn hash_record<'a>(&self, fields: impl Iterator<Item = (&'a str, TypeId)>) -> u64 {
        let mut hasher = self.keys.build_hasher();

        for (name, ty) in fields {
            name.hash(&mut hasher);
            ty.hash(&mut hasher);
        }

        hasher.finish()
    }

    fn hash_node(&self, node: &TypeNode) -> u64 {
        match node {
            TypeNode::Int | TypeNode::Bool | TypeNode::String => 0,
            TypeNode::List(t) => self.hash_ids(1, &[*t]),
            TypeNode::Option(t) => self.hash_ids(2, &[*t]),
            TypeNode::Record(fields) => {
                self.hash_record(fields.iter().map(|f| (f.name.as_str(), f.ty)))
            }
            TypeNode::Map { key, value } => self.hash_ids(4, &[*key, *value]),
        }
    }

    /// The most types that share one hash, for tests of flooding.
    #[cfg(test)]
    pub(crate) fn longest_chain(&self) -> usize {
        self.heads
            .values()
            .map(|&head| {
                std::iter::successors(Some(head), |id| {
                    self.next.get(id.index()).copied().flatten()
                })
                .count()
            })
            .max()
            .unwrap_or(0)
    }

    /// The type of this hash that `is` accepts.
    fn lookup(&self, hash: u64, is: impl Fn(&TypeNode) -> bool) -> Option<TypeId> {
        let mut at = self.heads.get(&hash).copied();

        while let Some(id) = at {
            if self.nodes.get(id.index()).is_some_and(&is) {
                return Some(id);
            }

            at = self.next.get(id.index()).copied().flatten();
        }

        None
    }

    /// Whether `==` and `!=` compare values of type `id`: an `Int`, a
    /// `Bool`, or an option of such a type, however deeply nested. Each
    /// node holds an earlier one, so the walk ends within the types held.
    pub(crate) fn comparable(&self, id: TypeId) -> bool {
        let mut at = id;

        for _ in 0..=self.nodes.len() {
            match self.get(at) {
                Some(TypeNode::Int | TypeNode::Bool) => return true,
                Some(TypeNode::Option(inner)) => at = *inner,
                _ => return false,
            }
        }

        false
    }

    /// Whether a value of type `id` is one word that is never
    /// [`Word::NONE`](crate::word::Word::NONE): a `Bool`, or a string, list
    /// or record, which are a word referring to their parts. An option of
    /// it can use `Word::NONE` for absent.
    pub(crate) fn has_niche(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Some(
                TypeNode::Bool
                    | TypeNode::String
                    | TypeNode::List(_)
                    | TypeNode::Record(_)
                    | TypeNode::Map { .. }
            )
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

    /// The field of the record `id` at `at` words from its start.
    pub(crate) fn field_at(&self, id: TypeId, at: usize) -> Option<&FieldNode> {
        let TypeNode::Record(fields) = self.get(id)? else {
            return None;
        };

        let n = self.offsets.get(id.index())?.binary_search(&at).ok()?;
        fields.get(n)
    }

    /// The field of the record `id` named `name`, and its word offset.
    pub(crate) fn field_named(&self, id: TypeId, name: &str) -> Option<(usize, &FieldNode)> {
        let (_, offset, field) = self.field_index(id, name)?;
        Some((offset, field))
    }

    /// The field of the record `id` named `name`, its index in name order
    /// and its word offset.
    pub(crate) fn field_index(&self, id: TypeId, name: &str) -> Option<(usize, usize, &FieldNode)> {
        let TypeNode::Record(fields) = self.get(id)? else {
            return None;
        };

        let n = fields
            .binary_search_by(|f| f.name.as_str().cmp(name))
            .ok()?;
        Some((n, *self.offsets.get(id.index())?.get(n)?, fields.get(n)?))
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

    /// The nodes type `id` has with its shared parts written out.
    pub(crate) fn size(&self, id: TypeId) -> usize {
        self.sizes.get(id.index()).copied().unwrap_or(usize::MAX)
    }

    /// The nodes `node` would have with its shared parts written out.
    fn node_size(&self, node: &TypeNode) -> usize {
        match node {
            TypeNode::Int | TypeNode::Bool | TypeNode::String => 1,
            TypeNode::List(id) | TypeNode::Option(id) => self.size(*id).saturating_add(1),
            TypeNode::Record(fields) => fields
                .iter()
                .fold(1usize, |n, f| n.saturating_add(self.size(f.ty))),
            TypeNode::Map { key, value } => self
                .size(*key)
                .saturating_add(self.size(*value))
                .saturating_add(1),
        }
    }

    /// Whether `node` would have more than [`MAX_TYPE_NODES`] nodes.
    pub(crate) fn is_large(&self, node: &TypeNode) -> bool {
        self.node_size(node) > MAX_TYPE_NODES
    }

    /// The id of `node`, adding it when new. `None` when the table is
    /// full, the type nests past [`MAX_TYPE_DEPTH`] or has more than
    /// [`MAX_TYPE_NODES`] nodes.
    pub(crate) fn add(&mut self, node: TypeNode) -> Option<TypeId> {
        if let Some(n) = leaf(&node) {
            if let Some(id) = self.leaves.get(n).copied().flatten() {
                return Some(id);
            }

            return self.add_new(node, 0);
        }

        let hash = self.hash_node(&node);

        if let Some(id) = self.lookup(hash, |n| *n == node) {
            return Some(id);
        }

        self.add_new(node, hash)
    }

    /// Add `node`, which is not in the table and has this hash.
    fn add_new(&mut self, node: TypeNode, hash: u64) -> Option<TypeId> {
        let depth = match &node {
            TypeNode::Int | TypeNode::Bool | TypeNode::String => 0,
            TypeNode::List(id) | TypeNode::Option(id) => self.depth(*id),
            TypeNode::Record(fields) => fields.iter().map(|f| self.depth(f.ty)).max().unwrap_or(0),
            TypeNode::Map { key, value } => self.depth(*key).max(self.depth(*value)),
        }
        .saturating_add(1);

        if depth > MAX_TYPE_DEPTH {
            return None;
        }

        let size = self.node_size(&node);

        if size > MAX_TYPE_NODES {
            return None;
        }

        let width = match &node {
            TypeNode::Option(inner) if !self.has_niche(*inner) => {
                self.width(*inner).saturating_add(1)
            }
            _ => 1,
        };

        let id = TypeId(u32::try_from(self.nodes.len()).ok()?);
        let leaf = leaf(&node);

        let offsets = match &node {
            TypeNode::Record(fields) => {
                let mut offsets = Vec::with_capacity(fields.len());
                let mut at = 0usize;

                for field in fields {
                    offsets.push(at);
                    at = at.saturating_add(self.width(field.ty));
                }

                offsets
            }
            _ => Vec::new(),
        };

        let before = match leaf {
            Some(_) => None,
            None => self.heads.insert(hash, id),
        };

        if let Some(slot) = leaf.and_then(|n| self.leaves.get_mut(n)) {
            *slot = Some(id);
        }

        self.nodes.push(node);
        self.next.push(before);
        self.depths.push(depth);
        self.sizes.push(size);
        self.widths.push(width);
        self.offsets.push(offsets);
        Some(id)
    }

    /// The id of `ty`, adding it and its parts. Iterative: the parts are
    /// added before what holds them.
    pub(crate) fn intern(&mut self, ty: &Type) -> Option<TypeId> {
        enum Step<'a> {
            Visit(&'a Type),
            Build(&'a Type),
        }

        // The leaves, which most types are, need no work lists.
        match ty {
            Type(Kind::Int) => return self.add(TypeNode::Int),
            Type(Kind::Bool) => return self.add(TypeNode::Bool),
            Type(Kind::String) => return self.add(TypeNode::String),
            _ => {}
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
                        // The key is built first, so it is found below the
                        // value.
                        Type(Kind::Map(key, value)) => {
                            steps.push(Step::Visit(value));
                            steps.push(Step::Visit(key));
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
                        Type(Kind::Map(..)) => {
                            let value = done.pop()?;
                            let key = done.pop()?;
                            TypeNode::Map { key, value }
                        }
                        Type(Kind::Record(fields)) => {
                            let at = done.len().checked_sub(fields.len())?;
                            let ids = done.get(at..)?;
                            let names =
                                || fields.keys().map(String::as_str).zip(ids.iter().copied());
                            let hash = self.hash_record(names());

                            // An existing record is found without cloning
                            // the names.
                            let found = self.lookup(hash, |n| match n {
                                TypeNode::Record(f) => {
                                    f.len() == fields.len()
                                        && f.iter()
                                            .zip(names())
                                            .all(|(f, (name, ty))| f.ty == ty && f.name == name)
                                }
                                _ => false,
                            });

                            let id = match found {
                                Some(id) => id,
                                None => {
                                    let node = TypeNode::Record(
                                        names()
                                            .map(|(name, ty)| FieldNode {
                                                name: name.to_owned(),
                                                ty,
                                            })
                                            .collect(),
                                    );

                                    self.add_new(node, hash)?
                                }
                            };

                            done.truncate(at);
                            done.push(id);
                            continue;
                        }
                    };

                    done.push(self.add(node)?);
                }
            }
        }

        done.pop()
    }

    /// Copy `id` and every type it reaches from `src` into `self`, caching
    /// mapped ids in `map` (indexed by `src`'s ids) so shared types across
    /// globals are copied once per compile.
    pub(crate) fn copy_from(
        &mut self,
        src: &Types,
        id: TypeId,
        map: &mut Vec<Option<TypeId>>,
    ) -> Option<TypeId> {
        if let Some(mapped) = map.get(id.index()).copied().flatten() {
            return Some(mapped);
        }

        match src.get(id)? {
            TypeNode::Int => {
                let dst = self.add(TypeNode::Int)?;
                record_map(map, id, dst)?;
                return Some(dst);
            }
            TypeNode::Bool => {
                let dst = self.add(TypeNode::Bool)?;
                record_map(map, id, dst)?;
                return Some(dst);
            }
            TypeNode::String => {
                let dst = self.add(TypeNode::String)?;
                record_map(map, id, dst)?;
                return Some(dst);
            }
            _ => {}
        }

        enum Step {
            Visit(TypeId),
            Build(TypeId),
        }

        let mut steps = Vec::with_capacity(8);
        steps.push(Step::Visit(id));

        while let Some(step) = steps.pop() {
            match step {
                Step::Visit(cur) => {
                    if map.get(cur.index()).copied().flatten().is_some() {
                        continue;
                    }

                    steps.push(Step::Build(cur));

                    match src.get(cur)? {
                        TypeNode::Int | TypeNode::Bool | TypeNode::String => {}
                        TypeNode::List(t) | TypeNode::Option(t) => {
                            if map.get(t.index()).copied().flatten().is_none() {
                                steps.push(Step::Visit(*t));
                            }
                        }
                        TypeNode::Map { key, value } => {
                            if map.get(value.index()).copied().flatten().is_none() {
                                steps.push(Step::Visit(*value));
                            }
                            if map.get(key.index()).copied().flatten().is_none() {
                                steps.push(Step::Visit(*key));
                            }
                        }
                        TypeNode::Record(fields) => {
                            let mut prev = None;
                            for field in fields.iter().rev() {
                                if prev != Some(field.ty)
                                    && map.get(field.ty.index()).copied().flatten().is_none()
                                {
                                    steps.push(Step::Visit(field.ty));
                                    prev = Some(field.ty);
                                }
                            }
                        }
                    }
                }
                Step::Build(cur) => {
                    if map.get(cur.index()).copied().flatten().is_some() {
                        continue;
                    }

                    let dst = match src.get(cur)? {
                        TypeNode::Int => self.add(TypeNode::Int)?,
                        TypeNode::Bool => self.add(TypeNode::Bool)?,
                        TypeNode::String => self.add(TypeNode::String)?,
                        TypeNode::List(t) => {
                            let inner = map.get(t.index()).copied().flatten()?;
                            self.add(TypeNode::List(inner))?
                        }
                        TypeNode::Option(t) => {
                            let inner = map.get(t.index()).copied().flatten()?;
                            self.add(TypeNode::Option(inner))?
                        }
                        TypeNode::Map { key, value } => {
                            let key = map.get(key.index()).copied().flatten()?;
                            let value = map.get(value.index()).copied().flatten()?;
                            self.add(TypeNode::Map { key, value })?
                        }
                        TypeNode::Record(fields) => {
                            let mut hasher = self.keys.build_hasher();

                            for field in fields {
                                let ty = map.get(field.ty.index()).copied().flatten()?;
                                field.name.as_str().hash(&mut hasher);
                                ty.hash(&mut hasher);
                            }

                            let hash = hasher.finish();
                            let found = self.lookup(hash, |n| match n {
                                TypeNode::Record(f) => {
                                    f.len() == fields.len()
                                        && f.iter().zip(fields).all(|(a, b)| {
                                            map.get(b.ty.index()).copied().flatten() == Some(a.ty)
                                                && a.name == b.name
                                        })
                                }
                                _ => false,
                            });

                            match found {
                                Some(existing) => existing,
                                None => {
                                    let mut remapped = Vec::with_capacity(fields.len());

                                    for field in fields {
                                        let ty = map.get(field.ty.index()).copied().flatten()?;
                                        remapped.push(FieldNode {
                                            name: field.name.clone(),
                                            ty,
                                        });
                                    }

                                    self.add_new(TypeNode::Record(remapped), hash)?
                                }
                            }
                        }
                    };

                    record_map(map, cur, dst)?;
                }
            }
        }

        map.get(id.index()).copied().flatten()
    }

    /// The [`Type`] of `id`, built from the leaves up so nothing recurses.
    pub(crate) fn to_type(&self, id: TypeId) -> Option<Type> {
        self.build(&[id])?.get_mut(id.index())?.take()
    }

    /// The [`Type`] of each of `ids`, built together so what they share is
    /// built once.
    pub(crate) fn to_types(&self, ids: &[TypeId]) -> Option<Vec<Type>> {
        if ids.is_empty() {
            return Some(Vec::new());
        }

        let built = self.build(ids)?;
        ids.iter()
            .map(|id| built.get(id.index())?.clone())
            .collect()
    }

    /// Whether `id` is `ty`, without building it: the two are walked
    /// together, stopping at the first part that differs, so it visits no
    /// more nodes than `id` has written out. Lists and options are followed
    /// down to what they hold without a work list, and `stack` is the work
    /// list for the records and maps a record or a map holds, left empty,
    /// so a caller comparing many reuses it and a record of leaves or
    /// options of leaves allocates nothing. `false` when `id` is not in
    /// the table.
    pub(crate) fn matches<'a>(
        &self,
        id: TypeId,
        ty: &'a Type,
        stack: &mut Vec<(TypeId, &'a Type)>,
    ) -> bool {
        stack.clear();
        let same = self.matches_with(id, ty, stack);
        stack.clear();
        same
    }

    fn matches_with<'a>(
        &self,
        id: TypeId,
        ty: &'a Type,
        stack: &mut Vec<(TypeId, &'a Type)>,
    ) -> bool {
        let mut next = match self.follow(id, ty) {
            Some(next) => next,
            None => return false,
        };

        while let Some((id, Type(kind))) = next.take().or_else(|| stack.pop()) {
            let mut part = |id: TypeId, ty: &'a Type| match self.follow(id, ty) {
                Some(Some(pair)) => {
                    stack.push(pair);
                    true
                }
                Some(None) => true,
                None => false,
            };

            let same = match (self.nodes.get(id.index()), kind) {
                (Some(TypeNode::Map { key, value }), Kind::Map(k, v)) => {
                    part(*key, k) && part(*value, v)
                }
                // Both sorted by name, each name once.
                (Some(TypeNode::Record(fields)), Kind::Record(record)) => {
                    fields.len() == record.len()
                        && fields
                            .iter()
                            .zip(record)
                            .all(|(field, (name, ty))| field.name == *name && part(field.ty, ty))
                }
                _ => false,
            };

            if !same {
                return false;
            }
        }

        true
    }

    /// Follow `id` and `ty` together through lists and options: `None`
    /// when they differ, else the record or map both reach, or nothing when
    /// both reach the same leaf. The record or map is not compared.
    fn follow<'a>(&self, mut id: TypeId, mut ty: &'a Type) -> Option<Option<(TypeId, &'a Type)>> {
        loop {
            match (self.nodes.get(id.index())?, &ty.0) {
                (TypeNode::Int, Kind::Int)
                | (TypeNode::Bool, Kind::Bool)
                | (TypeNode::String, Kind::String) => return Some(None),
                (TypeNode::List(a), Kind::List(b)) | (TypeNode::Option(a), Kind::Option(b)) => {
                    id = *a;
                    ty = b;
                }
                (TypeNode::Map { .. }, Kind::Map(..)) | (TypeNode::Record(_), Kind::Record(_)) => {
                    return Some(Some((id, ty)));
                }
                _ => return None,
            }
        }
    }

    /// Add `id` to what a program holds, keeping `held`'s counts as
    /// [`Program::verify`](crate::program::Program::verify) takes them.
    /// Each node is marked once over every call, so any number of calls
    /// walk the table once. `None` when `id` is not in the table.
    pub(crate) fn hold(&self, held: &mut Held, id: TypeId) -> Option<()> {
        let size = *self.sizes.get(id.index())?;
        held.nodes = held.nodes.saturating_add(size);

        if held.marked.get(id.index()).copied().unwrap_or(false) {
            return Some(());
        }

        if held.marked.len() <= id.index() {
            held.marked.resize(id.index().checked_add(1)?, false);
        }

        *held.marked.get_mut(id.index())? = true;
        held.reached = held.reached.saturating_add(size);

        let node = self.nodes.get(id.index())?;

        if matches!(node, TypeNode::Int | TypeNode::Bool | TypeNode::String) {
            return Some(());
        }

        let mut stack = Vec::new();
        for_each_part(node, |part| stack.push(part));

        while let Some(id) = stack.pop() {
            if held.marked.len() <= id.index() {
                held.marked.resize(id.index().checked_add(1)?, false);
            }

            let marked = held.marked.get_mut(id.index())?;

            if *marked {
                continue;
            }

            *marked = true;
            held.reached = held.reached.saturating_add(*self.sizes.get(id.index())?);
            for_each_part(self.nodes.get(id.index())?, |part| stack.push(part));
        }

        Some(())
    }

    /// The nodes written out for everything `ids` reach, each counted once:
    /// what [`Types::to_types`] holds while it builds.
    pub(crate) fn reached_size(&self, ids: &[TypeId]) -> Option<usize> {
        if ids.is_empty() {
            return Some(0);
        }

        let reach = self.reach(ids)?;

        Some(
            reach
                .iter()
                .enumerate()
                .filter(|(_, reached)| **reached)
                .fold(0usize, |n, (at, _)| {
                    n.saturating_add(self.sizes.get(at).copied().unwrap_or(usize::MAX))
                }),
        )
    }

    /// Which ids `ids` reach, themselves included; all at or below the
    /// greatest.
    fn reach(&self, ids: &[TypeId]) -> Option<Vec<bool>> {
        let top = ids.iter().map(|id| id.index()).max()?;
        let mut reach = vec![false; top.checked_add(1)?];

        for id in ids {
            *reach.get_mut(id.index())? = true;
        }

        for at in (0..reach.len()).rev() {
            if !reach.get(at).copied().unwrap_or(false) {
                continue;
            }

            let mut ok = true;
            for_each_part(self.nodes.get(at)?, |part| {
                if let Some(slot) = reach.get_mut(part.index()) {
                    *slot = true;
                } else {
                    ok = false;
                }
            });

            if !ok {
                return None;
            }
        }

        Some(reach)
    }

    /// The types of `ids` and what they reach, at their own ids.
    fn build(&self, ids: &[TypeId]) -> Option<Vec<Option<Type>>> {
        let reach = self.reach(ids)?;

        let mut built = Vec::<Option<Type>>::new();
        built.resize_with(reach.len(), || None);
        let part = |built: &[Option<Type>], t: TypeId| built.get(t.index())?.clone();

        for (at, reached) in reach.iter().enumerate() {
            if !reached {
                continue;
            }

            let ty = match self.nodes.get(at)? {
                TypeNode::Int => Type::int(),
                TypeNode::Bool => Type::bool(),
                TypeNode::String => Type::string(),
                TypeNode::List(t) => Type::list(part(&built, *t)?),
                TypeNode::Option(t) => Type::option(part(&built, *t)?),
                TypeNode::Map { key, value } => {
                    Type::map(part(&built, *key)?, part(&built, *value)?)
                }
                TypeNode::Record(fields) => {
                    let mut record = BTreeMap::new();

                    for field in fields {
                        record.insert(field.name.clone(), part(&built, field.ty)?);
                    }

                    Type::record(record)
                }
            };

            *built.get_mut(at)? = Some(ty);
        }

        Some(built)
    }

    /// Check a stored table: each part before what holds it, each node
    /// once, records sorted, maps keyed by `Int` or `String`, and nothing
    /// nested past [`MAX_TYPE_DEPTH`] or with more than [`MAX_TYPE_NODES`]
    /// nodes written out.
    pub(crate) fn from_stored(nodes: Vec<TypeNode>) -> Result<Self, StoreError> {
        let mut types = Self::with_capacity(nodes.len());

        for (at, node) in nodes.into_iter().enumerate() {
            let fail = |invalid: Invalid| {
                StoreError::from(invalid).context(Message::AtType {
                    at: u32::try_from(at).unwrap_or(u32::MAX),
                })
            };

            let mut refers_later = false;
            for_each_part(&node, |part| {
                if part.index() >= at {
                    refers_later = true;
                }
            });

            if refers_later {
                return Err(fail(Invalid::TypeRefersLater));
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
                TypeNode::Map { key, .. }
                    if !matches!(types.get(*key), Some(TypeNode::Int | TypeNode::String)) =>
                {
                    return Err(fail(Invalid::MapKeyType));
                }
                _ => {}
            }

            if types.node_size(&node) > MAX_TYPE_NODES {
                return Err(fail(Invalid::TypeTooLarge));
            }

            match types.add(node) {
                None => return Err(fail(Invalid::TypeTooDeep)),
                Some(id) if id.index() != at => return Err(fail(Invalid::TypeRepeated)),
                Some(_) => {}
            }
        }

        Ok(types)
    }
}

/// The types a program under compilation holds in its globals and host
/// functions, counted as [`Program::verify`](crate::program::Program::verify)
/// bounds them.
#[derive(Debug, Default)]
pub(crate) struct Held {
    /// The types held, each written out and counted every time held.
    nodes: usize,
    /// Which ids the types held reach.
    marked: Vec<bool>,
    /// The written-out sizes of the types reached, each counted once.
    reached: usize,
}

impl Held {
    /// Whether more is held than a program may hold.
    pub(crate) fn too_large(&self) -> bool {
        self.nodes > crate::program::MAX_GLOBAL_TYPE_NODES
            || self.reached > crate::program::MAX_GLOBAL_TYPE_NODES
    }
}

/// Record `src -> dst` in `map`, growing it as needed.
fn record_map(map: &mut Vec<Option<TypeId>>, src: TypeId, dst: TypeId) -> Option<()> {
    let at = src.index();

    if map.len() <= at {
        map.resize(at.checked_add(1)?, None);
    }

    *map.get_mut(at)? = Some(dst);
    Some(())
}

/// Visit each id a node refers to, without allocating.
fn for_each_part(node: &TypeNode, mut f: impl FnMut(TypeId)) {
    match node {
        TypeNode::Int | TypeNode::Bool | TypeNode::String => {}
        TypeNode::List(t) | TypeNode::Option(t) => f(*t),
        TypeNode::Record(fields) => {
            for field in fields {
                f(field.ty);
            }
        }
        TypeNode::Map { key, value } => {
            f(*key);
            f(*value);
        }
    }
}
