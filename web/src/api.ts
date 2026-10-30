export type Pr = {
  id: number;
  repo: string;
  base: string;
  head: string;
  title: string;
  description: string;
  state: "open" | "closed";
  created_at: number;
};

export type ReviewState = "approved" | "changes_requested" | "commented";

export type Review = {
  id: number;
  pr_id: number;
  author: string;
  state: ReviewState;
  body: string;
  created_at: number;
};

export type PrDetail = {
  pr: Pr;
  base_sha: string;
  head_sha: string;
  /** The branch was deleted; the PR is shown at the last commit seen. */
  base_missing: boolean;
  head_missing: boolean;
  /** Newest first. `oid` is the full id, `sha` the short one, `time` the committer time. */
  commits: { oid: string; sha: string; subject: string; time: number }[];
  /** Closing, reopening and title/base changes, oldest first. */
  activity: Activity[];
};

export type Activity = {
  id: number;
  kind: "pr_closed" | "pr_reopened" | "pr_edited";
  actor: string;
  /** For pr_edited: the changed fields' old and new values. */
  data: Partial<Record<"title" | "base", { old: string; new: string }>>;
  created_at: number;
};

export type Side = "old" | "new";

/** A range of the PR's commits by full id, `from` being the oldest; equal for a single commit. */
export type CommitRange = { from: string; to: string };

export type Comment = {
  id: number;
  pr_id: number;
  parent_id: number | null;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  side: Side | null;
  /** First line of a multi-line comment (`line` is the last). */
  start_line: number | null;
  start_side: Side | null;
  line_text: string | null;
  resolved: boolean;
  created_at: number;
  updated_at: number | null;
  /** Part of the reviewer's review in progress; agents see it once the review is submitted. */
  pending: boolean;
  /** The review that published this comment. */
  review_id: number | null;
};

export type NewComment = {
  body: string;
  parent_id?: number;
  path?: string;
  line?: number;
  side?: Side;
  start_line?: number;
  start_side?: Side;
  line_text?: string;
  pending?: boolean;
};

async function req<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    ...init,
    headers: { "Content-Type": "application/json" },
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error ?? res.statusText);
  return data as T;
}

export const api = {
  listPrs: () => req<Pr[]>("/api/prs"),
  getPr: (id: number) => req<PrDetail>(`/api/prs/${id}`),
  /** Whole-PR diff, or the combined diff of commits `from`..`to` (inclusive). */
  getDiff: (id: number, range?: CommitRange) =>
    req<{ diff: string }>(`/api/prs/${id}/diff${range ? `?from=${range.from}&to=${range.to}` : ""}`),
  getFile: (id: number, path: string, rev?: string) =>
    req<{ content: string }>(
      `/api/prs/${id}/file?path=${encodeURIComponent(path)}${rev ? `&rev=${encodeURIComponent(rev)}` : ""}`,
    ),
  listComments: (id: number) => req<Comment[]>(`/api/prs/${id}/comments`),
  addComment: (id: number, c: NewComment) =>
    req<Comment>(`/api/prs/${id}/comments`, { method: "POST", body: JSON.stringify(c) }),
  editPr: (id: number, changes: Partial<Pick<Pr, "title" | "description" | "base">>) =>
    req<Pr>(`/api/prs/${id}`, { method: "PATCH", body: JSON.stringify(changes) }),
  listBranches: (id: number) => req<{ branches: string[] }>(`/api/prs/${id}/branches`),
  listReviews: (id: number) => req<Review[]>(`/api/prs/${id}/reviews`),
  submitReview: (id: number, state: ReviewState, body: string) =>
    req<Review>(`/api/prs/${id}/reviews`, { method: "POST", body: JSON.stringify({ state, body }) }),
  setState: (id: number, state: Pr["state"]) =>
    req(`/api/prs/${id}/state`, { method: "POST", body: JSON.stringify({ state }) }),
  editComment: (commentId: number, body: string) =>
    req<Comment>(`/api/comments/${commentId}`, { method: "PATCH", body: JSON.stringify({ body }) }),
  deleteComment: (commentId: number) => req(`/api/comments/${commentId}`, { method: "DELETE" }),
  resolve: (commentId: number, resolved: boolean) =>
    req(`/api/comments/${commentId}/resolve`, { method: "POST", body: JSON.stringify({ resolved }) }),
};
