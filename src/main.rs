use std::{
    env,
    fmt::Write as _,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process,
};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use tiny_http::{Header, Request, Response, Server};

mod highlight;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";
const PAGE_TEMPLATE: &str = include_str!("page.html");

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: mdreader <file.md|dir> [addr]");
        process::exit(2);
    };
    let addr = args.next().unwrap_or_else(|| DEFAULT_ADDR.to_string());

    let path = fs::canonicalize(&path).unwrap_or_else(|err| {
        eprintln!("mdreader: cannot read {}: {err}", path.display());
        process::exit(1);
    });
    let is_dir = path.is_dir();

    let server = Server::http(&addr).unwrap_or_else(|err| {
        eprintln!("mdreader: cannot listen on {addr}: {err}");
        process::exit(1);
    });
    println!("Serving {} at http://{addr}/", path.display());

    for request in server.incoming_requests() {
        let response = if is_dir {
            serve_dir(&path, &request)
        } else {
            serve_file(&path, &request)
        };
        if let Err(err) = request.respond(response) {
            eprintln!("mdreader: failed to respond: {err}");
        }
    }
}

fn serve_file(path: &Path, request: &Request) -> Response<io::Cursor<Vec<u8>>> {
    match url_path(request.url()) {
        "/" => match fs::read_to_string(path) {
            Ok(markdown) => page_response(request, etag(&markdown), |etag| {
                render_page(&display_name(path), &render_markdown(&markdown), None, etag)
            }),
            Err(err) => text_response(format!("cannot read {}: {err}", path.display()), 500),
        },
        _ => text_response("not found".to_string(), 404),
    }
}

fn serve_dir(root: &Path, request: &Request) -> Response<io::Cursor<Vec<u8>>> {
    let tree = match read_tree(root, "") {
        Ok(tree) => tree,
        Err(err) => return text_response(format!("cannot read {}: {err}", root.display()), 500),
    };
    let title = display_name(root);

    let Some(current) = percent_decode(url_path(request.url())) else {
        return text_response("not found".to_string(), 404);
    };
    let current = current.strip_prefix('/').unwrap_or(&current);

    if current.is_empty() {
        return page_response(request, etag(&tree), |etag| {
            let body = if tree.is_empty() {
                "<p>No markdown files found.</p>\n"
            } else {
                "<p>Select a file from the sidebar.</p>\n"
            };
            let sidebar = render_sidebar(&title, &tree, None);
            render_page(&title, body, Some(&sidebar), etag)
        });
    }

    // Only serve files discovered by the walk, so the URL can never escape the root.
    if !contains_file(&tree, current) {
        return text_response("not found".to_string(), 404);
    }

    let file = root.join(current);
    match fs::read_to_string(&file) {
        Ok(markdown) => page_response(request, etag((&tree, &markdown)), |etag| {
            let sidebar = render_sidebar(&title, &tree, Some(current));
            render_page(
                &display_name(&file),
                &render_markdown(&markdown),
                Some(&sidebar),
                etag,
            )
        }),
        Err(err) => text_response(format!("cannot read {}: {err}", file.display()), 500),
    }
}

/// Responds with the page built by `render`, which embeds `etag` so the browser can poll for
/// changes. A request that already has this version gets 304 Not Modified without rendering.
fn page_response(
    request: &Request,
    etag: String,
    render: impl FnOnce(&str) -> String,
) -> Response<io::Cursor<Vec<u8>>> {
    let unchanged = request
        .headers()
        .iter()
        .filter(|header| header.field.equiv("If-None-Match"))
        .any(|header| etag_matches(header.value.as_str(), &etag));
    let response = if unchanged {
        Response::from_data(Vec::new()).with_status_code(304)
    } else {
        html_response(render(&etag), 200)
    };
    response.with_header(Header::from_bytes("ETag", etag).expect("valid header"))
}

/// Identifies a page version by hashing everything it is rendered from.
fn etag(source: impl Hash) -> String {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    format!("\"{:016x}\"", hasher.finish())
}

/// Checks an `If-None-Match` header value, a comma-separated list of entity tags or `*`.
fn etag_matches(if_none_match: &str, etag: &str) -> bool {
    if_none_match.split(',').any(|tag| {
        let tag = tag.trim();
        tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag
    })
}

#[derive(Hash)]
enum Node {
    Dir { name: String, children: Vec<Node> },
    File { name: String, path: String },
}

impl Node {
    fn sort_key(&self) -> (bool, &str) {
        match self {
            Node::Dir { name, .. } => (false, name),
            Node::File { name, .. } => (true, name),
        }
    }
}

/// Walks `root/dir` for markdown files. Hidden entries and symlinks are skipped, and
/// directories without any markdown files are omitted. Paths are relative to `root`.
fn read_tree(root: &Path, dir: &str) -> io::Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir(root.join(dir))? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let path = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}/{name}")
        };
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let children = read_tree(root, &path).unwrap_or_default();
            if !children.is_empty() {
                nodes.push(Node::Dir { name, children });
            }
        } else if file_type.is_file() && is_markdown(&name) {
            nodes.push(Node::File { name, path });
        }
    }
    nodes.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(nodes)
}

fn is_markdown(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

fn contains_file(nodes: &[Node], path: &str) -> bool {
    nodes.iter().any(|node| match node {
        Node::Dir { children, .. } => contains_file(children, path),
        Node::File { path: file, .. } => file == path,
    })
}

fn render_sidebar(title: &str, tree: &[Node], current: Option<&str>) -> String {
    // The hamburger button only shows on narrow screens, where the tree starts collapsed.
    let mut out = format!(
        "<nav class=\"sidebar\">\n<div class=\"sidebar-header\">\n\
         <button class=\"menu-toggle\" type=\"button\" aria-label=\"Toggle file list\" \
         aria-controls=\"sidebar-tree\" aria-expanded=\"false\">\
         <svg width=\"20\" height=\"20\" viewBox=\"0 0 20 20\" aria-hidden=\"true\">\
         <path d=\"M3 5h14M3 10h14M3 15h14\" stroke=\"currentColor\" stroke-width=\"2\" \
         stroke-linecap=\"round\"/></svg></button>\n\
         <a class=\"root\" href=\"/\">{}</a>\n\
         </div>\n<div id=\"sidebar-tree\" class=\"sidebar-tree\">\n",
        escape_html(title)
    );
    push_tree(&mut out, tree, current);
    out.push_str("</div>\n</nav>\n");
    out
}

fn push_tree(out: &mut String, nodes: &[Node], current: Option<&str>) {
    out.push_str("<ul>\n");
    for node in nodes {
        match node {
            Node::Dir { name, children } => {
                let _ = writeln!(out, "<li><span class=\"dir\">{}/</span>", escape_html(name));
                push_tree(out, children, current);
                out.push_str("</li>\n");
            }
            Node::File { name, path } => {
                let class = if current == Some(path.as_str()) {
                    " class=\"active\""
                } else {
                    ""
                };
                let _ = writeln!(
                    out,
                    "<li><a{class} href=\"/{}\">{}</a></li>",
                    percent_encode(path),
                    escape_html(name)
                );
            }
        }
    }
    out.push_str("</ul>\n");
}

/// Renders markdown to HTML. Fenced code blocks whose info string names a supported language are
/// syntax highlighted, `mermaid` blocks are left for `page.html` to draw as diagrams, and other
/// code blocks are rendered as plain text.
fn render_markdown(markdown: &str) -> String {
    let mut parser = Parser::new_ext(markdown, Options::all());
    let mut events = Vec::new();
    while let Some(event) = parser.next() {
        let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) = &event else {
            events.push(event);
            continue;
        };
        let Some(lang) = info.split_whitespace().next() else {
            events.push(event);
            continue;
        };

        // A code block contains only text events up to its end tag.
        let mut code = String::new();
        for event in parser.by_ref() {
            match event {
                Event::Text(text) => code.push_str(&text),
                _ => break,
            }
        }

        // The diagram source stays readable as text until the browser renders it.
        if lang.eq_ignore_ascii_case("mermaid") {
            let html = format!("<pre class=\"mermaid\">{}</pre>\n", escape_html(&code));
            events.push(Event::Html(html.into()));
            continue;
        }

        match highlight::highlight(lang, &code) {
            Some(highlighted) => {
                let html = format!(
                    "<pre><code class=\"language-{}\">{highlighted}</code></pre>\n",
                    escape_html(lang)
                );
                events.push(Event::Html(html.into()));
            }
            None => events.extend([
                event,
                Event::Text(code.into()),
                Event::End(TagEnd::CodeBlock),
            ]),
        }
    }

    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    body
}

/// Fills the `{{title}}`, `{{etag}}`, `{{sidebar}}` and `{{body}}` placeholders of `page.html` in
/// a single pass, so placeholder-like text inside the substituted values is left untouched.
fn render_page(title: &str, body: &str, sidebar: Option<&str>, etag: &str) -> String {
    let title = escape_html(title);
    let mut out = String::with_capacity(PAGE_TEMPLATE.len() + body.len());
    let mut rest = PAGE_TEMPLATE;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .expect("unterminated placeholder in page.html");
        match &after[..end] {
            "title" => out.push_str(&title),
            "etag" => out.push_str(&escape_html(etag)),
            "sidebar" => out.push_str(sidebar.unwrap_or_default()),
            "body" => out.push_str(body),
            name => panic!("unknown placeholder `{name}` in page.html"),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn url_path(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or_default()
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_response(body: String, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    with_content_type(
        Response::from_string(body).with_status_code(status),
        "text/html; charset=utf-8",
    )
}

fn text_response(body: String, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    with_content_type(
        Response::from_string(body).with_status_code(status),
        "text/plain; charset=utf-8",
    )
}

fn with_content_type(
    response: Response<std::io::Cursor<Vec<u8>>>,
    content_type: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes("Content-Type", content_type).expect("valid header");
    response.with_header(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_mermaid_blocks_for_the_browser() {
        let html = render_markdown("```Mermaid\ngraph TD\n  A --> B<br>\n```\n");
        assert_eq!(
            html,
            "<pre class=\"mermaid\">graph TD\n  A --&gt; B&lt;br&gt;\n</pre>\n"
        );
    }

    #[test]
    fn matches_if_none_match_etags() {
        let etag = "\"00ff\"";
        assert!(etag_matches("\"00ff\"", etag));
        assert!(etag_matches("W/\"00ff\"", etag));
        assert!(etag_matches("\"abcd\", \"00ff\"", etag));
        assert!(etag_matches("*", etag));
        assert!(!etag_matches("\"abcd\"", etag));
        assert!(!etag_matches("00ff", etag));
    }
}
