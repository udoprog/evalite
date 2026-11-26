//! Evaluating the editor's expression against the page's variables.

use std::ops::Range;

use evalite::{
    DEFAULT_FUEL, DEFAULT_MAX_DEPTH, DEFAULT_MAX_LEN, DEFAULT_MEMORY, Environment, Labels, Limits,
    Machine, Options, RunError, Type, Value,
};

/// The machine's bounds, as the page sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub fuel: u64,
    pub memory: u32,
    pub depth: usize,
    pub len: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            memory: DEFAULT_MEMORY,
            depth: DEFAULT_MAX_DEPTH,
            len: DEFAULT_MAX_LEN,
        }
    }
}

/// A variable the expression may read.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub name: String,
    pub ty: Type,
    pub value: Value,
}

/// What a successful run gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub value: String,
    pub ty: String,
    pub steps: u64,
    pub memory: u64,
}

/// One label of an error: its message, and the characters (not bytes) of
/// the expression it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub message: String,
    pub span: Option<Range<usize>>,
    pub primary: bool,
}

/// Compile and run `source` with `bindings` declared and bound.
pub fn evaluate(source: &str, bindings: &[Binding], bounds: Bounds) -> Result<Done, Vec<Mark>> {
    let env = environment(bindings)?;

    let options = Options::new().max_depth(bounds.depth).max_len(bounds.len);
    let (program, debug) = evalite::compile_debug(source, &env, &options).map_err(|errors| {
        errors
            .iter()
            .flat_map(|error| marks(source, error.labels()))
            .collect::<Vec<_>>()
    })?;

    let mut machine = Machine::new(Limits::new().memory(bounds.memory).fuel(bounds.fuel));
    let failed = |error: RunError| marks(source, error.labels(&program, Some(&debug)));
    let mut run = machine.load(&program).map_err(failed)?;

    let read = run
        .globals()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();

    for binding in bindings {
        if read.contains(&binding.name.as_str()) {
            run.bind(&binding.name, binding.value.clone())
                .map_err(failed)?;
        }
    }

    let outcome = run.run().map_err(failed)?;

    Ok(Done {
        value: outcome.value().to_string(),
        ty: program.result_type().to_string(),
        steps: outcome.steps(),
        memory: outcome.memory(),
    })
}

/// The builtins, with `bindings` declared.
pub fn environment(bindings: &[Binding]) -> Result<Environment, Vec<Mark>> {
    let mut env = Environment::with_builtins();

    for binding in bindings {
        env.value(&binding.name, binding.ty.clone())
            .map_err(|error| {
                vec![Mark {
                    message: format!("`{}`: {error}", binding.name),
                    span: None,
                    primary: true,
                }]
            })?;
    }

    Ok(env)
}

fn marks(source: &str, labels: Labels<'_>) -> Vec<Mark> {
    labels
        .map(|label| Mark {
            message: label.message().to_string(),
            span: label.span().map(|span| {
                let start = chars_before(source, span.start);
                let end = chars_before(source, span.end);
                start..end.max(start)
            }),
            primary: label.is_primary(),
        })
        .collect()
}

/// How many characters come before byte `at` of `source`.
fn chars_before(source: &str, at: u32) -> usize {
    let at = usize::try_from(at).unwrap_or(usize::MAX);
    source
        .char_indices()
        .take_while(|(index, _)| *index < at)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(name: &str, value: i64) -> Binding {
        Binding {
            name: name.into(),
            ty: Type::int(),
            value: Value::int(value),
        }
    }

    #[test]
    fn runs() {
        let done = evaluate("a + b * 2", &[int("a", 1), int("b", 3)], Bounds::default()).unwrap();
        assert_eq!(done.value, "7");
        assert_eq!(done.ty, "Int");
        assert!(done.steps > 0);
    }

    #[test]
    fn type_error_points_at_its_span() {
        let marks = evaluate("1 + true", &[], Bounds::default()).unwrap_err();
        let primary = marks.iter().find(|m| m.primary).unwrap();
        assert!(primary.span.is_some());
    }

    #[test]
    fn spans_count_characters() {
        let marks = evaluate("\"é\" + x", &[], Bounds::default()).unwrap_err();
        let primary = marks.iter().find(|m| m.primary).unwrap();
        assert_eq!(primary.span, Some(6..7));
    }

    #[test]
    fn runs_out_of_fuel() {
        let bounds = Bounds {
            fuel: 3,
            ..Bounds::default()
        };
        let marks = evaluate("1 + 2 + 3 + 4 + 5", &[], bounds).unwrap_err();
        assert!(marks.iter().any(|m| m.primary));
    }
}
