//! The expressions evalite is compared on, and each engine's closest
//! equivalent of compiling one and evaluating it. See `README.md`.

use std::collections::BTreeMap;
use std::fmt::{self, Write};

/// The engines compared, in the order results are reported.
pub const ENGINES: [&str; 3] = ["evalite", "evalexpr", "fasteval"];

/// The variables every expression may read, with their values.
pub const VARIABLES: [(&str, i64); 16] = [
    ("strength", 3),
    ("dexterity", 2),
    ("stamina", 4),
    ("wits", 3),
    ("composure", 2),
    ("resolve", 3),
    ("brawl", 2),
    ("athletics", 1),
    ("stealth", 3),
    ("armor", 1),
    ("size", 5),
    ("humanity", 7),
    ("blood_potency", 2),
    ("health", 7),
    ("damage", 3),
    ("bonus", 1),
];

/// What an expression gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Int,
    Bool,
}

/// One expression, written for each engine.
#[derive(Debug, Clone)]
pub struct Case {
    /// Its name in benchmark ids and the snapshot.
    pub name: &'static str,
    /// What it exercises.
    pub description: &'static str,
    /// What it gives.
    pub kind: Kind,
    /// Its source for evalite, evalexpr and fasteval, in [`ENGINES`] order.
    pub sources: [String; 3],
    /// What it evaluates to, with `true` as 1 and `false` as 0.
    pub expected: i64,
}

impl Case {
    /// Its source for `engine`.
    pub fn source(&self, engine: &str) -> Option<&str> {
        let n = ENGINES.iter().position(|e| *e == engine)?;
        self.sources.get(n).map(String::as_str)
    }
}

/// The same source for every engine.
fn same(source: &str) -> [String; 3] {
    [source.to_owned(), source.to_owned(), source.to_owned()]
}

/// `terms` terms summed, alternating `+` and `-`, each a variable or a
/// variable times a small constant, cycling through [`VARIABLES`].
fn formula(terms: usize) -> (String, i64) {
    let mut out = String::new();
    let mut total = 0i64;

    let factors = [1i64, 2, 3].into_iter().cycle();

    for (n, ((name, value), factor)) in VARIABLES
        .iter()
        .cycle()
        .zip(factors)
        .take(terms)
        .enumerate()
    {
        let term = value.saturating_mul(factor);

        let negative = n % 2 == 1;

        if n > 0 {
            out.push_str(if negative { " - " } else { " + " });
        }

        if factor == 1 {
            out.push_str(name);
        } else {
            // Writing to a String cannot fail.
            let _ = write!(out, "{name} * {factor}");
        }

        total = if negative {
            total.saturating_sub(term)
        } else {
            total.saturating_add(term)
        };
    }

    (out, total)
}

/// The expressions compared, smallest first.
pub fn cases() -> Vec<Case> {
    let sum = VARIABLES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(" + ");
    let sum_value = VARIABLES
        .iter()
        .fold(0i64, |acc, (_, v)| acc.saturating_add(*v));

    let (formula, formula_value) = formula(64);

    vec![
        Case {
            name: "arithmetic",
            description: "A few operators over three variables.",
            kind: Kind::Int,
            sources: same("(strength + brawl) * 2 - armor * 3 + 1"),
            expected: 8,
        },
        Case {
            name: "variables",
            description: "The sum of 16 variables.",
            kind: Kind::Int,
            sources: same(&sum),
            expected: sum_value,
        },
        Case {
            name: "functions",
            description: "Nested min and max calls.",
            kind: Kind::Int,
            sources: [
                "Int::min(Int::max(strength + brawl - armor, 0), 5) + Int::max(wits, composure, 1)"
                    .to_owned(),
                "min(max(strength + brawl - armor, 0), 5) + max(wits, composure, 1)".to_owned(),
                "min(max(strength + brawl - armor, 0), 5) + max(wits, composure, 1)".to_owned(),
            ],
            expected: 7,
        },
        Case {
            name: "logic",
            description: "Comparisons joined with && and ||.",
            kind: Kind::Bool,
            sources: same(
                "humanity >= 5 && health - damage > 2 || blood_potency == 3 && stealth < 2",
            ),
            expected: 1,
        },
        Case {
            name: "formula",
            description: "A 64-term formula of sums, differences and products.",
            kind: Kind::Int,
            sources: same(&formula),
            expected: formula_value,
        },
    ]
}

/// An engine failing to compile or evaluate an expression.
#[derive(Debug)]
pub struct Error {
    engine: &'static str,
    message: String,
}

impl Error {
    fn new(engine: &'static str, message: impl fmt::Display) -> Self {
        Self {
            engine,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.engine, self.message)
    }
}

impl std::error::Error for Error {}

/// evalite: an [`evalite::Environment`] declaring every variable as an
/// `Int`, and a reused [`evalite::Machine`].
pub struct Evalite {
    env: evalite::Environment,
    machine: evalite::Machine,
}

impl Evalite {
    pub fn new() -> Result<Self, Error> {
        let mut env = evalite::Environment::new();

        for (name, _) in VARIABLES {
            env.value(name, evalite::Type::int())
                .map_err(|e| Error::new("evalite", e))?;
        }

        Ok(Self {
            env,
            machine: evalite::Machine::new(evalite::Limits::new()),
        })
    }

    /// Compile: parse, type check and generate a verified program, then
    /// resolve the values it reads, as a caller holding its values would
    /// once per program.
    pub fn compile(&self, source: &str) -> Result<EvaliteProgram, Error> {
        let program = evalite::compile(source, &self.env).map_err(|e| Error::new("evalite", e))?;

        let bindings = program
            .globals()
            .map(|(name, _)| {
                let value = VARIABLES
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| Error::new("evalite", format!("no value for {name}")))?;
                Ok((name.to_owned(), evalite::Value::int(value)))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        Ok(EvaliteProgram { program, bindings })
    }

    /// Evaluate: load the program, bind its prepared values and run it.
    pub fn evaluate(&mut self, program: &EvaliteProgram) -> Result<i64, Error> {
        let mut run = self
            .machine
            .load(&program.program)
            .map_err(|e| Error::new("evalite", e))?;

        for (name, value) in &program.bindings {
            run.bind(name, value)
                .map_err(|e| Error::new("evalite", e))?;
        }

        let outcome = run.run().map_err(|e| Error::new("evalite", e))?;
        let value = outcome.value();

        value
            .as_int()
            .or_else(|| value.as_bool().map(i64::from))
            .ok_or_else(|| Error::new("evalite", format!("unexpected result {value}")))
    }
}

/// A compiled evalite program and the values it reads, resolved by name
/// once.
pub struct EvaliteProgram {
    program: evalite::Program,
    bindings: Vec<(String, evalite::Value)>,
}

/// evalexpr: a [`evalexpr::HashMapContext`] holding every variable as an
/// `Int`.
pub struct Evalexpr {
    context: evalexpr::HashMapContext,
}

impl Evalexpr {
    pub fn new() -> Result<Self, Error> {
        use evalexpr::ContextWithMutableVariables;

        let mut context = evalexpr::HashMapContext::new();

        for (name, value) in VARIABLES {
            context
                .set_value(name.to_owned(), evalexpr::Value::from_int(value))
                .map_err(|e| Error::new("evalexpr", e))?;
        }

        Ok(Self { context })
    }

    /// Compile: build the operator tree.
    pub fn compile(&self, source: &str) -> Result<evalexpr::Node, Error> {
        evalexpr::build_operator_tree(source).map_err(|e| Error::new("evalexpr", e))
    }

    /// Evaluate: evaluate the tree against the context.
    pub fn evaluate(&self, node: &evalexpr::Node) -> Result<i64, Error> {
        match node
            .eval_with_context(&self.context)
            .map_err(|e| Error::new("evalexpr", e))?
        {
            evalexpr::Value::Int(n) => Ok(n),
            evalexpr::Value::Boolean(b) => Ok(i64::from(b)),
            other => Err(Error::new("evalexpr", format!("unexpected result {other}"))),
        }
    }
}

/// fasteval: one [`fasteval::Slab`] per compiled expression, and a
/// namespace holding every variable as an `f64`, the only type it has.
pub struct Fasteval {
    namespace: BTreeMap<String, f64>,
}

/// A compiled fasteval expression and the slab it lives in.
pub struct FastevalProgram {
    slab: fasteval::Slab,
    instruction: fasteval::Instruction,
}

impl Fasteval {
    pub fn new() -> Self {
        Self {
            namespace: VARIABLES
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value as f64))
                .collect(),
        }
    }

    /// Compile: parse into `slab`, then compile, which folds constants.
    /// The slab is cleared first, as a caller reusing one would.
    pub fn compile_into(
        &self,
        source: &str,
        slab: &mut fasteval::Slab,
    ) -> Result<fasteval::Instruction, Error> {
        use fasteval::Compiler;

        slab.clear();
        let parser = fasteval::Parser::new();
        let expression = parser
            .parse_noclear(source, &mut slab.ps)
            .map_err(|e| Error::new("fasteval", e))?;
        Ok(expression.from(&slab.ps).compile(&slab.ps, &mut slab.cs))
    }

    /// Compile into a slab of its own.
    pub fn compile(&self, source: &str) -> Result<FastevalProgram, Error> {
        let mut slab = fasteval::Slab::new();
        let instruction = self.compile_into(source, &mut slab)?;
        Ok(FastevalProgram { slab, instruction })
    }

    /// Evaluate: evaluate the compiled instruction against the namespace.
    pub fn evaluate(&mut self, program: &FastevalProgram) -> Result<f64, Error> {
        use fasteval::Evaler;

        program
            .instruction
            .eval(&program.slab, &mut self.namespace)
            .map_err(|e| Error::new("fasteval", e))
    }
}

impl Default for Fasteval {
    fn default() -> Self {
        Self::new()
    }
}

/// Compile and evaluate `case` with every engine, failing unless each gives
/// what it should.
pub fn check(case: &Case) -> Result<(), Error> {
    let [evalite, evalexpr, fasteval] = &case.sources;

    let mut engine = Evalite::new()?;
    let program = engine.compile(evalite)?;
    let got = engine.evaluate(&program)?;

    if got != case.expected {
        return Err(Error::new(
            "evalite",
            format!("{}: gave {got}, expected {}", case.name, case.expected),
        ));
    }

    let engine = Evalexpr::new()?;
    let node = engine.compile(evalexpr)?;
    let got = engine.evaluate(&node)?;

    if got != case.expected {
        return Err(Error::new(
            "evalexpr",
            format!("{}: gave {got}, expected {}", case.name, case.expected),
        ));
    }

    let mut engine = Fasteval::new();
    let program = engine.compile(fasteval)?;
    let got = engine.evaluate(&program)?;

    // fasteval's `&&` and `||` give an operand, not 1 or 0.
    let ok = match case.kind {
        Kind::Int => got == case.expected as f64,
        Kind::Bool => (got != 0.0) == (case.expected != 0),
    };

    if !ok {
        return Err(Error::new(
            "fasteval",
            format!("{}: gave {got}, expected {}", case.name, case.expected),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_engine_agrees() {
        for case in super::cases() {
            super::check(&case).unwrap();
        }
    }
}
