import DOMPurify from "dompurify";
import { marked } from "marked";
import { useMemo } from "react";

/** GitHub-flavored markdown for PR descriptions and comments; agent-written text is sanitized. */
export function Markdown({ text }: { text: string }) {
  const html = useMemo(
    () => DOMPurify.sanitize(marked.parse(text, { gfm: true, breaks: true, async: false }) as string),
    [text],
  );
  return <div className="markdown" dangerouslySetInnerHTML={{ __html: html }} />;
}
