fn main() {
    // sockel's build passes its style directory to the crates depending on
    // it; the tests read it to check `CLASSES`.
    println!("cargo::rerun-if-env-changed=DEP_SOCKEL_STYLE");
    let style = std::env::var("DEP_SOCKEL_STYLE").unwrap_or_default();
    println!("cargo::rustc-env=SOCKEL_STYLE={style}");
}
