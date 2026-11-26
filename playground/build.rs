fn main() {
    let usage = sockel_build::Usage::new()
        .source("src/")
        .foreign(evalite_sockel::CLASSES.iter().copied());

    sockel_build::Config::new()
        .local("style/")
        .prefix("playground-")
        .prune(usage)
        .compile();
}
