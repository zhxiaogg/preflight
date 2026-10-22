import { useCallback, useEffect, useMemo, useState } from "react";
import { api, Comment, NewComment, Pr, PrDetail, Side } from "./api";
import { DiffLine, FileDiff, filePath, parseDiff } from "./diff";

const POLL_MS = 4000;

function timeAgo(ts: number) {
  const s = Math.floor(Date.now() / 1000 - ts);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return new Date(ts * 1000).toLocaleDateString();
}

export function App() {
  const m = window.location.pathname.match(/^\/pr\/(\d+)/);
  return (
    <>
      <header className="topbar">
        <a href="/">Preflight</a>
      </header>
      <main className="container">{m ? <PrPage id={Number(m[1])} /> : <PrList />}</main>
    </>
  );
}

function PrList() {
  const [prs, setPrs] = useState<Pr[] | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    api.listPrs().then(setPrs, (e) => setErr(String(e)));
  }, []);
  if (err) return <div className="error">{err}</div>;
  if (!prs) return <div className="muted">Loading…</div>;
  return (
    <div className="box">
      <div className="box-header">{prs.length} preflight PRs</div>
      {prs.length === 0 && (
        <div className="box-row muted">
          No PRs yet. Create one with <code>preflight create --repo . --base main --head my-branch --title "…"</code>
        </div>
      )}
      {prs.map((p) => (
        <a key={p.id} className="box-row pr-row" href={`/pr/${p.id}`}>
          <div className="pr-title">{p.title}</div>
          <div className="muted small">
            #{p.id} · {p.repo} · <span className="branch">{p.base}</span> ← <span className="branch">{p.head}</span> ·
            opened {timeAgo(p.created_at)}
          </div>
        </a>
      ))}
    </div>
  );
}

type Tab = "conversation" | "files";

function PrPage({ id }: { id: number }) {
  const [detail, setDetail] = useState<PrDetail | null>(null);
  const [files, setFiles] = useState<FileDiff[]>([]);
  const [comments, setComments] = useState<Comment[]>([]);
  const [err, setErr] = useState("");
  const [tab, setTab] = useState<Tab>(window.location.hash === "#files" ? "files" : "conversation");

  const refreshComments = useCallback(() => api.listComments(id).then(setComments), [id]);

  useEffect(() => {
    // Diff is computed live from the branches, so a reload picks up new agent commits.
    Promise.all([api.getPr(id), api.getDiff(id), refreshComments()]).then(
      ([d, diff]) => {
        setDetail(d);
        setFiles(parseDiff(diff.diff));
      },
      (e) => setErr(String(e)),
    );
    // Poll so agent replies show up without a reload.
    const t = setInterval(() => refreshComments().catch(() => {}), POLL_MS);
    return () => clearInterval(t);
  }, [id, refreshComments]);

  const addComment = async (c: NewComment) => {
    await api.addComment(id, c);
    await refreshComments();
  };
  const resolve = async (commentId: number, resolved: boolean) => {
    await api.resolve(commentId, resolved);
    await refreshComments();
  };
  const actions = { addComment, resolve };

  if (err) return <div className="error">{err}</div>;
  if (!detail) return <div className="muted">Loading…</div>;
  const { pr } = detail;
  const roots = comments.filter((c) => c.parent_id === null);
  const switchTab = (t: Tab) => {
    setTab(t);
    history.replaceState(null, "", t === "files" ? "#files" : "#");
  };

  return (
    <>
      <h1 className="pr-heading">
        {pr.title} <span className="muted">#{pr.id}</span>
      </h1>
      <div className="pr-meta">
        <span className="state-badge">Open</span>
        <span className="muted">
          wants to merge {detail.commits.length} commit{detail.commits.length === 1 ? "" : "s"} into{" "}
          <span className="branch">{pr.base}</span> from <span className="branch">{pr.head}</span> · {pr.repo}
        </span>
      </div>
      <nav className="tabs">
        <button className={tab === "conversation" ? "active" : ""} onClick={() => switchTab("conversation")}>
          Conversation <span className="count">{roots.length}</span>
        </button>
        <button className={tab === "files" ? "active" : ""} onClick={() => switchTab("files")}>
          Files changed <span className="count">{files.length}</span>
        </button>
      </nav>
      {tab === "conversation" ? (
        <Conversation
          detail={detail}
          files={files}
          comments={comments}
          actions={actions}
          onOpenFiles={() => switchTab("files")}
        />
      ) : (
        <FilesChanged files={files} comments={comments} actions={actions} />
      )}
    </>
  );
}

type Actions = {
  addComment: (c: NewComment) => Promise<void>;
  resolve: (id: number, resolved: boolean) => Promise<void>;
};

function lineKey(path: string, side: Side, line: number) {
  return `${path}\u0000${side}\u0000${line}`;
}

/** Current text of every commentable line in the diff, keyed by lineKey. */
function lineTexts(files: FileDiff[]) {
  const m = new Map<string, string>();
  for (const f of files)
    for (const l of f.lines) {
      if (l.kind === "del") m.set(lineKey(filePath(f), "old", l.oldNo!), l.text);
      else if (l.kind !== "hunk") m.set(lineKey(filePath(f), "new", l.newNo!), l.text);
    }
  return m;
}

/** Like GitHub: an inline thread is outdated once its line's content changes or disappears. */
function isOutdated(c: Comment, texts: Map<string, string>) {
  if (c.path === null || c.line === null) return false;
  const cur = texts.get(lineKey(c.path, c.side ?? "new", c.line));
  return cur === undefined || (c.line_text !== null && cur !== c.line_text);
}

function Conversation(props: {
  detail: PrDetail;
  files: FileDiff[];
  comments: Comment[];
  actions: Actions;
  onOpenFiles: () => void;
}) {
  const { detail, files, comments, actions } = props;
  // Inline threads whose line no longer exists in the current diff are "outdated".
  const texts = useMemo(() => lineTexts(files), [files]);
  const roots = comments.filter((c) => c.parent_id === null);

  return (
    <div className="conversation">
      <div className="timeline-item box">
        <div className="box-header">
          <b>agent</b> <span className="muted">opened this PR {timeAgo(detail.pr.created_at)}</span>
        </div>
        <div className="comment-body">{detail.pr.description || <span className="muted">No description provided.</span>}</div>
      </div>
      {detail.commits.length > 0 && (
        <div className="timeline-item commits">
          {detail.commits.map((c) => (
            <div key={c.sha} className="commit">
              <code>{c.sha}</code> {c.subject}
            </div>
          ))}
        </div>
      )}
      {roots.map((r) => {
        const outdated = isOutdated(r, texts);
        return (
          <div key={r.id} className="timeline-item">
            {r.path && (
              <div className="thread-file">
                <a
                  href="#files"
                  onClick={(e) => {
                    e.preventDefault();
                    props.onOpenFiles();
                  }}
                >
                  {r.path}
                  {r.line !== null && `:${r.line}`}
                </a>
                {outdated && <span className="label">Outdated</span>}
                {r.line_text !== null && <pre className="snippet">{r.line_text}</pre>}
              </div>
            )}
            <Thread root={r} comments={comments} actions={actions} />
          </div>
        );
      })}
      <div className="timeline-item">
        <CommentForm placeholder="Leave a comment" submitLabel="Comment" onSubmit={(body) => actions.addComment({ body })} />
      </div>
    </div>
  );
}

function FilesChanged(props: { files: FileDiff[]; comments: Comment[]; actions: Actions }) {
  const { files, comments, actions } = props;
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [composer, setComposer] = useState<string | null>(null);

  const threadsByLine = useMemo(() => {
    const texts = lineTexts(files);
    const m = new Map<string, Comment[]>();
    for (const c of comments) {
      // Outdated threads only appear in the Conversation tab.
      if (c.parent_id !== null || c.path === null || c.line === null || isOutdated(c, texts)) continue;
      const k = lineKey(c.path, c.side ?? "new", c.line);
      m.set(k, [...(m.get(k) ?? []), c]);
    }
    return m;
  }, [comments, files]);

  const [active, setActive] = useState<string | null>(null);

  const adds = files.reduce((n, f) => n + f.additions, 0);
  const dels = files.reduce((n, f) => n + f.deletions, 0);
  if (files.length === 0) return <div className="box box-row muted">No changes between these branches.</div>;

  const openThreads = new Map<string, number>();
  for (const ts of threadsByLine.values())
    for (const t of ts) if (!t.resolved) openThreads.set(t.path!, (openThreads.get(t.path!) ?? 0) + 1);

  const jumpTo = (path: string) => {
    setActive(path);
    setCollapsed({ ...collapsed, [path]: false });
    document.getElementById(fileAnchor(path))?.scrollIntoView({ block: "start" });
  };

  return (
    <div className="files-layout">
      <FileTree files={files} active={active} openThreads={openThreads} onSelect={jumpTo} />
      <div className="files-main">
        <div className="diffstat muted small">
          {files.length} files changed <span className="add-text">+{adds}</span> <span className="del-text">−{dels}</span>
        </div>
        {files.map((f) => {
          const path = filePath(f);
          const isCollapsed = collapsed[path];
          return (
            <div key={path} id={fileAnchor(path)} className="file box">
              <div className="file-header" onClick={() => setCollapsed({ ...collapsed, [path]: !isCollapsed })}>
                <span className="chevron">{isCollapsed ? "▸" : "▾"}</span>
                <span className="add-text">+{f.additions}</span> <span className="del-text">−{f.deletions}</span>
                <span className="file-path">
                  {f.status === "renamed" ? `${f.oldPath} → ${f.newPath}` : path}
                </span>
                {f.status !== "modified" && <span className="label">{f.status}</span>}
              </div>
              {!isCollapsed &&
                (f.binary ? (
                  <div className="box-row muted">Binary file not shown.</div>
                ) : (
                  <table className="diff">
                    <colgroup>
                      <col className="num-col" />
                      <col className="num-col" />
                      <col />
                    </colgroup>
                    <tbody>
                      {f.lines.map((l, i) => {
                        if (l.kind === "hunk")
                          return (
                            <tr key={i} className="hunk">
                              <td colSpan={2} />
                              <td>{l.text}</td>
                            </tr>
                          );
                        const side: Side = l.kind === "del" ? "old" : "new";
                        const lineNo = (side === "old" ? l.oldNo : l.newNo)!;
                        const k = lineKey(path, side, lineNo);
                        const threads = threadsByLine.get(k) ?? [];
                        return (
                          <DiffRow
                            key={i}
                            line={l}
                            onAdd={() => setComposer(k)}
                            threads={threads}
                            comments={comments}
                            actions={actions}
                            composing={composer === k}
                            onCancel={() => setComposer(null)}
                            onSubmit={async (body) => {
                              await actions.addComment({ body, path, line: lineNo, side, line_text: l.text });
                              setComposer(null);
                            }}
                          />
                        );
                      })}
                    </tbody>
                  </table>
                ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function fileAnchor(path: string) {
  return `file-${path}`;
}

type TreeNode = { name: string; path: string; dirs: TreeNode[]; files: FileDiff[] };

/** Nests changed files into folders; folders with a single subfolder are merged ("web/src") like GitHub. */
function buildTree(files: FileDiff[]): TreeNode {
  const root: TreeNode = { name: "", path: "", dirs: [], files: [] };
  for (const f of files) {
    const parts = filePath(f).split("/");
    let node = root;
    for (const part of parts.slice(0, -1)) {
      const path = node.path ? `${node.path}/${part}` : part;
      let next = node.dirs.find((d) => d.name === part);
      if (!next) node.dirs.push((next = { name: part, path, dirs: [], files: [] }));
      node = next;
    }
    node.files.push(f);
  }
  const compress = (n: TreeNode): TreeNode => {
    n.dirs = n.dirs.map(compress).sort((a, b) => a.name.localeCompare(b.name));
    if (n.path && n.files.length === 0 && n.dirs.length === 1) {
      const only = n.dirs[0];
      return { ...only, name: `${n.name}/${only.name}` };
    }
    return n;
  };
  return compress(root);
}

const STATUS_MARK: Record<FileDiff["status"], [string, string]> = {
  added: ["+", "add-text"],
  deleted: ["−", "del-text"],
  modified: ["•", "mod-text"],
  renamed: ["→", "mod-text"],
};

function FileTree(props: {
  files: FileDiff[];
  active: string | null;
  openThreads: Map<string, number>;
  onSelect: (path: string) => void;
}) {
  const [filter, setFilter] = useState("");
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const q = filter.toLowerCase();
  const tree = useMemo(
    () => buildTree(props.files.filter((f) => filePath(f).toLowerCase().includes(q))),
    [props.files, q],
  );

  const toggle = (path: string) => {
    const next = new Set(closed);
    next.has(path) ? next.delete(path) : next.add(path);
    setClosed(next);
  };

  const render = (n: TreeNode, depth: number): JSX.Element[] => [
    ...n.dirs.flatMap((d) => [
      <div key={`d:${d.path}`} className="tree-row" style={{ paddingLeft: 8 + depth * 14 }} onClick={() => toggle(d.path)}>
        <span className="chevron">{closed.has(d.path) ? "▸" : "▾"}</span>
        <span className="tree-dir">{d.name}</span>
      </div>,
      ...(closed.has(d.path) ? [] : render(d, depth + 1)),
    ]),
    ...n.files
      .slice()
      .sort((a, b) => filePath(a).localeCompare(filePath(b)))
      .map((f) => {
        const path = filePath(f);
        const [mark, cls] = STATUS_MARK[f.status];
        const open = props.openThreads.get(path);
        return (
          <div
            key={`f:${path}`}
            className={`tree-row ${props.active === path ? "active" : ""}`}
            style={{ paddingLeft: 8 + depth * 14 + 16 }}
            title={path}
            onClick={() => props.onSelect(path)}
          >
            <span className="tree-file">{path.split("/").pop()}</span>
            {open ? <span className="count">{open}</span> : null}
            <span className={`tree-mark ${cls}`}>{mark}</span>
          </div>
        );
      }),
  ];

  return (
    <aside className="file-tree">
      <input className="tree-filter" placeholder="Filter changed files" value={filter} onChange={(e) => setFilter(e.target.value)} />
      <div className="tree-list">{render(tree, 0)}</div>
    </aside>
  );
}

function DiffRow(props: {
  line: DiffLine;
  threads: Comment[];
  comments: Comment[];
  actions: Actions;
  composing: boolean;
  onAdd: () => void;
  onCancel: () => void;
  onSubmit: (body: string) => Promise<void>;
}) {
  const { line: l, threads, composing } = props;
  const sign = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
  return (
    <>
      <tr className={`line ${l.kind}`}>
        <td className="num">{l.oldNo ?? ""}</td>
        <td className="num">{l.newNo ?? ""}</td>
        <td className="code">
          <button className="add-comment" title="Add a comment" onClick={props.onAdd}>
            +
          </button>
          <span className="sign">{sign}</span>
          {l.text}
        </td>
      </tr>
      {(threads.length > 0 || composing) && (
        <tr className="inline-threads">
          <td colSpan={3}>
            {threads.map((t) => (
              <Thread key={t.id} root={t} comments={props.comments} actions={props.actions} />
            ))}
            {composing && (
              <div className="box inline-composer">
                <CommentForm
                  autoFocus
                  placeholder="Leave a comment"
                  submitLabel="Add comment"
                  onSubmit={props.onSubmit}
                  onCancel={props.onCancel}
                />
              </div>
            )}
          </td>
        </tr>
      )}
    </>
  );
}

function Thread({ root, comments, actions }: { root: Comment; comments: Comment[]; actions: Actions }) {
  const [open, setOpen] = useState(!root.resolved);
  const [replying, setReplying] = useState(false);
  useEffect(() => setOpen(!root.resolved), [root.resolved]);
  const replies = comments.filter((c) => c.parent_id === root.id);

  return (
    <div className={`box thread ${root.resolved ? "resolved" : ""}`}>
      {root.resolved && (
        <div className="thread-resolved-bar" onClick={() => setOpen(!open)}>
          <span className="muted small">
            {open ? "▾" : "▸"} Resolved conversation ({1 + replies.length})
          </span>
        </div>
      )}
      {open && (
        <>
          {[root, ...replies].map((c) => (
            <div key={c.id} className="comment">
              <div className="comment-head">
                <b className={c.author === "user" ? "" : "agent"}>{c.author}</b>{" "}
                <span className="muted small">
                  {timeAgo(c.created_at)} · #{c.id}
                </span>
              </div>
              <div className="comment-body">{c.body}</div>
            </div>
          ))}
          <div className="thread-footer">
            {replying ? (
              <CommentForm
                autoFocus
                placeholder="Reply…"
                submitLabel="Reply"
                onCancel={() => setReplying(false)}
                onSubmit={async (body) => {
                  await actions.addComment({ body, parent_id: root.id });
                  setReplying(false);
                }}
              />
            ) : (
              <>
                <input className="reply-stub" placeholder="Reply…" onFocus={() => setReplying(true)} readOnly />
                <button className="btn" onClick={() => actions.resolve(root.id, !root.resolved)}>
                  {root.resolved ? "Unresolve conversation" : "Resolve conversation"}
                </button>
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function CommentForm(props: {
  placeholder: string;
  submitLabel: string;
  autoFocus?: boolean;
  onSubmit: (body: string) => Promise<void>;
  onCancel?: () => void;
}) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const submit = async () => {
    if (!body.trim()) return;
    setBusy(true);
    setErr("");
    try {
      await props.onSubmit(body);
      setBody("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="comment-form">
      <textarea
        autoFocus={props.autoFocus}
        placeholder={props.placeholder}
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          if (e.key === "Escape") props.onCancel?.();
        }}
      />
      {err && <div className="error">{err}</div>}
      <div className="form-actions">
        {props.onCancel && (
          <button className="btn" onClick={props.onCancel}>
            Cancel
          </button>
        )}
        <button className="btn primary" disabled={busy || !body.trim()} onClick={submit}>
          {props.submitLabel}
        </button>
      </div>
    </div>
  );
}
