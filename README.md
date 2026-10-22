# preflight

Preflight PRs: review LLM agent work locally *before* it becomes a remote PR.
The agent opens a "preflight PR" between two local branches, you review it in a GitHub-like
web UI on localhost, and the agent reads your comments back via the CLI.

## Build

Requires Rust and Node.js. `cargo build` also builds the React app (`web/`) and embeds it in the binary.

```bash
cargo install --git https://github.com/zhxiaogg/preflight.git
```

## Use

Start the review server (keep it running):

```bash
preflight serve      # http://localhost:7777  (--port or PREFLIGHT_PORT to change)
```

Agent opens a preflight PR (diff is `base...head`, computed live, so new commits show up on reload):

```bash
preflight create --repo /path/to/repo --base main --head my-feature --title "Add X" --description "Why…"
```

Review in the browser: browse files in the tree on the left, hover a diff line → click `+` to comment,
reply to or resolve threads.

Agent reads feedback and responds:

```bash
preflight comments 1 --unresolved          # human-readable, includes file:line and the line's code
preflight comments 1 --unresolved --json   # threads as JSON
preflight reply 3 "Fixed in abc123"
preflight resolve 3
preflight list
```

State lives in SQLite at `~/.preflight/preflight.db` (override with `PREFLIGHT_DB`). The CLI works without the server running.

## Agent instructions snippet

Paste into your agent's `CLAUDE.md` / `AGENTS.md`:

```markdown
A "preflight PR" is a local review done with the `preflight` CLI, not a GitHub PR.
When asked to open a preflight PR for a branch:
`preflight create --repo . --base main --head <branch> --title "<title>" --description "<summary>"`
and share the printed URL. To get feedback run `preflight comments <id> --unresolved`.
For each thread: make the fix, commit it to the same branch, then `preflight reply <comment-id> "<what you did>"`.
Don't resolve threads yourself unless asked; the reviewer resolves them.
```

## HTTP API

| Method | Path | |
|---|---|---|
| GET/POST | `/api/prs` | list / create (`{repo, base, head, title, description}`) |
| GET | `/api/prs/:id` | PR + base/head sha + commits |
| GET | `/api/prs/:id/diff` | raw unified diff |
| GET/POST | `/api/prs/:id/comments` | list / add (`{body, parent_id?, path?, line?, side?, line_text?, author?}`) |
| POST | `/api/comments/:id/resolve` | `{resolved: bool}` |

The server binds to `127.0.0.1` only.

## Not in the MVP

Syntax highlighting, split diff view, multi-line comments, batched "Submit review" with approve/request-changes,
markdown rendering, merge/close, uncommitted working-tree diffs.
