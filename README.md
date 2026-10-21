# local-pr (`lpr`)

Local pull requests for reviewing LLM agent work *before* it becomes a remote PR.
The agent opens a "PR" between two local branches, you review it in a GitHub-like
web UI on localhost, and the agent reads your comments back via the CLI.

## Build

Requires Rust and Node.js. `cargo build` also builds the React app (`web/`) and embeds it in the binary.

```bash
cargo install --path .
```

## Use

Start the review server (keep it running):

```bash
lpr serve            # http://localhost:7777  (--port or LPR_PORT to change)
```

Agent opens a local PR (diff is `base...head`, computed live, so new commits show up on reload):

```bash
lpr create --repo /path/to/repo --base main --head my-feature --title "Add X" --description "Why…"
```

Review in the browser: hover a diff line → click `+` to comment; reply to or resolve threads.

Agent reads feedback and responds:

```bash
lpr comments 1 --unresolved          # human-readable, includes file:line and the line's code
lpr comments 1 --unresolved --json   # threads as JSON
lpr reply 3 "Fixed in abc123"
lpr resolve 3
lpr list
```

State lives in SQLite at `~/.local-pr/lpr.db` (override with `LPR_DB`). The CLI works without the server running.

## Agent instructions snippet

Paste into your agent's `CLAUDE.md` / `AGENTS.md`:

```markdown
When you finish a change on a branch, open a local review instead of a remote PR:
`lpr create --repo . --base main --head <branch> --title "<title>" --description "<summary>"`
and share the printed URL. To get feedback run `lpr comments <id> --unresolved`.
For each thread: make the fix, commit it to the same branch, then `lpr reply <comment-id> "<what you did>"`.
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
