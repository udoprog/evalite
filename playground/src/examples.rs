//! The examples menu: each an expression, the variables it reads, and the
//! fuel it runs with when that matters.

pub struct Example {
    pub title: &'static str,
    pub source: &'static str,
    /// `(name, type, value)`, written as the variables table takes them.
    pub vars: &'static [(&'static str, &'static str, &'static str)],
    pub fuel: Option<u64>,
}

pub const EXAMPLES: &[Example] = &[
    Example {
        title: "Dice pool",
        source: "wits.min(dexterity) + athletics + dice[0] + dice[1] + dice[2]",
        vars: &[
            ("wits", "Int", "3"),
            ("dexterity", "Int", "2"),
            ("athletics", "Int", "1"),
            ("dice", "List<Int>", "[4, 2, 6]"),
        ],
        fuel: None,
    },
    Example {
        title: "Optional bonus",
        source: "strength + bonus.unwrap_or(0) + specialties.brawl.unwrap_or(0)",
        vars: &[
            ("strength", "Int", "3"),
            ("bonus", "Option<Int>", "Some(2)"),
            ("specialties", "{brawl: Option<Int>}", "{brawl: None}"),
        ],
        fuel: None,
    },
    Example {
        title: "Table by index",
        source: "[10, 11, 12, 13, 14].get(blood_potency)",
        vars: &[("blood_potency", "Int", "2")],
        fuel: None,
    },
    Example {
        title: "Track record",
        source: "health.boxes - health.count[s]",
        vars: &[
            (
                "health",
                "{boxes: Int, count: List<Int>}",
                "{boxes: 7, count: [3, 1, 0]}",
            ),
            ("s", "Int", "1"),
        ],
        fuel: None,
    },
    Example {
        title: "Clamp chain",
        source: "(strength * 2 - penalty).clamp(0, 5).max(1)",
        vars: &[("strength", "Int", "4"), ("penalty", "Int", "1")],
        fuel: None,
    },
];
