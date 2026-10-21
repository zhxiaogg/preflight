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
    pub created_at: i64,
}

#[derive(Serialize, Clone)]
pub struct Comment {
    pub id: i64,
    pub pr_id: i64,
    pub parent_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub side: Option<String>,
    pub line_text: Option<String>,
    pub resolved: bool,
    pub created_at: i64,
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
    /// Snapshot of the commented line so agents get context even if the diff moves.
    pub line_text: Option<String>,
}

fn default_author() -> String {
    "user".into()
}

pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("LPR_DB") {
        return p.into();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local-pr").join("lpr.db")
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
           created_at INTEGER NOT NULL);",
    )?;
    Ok(conn)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

const PR_COLS: &str = "id, repo, base, head, title, description, created_at";
const COMMENT_COLS: &str =
    "id, pr_id, parent_id, author, body, path, line, side, line_text, resolved, created_at";

fn pr_row(r: &Row) -> rusqlite::Result<Pr> {
    Ok(Pr {
        id: r.get(0)?,
        repo: r.get(1)?,
        base: r.get(2)?,
        head: r.get(3)?,
        title: r.get(4)?,
        description: r.get(5)?,
        created_at: r.get(6)?,
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
        line_text: r.get(8)?,
        resolved: r.get(9)?,
        created_at: r.get(10)?,
    })
}

pub fn create_pr(c: &Connection, repo: &str, base: &str, head: &str, title: &str, desc: &str) -> Result<Pr> {
    c.execute(
        "INSERT INTO prs (repo, base, head, title, description, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![repo, base, head, title, desc, now()],
    )?;
    get_pr(c, c.last_insert_rowid())?.context("pr vanished after insert")
}

pub fn get_pr(c: &Connection, id: i64) -> Result<Option<Pr>> {
    Ok(c.query_row(&format!("SELECT {PR_COLS} FROM prs WHERE id = ?1"), [id], pr_row)
        .optional()?)
}

pub fn list_prs(c: &Connection) -> Result<Vec<Pr>> {
    let mut st = c.prepare(&format!("SELECT {PR_COLS} FROM prs ORDER BY id DESC"))?;
    let rows = st.query_map([], pr_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn list_comments(c: &Connection, pr_id: i64) -> Result<Vec<Comment>> {
    let mut st = c.prepare(&format!("SELECT {COMMENT_COLS} FROM comments WHERE pr_id = ?1 ORDER BY id"))?;
    let rows = st.query_map([pr_id], comment_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn get_comment(c: &Connection, id: i64) -> Result<Option<Comment>> {
    Ok(c.query_row(&format!("SELECT {COMMENT_COLS} FROM comments WHERE id = ?1"), [id], comment_row)
        .optional()?)
}

pub fn add_comment(c: &Connection, pr_id: i64, n: &NewComment) -> Result<Comment> {
    // Replies inherit the anchor of the thread root.
    let (path, line, side, line_text) = match n.parent_id {
        Some(pid) => {
            let parent = get_comment(c, pid)?.context("parent comment not found")?;
            anyhow::ensure!(parent.pr_id == pr_id, "parent comment belongs to another PR");
            anyhow::ensure!(parent.parent_id.is_none(), "can only reply to a thread root");
            (parent.path, parent.line, parent.side, parent.line_text)
        }
        None => (n.path.clone(), n.line, n.side.clone(), n.line_text.clone()),
    };
    c.execute(
        "INSERT INTO comments (pr_id, parent_id, author, body, path, line, side, line_text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![pr_id, n.parent_id, n.author, n.body, path, line, side, line_text, now()],
    )?;
    get_comment(c, c.last_insert_rowid())?.context("comment vanished after insert")
}

pub fn set_resolved(c: &Connection, id: i64, resolved: bool) -> Result<()> {
    c.execute("UPDATE comments SET resolved = ?1 WHERE id = ?2", params![resolved, id])?;
    Ok(())
}
