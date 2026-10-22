import hljs from "highlight.js/lib/common";
import { FileDiff, filePath } from "./diff";

// Extensions whose highlight.js language name differs from the extension itself.
const LANG_BY_EXT: Record<string, string> = {
  tsx: "typescript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  yml: "yaml",
  toml: "ini",
  h: "c",
  hpp: "cpp",
};

// Highlighting huge files (lockfiles, generated code) is slow and rarely useful.
const MAX_LINES = 5000;

function language(path: string): string | undefined {
  const name = path.split("/").pop() ?? "";
  if (name === "Dockerfile") return "dockerfile";
  if (name === "Makefile") return "makefile";
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  const lang = LANG_BY_EXT[ext] ?? ext;
  return lang && hljs.getLanguage(lang) ? lang : undefined;
}

/** Splits highlight.js HTML into lines, closing and reopening spans that cross line breaks. */
function splitLines(html: string): string[] {
  const lines: string[] = [];
  const open: string[] = [];
  let cur = "";
  for (const tok of html.match(/<span[^>]*>|<\/span>|\n|[^<\n]+/g) ?? []) {
    if (tok === "\n") {
      lines.push(cur + "</span>".repeat(open.length));
      cur = open.join("");
    } else {
      if (tok.startsWith("<span")) open.push(tok);
      else if (tok === "</span>") open.pop();
      cur += tok;
    }
  }
  lines.push(cur);
  return lines;
}

/**
 * Returns highlighted HTML for each line of the file's diff (aligned with `f.lines`), or null
 * when the language is unknown. Each side of a hunk is highlighted as one block so multi-line
 * strings and comments color correctly.
 */
export function highlightFile(f: FileDiff): (string | undefined)[] | null {
  const lang = language(filePath(f));
  if (!lang || f.binary || f.lines.length > MAX_LINES) return null;

  const out: (string | undefined)[] = new Array(f.lines.length);
  let start = 0;
  while (start < f.lines.length) {
    let end = start + 1;
    while (end < f.lines.length && f.lines[end].kind !== "hunk") end++;
    // Old side = context + deletions; new side = context + additions.
    const oldIdx: number[] = [];
    const newIdx: number[] = [];
    for (let i = start; i < end; i++) {
      const k = f.lines[i].kind;
      if (k === "ctx" || k === "del") oldIdx.push(i);
      if (k === "ctx" || k === "add") newIdx.push(i);
    }
    const run = (idx: number[]) => {
      const html = hljs.highlight(idx.map((i) => f.lines[i].text).join("\n"), { language: lang }).value;
      return splitLines(html);
    };
    const oldHtml = run(oldIdx);
    const newHtml = run(newIdx);
    oldIdx.forEach((i, n) => f.lines[i].kind === "del" && (out[i] = oldHtml[n]));
    newIdx.forEach((i, n) => (out[i] = newHtml[n]));
    start = end;
  }
  return out;
}
