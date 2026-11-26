use std::collections::BTreeMap;
use std::fmt;

use crate::Type;
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
    /// `lookup(index, list)`: the `List<T>`'s entry at an `Int` index,
    /// clamped to its ends, so `lookup(9, [1, 2])` is `2`.
    Lookup,
}

impl Builtin {
    pub const ALL: [Builtin; 3] = [Self::Min, Self::Max, Self::Lookup];

    /// The name it is called by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Lookup => "lookup",
        }
    }

    /// Its parameters as an error lists them.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Min | Self::Max => "(Int, ...)",
            Self::Lookup => "(Int, List<T>)",
        }
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
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Builtin(builtin) => builtin.describe().to_owned(),
            Self::Host { params, .. } => {
                let params = params.iter().map(Type::to_string).collect::<Vec<_>>();
                format!("({})", params.join(", "))
            }
        }
    }
}

/// Why a name or function cannot be declared.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeclareError {
    /// A name other than a lowercase letter or `_` followed by lowercase
    /// letters, digits and `_`.
    InvalidName(String),
    /// `Option<Option<T>>`, which is not a type.
    NestedOption(String),
    /// A type nesting more than [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH)
    /// levels.
    TypeTooDeep(String),
}

impl fmt::Display for DeclareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(
                f,
                "`{name}` is not a valid name, names are lowercase letters, digits and `_`, starting with a letter or `_`"
            ),
            Self::NestedOption(name) => write!(
                f,
                "the type of `{name}` has an option of an option, which is not a type"
            ),
            Self::TypeTooDeep(name) => write!(
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

    /// The type of the value `name`.
    pub fn value_type(&self, name: &str) -> Option<&Type> {
        self.values.get(name)
    }
}

fn check_name(name: &str) -> Result<(), DeclareError> {
    if is_value_name(name) {
        Ok(())
    } else {
        Err(DeclareError::InvalidName(name.to_owned()))
    }
}

fn check(name: &str, ty: &Type) -> Result<(), DeclareError> {
    check_type(ty).map_err(|fault| match fault {
        TypeFault::NestedOption => DeclareError::NestedOption(name.to_owned()),
        TypeFault::TooDeep => DeclareError::TypeTooDeep(name.to_owned()),
    })
}
