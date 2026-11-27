use std::collections::BTreeMap;
use std::fmt;

use crate::types::{Kind, TypeFault, check_type, is_value_name};
use crate::{Program, Type};

/// One way to call a caller's function, which the machine asks the caller
/// to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Overload {
    pub(crate) params: Vec<Type>,
    /// The type of a last parameter given any number of times, at least
    /// the count with it, after `params`.
    pub(crate) repeated: Option<(Type, usize)>,
    pub(crate) ret: Type,
}

impl Overload {
    /// It in full, called `name`: `rating(String) -> Int`, or
    /// `min(Int...) -> Int` for a repeated parameter.
    pub(crate) fn signature(&self, name: &str) -> String {
        let mut params = self.params.iter().map(Type::to_string).collect::<Vec<_>>();

        if let Some((ty, _)) = &self.repeated {
            params.push(format!("{ty}..."));
        }

        format!("{name}({}) -> {}", params.join(", "), self.ret)
    }

    /// Its parameters as an error lists them, such as `(String, Int)`,
    /// `one or more Int`, or `(String) then any number of Int`.
    pub(crate) fn describe(&self) -> String {
        let params = self.params.iter().map(Type::to_string).collect::<Vec<_>>();
        let fixed = format!("({})", params.join(", "));

        match &self.repeated {
            None => fixed,
            Some((ty, min)) if self.params.is_empty() => format!("{} {ty}", or_more(*min)),
            Some((ty, min)) => format!("{fixed} then {} {ty}", or_more(*min)),
        }
    }

    /// The fewest arguments it takes.
    pub(crate) fn fewest(&self) -> usize {
        let min = self.repeated.as_ref().map_or(0, |(_, min)| *min);
        self.params.len().saturating_add(min)
    }

    /// How many holes a completion of a call leaves: one per argument it
    /// takes at the fewest, and one for a repeated parameter that may be
    /// given none. The repeated parameter has at most
    /// [`MAX_REPEATED_HOLES`], so a large minimum costs no more.
    pub(crate) fn holes(&self) -> usize {
        let repeated = match self.repeated {
            None => 0,
            Some((_, min)) => min.clamp(1, MAX_REPEATED_HOLES),
        };

        self.params.len().saturating_add(repeated)
    }

    /// Whether it takes `n` arguments.
    pub(crate) fn takes(&self, n: usize) -> bool {
        match self.repeated {
            None => n == self.params.len(),
            Some(_) => n >= self.fewest(),
        }
    }

    /// The parameter argument `n` is given for.
    pub(crate) fn param(&self, n: usize) -> Option<&Type> {
        match self.params.get(n) {
            Some(param) => Some(param),
            None => self.repeated.as_ref().map(|(ty, _)| ty),
        }
    }

    /// Whether a call could match both it and `other`: some list of
    /// argument types both take. It costs their parameters.
    fn overlaps(&self, other: &Self) -> bool {
        // The one of fewer parameters, the repeating one of the same.
        let key = |o: &Self| (o.params.len(), o.repeated.is_none());

        let (short, long) = if key(self) <= key(other) {
            (self, other)
        } else {
            (other, self)
        };

        let shared = short.params.len();

        if long.params.get(..shared) != Some(short.params.as_slice()) {
            return false;
        }

        let Some((ty, min)) = &short.repeated else {
            // Neither repeats, which only the same parameters match both
            // of, or only the longer, which takes more than the shorter.
            return long.repeated.is_none() && long.params.len() == shared;
        };

        if !long.params.iter().skip(shared).all(|p| p == ty) {
            return false;
        }

        // A call that stops at the longer's fixed parameters, which the
        // shorter takes when they are at least its fewest.
        let at_fixed = long.params.len() >= shared.saturating_add(*min);

        match &long.repeated {
            // The longer takes exactly its own count.
            None => at_fixed,
            // Both take every count from the larger fewest on when they
            // repeat the same type, and the longer may stop at its fixed
            // parameters when its minimum is 0.
            Some((other, long_min)) => other == ty || (*long_min == 0 && at_fixed),
        }
    }
}

/// The most holes a completion of a call leaves for a repeated
/// parameter, however large its minimum.
const MAX_REPEATED_HOLES: usize = 8;

/// The largest minimum count of a repeated parameter
/// [`Environment::function_repeated`] accepts: the most arguments a call
/// can have in an expression of [`DEFAULT_MAX_LEN`](crate::DEFAULT_MAX_LEN)
/// bytes, each but the last followed by a comma.
pub const MAX_REPEATED_MIN: usize = crate::DEFAULT_MAX_LEN / 2;

/// `one or more`, `two or more` and on, or `any number of` for `0`.
fn or_more(n: usize) -> String {
    const WORDS: [&str; 10] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];

    if n == 0 {
        return "any number of".to_owned();
    }

    match WORDS.get(n.saturating_sub(1)) {
        Some(word) => format!("{word} or more"),
        None => format!("{n} or more"),
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
    /// A value of a name already declared, whose slot cannot move.
    Redeclared,
    /// A value whose slot would end past the last word a slot may be at.
    TooManySlots,
    /// A function overload a call could match as well as one declared
    /// before it, other than the same parameters again.
    Overlaps,
    /// A repeated parameter whose minimum count is past
    /// [`MAX_REPEATED_MIN`].
    MinTooLarge,
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
            DeclareKind::Redeclared => write!(
                f,
                "`{name}` is already declared, and a value is declared once"
            ),
            DeclareKind::TooManySlots => write!(
                f,
                "`{name}` would be held past the last of the {} words the values may take",
                u32::MAX
            ),
            DeclareKind::Overlaps => write!(
                f,
                "`{name}` is already declared to take some of the same arguments, so a call could match either"
            ),
            DeclareKind::MinTooLarge => write!(
                f,
                "the repeated parameter of `{name}` has a minimum past {MAX_REPEATED_MIN}, the most arguments a call of the default length can have"
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
    /// Every value, in declaration order, which is slot order.
    pub(crate) slots: Vec<Slot>,
    /// Where each value is in `slots`, by name.
    pub(crate) names: BTreeMap<String, usize>,
    pub(crate) functions: BTreeMap<String, Vec<Overload>>,
    /// The words the slots take together, where the next one starts.
    words: u32,
    /// The layout so far, (name, type) in declaration order.
    fingerprint: Fingerprint,
}

/// Where a declared value is held: `width` words from word `at` of the
/// values, which never moves once declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) at: u32,
}

impl Environment {
    /// An environment declaring nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a value `name` of type `ty`, held in the next slot: the
    /// words after every value declared before it. Slots never move, so
    /// a name is declared once.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("wits", Type::int())?;
    ///
    /// let error = env.value("wits", Type::bool()).unwrap_err();
    /// assert_eq!(error.to_string(), "`wits` is already declared, and a value is declared once");
    /// # Ok::<_, evalite::DeclareError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`DeclareError`] when `name` is not a valid name (lowercase
    /// letters, digits and `_`, starting with a letter or `_`), `name` is
    /// already declared, `ty` nests deeper than
    /// [`MAX_TYPE_DEPTH`](crate::MAX_TYPE_DEPTH) or has more than
    /// [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES) nodes, `ty` has a
    /// [`Type::map`] keyed by other than `Int` or `String`, or the values
    /// would take more than `u32::MAX` words.
    pub fn value(&mut self, name: &str, ty: Type) -> Result<(), DeclareError> {
        check_name(name)?;

        if self.names.contains_key(name) {
            return Err(DeclareError::new(DeclareKind::Redeclared, name));
        }

        check(name, &ty)?;

        let at = self.words;
        self.words = u32::try_from(width(&ty))
            .ok()
            .and_then(|width| at.checked_add(width))
            .ok_or_else(|| DeclareError::new(DeclareKind::TooManySlots, name))?;

        self.fingerprint.value(name, &ty);
        self.names.insert(name.to_owned(), self.slots.len());
        self.slots.push(Slot {
            name: name.to_owned(),
            ty,
            at,
        });
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
    /// than `Int` or `String`, or when a call could match both it and a
    /// [`function_repeated`](Self::function_repeated) declared before it.
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

        self.overload(
            name,
            Overload {
                params,
                repeated: None,
                ret,
            },
        )
    }

    /// Declare a function of the caller's, which takes `params` and then
    /// `repeated` given any number of times, at least `min`, and gives a
    /// `ret`. The caller is given every argument in order, as for
    /// [`function`](Self::function).
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.function_repeated("min", [], Type::int(), 1, Type::int())?;
    /// env.function_repeated("join", [Type::string()], Type::string(), 0, Type::string())?;
    ///
    /// assert!(evalite::compile("min(3, 1, 2)", &env).is_ok());
    /// assert!(evalite::compile("join(\", \")", &env).is_ok());
    ///
    /// let error = evalite::compile("min()", &env).unwrap_err();
    /// assert_eq!(error.to_string(), "`min` takes one or more Int, found 0");
    ///
    /// let (_, signatures) = env.functions().last().unwrap();
    /// assert_eq!(signatures, ["min(Int...) -> Int"]);
    ///
    /// // A call of three Ints could match either.
    /// let error = env.function("min", [Type::int(), Type::int(), Type::int()], Type::int()).unwrap_err();
    /// assert_eq!(
    ///     error.to_string(),
    ///     "`min` is already declared to take some of the same arguments, so a call could match either"
    /// );
    /// # Ok::<_, evalite::DeclareError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`function`](Self::function) does, when `min` is past
    /// [`MAX_REPEATED_MIN`], and when a call could match both it and an
    /// overload declared before it, such as `min(Int, Int)` and
    /// `min(Int...)`, or `f(Int...)` and `f(Bool...)`, which both take no
    /// arguments. Declaring the same parameters and `min` again
    /// replaces the return type.
    pub fn function_repeated(
        &mut self,
        name: &str,
        params: impl IntoIterator<Item = Type>,
        repeated: Type,
        min: usize,
        ret: Type,
    ) -> Result<(), DeclareError> {
        check_name(name)?;
        let params = params.into_iter().collect::<Vec<_>>();

        for ty in params.iter().chain([&repeated, &ret]) {
            check(name, ty)?;
        }

        if min > MAX_REPEATED_MIN {
            return Err(DeclareError::new(DeclareKind::MinTooLarge, name));
        }

        self.overload(
            name,
            Overload {
                params,
                repeated: Some((repeated, min)),
                ret,
            },
        )
    }

    fn overload(&mut self, name: &str, new: Overload) -> Result<(), DeclareError> {
        let overloads = self.functions.entry(name.to_owned()).or_default();

        if let Some(overload) = overloads
            .iter_mut()
            .find(|o| o.params == new.params && o.repeated == new.repeated)
        {
            overload.ret = new.ret;
            return Ok(());
        }

        if overloads.iter().any(|o| o.overlaps(&new)) {
            return Err(DeclareError::new(DeclareKind::Overlaps, name));
        }

        overloads.push(new);
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
        self.named().map(|slot| (slot.name.as_str(), &slot.ty))
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
        Some(&self.slot(name)?.ty)
    }

    /// A stable 64-bit hash of its layout: every declared value's name and
    /// type, in declaration order. It is the same in every process and on
    /// every platform, so a stored [`Program`] compiled against an
    /// environment of the same layout reads the same slots. Functions are
    /// not part of it.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut a = Environment::new();
    /// a.value("wits", Type::int())?;
    /// a.value("bonus", Type::option(Type::int()))?;
    ///
    /// let mut b = Environment::new();
    /// b.value("bonus", Type::option(Type::int()))?;
    /// b.value("wits", Type::int())?;
    ///
    /// // The same values in another order are another layout.
    /// assert_ne!(a.fingerprint(), b.fingerprint());
    ///
    /// let program = evalite::compile("wits", &a)?;
    /// assert_eq!(program.fingerprint(), a.fingerprint());
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint.0
    }

    /// Whether `program` reads its values where this environment holds
    /// them: each value it reads is declared here with the same type, in
    /// the same slot. Slots never move, so a program stays accepted as
    /// values are declared after the ones it reads. The check is made
    /// value by value however the [`fingerprint`](Self::fingerprint)s
    /// compare, as a hash that matches is no proof of the same layout.
    ///
    /// ```
    /// use evalite::{Environment, Type};
    ///
    /// let mut env = Environment::new();
    /// env.value("wits", Type::int())?;
    /// let program = evalite::compile("wits + 1", &env)?;
    ///
    /// env.value("bonus", Type::option(Type::int()))?;
    /// assert_ne!(program.fingerprint(), env.fingerprint());
    /// assert!(env.accepts(&program));
    ///
    /// let mut other = Environment::new();
    /// other.value("wits", Type::bool())?;
    /// assert!(!other.accepts(&program));
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    pub fn accepts(&self, program: &Program) -> bool {
        refused(program, |name| {
            self.slot(name).map(|slot| (slot.at, &slot.ty))
        })
        .is_none()
    }

    /// The words every slot takes together.
    pub(crate) fn slot_words(&self) -> u32 {
        self.words
    }

    /// The value `name`'s slot.
    pub(crate) fn slot(&self, name: &str) -> Option<&Slot> {
        self.slots.get(*self.names.get(name)?)
    }

    /// Every declared value, in name order.
    pub(crate) fn named(&self) -> impl Iterator<Item = &Slot> {
        self.names.values().filter_map(|n| self.slots.get(*n))
    }
}

/// The first of the values `program` reads that a layout, whose slot
/// (word and type) of a name `slot` gives, does not hold where and as the
/// program reads it, by its index among the program's globals; `None` when
/// it holds them all.
///
/// Every read is checked, however the fingerprints compare: a 64-bit hash
/// is no proof two layouts are the same, as one made to collide with
/// another (from a user's own sheet template, say) would have a program
/// read its slots as another type. The check costs the program's global
/// types, which `MAX_GLOBAL_TYPE_NODES` bounds.
pub(crate) fn refused<'a>(
    program: &Program,
    slot: impl Fn(&str) -> Option<(u32, &'a Type)>,
) -> Option<usize> {
    program
        .globals
        .iter()
        .zip(program.globals())
        .position(|(global, (name, ty))| {
            !slot(name).is_some_and(|(at, held)| at == global.slot && held == ty)
        })
}

/// The words a value of type `ty` is held in, as
/// [`Types::width`](crate::types::Types::width) gives them: one, but for
/// an option of a value without a niche (an `Int`, or an option), which
/// is a word more than its value.
pub(crate) fn width(ty: &Type) -> usize {
    let mut options = 0usize;
    let mut ty = ty;

    while let Type(Kind::Option(inner)) = ty {
        options = options.saturating_add(1);
        ty = inner;
    }

    match ty {
        _ if options == 0 => 1,
        Type(Kind::Int) => options.saturating_add(1),
        _ => options,
    }
}

/// An FNV-1a hash of a layout, fixed so it is the same everywhere: each
/// value's name, then its type written out in pre-order, with lengths
/// before names and counts before fields so no two layouts write the same
/// bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint(u64);

impl Default for Fingerprint {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fingerprint {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn len(&mut self, n: usize) {
        self.bytes(&u64::try_from(n).unwrap_or(u64::MAX).to_le_bytes());
    }

    fn name(&mut self, name: &str) {
        self.len(name.len());
        self.bytes(name.as_bytes());
    }

    /// Add the value `name` of type `ty`. It visits each node of `ty`
    /// once, without recursion; `ty` has been checked to be at most
    /// [`MAX_TYPE_NODES`](crate::MAX_TYPE_NODES) nodes.
    fn value(&mut self, name: &str, ty: &Type) {
        enum Step<'a> {
            Type(&'a Type),
            Field(&'a str),
        }

        self.name(name);
        let mut stack = vec![Step::Type(ty)];

        while let Some(step) = stack.pop() {
            let ty = match step {
                Step::Field(name) => {
                    self.name(name);
                    continue;
                }
                Step::Type(ty) => ty,
            };

            match &ty.0 {
                Kind::Int => self.bytes(&[0]),
                Kind::Bool => self.bytes(&[1]),
                Kind::String => self.bytes(&[2]),
                Kind::List(entry) => {
                    self.bytes(&[3]);
                    stack.push(Step::Type(entry));
                }
                Kind::Option(value) => {
                    self.bytes(&[4]);
                    stack.push(Step::Type(value));
                }
                Kind::Record(fields) => {
                    self.bytes(&[5]);
                    self.len(fields.len());

                    // Pushed last first, so they are written in name order.
                    for (name, ty) in fields.iter().rev() {
                        stack.push(Step::Type(ty));
                        stack.push(Step::Field(name));
                    }
                }
                Kind::Map(key, value) => {
                    self.bytes(&[6]);
                    stack.push(Step::Type(value));
                    stack.push(Step::Type(key));
                }
            }
        }
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
