//! Syntax highlighting for fenced code blocks, powered by tree-sitter.

use std::sync::LazyLock;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, Highlighter, HtmlRenderer};

/// Capture names reported by the highlighter. A capture such as `function.method` falls back to
/// its longest listed prefix, and each name is rendered as an `hl-*` CSS class styled in
/// `page.html` (dots become dashes).
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "keyword",
    "label",
    "module",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation.special",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable.builtin",
    "variable.parameter",
];

struct Grammar {
    aliases: &'static [&'static str],
    config: HighlightConfiguration,
}

static GRAMMARS: LazyLock<Vec<Grammar>> = LazyLock::new(|| {
    use tree_sitter_javascript as js;
    use tree_sitter_typescript as ts;

    vec![
        grammar(
            &["bash", "sh", "shell", "zsh"],
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        grammar(
            &["c", "h"],
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        grammar(
            &["cpp", "c++", "cc", "cxx", "hpp"],
            tree_sitter_cpp::LANGUAGE.into(),
            &[
                tree_sitter_cpp::HIGHLIGHT_QUERY,
                tree_sitter_c::HIGHLIGHT_QUERY,
            ]
            .concat(),
            "",
            "",
        ),
        grammar(
            &["css"],
            tree_sitter_css::LANGUAGE.into(),
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        grammar(
            &["elixir", "ex", "exs"],
            tree_sitter_elixir::LANGUAGE.into(),
            tree_sitter_elixir::HIGHLIGHTS_QUERY,
            tree_sitter_elixir::INJECTIONS_QUERY,
            "",
        ),
        grammar(
            &["go", "golang"],
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        grammar(
            &["html", "htm"],
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
            tree_sitter_html::INJECTIONS_QUERY,
            "",
        ),
        grammar(
            &["java"],
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        grammar(
            &["javascript", "js", "jsx", "mjs", "cjs"],
            js::LANGUAGE.into(),
            &[js::HIGHLIGHT_QUERY, js::JSX_HIGHLIGHT_QUERY].concat(),
            js::INJECTIONS_QUERY,
            js::LOCALS_QUERY,
        ),
        grammar(
            &["json"],
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        grammar(
            &["python", "py", "python3"],
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        grammar(
            &["ruby", "rb"],
            tree_sitter_ruby::LANGUAGE.into(),
            tree_sitter_ruby::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_ruby::LOCALS_QUERY,
        ),
        grammar(
            &["rust", "rs"],
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        grammar(
            &["toml"],
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        // TypeScript extends the JavaScript grammar, so its queries build on JavaScript's. The
        // TypeScript patterns come first so they take precedence.
        grammar(
            &["typescript", "ts", "mts", "cts"],
            ts::LANGUAGE_TYPESCRIPT.into(),
            &[ts::HIGHLIGHTS_QUERY, js::HIGHLIGHT_QUERY].concat(),
            js::INJECTIONS_QUERY,
            &[ts::LOCALS_QUERY, js::LOCALS_QUERY].concat(),
        ),
        grammar(
            &["tsx"],
            ts::LANGUAGE_TSX.into(),
            &[
                ts::HIGHLIGHTS_QUERY,
                js::HIGHLIGHT_QUERY,
                js::JSX_HIGHLIGHT_QUERY,
            ]
            .concat(),
            js::INJECTIONS_QUERY,
            &[ts::LOCALS_QUERY, js::LOCALS_QUERY].concat(),
        ),
        grammar(
            &["yaml", "yml"],
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    ]
});

fn grammar(
    aliases: &'static [&'static str],
    language: Language,
    highlights: &str,
    injections: &str,
    locals: &str,
) -> Grammar {
    let mut config =
        HighlightConfiguration::new(language, aliases[0], highlights, injections, locals)
            .unwrap_or_else(|err| panic!("invalid {} highlight query: {err}", aliases[0]));
    config.configure(HIGHLIGHT_NAMES);
    Grammar { aliases, config }
}

/// Looks up a grammar by a code block language name such as `rust` or `rs`, ignoring case.
fn find(name: &str) -> Option<&'static HighlightConfiguration> {
    GRAMMARS
        .iter()
        .find(|grammar| {
            grammar
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
        })
        .map(|grammar| &grammar.config)
}

/// Renders `code` as escaped HTML with `<span class="hl-*">` tokens, or returns `None` when the
/// language is not supported.
pub fn highlight(lang: &str, code: &str) -> Option<String> {
    let config = find(lang)?;
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, code.as_bytes(), None, None, |name| find(name))
        .ok()?;
    let mut renderer = HtmlRenderer::new();
    renderer
        .render(events, code.as_bytes(), &|highlight, out| {
            out.extend_from_slice(b"class=\"hl-");
            out.extend(
                HIGHLIGHT_NAMES[highlight.0]
                    .bytes()
                    .map(|b| if b == b'.' { b'-' } else { b }),
            );
            out.push(b'"');
        })
        .ok()?;
    Some(renderer.lines().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_grammar_queries_compile() {
        assert!(!GRAMMARS.is_empty());
    }

    #[test]
    fn highlights_supported_languages() {
        let html = highlight("RS", "fn main() {}\n").unwrap();
        assert!(
            html.contains("<span class=\"hl-keyword\">fn</span>"),
            "{html}"
        );

        // Injected languages are highlighted, and source text is escaped.
        let html = highlight("html", "<script>let x = 1 < 2;</script>\n").unwrap();
        assert!(
            html.contains("<span class=\"hl-keyword\">let</span>"),
            "{html}"
        );
        assert!(html.contains("&lt;"), "{html}");

        assert_eq!(highlight("unknown", "x\n"), None);
    }
}
