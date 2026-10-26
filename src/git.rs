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

/// Git's well-known empty tree, used as the "parent" of a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Combined changes of the commits `from` through `to` (inclusive), like picking a commit range
/// on GitHub; `from` == `to` gives a single commit's diff.
pub fn diff_range(repo: &str, from: &str, to: &str) -> Result<String> {
    let parent = rev_parse(repo, &format!("{from}^")).unwrap_or_else(|_| EMPTY_TREE.to_string());
    git(repo, &["diff", "--no-color", "--no-ext-diff", "-M", &parent, to])
}

/// Contents of `path` at `rev` (used to expand unchanged context around hunks).
pub fn show(repo: &str, rev: &str, path: &str) -> Result<String> {
    git(repo, &["show", &format!("{rev}:{path}")])
}

/// Local and remote-tracking branches, for picking a PR's base.
pub fn branches(repo: &str) -> Result<Vec<String>> {
    let out = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes"])?;
    Ok(out
        .lines()
        .filter(|b| !b.ends_with("/HEAD") && !b.is_empty())
        .map(String::from)
        .collect())
}

pub struct Commit {
    /// Full object id.
    pub oid: String,
    pub short: String,
    pub subject: String,
}

/// Commits on `head` not on `base`, newest first.
pub fn commits(repo: &str, base: &str, head: &str) -> Result<Vec<Commit>> {
    let out = git(repo, &["log", "--format=%H%x09%h%x09%s", &format!("{base}..{head}")])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            Some(Commit {
                oid: parts.next()?.to_string(),
                short: parts.next()?.to_string(),
                subject: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}
