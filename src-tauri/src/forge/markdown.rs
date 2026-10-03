//! Pull request text as HTML (SPEC.md, Pull request: "Markdown written by
//! other people"). Rendered here, in Rust, so the WebView receives markup
//! that cannot run anything or reach the network: raw HTML is reduced to a
//! few harmless tags without attributes and shown as text otherwise, links
//! keep only web and mail addresses, and images become links to themselves,
//! so opening a pull request loads nothing from it.

use pulldown_cmark::{html, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

/// Render Markdown as sanitized HTML.
pub fn render(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(text, options);
    let mut events: Vec<Event<'_>> = Vec::new();
    // An HTML block arrives one line per event; it is sanitized whole so a
    // comment spanning lines is dropped whole.
    let mut html_block: Option<String> = None;
    // Inside a code block, text is code, not prose to find links in.
    let mut in_code = false;
    // Inside a link, text is its label, which gets no links of its own.
    let mut in_link = 0u32;
    // An image or link whose address was refused keeps its text only; its
    // closing tag must then be dropped too.
    let mut dropped_tags: Vec<TagEnd> = Vec::new();
    for event in parser {
        match event {
            Event::Start(Tag::HtmlBlock) => html_block = Some(String::new()),
            Event::End(TagEnd::HtmlBlock) => {
                let sanitized = sanitize_html(&html_block.take().unwrap_or_default());
                if !sanitized.trim().is_empty() {
                    events.push(Event::Html(CowStr::from(sanitized)));
                }
            }
            Event::Html(raw) => match html_block.as_mut() {
                Some(block) => block.push_str(&raw),
                None => events.push(Event::Html(CowStr::from(sanitize_html(&raw)))),
            },
            Event::InlineHtml(raw) => {
                events.push(Event::InlineHtml(CowStr::from(sanitize_html(&raw))))
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                if safe_url(&dest_url) {
                    in_link += 1;
                    events.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url,
                        title,
                        id,
                    }));
                } else {
                    dropped_tags.push(TagEnd::Link);
                }
            }
            Event::End(TagEnd::Link) => {
                if dropped_tags.last() == Some(&TagEnd::Link) {
                    dropped_tags.pop();
                } else {
                    in_link = in_link.saturating_sub(1);
                    events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                if safe_url(&dest_url) {
                    in_link += 1;
                    events.push(Event::Start(Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url,
                        title,
                        id: CowStr::from(""),
                    }));
                    events.push(Event::Text(CowStr::from("Image: ")));
                } else {
                    dropped_tags.push(TagEnd::Image);
                }
            }
            Event::End(TagEnd::Image) => {
                if dropped_tags.last() == Some(&TagEnd::Image) {
                    dropped_tags.pop();
                } else {
                    in_link = in_link.saturating_sub(1);
                    events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code = true;
                events.push(Event::Start(Tag::CodeBlock(kind)));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                events.push(Event::End(TagEnd::CodeBlock));
            }
            Event::Text(text) if !in_code && in_link == 0 => autolink(&text, &mut events),
            other => events.push(other),
        }
    }
    let mut out = String::with_capacity(text.len() * 2);
    html::push_html(&mut out, events.into_iter());
    out
}

/// Addresses a link may keep: the web and mail. Anything else (`javascript:`,
/// `file:`, a relative path on the provider) is shown as text.
fn safe_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

/// Bare web addresses in prose become links, as both providers show them.
fn autolink<'a>(text: &str, events: &mut Vec<Event<'a>>) {
    let mut rest = text;
    while let Some(start) = find_url_start(rest) {
        let (before, from) = rest.split_at(start);
        let end = from
            .find(|c: char| c.is_whitespace() || c == '<' || c == '>')
            .unwrap_or(from.len());
        let (url, after) = from.split_at(end);
        let url = trim_url(url);
        let after_url = &from[url.len()..];
        if !before.is_empty() {
            events.push(Event::Text(CowStr::from(before.to_string())));
        }
        events.push(Event::Start(Tag::Link {
            link_type: LinkType::Autolink,
            dest_url: CowStr::from(url.to_string()),
            title: CowStr::from(""),
            id: CowStr::from(""),
        }));
        events.push(Event::Text(CowStr::from(url.to_string())));
        events.push(Event::End(TagEnd::Link));
        rest = after_url;
        let _ = after;
    }
    if !rest.is_empty() {
        events.push(Event::Text(CowStr::from(rest.to_string())));
    }
}

fn find_url_start(text: &str) -> Option<usize> {
    let a = text.find("https://");
    let b = text.find("http://");
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
    .filter(|&i| {
        // Not the tail of a longer word, such as `xhttp://`.
        i == 0
            || !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric())
    })
}

/// Punctuation that ends a sentence is not part of the address; a closing
/// parenthesis is only when the address opened one.
fn trim_url(url: &str) -> &str {
    let mut end = url.len();
    while end > 0 {
        let last = url[..end].chars().next_back().unwrap();
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => url[..end].matches('(').count() < url[..end].matches(')').count(),
            _ => false,
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    &url[..end]
}

/// Tags kept as they are, when written without attributes: the formatting
/// GitHub and Bitbucket text commonly carries. Everything else is escaped.
const HARMLESS_TAGS: &[&str] = &[
    "details",
    "summary",
    "br",
    "b",
    "i",
    "em",
    "strong",
    "kbd",
    "sub",
    "sup",
    "s",
    "del",
    "ins",
    "u",
    "code",
    "pre",
    "p",
    "div",
    "span",
    "blockquote",
    "hr",
    "ul",
    "ol",
    "li",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
];

/// Raw HTML as safe HTML: comments dropped, harmless tags kept, every other
/// tag and all text escaped.
fn sanitize_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        push_escaped(&mut out, &rest[..lt]);
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            match after.find("-->") {
                Some(end) => rest = &after[end + 3..],
                None => rest = "",
            }
            continue;
        }
        let Some(gt) = rest.find('>') else {
            push_escaped(&mut out, rest);
            rest = "";
            break;
        };
        let tag = &rest[..=gt];
        if harmless_tag(tag) {
            out.push_str(&tag.to_ascii_lowercase());
        } else {
            push_escaped(&mut out, tag);
        }
        rest = &rest[gt + 1..];
    }
    push_escaped(&mut out, rest);
    out
}

/// `<b>`, `</b>`, `<br/>`, `<details open>`: a listed name and no attributes.
fn harmless_tag(tag: &str) -> bool {
    let inner = tag
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .trim();
    let inner = inner.strip_prefix('/').unwrap_or(inner).trim();
    let lower = inner.to_ascii_lowercase();
    let (name, attrs) = match lower.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (lower.as_str(), ""),
    };
    HARMLESS_TAGS.contains(&name) && (attrs.is_empty() || (name == "details" && attrs == "open"))
}

fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_and_handlers_become_text() {
        let html = render("Hi <script>alert(1)</script> there\n\n<img src=x onerror=alert(1)>\n");
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{html}"
        );
        assert!(
            html.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "{html}"
        );
    }

    #[test]
    fn links_keep_only_web_and_mail_addresses() {
        let html = render("[a](https://example.com) [b](javascript:alert(1)) [c](mailto:x@y.z)");
        assert!(
            html.contains(r#"<a href="https://example.com">a</a>"#),
            "{html}"
        );
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains(">b<") || html.contains("b "), "{html}");
        assert!(html.contains(r#"<a href="mailto:x@y.z">c</a>"#), "{html}");
    }

    #[test]
    fn images_become_links_and_load_nothing() {
        let html = render("![shot](https://example.com/a.png)");
        assert!(!html.contains("<img"), "{html}");
        assert!(
            html.contains(r#"<a href="https://example.com/a.png">Image: shot</a>"#),
            "{html}"
        );
    }

    #[test]
    fn harmless_html_stays_and_comments_go() {
        let html = render(
            "<details open>\n<summary>More</summary>\n\n<!-- a\nmulti-line comment -->\nText <b>bold</b> <b onclick=x>no</b>\n\n</details>\n",
        );
        assert!(html.contains("<details open>"), "{html}");
        assert!(html.contains("<summary>More</summary>"), "{html}");
        assert!(!html.contains("comment"), "{html}");
        assert!(html.contains("<b>bold</b>"), "{html}");
        assert!(html.contains("&lt;b onclick=x&gt;no"), "{html}");
        assert!(html.contains("</details>"), "{html}");
    }

    #[test]
    fn bare_addresses_become_links_outside_code() {
        let html = render("See https://example.com/x). Not `https://code.example` nor xhttp://a.\n\n```\nhttps://block.example\n```\n");
        assert!(
            html.contains(r#"<a href="https://example.com/x">https://example.com/x</a>)."#),
            "{html}"
        );
        assert!(!html.contains(r#"href="https://code.example"#), "{html}");
        assert!(!html.contains(r#"href="https://block.example"#), "{html}");
        assert!(!html.contains(r#"href="http://a"#), "{html}");
    }

    #[test]
    fn github_flavored_tables_tasks_and_strikethrough_render() {
        let html = render("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] not\n\n~~gone~~\n");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("checked"), "{html}");
        assert!(html.contains("<del>gone</del>"), "{html}");
    }
}
