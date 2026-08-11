//! Markdown → sanitized HTML for post bodies.
//!
//! The pipeline is: `pulldown-cmark` parses (never trusts) the source, then
//! `ammonia` strips everything outside a strict allowlist before the HTML is
//! ever served. Post bodies are user content — this module is the single
//! enforcement point between a writer's markdown and the reader's DOM.
//!
//! The allowlist mirrors the documented editorial grammar: text formatting,
//! headings (used for the reader's table of contents), lists, links, code,
//! blockquotes, tables. Links are force-rewritten to `rel="noopener
//! noreferrer"` and `target="_blank"` so outbound links never hand the new
//! context a handle on the reader window.

use ammonia::Builder;
use pulldown_cmark::{html, Options, Parser};
use std::collections::{HashMap, HashSet};

/// Render markdown to sanitized HTML. Empty input yields empty output.
pub fn render(markdown: &str) -> String {
    if markdown.trim().is_empty() {
        return String::new();
    }
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(markdown, options);
    let mut raw = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut raw, parser);

    let mut builder = Builder::default();
    builder
        .add_tags([
            "p",
            "br",
            "hr",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "ul",
            "ol",
            "li",
            "blockquote",
            "pre",
            "code",
            "em",
            "strong",
            "del",
            "a",
            "img",
            "table",
            "thead",
            "tbody",
            "tr",
            "th",
            "td",
            "span",
            "input",
            "sup",
            "sub",
        ])
        .add_generic_attributes(["id", "class"])
        .tag_attributes(HashMap::from([
            ("a", HashSet::from(["href", "title", "target"])),
            ("img", HashSet::from(["src", "alt", "title"])),
            ("code", HashSet::from(["class"])),
            ("input", HashSet::from(["type", "checked", "disabled"])),
        ]))
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank");
    builder.clean(&raw).to_string()
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn plain_text_is_passthrough() {
        let html = render("Hello **world**");
        assert!(html.contains("<strong>world</strong>"));
        assert!(html.contains("<p>"));
    }

    #[test]
    fn headings_survive_for_toc() {
        let html = render("## Chapter one\n\nBody text.\n\n### Chapter two");
        assert!(html.contains("<h2>Chapter one</h2>"));
        assert!(html.contains("<h3>Chapter two</h3>"));
    }

    #[test]
    fn scripts_and_event_handlers_are_stripped() {
        let html = render("<script>alert(1)</script>\n\n## Safe\n\n<img src=x onerror=alert(1)>");
        assert!(!html.contains("<script"), "script tags never survive");
        assert!(!html.contains("onerror"), "event handlers never survive");
        assert!(
            !html.contains("alert("),
            "javascript payloads never survive"
        );
        assert!(html.contains("<h2>Safe</h2>"));
    }

    #[test]
    fn javascript_urls_are_removed() {
        let html = render("[click](javascript:alert(1))");
        assert!(
            !html.contains("javascript:"),
            "javascript: hrefs are dropped"
        );
    }

    #[test]
    fn links_gain_noopener() {
        let html = render("[rust](https://rust-lang.org)");
        assert!(html.contains("rel=\"noopener noreferrer\""));
        assert!(html.contains("target=\"_blank\""));
    }

    #[test]
    fn tables_and_tasklists_render() {
        let html = render("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] todo");
        assert!(html.contains("<table>"));
        assert!(html.contains("checked"));
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn fuzzish_garbage_never_panics() {
        let samples = [
            "</textarea><script>alert(1)</script>",
            "[a](data:text/html;base64,PHNjcmlwdD4=)",
            "<!--[if IE]><script>alert(1)</script><![endif]-->",
            "<math><mtext><table><mglyph><style><!--</style><img title=\"--><img src=1 onerror=alert(1)>\">",
            "\u{0000}\u{0008}\u{001b} weird \u{fffe} chars",
            "####### not a heading #######\n\n`unclosed",
            &"a".repeat(50_000),
        ];
        for sample in samples {
            let html = render(sample);
            assert!(!html.contains("<script"), "no script ever survives");
            assert!(!html.contains("javascript:"), "no javascript: url survives");
        }
    }
}
