use std::collections::BTreeSet;

use sockel_build::{Config, Error, Usage};

const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

/// The sockel classes pruning drops with `usage`, from a stylesheet of all
/// of sockel's features, so `CLASSES` holds whichever an app enables.
fn pruned(usage: Usage) -> Result<BTreeSet<String>, Error> {
    let config = Config::new().style(env!("SOCKEL_STYLE")).prune(usage);

    Usage::new()
        .source(SOURCE)
        .report(&config)
        .map(|report| report.pruned.into_iter().collect())
}

#[test]
fn classes_are_what_the_components_reach() -> Result<(), Error> {
    let reached = pruned(Usage::new())?
        .difference(&pruned(Usage::new().source(SOURCE))?)
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(
        evalite_sockel::CLASSES,
        reached,
        "CLASSES should be: {reached:#?}"
    );

    Ok(())
}

#[test]
fn foreign_classes_prune_as_the_sources_do() -> Result<(), Error> {
    assert_eq!(
        pruned(Usage::new().foreign(evalite_sockel::CLASSES.iter().copied()))?,
        pruned(Usage::new().source(SOURCE))?,
    );

    Ok(())
}
