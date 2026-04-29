fn main() {
    let schema = "data/io.github.hebbihebb.Sharpr.gschema.xml";
    println!("cargo:rerun-if-changed={schema}");
    track_git_version_inputs();
    println!(
        "cargo:rustc-env=SHARPR_DISPLAY_VERSION={}",
        compute_display_version()
    );

    let status = std::process::Command::new("glib-compile-schemas")
        .arg("data")
        .status()
        .expect("glib-compile-schemas not found — install libglib2.0-dev or glib2-devel");

    assert!(status.success(), "glib-compile-schemas failed");

    // Compile GResource bundle (splash image + future assets).
    let gresource = "data/io.github.hebbihebb.Sharpr.gresource.xml";
    println!("cargo:rerun-if-changed={gresource}");
    println!("cargo:rerun-if-changed=data/splash.png");
    println!("cargo:rerun-if-changed=data/io.github.hebbihebb.Sharpr.png");

    glib_build_tools::compile_resources(&["data"], gresource, "sharpr.gresource");
}

fn compute_display_version() -> String {
    let git_count = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|stdout| stdout.trim().parse::<u32>().ok());

    match git_count {
        Some(count) => {
            let minor = (count / 20) + 1;
            format!("0.{minor}.0")
        }
        None => "0.1.0".to_string(),
    }
}

fn track_git_version_inputs() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    // Walk up from the manifest dir to find the .git directory (may be a parent).
    let git_dir = std::iter::successors(Some(manifest_dir.as_path()), |p| p.parent())
        .find_map(|dir| {
            let candidate = dir.join(".git");
            candidate.exists().then_some(candidate)
        });
    let Some(git_dir) = git_dir else { return };
    let head = git_dir.join("HEAD");
    if head.exists() {
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(head_contents) = std::fs::read_to_string(&head) {
            if let Some(reference) = head_contents.strip_prefix("ref: ") {
                let ref_path = git_dir.join(reference.trim());
                if ref_path.exists() {
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
    }
}
