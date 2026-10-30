use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone)]
pub struct Pr {
    pub id: i64,
    pub repo: String,
    pub base: String,
    pub head: String,
    pub title: String,
    pub description: String,
    /// "open" or "closed".
    pub state: String,
    /// Last commits seen for base/head, so the PR stays viewable after a branch is deleted.
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub created_at: i64,
}

#[derive(Serialize, Clone)]
pub struct Review {
    pub id: i64,
    pub pr_id: i64,
    pub author: String,
    /// "approved", "changes_requested" or "commented".
    pub state: String,
    pub body: String,
    pub created_at: i64,
}

/// One change to a PR, so agents can catch up on everything (including edits and deletions)
/// with `preflight pr events --since <id>`.
#[derive(Serialize, Clone)]
pub struct Event {
    pub id: i64,
    pub pr_id: i64,
    /// comment_added, comment_edited, comment_deleted, comment_resolved, comment_unresolved,
    /// review_submitted, pr_closed or pr_reopened.
    pub kind: String,
    /// "user" for the web UI, the comment author for comments, "cli" for other CLI actions.
    pub actor: String,
    pub comment_id: Option<i64>,
    /// Snapshot of what changed (the comment, its old body, deleted replies, the review...).
    pub data: serde_json::Value,
    pub created_at: i64,
}

pub const REVIEW_STATES: [&str; 3] = ["approved", "changes_requested", "commented"];

#[derive(Serialize, Deserialize, Clone)]
pub struct Comment {
    pub id: i64,
    pub pr_id: i64,
    pub parent_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub side: Option<String>,
    /// First line of a multi-line comment (`line` is the last); None for single-line comments.
    pub start_line: Option<i64>,
    pub start_side: Option<String>,
    pub line_text: Option<String>,
    pub resolved: bool,
    pub created_at: i64,
    /// Set when the body was edited.
    pub updated_at: Option<i64>,
    /// Part of the reviewer's review in progress: only the web UI sees it until the review is submitted.
    pub pending: bool,
    /// The review that published this comment, if it was part of one.
    pub review_id: Option<i64>,
}

#[derive(Deserialize)]
pub struct NewComment {
    pub parent_id: Option<i64>,
    #[serde(default = "default_author")]
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    /// "old" or "new": which side of the diff the line number refers to.
    pub side: Option<String>,
    pub start_line: Option<i64>,
    pub start_side: Option<String>,
    /// Snapshot of the commented line(s) so agents get context even if the diff moves.
    pub line_text: Option<String>,
    /// Hold the comment back until the reviewer submits their review.
    #[serde(default)]
    pub pending: bool,
}

fn default_author() -> String {
    "user".into()
}

pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("PREFLIGHT_DB") {
        return p.into();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".preflight").join("preflight.db")
}

pub fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS prs (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           repo TEXT NOT NULL, base TEXT NOT NULL, head TEXT NOT NULL,
           title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS comments (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           pr_id INTEGER NOT NULL REFERENCES prs(id),
           parent_id INTEGER REFERENCES comments(id),
           author TEXT NOT NULL, body TEXT NOT NULL,
           path TEXT, line INTEGER, side TEXT, line_text TEXT,
           resolved INTEGER NOT NULL DEFAULT 0,
           created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS events (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           pr_id INTEGER NOT NULL REFERENCES prs(id),
           kind TEXT NOT NULL, actor TEXT NOT NULL, comment_id INTEGER,
           data TEXT NOT NULL DEFAULT '{}',
           created_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS reviews (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           pr_id INTEGER NOT NULL REFERENCES prs(id),
           author TEXT NOT NULL, state TEXT NOT NULL, body TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL);",
    )?;
    // Columns added after the first release.
    for (table, column, ddl) in [
        ("prs", "state", "ALTER TABLE prs ADD COLUMN state TEXT NOT NULL DEFAULT 'open'"),
        ("comments", "start_line", "ALTER TABLE comments ADD COLUMN start_line INTEGER"),
        ("comments", "start_side", "ALTER TABLE comments ADD COLUMN start_side TEXT"),
        ("prs", "base_sha", "ALTER TABLE prs ADD COLUMN base_sha TEXT"),
        ("prs", "head_sha", "ALTER TABLE prs ADD COLUMN head_sha TEXT"),
        ("comments", "updated_at", "ALTER TABLE comments ADD COLUMN updated_at INTEGER"),
        ("comments", "pending", "ALTER TABLE comments ADD COLUMN pending INTEGER NOT NULL DEFAULT 0"),
        ("comments", "review_id", "ALTER TABLE comments ADD COLUMN review_id INTEGER REFERENCES reviews(id)"),
    ] {
        let exists = conn
            .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
            .exists([column])?;
        if !exists {
            conn.execute_batch(ddl)?;
        }
    }
    // v1: PRs opened inside a linked worktree point at its main repo, so they survive the worktree.
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        for pr in list_prs(&conn)? {
            match crate::git::main_repo(&pr.repo) {
                Ok(root) if root != pr.repo => {
                    conn.execute("UPDATE prs SET repo = ?1 WHERE id = ?2", params![root, pr.id])?;
                }
                _ => {}
            }
        }
        conn.execute_batch("PRAGMA user_version = 1")?;
    }
    Ok(conn)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

const PR_COLS: &str = "id, repo, base, head, title, description, state, base_sha, head_sha, created_at";
const REVIEW_COLS: &str = "id, pr_id, author, state, body, created_at";
const COMMENT_COLS: &str = "id, pr_id, parent_id, author, body, path, line, side, start_line, start_side, \
                            line_text, resolved, created_at, updated_at, pending, review_id";

fn pr_row(r: &Row) -> rusqlite::Result<Pr> {
    Ok(Pr {
        id: r.get(0)?,
        repo: r.get(1)?,
        base: r.get(2)?,
        head: r.get(3)?,
        title: r.get(4)?,
        description: r.get(5)?,
        state: r.get(6)?,
        base_sha: r.get(7)?,
        head_sha: r.get(8)?,
        created_at: r.get(9)?,
    })
}

fn review_row(r: &Row) -> rusqlite::Result<Review> {
    Ok(Review {
        id: r.get(0)?,
        pr_id: r.get(1)?,
        author: r.get(2)?,
        state: r.get(3)?,
        body: r.get(4)?,
        created_at: r.get(5)?,
    })
}

fn comment_row(r: &Row) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: r.get(0)?,
        pr_id: r.get(1)?,
        parent_id: r.get(2)?,
        author: r.get(3)?,
        body: r.get(4)?,
        path: r.get(5)?,
        line: r.get(6)?,
        side: r.get(7)?,
        start_line: r.get(8)?,
        start_side: r.get(9)?,
        line_text: r.get(10)?,
        resolved: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
        pending: r.get(14)?,
        review_id: r.get(15)?,
    })
}

pub fn create_pr(c: &Connection, repo: &str, base: &str, head: &str, title: &str, desc: &str) -> Result<Pr> {
    c.execute(
        "INSERT INTO prs (repo, base, head, title, description, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![repo, base, head, title, desc, now()],
    )?;
    get_pr(c, c.last_insert_rowid())?.context("pr vanished after insert")
}

pub fn set_shas(c: &Connection, id: i64, base_sha: &str, head_sha: &str) -> Result<()> {
    c.execute(
        "UPDATE prs SET base_sha = ?1, head_sha = ?2 WHERE id = ?3",
        params![base_sha, head_sha, id],
    )?;
    Ok(())
}

pub fn get_pr(c: &Connection, id: i64) -> Result<Option<Pr>> {
    Ok(c.query_row(&format!("SELECT {PR_COLS} FROM prs WHERE id = ?1"), [id], pr_row)
        .optional()?)
}

/// Updates the given fields and logs a `pr_edited` event with old/new values of those that changed.
pub fn update_pr(
    c: &Connection,
    id: i64,
    title: Option<&str>,
    description: Option<&str>,
    base: Option<&str>,
    repo: Option<&str>,
    actor: &str,
) -> Result<Pr> {
    let old = get_pr(c, id)?.context("pr not found")?;
    let mut changes = serde_json::Map::new();
    for (field, old_value, new_value) in [
        ("title", &old.title, title),
        ("description", &old.description, description),
        ("base", &old.base, base),
        ("repo", &old.repo, repo),
    ] {
        let Some(new_value) = new_value.filter(|v| *v != old_value.as_str()) else { continue };
        // `field` comes from the fixed list above, so it is safe to splice into the SQL.
        c.execute(&format!("UPDATE prs SET {field} = ?1 WHERE id = ?2"), params![new_value, id])?;
        changes.insert(field.into(), serde_json::json!({ "old": old_value, "new": new_value }));
    }
    if !changes.is_empty() {
        log_event(c, id, "pr_edited", actor, None, serde_json::Value::Object(changes))?;
    }
    get_pr(c, id)?.context("pr vanished after update")
}

pub fn set_pr_state(c: &Connection, id: i64, state: &str, actor: &str) -> Result<()> {
    anyhow::ensure!(state == "open" || state == "closed", "state must be 'open' or 'closed'");
    c.execute("UPDATE prs SET state = ?1 WHERE id = ?2", params![state, id])?;
    let kind = if state == "closed" { "pr_closed" } else { "pr_reopened" };
    log_event(c, id, kind, actor, None, serde_json::json!({}))
}

fn log_event(c: &Connection, pr_id: i64, kind: &str, actor: &str, comment_id: Option<i64>, data: serde_json::Value) -> Result<()> {
    c.execute(
        "INSERT INTO events (pr_id, kind, actor, comment_id, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![pr_id, kind, actor, comment_id, data.to_string(), now()],
    )?;
    Ok(())
}

pub fn list_events(c: &Connection, pr_id: i64, since: i64) -> Result<Vec<Event>> {
    let mut st = c.prepare(
        "SELECT id, pr_id, kind, actor, comment_id, data, created_at FROM events
         WHERE pr_id = ?1 AND id > ?2 ORDER BY id",
    )?;
    let rows = st
        .query_map(params![pr_id, since], |r| {
            let data: String = r.get(5)?;
            Ok(Event {
                id: r.get(0)?,
                pr_id: r.get(1)?,
                kind: r.get(2)?,
                actor: r.get(3)?,
                comment_id: r.get(4)?,
                data: serde_json::from_str(&data).unwrap_or_default(),
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn last_event_id(c: &Connection, pr_id: i64) -> Result<i64> {
    Ok(c.query_row("SELECT COALESCE(MAX(id), 0) FROM events WHERE pr_id = ?1", [pr_id], |r| r.get(0))?)
}

pub fn list_prs(c: &Connection) -> Result<Vec<Pr>> {
    let mut st = c.prepare(&format!("SELECT {PR_COLS} FROM prs ORDER BY id DESC"))?;
    let rows = st.query_map([], pr_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// The PR's comments, without the reviewer's pending ones (what agents see).
pub fn list_comments(c: &Connection, pr_id: i64) -> Result<Vec<Comment>> {
    Ok(list_all_comments(c, pr_id)?.into_iter().filter(|c| !c.pending).collect())
}

/// The PR's comments including pending ones, for the reviewer's web UI.
pub fn list_all_comments(c: &Connection, pr_id: i64) -> Result<Vec<Comment>> {
    let mut st = c.prepare(&format!("SELECT {COMMENT_COLS} FROM comments WHERE pr_id = ?1 ORDER BY id"))?;
    let rows = st.query_map([pr_id], comment_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn get_comment(c: &Connection, id: i64) -> Result<Option<Comment>> {
    Ok(c.query_row(&format!("SELECT {COMMENT_COLS} FROM comments WHERE id = ?1"), [id], comment_row)
        .optional()?)
}

pub fn add_comment(c: &Connection, pr_id: i64, n: &NewComment) -> Result<Comment> {
    // Replies inherit the anchor of the thread root; replies to a pending thread are pending too.
    let mut pending = n.pending;
    let (path, line, side, start_line, start_side, line_text) = match n.parent_id {
        Some(pid) => {
            let parent = get_comment(c, pid)?.context("parent comment not found")?;
            anyhow::ensure!(parent.pr_id == pr_id, "parent comment belongs to another PR");
            anyhow::ensure!(parent.parent_id.is_none(), "can only reply to a thread root");
            pending |= parent.pending;
            (parent.path, parent.line, parent.side, parent.start_line, parent.start_side, parent.line_text)
        }
        None => (
            n.path.clone(),
            n.line,
            n.side.clone(),
            n.start_line,
            n.start_side.clone(),
            n.line_text.clone(),
        ),
    };
    c.execute(
        "INSERT INTO comments
           (pr_id, parent_id, author, body, path, line, side, start_line, start_side, line_text, created_at, pending)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![pr_id, n.parent_id, n.author, n.body, path, line, side, start_line, start_side, line_text, now(), pending],
    )?;
    let comment = get_comment(c, c.last_insert_rowid())?.context("comment vanished after insert")?;
    // Pending comments are logged when their review is submitted.
    if !pending {
        log_event(c, pr_id, "comment_added", &comment.author, Some(comment.id), serde_json::json!({ "comment": comment }))?;
    }
    Ok(comment)
}

pub fn edit_comment(c: &Connection, id: i64, body: &str) -> Result<()> {
    let old = get_comment(c, id)?.context("comment not found")?;
    c.execute("UPDATE comments SET body = ?1, updated_at = ?2 WHERE id = ?3", params![body, now(), id])?;
    let comment = get_comment(c, id)?.context("comment vanished after update")?;
    if old.pending {
        return Ok(());
    }
    let data = serde_json::json!({ "old_body": old.body, "comment": comment });
    log_event(c, old.pr_id, "comment_edited", &old.author, Some(id), data)
}

/// Deletes a comment; deleting a thread root deletes its replies too.
pub fn delete_comment(c: &Connection, id: i64, actor: &str) -> Result<()> {
    let comment = get_comment(c, id)?.context("comment not found")?;
    // Pending replies were never seen by agents, so they aren't reported as deleted.
    let replies: Vec<_> = list_comments(c, comment.pr_id)?
        .into_iter()
        .filter(|r| r.parent_id == Some(id))
        .collect();
    c.execute("DELETE FROM comments WHERE parent_id = ?1", [id])?;
    c.execute("DELETE FROM comments WHERE id = ?1", [id])?;
    if comment.pending {
        return Ok(());
    }
    let data = serde_json::json!({ "comment": comment, "deleted_replies": replies });
    log_event(c, comment.pr_id, "comment_deleted", actor, Some(id), data)
}

pub fn set_resolved(c: &Connection, id: i64, resolved: bool, actor: &str) -> Result<()> {
    let comment = get_comment(c, id)?.context("comment not found")?;
    anyhow::ensure!(!comment.pending, "submit the review before resolving its comments");
    c.execute("UPDATE comments SET resolved = ?1 WHERE id = ?2", params![resolved, id])?;
    let kind = if resolved { "comment_resolved" } else { "comment_unresolved" };
    log_event(c, comment.pr_id, kind, actor, Some(id), serde_json::json!({ "comment": comment }))
}

/// Submits a review, publishing the author's pending comments with it in one transaction, so an
/// agent waiting on the review sees all of its comments at once.
pub fn add_review(c: &Connection, pr_id: i64, author: &str, state: &str, body: &str) -> Result<Review> {
    anyhow::ensure!(REVIEW_STATES.contains(&state), "state must be one of {REVIEW_STATES:?}");
    let tx = c.unchecked_transaction()?;
    let pending: Vec<_> = list_all_comments(&tx, pr_id)?
        .into_iter()
        .filter(|c| c.pending && c.author == author)
        .collect();
    anyhow::ensure!(
        !body.trim().is_empty() || !pending.is_empty() || state == "approved",
        "add a summary or some comments before submitting the review"
    );
    let at = now();
    tx.execute(
        "INSERT INTO reviews (pr_id, author, state, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![pr_id, author, state, body, at],
    )?;
    let id = tx.last_insert_rowid();
    for p in &pending {
        tx.execute(
            "UPDATE comments SET pending = 0, created_at = ?1, review_id = ?2 WHERE id = ?3",
            params![at, id, p.id],
        )?;
        let comment = get_comment(&tx, p.id)?.context("comment vanished")?;
        log_event(&tx, pr_id, "comment_added", author, Some(p.id), serde_json::json!({ "comment": comment }))?;
    }
    let review = tx.query_row(&format!("SELECT {REVIEW_COLS} FROM reviews WHERE id = ?1"), [id], review_row)?;
    let comment_ids: Vec<_> = pending.iter().map(|p| p.id).collect();
    let data = serde_json::json!({ "review": review, "comment_ids": comment_ids });
    log_event(&tx, pr_id, "review_submitted", author, None, data)?;
    tx.commit()?;
    Ok(review)
}

pub fn list_reviews(c: &Connection, pr_id: i64) -> Result<Vec<Review>> {
    let mut st = c.prepare(&format!("SELECT {REVIEW_COLS} FROM reviews WHERE pr_id = ?1 ORDER BY id"))?;
    let rows = st.query_map([pr_id], review_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
