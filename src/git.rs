use anyhow::{bail, Result};
use std::process::Command;

fn git(repo: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output()?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn toplevel(repo: &str) -> Result<String> {
    Ok(git(repo, &["rev-parse", "--show-toplevel"])?.trim().to_string())
}

pub fn rev_parse(repo: &str, rev: &str) -> Result<String> {
    Ok(git(repo, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?.trim().to_string())
}

/// Same as GitHub: changes on `head` since it diverged from `base`.
pub fn diff(repo: &str, base: &str, head: &str) -> Result<String> {
    git(repo, &["diff", "--no-color", "--no-ext-diff", "-M", &format!("{base}...{head}")])
}

/// Commits on `head` not on `base`, newest first, as (sha, subject).
pub fn commits(repo: &str, base: &str, head: &str) -> Result<Vec<(String, String)>> {
    let out = git(repo, &["log", "--format=%h%x09%s", &format!("{base}..{head}")])?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect())
}
