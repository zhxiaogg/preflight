use crate::{db, git};
use axum::{
    extract::{Path, Query},
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use rust_embed::Embed;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Embed)]
#[folder = "web/dist/"]
struct Assets;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

fn load_pr(id: i64) -> Result<db::Pr, ApiError> {
    db::get_pr(&db::open()?, id)?.ok_or_else(|| not_found("pr"))
}

async fn list_prs() -> ApiResult {
    Ok(Json(json!(db::list_prs(&db::open()?)?)))
}

#[derive(Deserialize)]
struct CreatePr {
    repo: String,
    base: String,
    head: String,
    title: String,
    #[serde(default)]
    description: String,
}

async fn create_pr(Json(b): Json<CreatePr>) -> ApiResult {
    Ok(Json(json!(crate::create_pr(&b.repo, &b.base, &b.head, &b.title, &b.description)?)))
}

/// Commits a PR currently points at.
pub struct Refs {
    pub base: String,
    pub head: String,
    pub base_missing: bool,
    pub head_missing: bool,
}

/// Resolves the PR's branches, falling back to the last seen commit when a branch was deleted
/// (like GitHub, a PR stays viewable after its branch is gone).
pub fn resolve(pr: &db::Pr) -> anyhow::Result<Refs> {
    anyhow::ensure!(
        std::path::Path::new(&pr.repo).exists(),
        "{} no longer exists (a removed worktree?). Point the PR at the main repo: preflight pr edit {} --repo <path>",
        pr.repo,
        pr.id
    );
    // A closed PR is frozen at the commits it had when it was closed.
    if let (true, Some(base), Some(head)) = (pr.state == "closed", &pr.base_sha, &pr.head_sha) {
        return Ok(Refs {
            base: base.clone(),
            head: head.clone(),
            base_missing: git::rev_parse(&pr.repo, &pr.base).is_err(),
            head_missing: git::rev_parse(&pr.repo, &pr.head).is_err(),
        });
    }
    let pick = |branch: &str, last: &Option<String>| -> anyhow::Result<(String, bool)> {
        match git::rev_parse(&pr.repo, branch) {
            Ok(sha) => Ok((sha, false)),
            Err(e) => last
                .clone()
                .map(|sha| (sha, true))
                .ok_or_else(|| e.context(format!("branch '{branch}' no longer exists"))),
        }
    };
    let (base, base_missing) = pick(&pr.base, &pr.base_sha)?;
    let (head, head_missing) = pick(&pr.head, &pr.head_sha)?;
    let base = git::pr_base(&pr.repo, base, &head, pr.base_sha.as_deref());
    if pr.base_sha.as_deref() != Some(base.as_str()) || pr.head_sha.as_deref() != Some(head.as_str()) {
        db::set_shas(&db::open()?, pr.id, &base, &head)?;
    }
    Ok(Refs { base, head, base_missing, head_missing })
}

async fn get_pr(Path(id): Path<i64>) -> ApiResult {
    let pr = load_pr(id)?;
    let refs = resolve(&pr)?;
    let commits: Vec<Value> = git::commits(&pr.repo, &refs.base, &refs.head)?
        .into_iter()
        .map(|c| json!({ "oid": c.oid, "sha": c.short, "subject": c.subject }))
        .collect();
    Ok(Json(json!({
        "pr": pr,
        "base_sha": refs.base,
        "head_sha": refs.head,
        "base_missing": refs.base_missing,
        "head_missing": refs.head_missing,
        "commits": commits,
    })))
}

/// Position of `oid` among the PR's commits (newest first); only the PR's own commits may be
/// viewed through these endpoints.
fn commit_index(commits: &[git::Commit], oid: &str) -> Result<usize, ApiError> {
    commits
        .iter()
        .position(|c| c.oid == oid)
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, format!("{oid} is not a commit of this PR")))
}

#[derive(Deserialize)]
struct DiffQuery {
    /// Oldest commit of the range to show (inclusive). Omit both for the whole PR.
    from: Option<String>,
    /// Newest commit of the range (inclusive); defaults to `from`.
    to: Option<String>,
}

async fn get_diff(Path(id): Path<i64>, Query(q): Query<DiffQuery>) -> ApiResult {
    let pr = load_pr(id)?;
    let refs = resolve(&pr)?;
    let Some(from) = q.from.or(q.to.clone()) else {
        return Ok(Json(json!({ "diff": git::diff(&pr.repo, &refs.base, &refs.head)? })));
    };
    let to = q.to.unwrap_or_else(|| from.clone());
    let commits = git::commits(&pr.repo, &refs.base, &refs.head)?;
    let (fi, ti) = (commit_index(&commits, &from)?, commit_index(&commits, &to)?);
    // Accept the two commits in either order; the older one has the larger index.
    let (old, new) = if fi >= ti { (&from, &to) } else { (&to, &from) };
    Ok(Json(json!({ "diff": git::diff_range(&pr.repo, old, new)? })))
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
    /// A commit of the PR to read the file at; defaults to the PR's head.
    rev: Option<String>,
}

/// Full file at the PR's head (or one of its commits), for expanding hidden context lines.
async fn get_file(Path(id): Path<i64>, Query(q): Query<FileQuery>) -> ApiResult {
    let pr = load_pr(id)?;
    let refs = resolve(&pr)?;
    let rev = match q.rev {
        Some(rev) => {
            commit_index(&git::commits(&pr.repo, &refs.base, &refs.head)?, &rev)?;
            rev
        }
        None => refs.head,
    };
    Ok(Json(json!({ "content": git::show(&pr.repo, &rev, &q.path)? })))
}

async fn list_comments(Path(id): Path<i64>) -> ApiResult {
    Ok(Json(json!(db::list_all_comments(&db::open()?, id)?)))
}

async fn add_comment(Path(id): Path<i64>, Json(n): Json<db::NewComment>) -> ApiResult {
    load_pr(id)?;
    Ok(Json(json!(db::add_comment(&db::open()?, id, &n)?)))
}

#[derive(Deserialize)]
struct SetState {
    state: String,
}

#[derive(Deserialize)]
struct EditPr {
    title: Option<String>,
    description: Option<String>,
    base: Option<String>,
}

async fn edit_pr(Path(id): Path<i64>, Json(b): Json<EditPr>) -> ApiResult {
    let pr = load_pr(id)?;
    let conn = db::open()?;
    let pr = crate::edit_pr(&conn, &pr, b.title.as_deref(), b.description.as_deref(), b.base.as_deref(), None, "user")?;
    Ok(Json(json!(pr)))
}

async fn list_branches(Path(id): Path<i64>) -> ApiResult {
    let pr = load_pr(id)?;
    Ok(Json(json!({ "branches": git::branches(&pr.repo)? })))
}

async fn set_pr_state(Path(id): Path<i64>, Json(b): Json<SetState>) -> ApiResult {
    let pr = load_pr(id)?;
    let conn = db::open()?;
    crate::snapshot_shas(&conn, &pr)?;
    db::set_pr_state(&conn, id, &b.state, "user")?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_reviews(Path(id): Path<i64>) -> ApiResult {
    Ok(Json(json!(db::list_reviews(&db::open()?, id)?)))
}

#[derive(Deserialize)]
struct NewReview {
    state: String,
    #[serde(default)]
    body: String,
}

async fn add_review(Path(id): Path<i64>, Json(b): Json<NewReview>) -> ApiResult {
    load_pr(id)?;
    Ok(Json(json!(db::add_review(&db::open()?, id, "user", &b.state, &b.body)?)))
}

/// Loads a comment the reviewer may change: the web UI acts as "user", and agents' comments are theirs.
fn own_comment(conn: &rusqlite::Connection, id: i64) -> Result<db::Comment, ApiError> {
    let c = db::get_comment(conn, id)?.ok_or_else(|| not_found("comment"))?;
    if c.author != "user" {
        return Err(ApiError(StatusCode::FORBIDDEN, "you can only change your own comments".into()));
    }
    Ok(c)
}

#[derive(Deserialize)]
struct EditComment {
    body: String,
}

async fn edit_comment(Path(id): Path<i64>, Json(b): Json<EditComment>) -> ApiResult {
    let conn = db::open()?;
    own_comment(&conn, id)?;
    if b.body.trim().is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "comment body is empty".into()));
    }
    db::edit_comment(&conn, id, &b.body)?;
    Ok(Json(json!(db::get_comment(&conn, id)?)))
}

async fn delete_comment(Path(id): Path<i64>) -> ApiResult {
    let conn = db::open()?;
    own_comment(&conn, id)?;
    db::delete_comment(&conn, id, "user")?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct Resolve {
    resolved: bool,
}

async fn resolve_comment(Path(id): Path<i64>, Json(r): Json<Resolve>) -> ApiResult {
    let conn = db::open()?;
    db::get_comment(&conn, id)?.ok_or_else(|| not_found("comment"))?;
    db::set_resolved(&conn, id, r.resolved, "user")?;
    Ok(Json(json!({ "ok": true })))
}

/// Serves the embedded React app; unknown paths fall back to index.html (client-side routing).
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, name) = match Assets::get(path) {
        Some(f) if !path.is_empty() => (f, path),
        _ => match Assets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return (StatusCode::NOT_FOUND, "web UI not built").into_response(),
        },
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    // Vite hashes asset filenames, so only index.html must be revalidated after an upgrade.
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    (
        [(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CACHE_CONTROL, cache.to_string())],
        file.data,
    )
        .into_response()
}

pub async fn serve(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/prs", get(list_prs).post(create_pr))
        .route("/api/prs/:id", get(get_pr).patch(edit_pr))
        .route("/api/prs/:id/branches", get(list_branches))
        .route("/api/prs/:id/diff", get(get_diff))
        .route("/api/prs/:id/file", get(get_file))
        .route("/api/prs/:id/comments", get(list_comments).post(add_comment))
        .route("/api/prs/:id/reviews", get(list_reviews).post(add_review))
        .route("/api/prs/:id/state", post(set_pr_state))
        .route("/api/comments/:id", patch(edit_comment).delete(delete_comment))
        .route("/api/comments/:id/resolve", post(resolve_comment))
        .fallback(static_handler);
    // Bind to loopback only: this exposes your local repos' contents.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("preflight serving on http://localhost:{port}  (db: {})", db::db_path().display());
    axum::serve(listener, app).await?;
    Ok(())
}
