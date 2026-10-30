use anyhow::{bail, Result};
use std::process::Command;

fn git(repo: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output()?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The main repo that `path` belongs to: for a linked worktree, the repo it was added from. Worktrees
/// share the main repo's branches and commits, so PRs stored with this path keep working after the
/// worktree is removed. Bare repos and submodules resolve to their git dir, where git works too.
pub fn main_repo(path: &str) -> Result<String> {
    let dir = git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let dir = dir.trim();
    Ok(dir.strip_suffix("/.git").unwrap_or(dir).to_string())
}

pub fn rev_parse(repo: &str, rev: &str) -> Result<String> {
    Ok(git(repo, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?.trim().to_string())
}

fn is_ancestor(repo: &str, ancestor: &str, rev: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", ancestor, rev]).is_ok()
}

/// The base commit to compare a PR against. Once `head` has been merged into the base branch, the
/// branch's tip would show an empty diff, so the last base the PR was seen with is kept instead.
pub fn pr_base(repo: &str, current: String, head: &str, last: Option<&str>) -> String {
    match last {
        Some(last) if last != current && is_ancestor(repo, head, &current) && !is_ancestor(repo, head, last) => {
            last.to_string()
        }
        _ => current,
    }
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

/// A line shown in a file's diff, i.e. a line a reviewer can comment on.
pub struct DiffRow {
    /// Index of the hunk the line belongs to.
    pub hunk: usize,
    /// Line number on the old side (context and deleted lines).
    pub old: Option<i64>,
    /// Line number on the new side (context and added lines).
    pub new: Option<i64>,
    pub text: String,
}

/// The rows of `path`'s diff in display order, or None when the diff doesn't touch `path`.
/// `path` is the new path, or the old one for a deleted file (as the web UI names files).
pub fn diff_rows(diff: &str, path: &str) -> Option<Vec<DiffRow>> {
    let mut files: Vec<(String, String, bool, Vec<DiffRow>)> = Vec::new();
    let (mut old, mut new, mut hunks) = (0, 0, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let (a, b) = rest.rsplit_once(" b/").unwrap_or((rest, rest));
            files.push((a.to_string(), b.to_string(), false, Vec::new()));
            hunks = 0;
            continue;
        }
        let Some((_, _, deleted, rows)) = files.last_mut() else { continue };
        if let Some(header) = line.strip_prefix("@@ -") {
            // "@@ -<old>[,n] +<new>[,n] @@"
            let num = |s: &str| s.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let mut parts = header.split_whitespace();
            old = parts.next().map_or(0, num);
            new = parts.next().map_or(0, |p| num(p.trim_start_matches('+')));
            hunks += 1;
        } else if hunks == 0 {
            *deleted |= line.starts_with("deleted file mode");
        } else if let Some(text) = line.strip_prefix('+') {
            rows.push(DiffRow { hunk: hunks, old: None, new: Some(new), text: text.into() });
            new += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            rows.push(DiffRow { hunk: hunks, old: Some(old), new: None, text: text.into() });
            old += 1;
        } else if let Some(text) = line.strip_prefix(' ') {
            rows.push(DiffRow { hunk: hunks, old: Some(old), new: Some(new), text: text.into() });
            old += 1;
            new += 1;
        }
    }
    files
        .into_iter()
        .find(|(a, b, deleted, _)| if *deleted { a == path } else { b == path })
        .map(|(_, _, _, rows)| rows)
}
