use std::collections::BTreeMap;
use std::fmt;

use crate::Type;
use crate::types::{TypeFault, check_type, is_value_name};

/// One way to call a caller's function, which the machine asks the caller
/// to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Overload {
    pub(crate) params: Vec<Type>,
    pub(crate) ret: Type,
}

impl Overload {
    /// It in full, called `name`: `rating(String) -> Int`.
    pub(crate) fn signature(&self, name: &str) -> String {
        format!("{name}{} -> {}", self.describe(), self.ret)
    }

    /// Its parameters as an error lists them, such as `(String, Int)`.
    pub(crate) fn describe(&self) -> String {
        let params = self.params.iter().map(Type::to_string).collect::<Vec<_>>();
        format!("({})", params.join(", "))
    }
}

/// Why a name or function cannot be declared.
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// let error = env.value("Strength", Type::int()).unwrap_err();
/// assert_eq!(
///     error.to_string(),
///     "`Strength` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`"
/// );
///
/// let mut deep = Type::int();
///
/// for _ in 0..evalite::MAX_TYPE_DEPTH {
///     deep = Type::list(deep);
/// }
///
/// let error = env.value("x", deep).unwrap_err();
/// assert_eq!(error.to_string(), "the type of `x` nests more than 64 levels");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareError {
    name: Box<str>,
    kind: DeclareKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclareKind {
    /// A name other than a lowercase letter or `_` followed by lowercase
    /// letters, digits and `_`.
    InvalidName,
    /// A type nesting more than [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH)
    /// levels.
    TypeTooDeep,
    /// A type with more than [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES)
    /// nodes.
    TypeTooLarge,
    /// A map whose key type is neither `Int` nor `String`.
    MapKey,
}

impl DeclareError {
    #[cold]
    pub(crate) fn new(kind: DeclareKind, name: &str) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }
}

impl fmt::Display for DeclareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;

        match self.kind {
            DeclareKind::InvalidName => write!(
                f,
                "`{name}` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`"
            ),
            DeclareKind::TypeTooDeep => write!(
                f,
                "the type of `{name}` nests more than {} levels",
                crate::MAX_TYPE_DEPTH
            ),
            DeclareKind::TypeTooLarge => write!(
                f,
                "the type of `{name}` has more than {} parts once written out, which is the most a type may have",
                crate::MAX_TYPE_NODES
            ),
            DeclareKind::MapKey => write!(
                f,
                "the type of `{name}` has a map keyed by other than Int or String, a map's keys are Int or String"
            ),
        }
    }
}

impl std::error::Error for DeclareError {}

/// Every name an expression may use, with its type, and every function it
/// may call, which [`compile`](crate::compile) checks expressions against.
///
/// ```
/// use evalite::{Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
/// env.function("rating", [Type::string()], Type::int())?;
/// # Ok::<_, evalite::DeclareError>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub(crate) values: BTreeMap<String, Type>,
    pub(crate) functions: BTreeMap<String, Vec<Overload>>,
}

impl Environment {
    /// An environment declaring nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a value `name` of type `ty`, replacing any value of that
    /// name.
    ///
    /// # Errors
    ///
    /// Returns a [`DeclareError`] when `name` is not a valid name (lowercase
    /// letters, digits and `_`, starting with a letter or `_`), `ty` nests
    /// deeper than [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH) or has more than
    /// [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES) nodes, or `ty` has a
    /// [`Type::map`] keyed by other than `Int` or `String`.
    pub fn value(&mut self, name: &str, ty: Type) -> Result<(), DeclareError> {
        check_name(name)?;
        check(name, &ty)?;
        self.values.insert(name.to_owned(), ty);
        Ok(())
    }

    /// Declare a function of the caller's, which takes `params` and gives
    /// a `ret`. A function may be declared again with other parameters,
    /// such as `rating(String)` and `rating(String, Int)`; a call takes the
    /// first declaration its arguments' types match. Declaring the same
    /// parameters again replaces the return type.
    ///
    /// # Errors
    ///
    /// Returns a [`DeclareError`] when `name` is not a valid name, or a
    /// parameter or `ret` nests deeper than
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH), has more than
    /// [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES) nodes or has a map keyed by other
    /// than `Int` or `String`.
    pub fn function(
        &mut self,
        name: &str,
        params: impl IntoIterator<Item = Type>,
        ret: Type,
    ) -> Result<(), DeclareError> {
        check_name(name)?;
        let params = params.into_iter().collect::<Vec<_>>();

        for ty in params.iter().chain([&ret]) {
            check(name, ty)?;
        }

        let overloads = self.functions.entry(name.to_owned()).or_default();

        if let Some(overload) = overloads.iter_mut().find(|o| o.params == params) {
            overload.ret = ret;
            return Ok(());
        }

        overloads.push(Overload { params, ret });
        Ok(())
    }

    /// Every declared value with its type, in name order.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("wits", Type::int())?;
    /// env.value("bonus", Type::option(Type::int()))?;
    ///
    /// let values = env.values().map(|(name, ty)| format!("{name}: {ty}")).collect::<Vec<_>>();
    /// assert_eq!(values, ["bonus: Option<Int>", "wits: Int"]);
    /// # Ok::<_, evalite::DeclareError>(())
    /// ```
    pub fn values(&self) -> impl Iterator<Item = (&str, &Type)> {
        self.values.iter().map(|(name, ty)| (name.as_str(), ty))
    }

    /// Every declared function with each of its overloads written out, such
    /// as `rating(String) -> Int`, in name order, and in declaration order
    /// among a name's overloads.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.function("rating", [Type::string()], Type::int())?;
    /// env.function("rating", [Type::string(), Type::int()], Type::int())?;
    /// env.function("count", [], Type::int())?;
    ///
    /// let (name, signatures) = env.functions().next().unwrap();
    /// assert_eq!(name, "count");
    /// assert_eq!(signatures, ["count() -> Int"]);
    ///
    /// let (name, signatures) = env.functions().last().unwrap();
    /// assert_eq!(name, "rating");
    /// assert_eq!(signatures, ["rating(String) -> Int", "rating(String, Int) -> Int"]);
    /// # Ok::<_, evalite::DeclareError>(())
    /// ```
    pub fn functions(&self) -> impl Iterator<Item = (&str, Vec<String>)> {
        self.functions.iter().map(|(name, overloads)| {
            let signatures = overloads.iter().map(|o| o.signature(name)).collect();
            (name.as_str(), signatures)
        })
    }

    /// The type of the value `name`.
    pub fn value_type(&self, name: &str) -> Option<&Type> {
        self.values.get(name)
    }
}

fn check_name(name: &str) -> Result<(), DeclareError> {
    if is_value_name(name) {
        Ok(())
    } else {
        Err(DeclareError::new(DeclareKind::InvalidName, name))
    }
}

fn check(name: &str, ty: &Type) -> Result<(), DeclareError> {
    check_type(ty).map_err(|fault| match fault {
        TypeFault::TooDeep => DeclareError::new(DeclareKind::TypeTooDeep, name),
        TypeFault::TooLarge => DeclareError::new(DeclareKind::TypeTooLarge, name),
        TypeFault::MapKey => DeclareError::new(DeclareKind::MapKey, name),
    })
}
