fn main() {
    sockel_build::Config::new()
        .local("style/")
        .prefix("playground-")
        .compile();
}
