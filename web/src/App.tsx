import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, Comment, NewComment, Pr, PrDetail, Review, ReviewState, Side } from "./api";
import { DiffLine, FileDiff, filePath, parseDiff } from "./diff";
import { highlightFile, highlightText } from "./highlight";
import { Markdown } from "./Markdown";

const POLL_MS = 4000;
const REPO_URL = "https://github.com/zhxiaogg/preflight";

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
        <a className="topbar-link" href={REPO_URL} target="_blank" rel="noreferrer" title="Preflight on GitHub">
          <svg viewBox="0 0 16 16" width="20" height="20" aria-hidden="true">
            <path
              fill="currentColor"
              d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
            />
          </svg>
          <span>GitHub</span>
        </a>
      </header>
      <main className="container">{m ? <PrPage id={Number(m[1])} /> : <PrList />}</main>
    </>
  );
}

function PrList() {
  const [all, setAll] = useState<Pr[] | null>(null);
  const [err, setErr] = useState("");
  const [show, setShow] = useState<Pr["state"]>("open");
  useEffect(() => {
    api.listPrs().then(setAll, (e) => setErr(String(e)));
  }, []);
  if (err) return <div className="error">{err}</div>;
  if (!all) return <div className="muted">Loading…</div>;
  const prs = all.filter((p) => p.state === show);
  const count = (st: Pr["state"]) => all.filter((p) => p.state === st).length;
  return (
    <div className="box">
      <div className="box-header list-filter">
        {(["open", "closed"] as const).map((st) => (
          <button key={st} className={show === st ? "active" : ""} onClick={() => setShow(st)}>
            {count(st)} {st === "open" ? "Open" : "Closed"}
          </button>
        ))}
      </div>
      {all.length === 0 && (
        <div className="box-row muted">
          No PRs yet. Create one with <code>preflight create --repo . --base main --head my-branch --title "…"</code>
        </div>
      )}
      {prs.map((p) => (
        <a key={p.id} className="box-row pr-row" href={`/pr/${p.id}`}>
          <div className="pr-title">
            <span className={`state-dot ${p.state}`} /> {p.title}
          </div>
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
  const [reviews, setReviews] = useState<Review[]>([]);
  const [latestHead, setLatestHead] = useState<string | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [err, setErr] = useState("");
  const [tab, setTab] = useState<Tab>(window.location.hash === "#files" ? "files" : "conversation");

  const refreshComments = useCallback(
    () => Promise.all([api.listComments(id).then(setComments), api.listReviews(id).then(setReviews)]),
    [id],
  );

  // Diff is computed live from the branches, so reloading picks up new agent commits.
  const load = useCallback(
    () =>
      Promise.all([api.getPr(id), api.getDiff(id), refreshComments()]).then(
        ([d, diff]) => {
          setDetail(d);
          setLatestHead(d.head_sha);
          setFiles(treeOrder(parseDiff(diff.diff)));
        },
        (e) => setErr(String(e)),
      ),
    [id, refreshComments],
  );

  useEffect(() => {
    load();
    // Poll so agent replies and new commits show up without a reload.
    const t = setInterval(() => {
      refreshComments().catch(() => {});
      api.getPr(id).then((d) => setLatestHead(d.head_sha), () => {});
    }, POLL_MS);
    return () => clearInterval(t);
  }, [id, load, refreshComments]);

  const addComment = async (c: NewComment) => {
    await api.addComment(id, c);
    await refreshComments();
  };
  const resolve = async (commentId: number, resolved: boolean) => {
    await api.resolve(commentId, resolved);
    await refreshComments();
  };
  const setState = async (state: Pr["state"]) => {
    await api.setState(id, state);
    await load();
  };
  const submitReview = async (state: ReviewState, body: string) => {
    await api.submitReview(id, state, body);
    await refreshComments();
    setReviewing(false);
  };
  const editComment = async (commentId: number, body: string) => {
    await api.editComment(commentId, body);
    await refreshComments();
  };
  const deleteComment = async (commentId: number) => {
    await api.deleteComment(commentId);
    await refreshComments();
  };
  const actions = { addComment, resolve, editComment, deleteComment };

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
        <span className={`state-badge ${pr.state}`}>{pr.state === "open" ? "Open" : "Closed"}</span>
        <span className="muted">
          wants to merge {detail.commits.length} commit{detail.commits.length === 1 ? "" : "s"} into{" "}
          <span className={`branch ${detail.base_missing ? "deleted" : ""}`}>{pr.base}</span> from{" "}
          <span className={`branch ${detail.head_missing ? "deleted" : ""}`}>{pr.head}</span>
          {(detail.base_missing || detail.head_missing) && (
            <span className="label" title="Showing the last commit seen before the branch was deleted">
              branch deleted
            </span>
          )}{" "}
          · {pr.repo}
        </span>
      </div>
      {latestHead && latestHead !== detail.head_sha && (
        <div className="banner">
          New commits were pushed to <span className="branch">{pr.head}</span>.
          <button className="btn" onClick={load}>
            Refresh
          </button>
        </div>
      )}
      <nav className="tabs">
        <button className={tab === "conversation" ? "active" : ""} onClick={() => switchTab("conversation")}>
          Conversation <span className="count">{roots.length}</span>
        </button>
        <button className={tab === "files" ? "active" : ""} onClick={() => switchTab("files")}>
          Files changed <span className="count">{files.length}</span>
        </button>
        <div className="tabs-actions">
          <button className="btn primary" onClick={() => setReviewing(!reviewing)}>
            Review changes ▾
          </button>
          {reviewing && <ReviewPanel onSubmit={submitReview} onCancel={() => setReviewing(false)} />}
        </div>
      </nav>
      {tab === "conversation" ? (
        <Conversation
          detail={detail}
          files={files}
          comments={comments}
          reviews={reviews}
          actions={actions}
          onOpenFiles={() => switchTab("files")}
          onSetState={setState}
        />
      ) : (
        <FilesChanged prId={id} files={files} comments={comments} actions={actions} />
      )}
    </>
  );
}

/** Lines selected for a comment: indices into the file's diff lines, within one hunk. */
type LineSelection = { path: string; anchor: number; head: number; dragging: boolean };

type Actions = {
  addComment: (c: NewComment) => Promise<void>;
  resolve: (id: number, resolved: boolean) => Promise<void>;
  editComment: (id: number, body: string) => Promise<void>;
  deleteComment: (id: number) => Promise<void>;
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
  if (cur === undefined) return true;
  if (c.line_text === null) return false;
  if (c.start_line === null) return cur !== c.line_text;
  // Multi-line: outdated once either end of the selected range changes.
  const snap = c.line_text.split("\n");
  const first = texts.get(lineKey(c.path, c.start_side ?? c.side ?? "new", c.start_line));
  return first !== snap[0] || cur !== snap[snap.length - 1];
}

/** "+10 to +14" style label for a multi-line comment, or null for a single line. */
function rangeLabel(c: Comment) {
  if (c.start_line === null || c.line === null) return null;
  const sign = (s: Side | null) => (s === "old" ? "-" : "+");
  return `${sign(c.start_side)}${c.start_line} to ${sign(c.side)}${c.line}`;
}

function Conversation(props: {
  detail: PrDetail;
  files: FileDiff[];
  comments: Comment[];
  reviews: Review[];
  actions: Actions;
  onOpenFiles: () => void;
  onSetState: (state: Pr["state"]) => Promise<void>;
}) {
  const { detail, files, comments, actions } = props;
  // Inline threads whose line no longer exists in the current diff are "outdated".
  const texts = useMemo(() => lineTexts(files), [files]);
  const roots = comments.filter((c) => c.parent_id === null);
  type Item = { at: number; thread?: Comment; review?: Review };
  const timeline: Item[] = [
    ...roots.map((c) => ({ at: c.created_at, thread: c })),
    ...props.reviews.map((r) => ({ at: r.created_at, review: r })),
  ].sort((a, b) => a.at - b.at);

  return (
    <div className="conversation">
      <div className="timeline-item box">
        <div className="box-header">
          <b>agent</b> <span className="muted">opened this PR {timeAgo(detail.pr.created_at)}</span>
        </div>
        <div className="comment-body">
          {detail.pr.description ? <Markdown text={detail.pr.description} /> : <span className="muted">No description provided.</span>}
        </div>
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
      {timeline.map((item) => {
        if (item.review) return <ReviewItem key={`r${item.review.id}`} review={item.review} />;
        const r = item.thread!;
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
                  {r.line !== null && `:${r.start_line !== null ? `${r.start_line}-` : ""}${r.line}`}
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
        <div className="form-actions">
          {detail.pr.state === "open" ? (
            <button className="btn" onClick={() => props.onSetState("closed")}>
              Close pull request
            </button>
          ) : (
            <button className="btn" onClick={() => props.onSetState("open")}>
              Reopen pull request
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function FilesChanged(props: { prId: number; files: FileDiff[]; comments: Comment[]; actions: Actions }) {
  const { files, comments, actions } = props;
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [viewed, setViewedState] = useState<Record<string, string>>(() => loadViewed(props.prId));
  const highlighted = useMemo(() => files.map(highlightFile), [files]);
  const fingerprints = useMemo(() => files.map(fingerprint), [files]);
  // Like GitHub, a file stops counting as viewed once its diff changes.
  const isViewed = (i: number) => viewed[filePath(files[i])] === fingerprints[i];
  const toggleViewed = (i: number) => {
    const path = filePath(files[i]);
    const next = { ...viewed };
    if (isViewed(i)) delete next[path];
    else next[path] = fingerprints[i];
    setViewedState(next);
    saveViewed(props.prId, next);
    setCollapsed({ ...collapsed, [path]: !isViewed(i) });
  };
  const [sel, setSel] = useState<LineSelection | null>(null);
  // A drag that started on a "+" ends wherever the mouse is released.
  useEffect(() => {
    const up = () => setSel((s) => (s?.dragging ? { ...s, dragging: false } : s));
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

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
      <FileTree
        files={files}
        active={active}
        openThreads={openThreads}
        viewed={new Set(files.filter((_, i) => isViewed(i)).map(filePath))}
        onSelect={jumpTo}
      />
      <div className="files-main">
        <div className="diffstat muted small">
          {files.length} files changed <span className="add-text">+{adds}</span> <span className="del-text">−{dels}</span>
          <span className="viewed-count">
            {files.filter((_, i) => isViewed(i)).length} / {files.length} viewed
          </span>
        </div>
        {files.map((f, fi) => {
          const path = filePath(f);
          const isCollapsed = collapsed[path] ?? isViewed(fi);
          return (
            <div key={path} id={fileAnchor(path)} className="file box">
              <div className="file-header" onClick={() => setCollapsed({ ...collapsed, [path]: !isCollapsed })}>
                <span className="chevron">{isCollapsed ? "▸" : "▾"}</span>
                <span className="add-text">+{f.additions}</span> <span className="del-text">−{f.deletions}</span>
                <span className="file-path">
                  {f.status === "renamed" ? `${f.oldPath} → ${f.newPath}` : path}
                </span>
                {f.status !== "modified" && <span className="label">{f.status}</span>}
                <label className="viewed-toggle" onClick={(e) => e.stopPropagation()}>
                  <input type="checkbox" checked={isViewed(fi)} onChange={() => toggleViewed(fi)} /> Viewed
                </label>
              </div>
              {!isCollapsed &&
                (f.binary ? (
                  <div className="box-row muted">Binary file not shown.</div>
                ) : (
                  <FileDiffTable
                    prId={props.prId}
                    file={f}
                    html={highlighted[fi]}
                    threadsByLine={threadsByLine}
                    comments={comments}
                    actions={actions}
                    sel={sel?.path === path ? sel : null}
                    setSel={setSel}
                  />
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

/** Cheap hash of a file's diff, used to reset "Viewed" when the file changes. */
function fingerprint(f: FileDiff) {
  let h = 0;
  for (const l of f.lines) for (let i = 0; i < l.text.length; i++) h = (h * 31 + l.text.charCodeAt(i)) | 0;
  return `${f.lines.length}:${h}`;
}

// "Viewed" marks are a per-browser convenience, so localStorage is enough.
function loadViewed(prId: number): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(`preflight:viewed:${prId}`) ?? "{}");
  } catch {
    return {};
  }
}

function saveViewed(prId: number, v: Record<string, string>) {
  try {
    localStorage.setItem(`preflight:viewed:${prId}`, JSON.stringify(v));
  } catch {
    // Storage unavailable (private mode): marks just won't persist.
  }
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

/** Files in the order the tree shows them (folders first, then files, by name), so diffs match the tree. */
function treeOrder(files: FileDiff[]): FileDiff[] {
  const walk = (n: TreeNode): FileDiff[] => [
    ...n.dirs.flatMap(walk),
    ...n.files.slice().sort((a, b) => filePath(a).localeCompare(filePath(b))),
  ];
  return walk(buildTree(files));
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
  viewed: Set<string>;
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
            className={`tree-row ${props.active === path ? "active" : ""} ${props.viewed.has(path) ? "viewed" : ""}`}
            style={{ paddingLeft: 8 + depth * 14 + 16 }}
            title={path}
            onClick={() => props.onSelect(path)}
          >
            <span className="tree-file">{path.split("/").pop()}</span>
            {open ? <span className="count">{open}</span> : null}
            {props.viewed.has(path) && <span className="tree-viewed">✓</span>}
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

/** Hidden unchanged lines revealed per click, like GitHub. */
const EXPAND_STEP = 20;

function FileDiffTable(props: {
  prId: number;
  file: FileDiff;
  html: (string | undefined)[] | null;
  threadsByLine: Map<string, Comment[]>;
  comments: Comment[];
  actions: Actions;
  sel: LineSelection | null;
  setSel: (s: LineSelection | null) => void;
}) {
  const { file: f, threadsByLine, comments, actions, sel, setSel } = props;
  const path = filePath(f);
  // Index of the hunk header each line belongs to; a selection can't cross hunks.
  const hunkOf = useMemo(() => {
    let h = 0;
    return f.lines.map((l, i) => (l.kind === "hunk" ? (h = i) : h));
  }, [f]);
  const lo = sel ? Math.min(sel.anchor, sel.head) : -1;
  const hi = sel ? Math.max(sel.anchor, sel.head) : -1;
  const extendable = (i: number) => sel !== null && hunkOf[i] === hunkOf[sel.anchor];
  const select = (i: number, extend: boolean, dragging: boolean) =>
    setSel(extend && extendable(i) ? { ...sel!, head: i, dragging } : { path, anchor: i, head: i, dragging });
  // Full new-side file, fetched on the first expand.
  const [content, setContent] = useState<string[] | null>(null);
  const [loadErr, setLoadErr] = useState("");
  // Lines revealed in the gap before each hunk (keyed by the hunk's index in f.lines): from the
  // top of the gap ("down", continuing the previous hunk) and from the bottom ("up"). `tail` is
  // revealed after the last hunk.
  const [down, setDown] = useState<Record<number, number>>({});
  const [up, setUp] = useState<Record<number, number>>({});
  const [tail, setTail] = useState(0);
  const contentHtml = useMemo(() => content && highlightText(path, content.join("\n")), [content, path]);
  const canExpand = f.status === "modified" || f.status === "renamed";

  const ensureContent = async () => {
    if (content) return;
    try {
      const { content: text } = await api.getFile(props.prId, path);
      const lines = text.split("\n");
      if (lines[lines.length - 1] === "") lines.pop();
      setContent(lines);
    } catch (e) {
      setLoadErr(String(e));
      throw e;
    }
  };
  const expand = (set: typeof setUp, hunk: number) =>
    ensureContent().then(() => set((m) => ({ ...m, [hunk]: (m[hunk] ?? 0) + EXPAND_STEP })), () => {});
  const expandTail = () => ensureContent().then(() => setTail((t) => t + EXPAND_STEP), () => {});

  const contextRow = (newNo: number, oldNo: number) => (
    <DiffRow
      key={`x${newNo}`}
      line={{ kind: "ctx", text: content![newNo - 1] ?? "", oldNo, newNo }}
      html={contentHtml?.[newNo - 1]}
      expanded
      threads={[]}
      comments={comments}
      actions={actions}
      composing={false}
      onCancel={() => {}}
      onSubmit={async () => {}}
    />
  );

  const rows: JSX.Element[] = [];
  let lastNew = 0;
  let lastOld = 0;
  f.lines.forEach((l, i) => {
    if (l.kind === "hunk") {
      const start = l.newNo!;
      const delta = l.newNo! - l.oldNo!;
      const gap = canExpand ? start - 1 - lastNew : 0;
      const downShown = Math.min(down[i] ?? 0, gap);
      const upShown = Math.min(up[i] ?? 0, gap - downShown);
      const hidden = gap - downShown - upShown;
      for (let n = lastNew + 1; n <= lastNew + downShown; n++) rows.push(contextRow(n, n - delta));
      // Once a gap is fully expanded the hunks merge and the header disappears, as on GitHub.
      if (hidden > 0 || (i === 0 && gap === 0))
        rows.push(
          <tr key={`h${i}`} className="hunk">
            <td colSpan={2} className="expander">
              {hidden > 0 && lastNew > 0 && (
                <button title="Expand down" onClick={() => expand(setDown, i)}>
                  ↓
                </button>
              )}
              {hidden > 0 && (
                <button title="Expand up" onClick={() => expand(setUp, i)}>
                  ↑
                </button>
              )}
            </td>
            <td>{l.text}</td>
          </tr>,
        );
      for (let n = start - upShown; n < start; n++) rows.push(contextRow(n, n - delta));
      return;
    }
    if (l.newNo !== undefined) lastNew = l.newNo;
    if (l.oldNo !== undefined) lastOld = l.oldNo;
    const side: Side = l.kind === "del" ? "old" : "new";
    const lineNo = (side === "old" ? l.oldNo : l.newNo)!;
    const k = lineKey(path, side, lineNo);
    rows.push(
      <DiffRow
        key={i}
        line={l}
        html={props.html?.[i]}
        selected={i >= lo && i <= hi}
        onAddMouseDown={(shift) => select(i, shift, !shift)}
        onNumClick={(shift) => select(i, shift, false)}
        onMouseEnter={() => sel?.dragging && extendable(i) && setSel({ ...sel, head: i })}
        threads={threadsByLine.get(k) ?? []}
        comments={comments}
        actions={actions}
        composing={i === hi && !sel!.dragging}
        composeLabel={lo < hi ? `Commenting on ${hi - lo + 1} lines` : undefined}
        onCancel={() => setSel(null)}
        onSubmit={async (body) => {
          const start = f.lines[lo];
          const startSide: Side = start.kind === "del" ? "old" : "new";
          await actions.addComment({
            body,
            path,
            line: lineNo,
            side,
            ...(lo < i && { start_line: (startSide === "old" ? start.oldNo : start.newNo)!, start_side: startSide }),
            line_text: f.lines
              .slice(lo, i + 1)
              .map((x) => x.text)
              .join("\n"),
          });
          setSel(null);
        }}
      />,
    );
  });

  // Lines after the last hunk. Until the file is fetched we can't know whether any exist.
  if (canExpand) {
    const remaining = content ? content.length - lastNew : Infinity;
    const shown = Math.min(tail, remaining);
    for (let n = lastNew + 1; n <= lastNew + shown; n++) rows.push(contextRow(n, n - (lastNew - lastOld)));
    if (remaining - shown > 0)
      rows.push(
        <tr key="tail" className="hunk">
          <td colSpan={2} className="expander">
            <button title="Expand down" onClick={expandTail}>
              ↓
            </button>
          </td>
          <td>{loadErr}</td>
        </tr>,
      );
  }

  return (
    <table className="diff">
      <colgroup>
        <col className="num-col" />
        <col className="num-col" />
        <col />
      </colgroup>
      <tbody>{rows}</tbody>
    </table>
  );
}

function DiffRow(props: {
  line: DiffLine;
  /** Unchanged line revealed by expanding context; not commentable since it's outside the diff. */
  expanded?: boolean;
  /** Syntax-highlighted line (escaped by highlight.js), if the language is known. */
  html?: string;
  threads: Comment[];
  comments: Comment[];
  actions: Actions;
  selected?: boolean;
  composing: boolean;
  composeLabel?: string;
  /** Mouse down on "+": starts a drag selection, or extends the current one with shift. */
  onAddMouseDown?: (shift: boolean) => void;
  /** Click on a line number: selects the line, or extends the selection with shift. */
  onNumClick?: (shift: boolean) => void;
  onMouseEnter?: () => void;
  onCancel: () => void;
  onSubmit: (body: string) => Promise<void>;
}) {
  const { line: l, threads, composing } = props;
  const sign = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
  const numClick = props.onNumClick && ((e: React.MouseEvent) => props.onNumClick!(e.shiftKey));
  return (
    <>
      <tr
        className={`line ${l.kind} ${props.expanded ? "expanded" : ""} ${props.selected ? "selected" : ""}`}
        onMouseEnter={props.onMouseEnter}
      >
        <td className={`num ${numClick ? "clickable" : ""}`} onClick={numClick}>
          {l.oldNo ?? ""}
        </td>
        <td className={`num ${numClick ? "clickable" : ""}`} onClick={numClick}>
          {l.newNo ?? ""}
        </td>
        <td className="code">
          {props.onAddMouseDown && (
            <button
              className="add-comment"
              title="Add a comment (drag or shift-click to select lines)"
              onMouseDown={(e) => {
                e.preventDefault(); // don't start a text selection while dragging
                props.onAddMouseDown!(e.shiftKey);
              }}
            >
              +
            </button>
          )}
          <span className="sign">{sign}</span>
          {props.html !== undefined ? <span dangerouslySetInnerHTML={{ __html: props.html }} /> : l.text}
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
                {props.composeLabel && <div className="muted small compose-label">{props.composeLabel}</div>}
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

  const lines = rangeLabel(root);
  return (
    <div className={`box thread ${root.resolved ? "resolved" : ""}`}>
      {lines && <div className="thread-lines muted small">Comment on lines {lines}</div>}
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
            <CommentView key={c.id} comment={c} isRoot={c.id === root.id} replies={replies.length} actions={actions} />
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

function CommentView(props: { comment: Comment; isRoot: boolean; replies: number; actions: Actions }) {
  const { comment: c, actions } = props;
  const [editing, setEditing] = useState(false);
  const remove = () => {
    const what =
      props.isRoot && props.replies > 0
        ? `Delete this comment and its ${props.replies} ${props.replies === 1 ? "reply" : "replies"}?`
        : "Delete this comment?";
    if (window.confirm(what)) actions.deleteComment(c.id).catch((e) => alert(String(e)));
  };
  return (
    <div className="comment">
      <div className="comment-head">
        <b className={c.author === "user" ? "" : "agent"}>{c.author}</b>{" "}
        <span className="muted small">
          {timeAgo(c.created_at)} · #{c.id}
          {c.updated_at !== null && <span title={new Date(c.updated_at * 1000).toLocaleString()}> · edited</span>}
        </span>
        {/* The web UI is the "user"; agents' comments are read-only here, like others' comments on GitHub. */}
        {c.author === "user" && !editing && (
          <span className="comment-actions">
            <button className="link-btn" onClick={() => setEditing(true)}>
              Edit
            </button>
            <button className="link-btn danger" onClick={remove}>
              Delete
            </button>
          </span>
        )}
      </div>
      <div className="comment-body">
        {editing ? (
          <CommentForm
            autoFocus
            initial={c.body}
            placeholder="Edit comment"
            submitLabel="Update comment"
            onCancel={() => setEditing(false)}
            onSubmit={async (body) => {
              await actions.editComment(c.id, body);
              setEditing(false);
            }}
          />
        ) : (
          <Markdown text={c.body} />
        )}
      </div>
    </div>
  );
}

function CommentForm(props: {
  placeholder: string;
  submitLabel: string;
  autoFocus?: boolean;
  initial?: string;
  onSubmit: (body: string) => Promise<void>;
  onCancel?: () => void;
}) {
  const [body, setBody] = useState(props.initial ?? "");
  const textarea = useRef<HTMLTextAreaElement>(null);
  // When editing, start with the caret after the existing text, like GitHub.
  useEffect(() => {
    const t = textarea.current;
    if (t && props.initial) t.setSelectionRange(t.value.length, t.value.length);
  }, [props.initial]);
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
        ref={textarea}
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

const REVIEW_TEXT: Record<ReviewState, [string, string]> = {
  approved: ["approved these changes", "approved"],
  changes_requested: ["requested changes", "changes"],
  commented: ["reviewed", "commented"],
};

function ReviewItem({ review }: { review: Review }) {
  const [text, cls] = REVIEW_TEXT[review.state];
  return (
    <div className="timeline-item">
      <div className={`review-event ${cls}`}>
        <span className="review-icon">{review.state === "approved" ? "✓" : review.state === "changes_requested" ? "±" : "💬"}</span>
        <b>{review.author}</b> <span className="muted">{text} {timeAgo(review.created_at)}</span>
      </div>
      {review.body && (
        <div className="box">
          <div className="comment-body">
            <Markdown text={review.body} />
          </div>
        </div>
      )}
    </div>
  );
}

function ReviewPanel(props: { onSubmit: (state: ReviewState, body: string) => Promise<void>; onCancel: () => void }) {
  const [state, setState] = useState<ReviewState>("commented");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const submit = async () => {
    setBusy(true);
    setErr("");
    try {
      await props.onSubmit(state, body);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };
  const options: [ReviewState, string, string][] = [
    ["commented", "Comment", "Submit general feedback without explicit approval."],
    ["approved", "Approve", "Give your approval to these changes."],
    ["changes_requested", "Request changes", "Submit feedback the agent must address."],
  ];
  return (
    <div className="review-panel box">
      <div className="box-header">
        <b>Finish your review</b>
      </div>
      <div className="review-panel-body">
        <textarea autoFocus placeholder="Leave a summary (optional)" value={body} onChange={(e) => setBody(e.target.value)} />
        {options.map(([value, label, hint]) => (
          <label key={value} className="review-option">
            <input type="radio" name="review-state" checked={state === value} onChange={() => setState(value)} />
            <span>
              <b>{label}</b>
              <div className="muted small">{hint}</div>
            </span>
          </label>
        ))}
        {err && <div className="error">{err}</div>}
        <div className="form-actions">
          <button className="btn" onClick={props.onCancel}>
            Cancel
          </button>
          <button className="btn primary" disabled={busy || (state !== "approved" && !body.trim())} onClick={submit}>
            Submit review
          </button>
        </div>
      </div>
    </div>
  );
}
