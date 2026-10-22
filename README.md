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

Review in the browser like on GitHub: syntax-highlighted diffs with a file tree, `+` on a line to comment (drag the `+` or shift-click line numbers to comment on several lines),
expandable context around hunks (↑ expand up, ↓ expand down), markdown in descriptions and comments, "Viewed" checkboxes, reply/resolve threads, and **Review changes**
(Comment / Approve / Request changes). A banner appears when the agent pushes new commits.

Agent reads feedback and responds:

```bash
preflight wait 1                           # block until the reviewer submits a review (--timeout N: exit 2)
preflight comments 1 --unresolved          # latest review + threads, with file:line and the line's code
preflight comments 1 --unresolved --json   # {state, latest_review, threads}
preflight reply 3 "Fixed in abc123"
preflight resolve 3
preflight close 1                          # or reopen; closed PRs are hidden from `list` (use --all)
preflight list
```

State lives in SQLite at `~/.preflight/preflight.db` (override with `PREFLIGHT_DB`). The CLI works without the server running.

## Agent instructions snippet

Paste into your agent's `CLAUDE.md` / `AGENTS.md`:

```markdown
A "preflight PR" is a local review done with the `preflight` CLI, not a GitHub PR.
When asked to open a preflight PR for a branch:
`preflight create --repo . --base main --head <branch> --title "<title>" --description "<summary>"`
and share the printed URL. Then run `preflight wait <id>` to block until the reviewer submits a review;
it prints the review and the unresolved comments.
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
| GET/POST | `/api/prs/:id/reviews` | list / submit (`{state: approved\|changes_requested\|commented, body}`) |
| POST | `/api/prs/:id/state` | `{state: open\|closed}` |
| POST | `/api/comments/:id/resolve` | `{resolved: bool}` |

The server binds to `127.0.0.1` only.

## Not in the MVP

Split diff view, multi-line comments, pending (batched) review comments, merging, uncommitted working-tree diffs.
