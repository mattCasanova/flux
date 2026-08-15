use std::process::Command;

fn main() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=FLUX_GIT_SHA={}", sha);

    // Rerun when HEAD moves. `.git/HEAD` alone only changes on branch
    // switches — a commit on the current branch updates the ref file it
    // points at, so watch that too (plus packed-refs, where refs land
    // after `git gc`).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    if let Ok(head) = std::fs::read_to_string("../../.git/HEAD")
        && let Some(reference) = head.strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=../../.git/{}", reference.trim());
    }
}
