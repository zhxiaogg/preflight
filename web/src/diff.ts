// Minimal parser for `git diff` unified output.

export type DiffLine = {
  kind: "add" | "del" | "ctx" | "hunk";
  text: string;
  oldNo?: number;
  newNo?: number;
};

export type FileDiff = {
  oldPath: string;
  newPath: string;
  status: "added" | "deleted" | "renamed" | "modified";
  binary: boolean;
  lines: DiffLine[];
  additions: number;
  deletions: number;
};

export function parseDiff(text: string): FileDiff[] {
  const files: FileDiff[] = [];
  let f: FileDiff | null = null;
  let oldNo = 0;
  let newNo = 0;

  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const m = line.match(/^diff --git a\/(.*) b\/(.*)$/);
      f = {
        oldPath: m?.[1] ?? "",
        newPath: m?.[2] ?? "",
        status: "modified",
        binary: false,
        lines: [],
        additions: 0,
        deletions: 0,
      };
      files.push(f);
      continue;
    }
    if (!f) continue;

    if (line.startsWith("@@")) {
      const m = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      oldNo = Number(m?.[1] ?? 0);
      newNo = Number(m?.[2] ?? 0);
      f.lines.push({ kind: "hunk", text: line });
    } else if (f.lines.length === 0) {
      // File header section, before the first hunk.
      if (line.startsWith("new file mode")) f.status = "added";
      else if (line.startsWith("deleted file mode")) f.status = "deleted";
      else if (line.startsWith("rename from ")) f.status = "renamed";
      else if (line.startsWith("Binary files")) f.binary = true;
    } else if (line.startsWith("+")) {
      f.lines.push({ kind: "add", text: line.slice(1), newNo: newNo++ });
      f.additions++;
    } else if (line.startsWith("-")) {
      f.lines.push({ kind: "del", text: line.slice(1), oldNo: oldNo++ });
      f.deletions++;
    } else if (line.startsWith(" ")) {
      f.lines.push({ kind: "ctx", text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
    }
    // "\ No newline at end of file" and trailing blank lines are ignored.
  }
  return files;
}

export function filePath(f: FileDiff) {
  return f.status === "deleted" ? f.oldPath : f.newPath;
}
