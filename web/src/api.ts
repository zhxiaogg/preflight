export type Pr = {
  id: number;
  repo: string;
  base: string;
  head: string;
  title: string;
  description: string;
  created_at: number;
};

export type PrDetail = {
  pr: Pr;
  base_sha: string;
  head_sha: string;
  commits: { sha: string; subject: string }[];
};

export type Side = "old" | "new";

export type Comment = {
  id: number;
  pr_id: number;
  parent_id: number | null;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  side: Side | null;
  line_text: string | null;
  resolved: boolean;
  created_at: number;
};

export type NewComment = {
  body: string;
  parent_id?: number;
  path?: string;
  line?: number;
  side?: Side;
  line_text?: string;
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
  getDiff: (id: number) => req<{ diff: string }>(`/api/prs/${id}/diff`),
  listComments: (id: number) => req<Comment[]>(`/api/prs/${id}/comments`),
  addComment: (id: number, c: NewComment) =>
    req<Comment>(`/api/prs/${id}/comments`, { method: "POST", body: JSON.stringify(c) }),
  resolve: (commentId: number, resolved: boolean) =>
    req(`/api/comments/${commentId}/resolve`, { method: "POST", body: JSON.stringify({ resolved }) }),
};
