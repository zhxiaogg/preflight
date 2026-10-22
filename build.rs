// Builds the React app into web/dist so rust-embed can bundle it into the binary.
// Set PREFLIGHT_SKIP_WEB_BUILD=1 to skip (web/dist must then already exist).
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-env-changed=PREFLIGHT_SKIP_WEB_BUILD");

    if std::env::var("PREFLIGHT_SKIP_WEB_BUILD").is_ok() {
        std::fs::create_dir_all("web/dist").unwrap();
        return;
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if !Path::new("web/node_modules").exists() {
        run(Command::new(npm).arg("install").current_dir("web"));
    }
    run(Command::new(npm).args(["run", "build"]).current_dir("web"));
}

fn run(cmd: &mut Command) {
    let status = cmd.status().expect("failed to run npm (is Node.js installed?)");
    assert!(status.success(), "web build failed: {cmd:?}");
}
