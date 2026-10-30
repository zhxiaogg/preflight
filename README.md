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
preflight pr create --repo /path/to/repo --base main --head my-feature --title "Add X" --description "Why…"
```

Review in the browser like on GitHub: syntax-highlighted unified or split (side-by-side) diffs with a file tree, `+` on a line to comment (drag the `+` or shift-click line numbers to comment on several lines),
expandable context around hunks (↑ expand up, ↓ expand down), markdown in descriptions and comments, editing/deleting your own comments, "Viewed" checkboxes, reply/resolve threads, and **Review changes**
(Comment / Approve / Request changes). A banner appears when the agent pushes new commits.
Click a commit to see just its changes, or check two commits (in the Conversation tab or the
**Changes from** menu on the Files tab) to see all changes between them; those views are read-only.

Agent reads feedback and responds:

```bash
preflight pr wait 1                    # block until the reviewer submits a review (--timeout N: exit 2)
preflight pr wait 1 --any              # ...or until any reviewer activity, printing what changed
preflight pr events 1 --since 0        # everything that changed, incl. edited/deleted comments
preflight pr edit 1 --title "…" --base dev   # change title / base; --description - reads stdin
preflight pr edit 1 --repo ~/src/app       # repoint at another checkout (e.g. after its worktree was removed)
preflight pr close 1                   # or reopen; closed PRs are hidden from `pr list` (use --all)
preflight pr list

preflight comment list 1 --unresolved          # latest review + threads, with file:line and the line's code
preflight comment list 1 --unresolved --json   # {state, latest_review, threads, event_cursor}
preflight comment create 1 "Ready for another look"                # on the conversation
preflight comment create 1 "Why this?" --file src/a.rs --line 42   # on a diff line (--side old: deleted line)
preflight comment create 1 "…" --file src/a.rs --start-line 40 --line 42   # a range; body "-" reads stdin
preflight comment reply 3 "Fixed in abc123"
preflight comment edit 7 "Updated: …"   # only the agent's own comments; body "-" reads stdin
preflight comment delete 7
preflight comment resolve 3
```

PRs created inside a git worktree are stored against the main repo, so they keep working after the worktree is removed.
State lives in SQLite at `~/.preflight/preflight.db` (override with `PREFLIGHT_DB`). The CLI works without the server running.

## Agent instructions snippet

Paste into your agent's `CLAUDE.md` / `AGENTS.md`:

```markdown
A "preflight PR" is a local review done with the `preflight` CLI, not a GitHub PR.
When asked to open a preflight PR for a branch:
`preflight pr create --repo . --base main --head <branch> --title "<title>" --description "<summary>"`
and share the printed URL. Then run `preflight pr wait <id>` to block until the reviewer submits a review;
it prints the review and the unresolved comments. Reviewers may edit or delete comments later, so
catch up with `preflight pr events <id> --since <cursor>` (the cursor is printed at the end, and
`comment list --json` includes it as `event_cursor`) before treating feedback as final.
For each thread: make the fix, commit it to the same branch, then `preflight comment reply <comment-id> "<what you did>"`.
To point the reviewer at something (a question, a trade-off, a risky spot), comment on the diff with
`preflight comment create <id> "<note>" --file <path> --line <n>`; only lines shown in the diff can be commented on.
Keep the description current with `preflight pr edit <id> --description -` (reads markdown from stdin).
Don't resolve threads yourself unless asked; the reviewer resolves them.
```

## HTTP API

| Method | Path | |
|---|---|---|
| GET/POST | `/api/prs` | list / create (`{repo, base, head, title, description}`) |
| GET/PATCH | `/api/prs/:id` | PR + base/head sha + commits / edit (`{title?, description?, base?}`) |
| GET | `/api/prs/:id/branches` | branches that can be picked as the base |
| GET | `/api/prs/:id/diff` | raw unified diff (`?from=<oid>&to=<oid>` for a range of the PR's commits) |
| GET/POST | `/api/prs/:id/comments` | list / add (`{body, parent_id?, path?, line?, side?, line_text?, author?}`) |
| GET/POST | `/api/prs/:id/reviews` | list / submit (`{state: approved\|changes_requested\|commented, body}`) |
| POST | `/api/prs/:id/state` | `{state: open\|closed}` |
| PATCH/DELETE | `/api/comments/:id` | edit (`{body}`) / delete your own (`user`) comment |
| POST | `/api/comments/:id/resolve` | `{resolved: bool}` |

The server binds to `127.0.0.1` only.

## Not in the MVP

Split diff view, multi-line comments, pending (batched) review comments, merging, uncommitted working-tree diffs.
