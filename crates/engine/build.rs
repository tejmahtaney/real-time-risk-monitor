//! Captures the git commit hash at build time into `RRM_GIT_COMMIT_BUILD_TIME`, so
//! `engine::recorder::git_commit` does not need `git` or a `.git` folder at runtime
//! (a container image built in Part 17 typically has neither). `println!` here is the
//! Cargo build-script protocol (`cargo:...` directives), not application logging —
//! CLAUDE.md's "no `println!`" rule is about the app's own runtime code.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_owned());
    if let Some(hash) = hash {
        println!("cargo:rustc-env=RRM_GIT_COMMIT_BUILD_TIME={hash}");
    }
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
