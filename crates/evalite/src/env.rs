use std::collections::BTreeMap;
use std::fmt;

use crate::Type;
use crate::sig::{Shape, Signature};
use crate::types::{TypeFault, check_type, is_value_name};

/// A function evalite itself provides, which a caller adds to an
/// [`Environment`] with [`Environment::builtin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Builtin {
    /// `min(a, b, ...)`: the least of one or more `Int`s.
    Min,
    /// `max(a, b, ...)`: the greatest of one or more `Int`s.
    Max,
}

impl Builtin {
    /// Every builtin, as [`Environment::with_builtins`] declares them.
    ///
    /// ```
    /// use evalite::Builtin;
    ///
    /// let names = Builtin::ALL.map(Builtin::name);
    /// assert_eq!(names, ["min", "max"]);
    /// ```
    pub const ALL: [Builtin; 2] = [Self::Min, Self::Max];

    /// The name it is called by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
        }
    }

    /// What it takes and gives.
    pub(crate) fn signature(self) -> Signature {
        match self {
            Self::Min | Self::Max => Signature::repeating(&[Shape::Int], Shape::Int),
        }
    }
}

/// Its signature.
///
/// ```
/// use evalite::Builtin;
///
/// let signatures = Builtin::ALL.map(|b| b.to_string());
/// assert_eq!(signatures, [
///     "min(Int, ...) -> Int",
///     "max(Int, ...) -> Int",
/// ]);
/// ```
impl fmt::Display for Builtin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.signature().describe(self.name()))
    }
}

/// One way to call a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Overload {
    Builtin(Builtin),
    /// The caller's function, which the machine asks the caller to run.
    Host {
        params: Vec<Type>,
        ret: Type,
    },
}

impl Overload {
    /// It in full, called `name`: `max(Int, ...) -> Int`.
    pub(crate) fn signature(&self, name: &str) -> String {
        match self {
            Self::Builtin(builtin) => builtin.signature().describe(name),
            Self::Host { ret, .. } => format!("{name}{} -> {ret}", self.describe()),
        }
    }

    /// Its parameters as an error lists them, such as `(Int, ...)`.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Builtin(builtin) => builtin.signature().describe_params(),
            Self::Host { params, .. } => {
                let params = params.iter().map(Type::to_string).collect::<Vec<_>>();
                format!("({})", params.join(", "))
            }
        }
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
        }
    }
}

impl std::error::Error for DeclareError {}

/// Every name an expression may use, with its type, and every function it
/// may call, which [`compile`](crate::compile) checks expressions against.
///
/// ```
/// use evalite::{Builtin, Environment, Type};
///
/// let mut env = Environment::new();
/// env.value("strength", Type::int())?;
/// env.value("bonus", Type::option(Type::int()))?;
/// env.builtin(Builtin::Min);
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

    /// An environment with every [`Builtin`].
    pub fn with_builtins() -> Self {
        let mut env = Self::new();

        for builtin in Builtin::ALL {
            env.builtin(builtin);
        }

        env
    }

    /// Declare a value `name` of type `ty`, replacing any value of that
    /// name.
    ///
    /// # Errors
    ///
    /// Returns a [`DeclareError`] when `name` is not a valid name (lowercase
    /// letters, digits and `_`, starting with a letter or `_`), or `ty` nests
    /// deeper than [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH).
    pub fn value(&mut self, name: &str, ty: Type) -> Result<(), DeclareError> {
        check_name(name)?;
        check(name, &ty)?;
        self.values.insert(name.to_owned(), ty);
        Ok(())
    }

    /// Add a builtin function under its name.
    pub fn builtin(&mut self, builtin: Builtin) {
        let overloads = self.functions.entry(builtin.name().to_owned()).or_default();

        if !overloads.contains(&Overload::Builtin(builtin)) {
            overloads.push(Overload::Builtin(builtin));
        }
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
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH).
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

        for overload in overloads.iter_mut() {
            if let Overload::Host { params: p, ret: r } = overload
                && *p == params
            {
                *r = ret;
                return Ok(());
            }
        }

        overloads.push(Overload::Host { params, ret });
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
    /// as `max(Int, ...) -> Int`, in name order, and in
    /// declaration order among a name's overloads.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::with_builtins();
    /// env.function("rating", [Type::string()], Type::int())?;
    ///
    /// let (name, signatures) = env.functions().next().unwrap();
    /// assert_eq!(name, "max");
    /// assert_eq!(signatures, ["max(Int, ...) -> Int"]);
    ///
    /// let (name, signatures) = env.functions().last().unwrap();
    /// assert_eq!(name, "rating");
    /// assert_eq!(signatures, ["rating(String) -> Int"]);
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
    })
}
