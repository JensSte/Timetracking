import { useLayoutEffect, useRef } from "react";
import type { TextareaHTMLAttributes } from "react";

/** The one comment field of the app (ADR-0044): a multi-line textarea that grows with its
 *  content. Enter inserts a line break; Cmd/Ctrl+Enter blurs the field, which is how the
 *  running timer's comment gets saved without a button. */
export function CommentTextarea({
  value,
  onKeyDown,
  ...rest
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { value: string }) {
  const ref = useRef<HTMLTextAreaElement>(null);

  // Reset to `auto` first so the field also shrinks when text is deleted; the CSS
  // `max-height` caps the growth and turns the overflow into a scrollbar.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);

  return (
    <textarea
      {...rest}
      ref={ref}
      className="comment-input"
      rows={4}
      value={value}
      onKeyDown={(e) => {
        onKeyDown?.(e);
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) e.currentTarget.blur();
      }}
    />
  );
}
