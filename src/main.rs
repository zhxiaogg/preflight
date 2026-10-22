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
        /// Give up after this many seconds (exit code 2)
        #[arg(long)]
        timeout: Option<u64>,
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
    /// Mark a comment thread resolved
    Resolve { comment_id: i64 },
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
    git::rev_parse(&root, base).with_context(|| format!("unknown base '{base}'"))?;
    git::rev_parse(&root, head).with_context(|| format!("unknown head '{head}'"))?;
    db::create_pr(&db::open()?, &root, base, head, title, desc)
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn load_pr(conn: &Connection, pr_id: i64) -> Result<db::Pr> {
    db::get_pr(conn, pr_id)?.with_context(|| format!("PR #{pr_id} not found"))
}

fn set_state(pr_id: i64, state: &str) -> Result<()> {
    let conn = db::open()?;
    load_pr(&conn, pr_id)?;
    db::set_pr_state(&conn, pr_id, state)?;
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
            (Some(p), Some(l)) => format!("{p}:{l} ({} side)", r.side.as_deref().unwrap_or("new")),
            (Some(p), None) => p.clone(),
            _ => "general".into(),
        };
        println!("#{} [{state}] {loc} — {}", r.id, r.author);
        if let Some(t) = &r.line_text {
            println!("    > {t}");
        }
        for l in r.body.lines() {
            println!("    {l}");
        }
        for c in all.iter().filter(|c| c.parent_id == Some(r.id)) {
            println!("    ↳ #{} {}: {}", c.id, c.author, c.body.replace('\n', "\n      "));
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
        Cmd::Wait { pr_id, timeout, json } => {
            let conn = db::open()?;
            load_pr(&conn, pr_id)?;
            let seen = db::list_reviews(&conn, pr_id)?.last().map_or(0, |r| r.id);
            let deadline = timeout.map(|t| Instant::now() + Duration::from_secs(t));
            eprintln!("Waiting for a review on {} …", pr_url(pr_id));
            loop {
                if db::list_reviews(&conn, pr_id)?.last().is_some_and(|r| r.id > seen) {
                    return print_feedback(&conn, pr_id, true, json);
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    eprintln!("Timed out waiting for a review.");
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_secs(1));
            }
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
                line_text: None,
            };
            let c = db::add_comment(&conn, parent.pr_id, &n)?;
            println!("Replied with comment #{}", c.id);
            Ok(())
        }
        Cmd::Resolve { comment_id } => {
            let conn = db::open()?;
            db::get_comment(&conn, comment_id)?.context("comment not found")?;
            db::set_resolved(&conn, comment_id, true)?;
            println!("Resolved #{comment_id}");
            Ok(())
        }
        Cmd::Close { pr_id } => set_state(pr_id, "closed"),
        Cmd::Reopen { pr_id } => set_state(pr_id, "open"),
    }
}
