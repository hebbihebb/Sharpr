fn main() {
    let schema = "data/io.github.hebbihebb.Sharpr.gschema.xml";
    println!("cargo:rerun-if-changed={schema}");

    let status = std::process::Command::new("glib-compile-schemas")
        .arg("data")
        .status()
        .expect("glib-compile-schemas not found — install libglib2.0-dev or glib2-devel");

    assert!(status.success(), "glib-compile-schemas failed");
}
