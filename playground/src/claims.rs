//! What the page claims about evalite, each with examples that show it.
//! Every expression example is checked by the tests below against what the
//! playground computes, so the page cannot claim a result evalite does not
//! give. Clicking one loads it into the playground.

use crate::examples::Example;

/// One claim: a headline, a sentence in plain words, and what shows it.
pub struct Claim {
    pub head: &'static str,
    pub body: &'static str,
    /// Expressions to run, each loaded into the playground when clicked.
    pub cases: &'static [Case],
    /// Rust, for what an expression alone cannot show.
    pub rust: Option<&'static str>,
    /// A line under the examples, such as how to try them.
    pub hint: Option<&'static str>,
}

/// An expression and what it gives.
pub struct Case {
    /// What the page shows, when the expression is too long to.
    pub shown: Option<&'static str>,
    pub example: Example,
    pub gives: Gives,
}

pub enum Gives {
    /// A value and its type, as `5 : Int`.
    Value(&'static str),
    /// An error's message.
    Error(&'static str),
    /// Every error's message, in source order.
    Errors(&'static [&'static str]),
    /// What completing at the end of the expression offers: some of what it
    /// lists, and what it leaves out.
    Offers {
        some: &'static [&'static str],
        not: &'static [&'static str],
        /// It lists more than `some`.
        more: bool,
    },
}

const TYPED_VARS: &[(&str, &str, &str)] =
    &[("rank", "Int", "3"), ("bonus", "Option<Int>", "some(2)")];

const SAFE_VARS: &[(&str, &str, &str)] =
    &[("rank", "Int", "2"), ("dice", "List<Int>", "[4, 2, 6]")];

const COMPLETE_VARS: &[(&str, &str, &str)] = &[
    ("rank", "Int", "3"),
    ("bonus", "Option<Int>", "some(2)"),
    ("name", "String", "\"Ada\""),
];

const fn case(
    source: &'static str,
    vars: &'static [(&'static str, &'static str, &'static str)],
    gives: Gives,
) -> Case {
    Case {
        shown: None,
        example: Example {
            title: "",
            source,
            vars,
            fuel: None,
        },
        gives,
    }
}

/// 40 nested parentheses around `1`, past the default limit of 32.
const DEEP: &str =
    "((((((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))))))";

pub const CLAIMS: &[Claim] = &[
    Claim {
        head: "Strictly typed",
        body: "Every expression has one type, checked before it runs. Nothing converts on its own, \
               and a value that might be missing has to be handled.",
        cases: &[
            case(
                "rank + true",
                TYPED_VARS,
                Gives::Error("`+` takes Int and Int, found Int and Bool"),
            ),
            case(
                "rank + bonus",
                TYPED_VARS,
                Gives::Error(
                    "`+` takes Int and Int, found Int and Option<Int>, \
                     unwrap the optional value first with `.unwrap_or(...)`",
                ),
            ),
            case(
                "rank + bonus.unwrap_or(0)",
                TYPED_VARS,
                Gives::Value("5 : Int"),
            ),
        ],
        rust: None,
        hint: None,
    },
    Claim {
        head: "Safe on untrusted input",
        body: "Whatever a user types, the worst that can happen is an error. Nothing crashes, \
               hangs, overflows the stack or eats memory.",
        cases: &[
            Case {
                shown: Some("((((…40 levels…1…))))"),
                ..case(
                    DEEP,
                    &[],
                    Gives::Error(
                        "this parenthesis nests too deeply, the limit is 32 levels of nesting",
                    ),
                )
            },
            case(
                "rank * 9223372036854775807",
                SAFE_VARS,
                Gives::Error("`2 * 9223372036854775807` overflows an Int"),
            ),
            case(
                "dice[5]",
                SAFE_VARS,
                Gives::Error("index 5 is out of range of a list of 3 entries"),
            ),
            Case {
                shown: Some("x + x + … + x   (allowed 20 steps)"),
                example: Example {
                    title: "",
                    source: "x + x + x + x + x + x + x + x + x + x + x + x + x + x + x + x",
                    vars: &[("x", "Int", "1")],
                    fuel: Some(20),
                },
                gives: Gives::Error("the expression took too long: it used all of its 20 steps"),
            },
        ],
        rust: None,
        hint: None,
    },
    Claim {
        head: "Compile once, run many times",
        body: "Check an expression when the user saves it, and keep the compiled program as bytes \
               next to your data. Running it later skips parsing and checking.",
        cases: &[],
        rust: Some(
            "// When the user saves the formula:\n\
             let bytes = evalite::compile(\"Int::min(wits, dexterity) + athletics\", &env)?.to_bytes()?;\n\
             \n\
             // Later, whenever the values change:\n\
             let program = Program::from_bytes(&bytes)?;\n\
             let mut run = machine.load(&program)?;\n\
             run.bind(\"wits\", Value::int(3))?;\n\
             run.bind(\"dexterity\", Value::int(2))?;\n\
             run.bind(\"athletics\", Value::int(1))?;\n\
             run.run()?; // 3",
        ),
        hint: None,
    },
    Claim {
        head: "Stored bytes are checked",
        body: "Bytes from a database or a file may be corrupt or tampered with. Loading checks them \
               again and refuses anything that isn't a valid, well-typed program.",
        cases: &[],
        rust: Some(
            "let bytes = program.to_bytes()?;\n\
             Program::from_bytes(&bytes[..bytes.len() - 1])\n\
             // error: the bytes do not decode",
        ),
        hint: None,
    },
    Claim {
        head: "Errors that point",
        body: "Each error says what it expected and what it found, and marks the exact place, so \
               you can underline it in your own UI. Errors while running point too.",
        cases: &[
            case(
                "Int::min(1, (2 + 3)",
                &[],
                Gives::Error(
                    "expected `)` to close the `(` at byte 8, found the end of the expression",
                ),
            ),
            case(
                "Int::min(rank, true) + bonus",
                &[("rank", "Int", "3")],
                Gives::Errors(&[
                    "argument 2 of `Int::min` must be Int, found Bool",
                    "unknown name `bonus`",
                ]),
            ),
            case(
                "10 / rank",
                &[("rank", "Int", "0")],
                Gives::Error("`10 / 0` divides by zero"),
            ),
        ],
        rust: None,
        hint: None,
    },
    Claim {
        head: "Autocomplete that only offers what fits",
        body: "Ask what can go at a spot and evalite lists only what would type-check there. You can \
               build an editor where users pick from a list and every pick is valid.",
        cases: &[
            case(
                "rank + ",
                COMPLETE_VARS,
                Gives::Offers {
                    some: &[
                        "rank",
                        "bonus.unwrap_or(_)",
                        "Int::min(_, _)",
                        "Int::max(_, _)",
                    ],
                    not: &["name", "bonus"],
                    more: true,
                },
            ),
            case(
                "rank + bonus.",
                COMPLETE_VARS,
                Gives::Offers {
                    some: &["unwrap_or(_)"],
                    not: &["is_some()", "is_none()"],
                    more: false,
                },
            ),
        ],
        rust: None,
        hint: Some("Click one, then press Ctrl+Space at the end of the expression."),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use evalite::Structure;

    use crate::eval::Failed;
    use crate::{Input, compute};

    fn primaries(failed: &Failed) -> Vec<String> {
        match failed {
            Failed::Compile(diagnostics) => diagnostics.iter().map(|d| d.to_string()).collect(),
            Failed::Run(ran, _) => ran.errors().into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn every_case_gives_what_the_page_says() {
        for claim in CLAIMS {
            for case in claim.cases {
                let input = Input::example(&case.example);
                let source = case.example.source;

                match &case.gives {
                    Gives::Value(shown) => {
                        let done = compute(&input).unwrap_or_else(|e| panic!("{source}: {e:?}"));
                        assert_eq!(format!("{} : {}", done.value, done.ty), *shown, "{source}");
                    }
                    Gives::Error(shown) => {
                        let failed = compute(&input)
                            .err()
                            .unwrap_or_else(|| panic!("{source} ran"));
                        let primary = primaries(&failed);
                        assert_eq!(
                            primary.first().map(String::as_str),
                            Some(*shown),
                            "{source}"
                        );
                    }
                    Gives::Errors(shown) => {
                        let failed = compute(&input)
                            .err()
                            .unwrap_or_else(|| panic!("{source} ran"));
                        assert_eq!(primaries(&failed), *shown, "{source}");
                    }
                    Gives::Offers { some, not, more } => {
                        let env = crate::eval::environment(&input.bindings()).unwrap();
                        let structure = Structure::from_text(&format!("{source}_")).unwrap();
                        let hole = *structure.holes().last().unwrap();
                        let offers = structure.complete(&env, hole, None).unwrap();
                        let labels = offers.iter().map(|o| o.label()).collect::<Vec<_>>();

                        for label in *some {
                            assert!(labels.contains(label), "{source}: {label} in {labels:?}");
                        }

                        for label in *not {
                            assert!(!labels.contains(label), "{source}: {label} in {labels:?}");
                        }

                        assert_eq!(labels.len() > some.len(), *more, "{source}: {labels:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn storing_and_loading_as_shown() {
        use evalite::{Environment, Limits, Machine, Program, Type, Value};

        let mut env = Environment::new();

        for name in ["wits", "dexterity", "athletics"] {
            env.value(name, Type::int()).unwrap();
        }

        let bytes = evalite::compile("Int::min(wits, dexterity) + athletics", &env)
            .unwrap()
            .to_bytes()
            .unwrap();
        let program = Program::from_bytes(&bytes).unwrap();
        let mut machine = Machine::new(Limits::new());
        let mut run = machine.load(&program).unwrap();
        run.bind("wits", Value::int(3)).unwrap();
        run.bind("dexterity", Value::int(2)).unwrap();
        run.bind("athletics", Value::int(1)).unwrap();
        assert_eq!(run.run().unwrap().value().as_int(), Some(3));

        let error = Program::from_bytes(&bytes[..bytes.len() - 1]).unwrap_err();
        assert_eq!(error.to_string(), "the bytes do not decode");
    }
}
