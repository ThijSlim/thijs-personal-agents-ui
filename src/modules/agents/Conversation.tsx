import { useEffect, useRef } from "react";

/**
 * An agent's recent output as plain text. A `log` that screen readers don't
 * read out on every change (output can update constantly); it is focusable so
 * keyboard users can scroll it. Keeps the view pinned to the bottom unless the
 * user scrolled up.
 */
export function Conversation({ text }: { text: string }) {
  const ref = useRef<HTMLPreElement>(null);
  const pinned = useRef(true);

  useEffect(() => {
    const el = ref.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [text]);

  return (
    <pre
      ref={ref}
      role="log"
      aria-live="off"
      aria-label="Conversation"
      tabIndex={0}
      className="conversation"
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      {text}
    </pre>
  );
}
