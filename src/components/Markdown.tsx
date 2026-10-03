import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef } from "react";

/**
 * Pull request text, rendered and sanitized in Rust (`forge::markdown`):
 * raw HTML reduced to harmless tags, links kept to the web and mail, images
 * as links. Links open in the browser, never in the WebView.
 */
export function Markdown({
  html,
  className = "",
}: {
  html: string;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onClick = (e: MouseEvent) => {
      const link = (e.target as HTMLElement).closest("a");
      if (!link) return;
      e.preventDefault();
      const href = link.getAttribute("href") ?? "";
      if (/^(https?|mailto):/i.test(href)) void openUrl(href).catch(() => {});
    };
    el.addEventListener("click", onClick);
    return () => el.removeEventListener("click", onClick);
  }, []);
  return (
    <div
      ref={ref}
      className={`md selectable ${className}`}
      // biome-ignore lint/security/noDangerouslySetInnerHtml: the HTML comes from forge::markdown in Rust, which escapes raw HTML and keeps only web links; nothing from the provider reaches here unsanitized.
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
