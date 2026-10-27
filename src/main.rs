mod db;
mod git;
mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::time::{Duration, Instant};

/// Preflight PRs: review an agent's branch in a GitHub-like web UI before pushing.
#[derive(Parser)]
#[command(name = "preflight", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the review web server (http://localhost:<port>)
    Serve {
        #[arg(long, env = "PREFLIGHT_PORT", default_value_t = 7777)]
        port: u16,
    },
    /// Open a preflight PR comparing <head> against <base>
    Create {
        /// Path to the git repo (any directory inside it)
        #[arg(long, default_value = ".")]
        repo: String,
        /// Target branch (what the PR merges into)
        #[arg(long)]
        base: String,
        /// Source branch (the changes under review)
        #[arg(long)]
        head: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        json: bool,
    },
    /// List open preflight PRs
    List {
        /// Include closed PRs
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show the latest review and the comments on a PR
    Comments {
        pr_id: i64,
        /// Only show threads that are not resolved
        #[arg(long)]
        unresolved: bool,
        #[arg(long)]
        json: bool,
    },
    /// Block until the reviewer submits a review, then print it with the unresolved comments
    Wait {
        pr_id: i64,
        /// Return on any reviewer activity (comment added/edited/deleted/resolved, review...)
        /// and print what changed
        #[arg(long)]
        any: bool,
        /// Give up after this many seconds (exit code 2)
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Show everything that changed on a PR, including edited and deleted comments
    Events {
        pr_id: i64,
        /// Only events after this id (the cursor printed by the previous call)
        #[arg(long, default_value_t = 0)]
        since: i64,
        #[arg(long)]
        json: bool,
    },
    /// Comment on a PR (as the agent): on the conversation, or on diff lines with --file/--line
    Comment {
        pr_id: i64,
        /// Comment text (markdown); "-" reads it from stdin
        body: String,
        /// File to comment on, as named in the diff
        #[arg(long, requires = "line")]
        file: Option<String>,
        /// Line to comment on; with --start-line, the last line of the range
        #[arg(long, requires = "file")]
        line: Option<i64>,
        /// First line of a multi-line comment
        #[arg(long, requires = "line")]
        start_line: Option<i64>,
        /// Which side the line numbers refer to: "new" (head) or "old" (deleted lines)
        #[arg(long, default_value = "new", value_parser = ["new", "old"])]
        side: String,
        #[arg(long, default_value = "agent")]
        author: String,
        #[arg(long)]
        json: bool,
    },
    /// Reply to a comment thread (as the agent)
    Reply {
        comment_id: i64,
        body: String,
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Change the text of a comment you wrote (as the agent)
    EditComment {
        comment_id: i64,
        /// New text (markdown); "-" reads it from stdin
        body: String,
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Delete a comment you wrote (as the agent); deleting a thread's first comment deletes its replies
    DeleteComment {
        comment_id: i64,
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Mark a comment thread resolved
    Resolve { comment_id: i64 },
    /// Change a PR's title, description or base branch
    Edit {
        pr_id: i64,
        #[arg(long)]
        title: Option<String>,
        /// New description (markdown); "-" reads it from stdin
        #[arg(long)]
        description: Option<String>,
        /// New target branch
        #[arg(long)]
        base: Option<String>,
    },
    /// Close a PR (e.g. after it was pushed and merged remotely)
    Close { pr_id: i64 },
    /// Reopen a closed PR
    Reopen { pr_id: i64 },
}

fn port() -> u16 {
    std::env::var("PREFLIGHT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7777)
}

fn pr_url(id: i64) -> String {
    format!("http://localhost:{}/pr/{id}", port())
}

pub fn create_pr(repo: &str, base: &str, head: &str, title: &str, desc: &str) -> Result<db::Pr> {
    let root = git::toplevel(repo).with_context(|| format!("{repo} is not a git repo"))?;
    let base_sha = git::rev_parse(&root, base).with_context(|| format!("unknown base '{base}'"))?;
    let head_sha = git::rev_parse(&root, head).with_context(|| format!("unknown head '{head}'"))?;
    let conn = db::open()?;
    let pr = db::create_pr(&conn, &root, base, head, title, desc)?;
    db::set_shas(&conn, pr.id, &base_sha, &head_sha)?;
    load_pr(&conn, pr.id)
}

/// The diff text of lines `start..=end` of `path` (same side), which must be in one hunk of the
/// PR's diff: like GitHub, only lines shown in the diff can be commented on.
fn diff_snippet(pr: &db::Pr, path: &str, side: &str, start: i64, end: i64) -> Result<String> {
    anyhow::ensure!(start <= end, "--start-line must not be after --line");
    let refs = server::resolve(pr)?;
    let diff = git::diff(&pr.repo, &refs.base, &refs.head)?;
    let rows = git::diff_rows(&diff, path).with_context(|| format!("{path} is not changed in PR #{}", pr.id))?;
    let no = |r: &git::DiffRow| if side == "old" { r.old } else { r.new };
    let find = |n: i64| rows.iter().position(|r| no(r) == Some(n));
    let shown = || {
        let mut ranges: Vec<String> = Vec::new();
        for hunk in 1..=rows.last().map_or(0, |r| r.hunk) {
            let nos: Vec<i64> = rows.iter().filter(|r| r.hunk == hunk).filter_map(no).collect();
            if let (Some(a), Some(b)) = (nos.first(), nos.last()) {
                ranges.push(format!("{a}-{b}"));
            }
        }
        format!("lines shown on the {side} side of {path}: {}", ranges.join(", "))
    };
    let (Some(lo), Some(hi)) = (find(start), find(end)) else {
        anyhow::bail!("line {} is not in the diff ({})", if find(start).is_none() { start } else { end }, shown());
    };
    anyhow::ensure!(rows[lo].hunk == rows[hi].hunk, "lines {start}-{end} span several hunks ({})", shown());
    Ok(rows[lo..=hi].iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("\n"))
}

/// Loads a comment, refusing to change someone else's (e.g. the reviewer's).
fn own_comment(conn: &Connection, id: i64, author: &str) -> Result<db::Comment> {
    let c = db::get_comment(conn, id)?.with_context(|| format!("comment #{id} not found"))?;
    anyhow::ensure!(c.author == author, "comment #{id} was written by {}, not {author}", c.author);
    Ok(c)
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn load_pr(conn: &Connection, pr_id: i64) -> Result<db::Pr> {
    db::get_pr(conn, pr_id)?.with_context(|| format!("PR #{pr_id} not found"))
}

/// Records the branches' current commits (where they still exist), e.g. right before closing so
/// the closed PR keeps showing what was reviewed.
pub fn snapshot_shas(conn: &Connection, pr: &db::Pr) -> Result<()> {
    let base = git::rev_parse(&pr.repo, &pr.base).ok().or_else(|| pr.base_sha.clone());
    let head = git::rev_parse(&pr.repo, &pr.head).ok().or_else(|| pr.head_sha.clone());
    if let (Some(base), Some(head)) = (base, head) {
        db::set_shas(conn, pr.id, &base, &head)?;
    }
    Ok(())
}

/// Validates and applies an edit of a PR's title, description or base, from the CLI or the web UI.
pub fn edit_pr(
    conn: &Connection,
    pr: &db::Pr,
    title: Option<&str>,
    description: Option<&str>,
    base: Option<&str>,
    actor: &str,
) -> Result<db::Pr> {
    let title = title.map(str::trim);
    anyhow::ensure!(title != Some(""), "title can't be empty");
    if let Some(base) = base.filter(|b| *b != pr.base) {
        // A closed PR is frozen at its final commits; changing its base would make that misleading.
        anyhow::ensure!(pr.state == "open", "reopen the PR before changing its base");
        anyhow::ensure!(base != pr.head, "base can't be the same as the head branch");
        git::rev_parse(&pr.repo, base).with_context(|| format!("unknown base '{base}'"))?;
    }
    db::update_pr(conn, pr.id, title, description, base, actor)
}

fn set_state(pr_id: i64, state: &str) -> Result<()> {
    let conn = db::open()?;
    let pr = load_pr(&conn, pr_id)?;
    snapshot_shas(&conn, &pr)?;
    db::set_pr_state(&conn, pr_id, state, "cli")?;
    println!("PR #{pr_id} is now {state}");
    Ok(())
}

fn review_label(state: &str) -> &str {
    match state {
        "approved" => "APPROVED",
        "changes_requested" => "CHANGES REQUESTED",
        _ => "COMMENTED",
    }
}

fn comment_loc(c: &db::Comment) -> String {
    match (&c.path, c.line) {
        (Some(p), Some(l)) => match c.start_line {
            Some(s) if s != l => format!("{p}:{s}-{l}"),
            _ => format!("{p}:{l}"),
        },
        (Some(p), None) => p.clone(),
        _ => "the conversation".into(),
    }
}

fn indent(text: &str, prefix: &str) {
    for l in text.lines() {
        println!("{prefix}{l}");
    }
}

/// Prints events with a cursor for the next call, as text or as one JSON object.
fn print_events(pr_id: i64, events: &[db::Event], cursor: i64, json: bool) -> Result<()> {
    if json {
        return print_json(&serde_json::json!({ "events": events, "cursor": cursor }));
    }
    if events.is_empty() {
        println!("No new activity.");
    }
    for e in events {
        let comment: Option<db::Comment> = serde_json::from_value(e.data["comment"].clone()).ok();
        let what = comment.as_ref().map_or(String::new(), |c| format!(" #{} on {}", c.id, comment_loc(c)));
        match e.kind.as_str() {
            "comment_added" => {
                let c = comment.as_ref().unwrap();
                let kind = if c.parent_id.is_some() { "replied" } else { "commented" };
                println!("[{}] {} {kind}{what}", e.id, e.actor);
                indent(&c.body, "    ");
            }
            "comment_edited" => {
                println!("[{}] {} edited comment{what}", e.id, e.actor);
                indent(e.data["old_body"].as_str().unwrap_or(""), "    was: ");
                indent(&comment.as_ref().unwrap().body, "    now: ");
            }
            "comment_deleted" => {
                let replies: Vec<db::Comment> =
                    serde_json::from_value(e.data["deleted_replies"].clone()).unwrap_or_default();
                let suffix = match replies.len() {
                    0 => String::new(),
                    1 => " and its reply".into(),
                    n => format!(" and its {n} replies"),
                };
                println!("[{}] {} deleted comment{what}{suffix}", e.id, e.actor);
                indent(&comment.as_ref().unwrap().body, "    was: ");
                for r in &replies {
                    indent(&r.body, &format!("    reply #{} {}: ", r.id, r.author));
                }
            }
            "comment_resolved" => println!("[{}] {} resolved thread{what}", e.id, e.actor),
            "comment_unresolved" => println!("[{}] {} unresolved thread{what}", e.id, e.actor),
            "review_submitted" => {
                let state = e.data["review"]["state"].as_str().unwrap_or("");
                println!("[{}] {} submitted a review: {}", e.id, e.actor, review_label(state));
                indent(e.data["review"]["body"].as_str().unwrap_or(""), "    ");
            }
            "pr_edited" => {
                println!("[{}] {} edited the PR", e.id, e.actor);
                for field in ["title", "base"] {
                    if let Some(ch) = e.data.get(field) {
                        println!("    {field}: {} → {}", ch["old"].as_str().unwrap_or(""), ch["new"].as_str().unwrap_or(""));
                    }
                }
                if let Some(ch) = e.data.get("description") {
                    println!("    description changed to:");
                    indent(ch["new"].as_str().unwrap_or(""), "      ");
                }
            }
            "pr_closed" => println!("[{}] {} closed the PR", e.id, e.actor),
            "pr_reopened" => println!("[{}] {} reopened the PR", e.id, e.actor),
            other => println!("[{}] {} {other}", e.id, e.actor),
        }
    }
    println!("\nNext: preflight events {pr_id} --since {cursor}");
    Ok(())
}

/// Prints the latest review and the comment threads, as text or as one JSON object.
fn print_feedback(conn: &Connection, pr_id: i64, unresolved: bool, json: bool) -> Result<()> {
    let pr = load_pr(conn, pr_id)?;
    let latest_review = db::list_reviews(conn, pr_id)?.pop();
    let all = db::list_comments(conn, pr_id)?;
    let roots: Vec<_> = all
        .iter()
        .filter(|c| c.parent_id.is_none() && !(unresolved && c.resolved))
        .collect();
    if json {
        let threads: Vec<_> = roots
            .iter()
            .map(|r| {
                let replies: Vec<_> = all.iter().filter(|c| c.parent_id == Some(r.id)).collect();
                serde_json::json!({ "comment": r, "replies": replies })
            })
            .collect();
        return print_json(&serde_json::json!({
            "state": pr.state,
            "latest_review": latest_review,
            "threads": threads,
            // Pass to `preflight events --since` to get only later changes.
            "event_cursor": db::last_event_id(conn, pr_id)?,
        }));
    }
    if pr.state != "open" {
        println!("PR #{pr_id} is {}.\n", pr.state);
    }
    if let Some(r) = &latest_review {
        println!("Latest review #{} [{}] — {}", r.id, review_label(&r.state), r.author);
        for l in r.body.lines() {
            println!("    {l}");
        }
        println!();
    }
    if roots.is_empty() {
        println!("No comments.");
    }
    for r in roots {
        let state = if r.resolved { "resolved" } else { "unresolved" };
        let loc = match (&r.path, r.line) {
            (Some(p), Some(l)) => {
                let lines = match r.start_line {
                    Some(s) if s != l => format!("{s}-{l}"),
                    _ => l.to_string(),
                };
                format!("{p}:{lines} ({} side)", r.side.as_deref().unwrap_or("new"))
            }
            (Some(p), None) => p.clone(),
            _ => "general".into(),
        };
        let edited = if r.updated_at.is_some() { " (edited)" } else { "" };
        println!("#{} [{state}] {loc} — {}{edited}", r.id, r.author);
        if let Some(t) = &r.line_text {
            for l in t.lines() {
                println!("    > {l}");
            }
        }
        for l in r.body.lines() {
            println!("    {l}");
        }
        for c in all.iter().filter(|c| c.parent_id == Some(r.id)) {
            let edited = if c.updated_at.is_some() { " (edited)" } else { "" };
            println!("    ↳ #{} {}{edited}: {}", c.id, c.author, c.body.replace('\n', "\n      "));
        }
        println!();
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Serve { port } => tokio::runtime::Runtime::new()?.block_on(server::serve(port)),
        Cmd::Create { repo, base, head, title, description, json } => {
            let pr = create_pr(&repo, &base, &head, &title, &description)?;
            if json {
                return print_json(&serde_json::json!({ "pr": pr, "url": pr_url(pr.id) }));
            }
            println!("Created preflight PR #{}: {}\n{}", pr.id, pr.title, pr_url(pr.id));
            Ok(())
        }
        Cmd::List { all, json } => {
            let prs: Vec<_> = db::list_prs(&db::open()?)?
                .into_iter()
                .filter(|p| all || p.state == "open")
                .collect();
            if json {
                return print_json(&prs);
            }
            for p in prs {
                println!("#{:<4} [{}] {}  ({} <- {})  {}", p.id, p.state, p.title, p.base, p.head, p.repo);
            }
            Ok(())
        }
        Cmd::Comments { pr_id, unresolved, json } => print_feedback(&db::open()?, pr_id, unresolved, json),
        Cmd::Events { pr_id, since, json } => {
            let conn = db::open()?;
            load_pr(&conn, pr_id)?;
            let events = db::list_events(&conn, pr_id, since)?;
            let cursor = events.last().map_or(since, |e| e.id);
            print_events(pr_id, &events, cursor, json)
        }
        Cmd::Wait { pr_id, any, timeout, json } => {
            let conn = db::open()?;
            load_pr(&conn, pr_id)?;
            let seen = db::list_reviews(&conn, pr_id)?.last().map_or(0, |r| r.id);
            let seen_event = db::last_event_id(&conn, pr_id)?;
            let deadline = timeout.map(|t| Instant::now() + Duration::from_secs(t));
            let what = if any { "reviewer activity" } else { "a review" };
            eprintln!("Waiting for {what} on {} …", pr_url(pr_id));
            loop {
                if any {
                    let events = db::list_events(&conn, pr_id, seen_event)?;
                    // Only the reviewer's actions in the web UI count; ignore the agent's own.
                    if events.iter().any(|e| e.actor == "user") {
                        return print_events(pr_id, &events, events.last().unwrap().id, json);
                    }
                } else if db::list_reviews(&conn, pr_id)?.last().is_some_and(|r| r.id > seen) {
                    return print_feedback(&conn, pr_id, true, json);
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    eprintln!("Timed out waiting for a review.");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        Cmd::Comment { pr_id, body, file, line, start_line, side, author, json } => {
            let conn = db::open()?;
            let pr = load_pr(&conn, pr_id)?;
            let body = if body == "-" { std::io::read_to_string(std::io::stdin())?.trim_end().to_string() } else { body };
            anyhow::ensure!(!body.trim().is_empty(), "comment body is empty");
            let mut n = db::NewComment {
                parent_id: None,
                author,
                body,
                path: None,
                line: None,
                side: None,
                start_line: None,
                start_side: None,
                line_text: None,
            };
            if let (Some(path), Some(line)) = (file, line) {
                n.line_text = Some(diff_snippet(&pr, &path, &side, start_line.unwrap_or(line), line)?);
                if start_line.is_some_and(|s| s != line) {
                    n.start_line = start_line;
                    n.start_side = Some(side.clone());
                }
                n.path = Some(path);
                n.line = Some(line);
                n.side = Some(side);
            }
            let c = db::add_comment(&conn, pr_id, &n)?;
            if json {
                return print_json(&c);
            }
            println!("Added comment #{} on {}\n{}", c.id, comment_loc(&c), pr_url(pr_id));
            Ok(())
        }
        Cmd::Reply { comment_id, body, author } => {
            let conn = db::open()?;
            let parent = db::get_comment(&conn, comment_id)?.context("comment not found")?;
            let root = parent.parent_id.unwrap_or(parent.id);
            let n = db::NewComment {
                parent_id: Some(root),
                author,
                body,
                path: None,
                line: None,
                side: None,
                start_line: None,
                start_side: None,
                line_text: None,
            };
            let c = db::add_comment(&conn, parent.pr_id, &n)?;
            println!("Replied with comment #{}", c.id);
            Ok(())
        }
        Cmd::EditComment { comment_id, body, author } => {
            let conn = db::open()?;
            own_comment(&conn, comment_id, &author)?;
            let body = if body == "-" { std::io::read_to_string(std::io::stdin())?.trim_end().to_string() } else { body };
            anyhow::ensure!(!body.trim().is_empty(), "comment body is empty");
            db::edit_comment(&conn, comment_id, &body)?;
            println!("Edited comment #{comment_id}");
            Ok(())
        }
        Cmd::DeleteComment { comment_id, author } => {
            let conn = db::open()?;
            own_comment(&conn, comment_id, &author)?;
            db::delete_comment(&conn, comment_id, &author)?;
            println!("Deleted comment #{comment_id}");
            Ok(())
        }
        Cmd::Resolve { comment_id } => {
            let conn = db::open()?;
            db::get_comment(&conn, comment_id)?.context("comment not found")?;
            db::set_resolved(&conn, comment_id, true, "agent")?;
            println!("Resolved #{comment_id}");
            Ok(())
        }
        Cmd::Edit { pr_id, title, description, base } => {
            let conn = db::open()?;
            let pr = load_pr(&conn, pr_id)?;
            let description = match description.as_deref() {
                Some("-") => Some(std::io::read_to_string(std::io::stdin())?),
                _ => description,
            };
            anyhow::ensure!(
                title.is_some() || description.is_some() || base.is_some(),
                "nothing to change: pass --title, --description or --base"
            );
            let pr = edit_pr(&conn, &pr, title.as_deref(), description.as_deref(), base.as_deref(), "cli")?;
            println!("Updated PR #{}: {}  ({} <- {})\n{}", pr.id, pr.title, pr.base, pr.head, pr_url(pr.id));
            Ok(())
        }
        Cmd::Close { pr_id } => set_state(pr_id, "closed"),
        Cmd::Reopen { pr_id } => set_state(pr_id, "open"),
    }
}
