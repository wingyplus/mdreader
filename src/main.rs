use std::{
    collections::HashSet,
    env,
    fmt::Write as _,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process,
};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use tiny_http::{Header, Method, Request, Response, Server};

mod highlight;

const DEFAULT_ADDR: &str = "127.0.0.1:8080";
const PAGE_TEMPLATE: &str = include_str!("page.html");

fn main() {
    let mut positional = Vec::new();
    let mut injected = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--inject=") {
            injected.push(PathBuf::from(value));
        } else if arg == "--inject" {
            let Some(value) = args.next() else {
                eprintln!("mdreader: --inject needs a file");
                process::exit(2);
            };
            injected.push(PathBuf::from(value));
        } else {
            positional.push(arg);
        }
    }

    let mut positional = positional.into_iter();
    let Some(path) = positional.next().map(PathBuf::from) else {
        eprintln!("usage: mdreader <file.md|dir> [addr] [--inject <file.css|file.js>]...");
        process::exit(2);
    };
    let addr = positional
        .next()
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());

    let path = fs::canonicalize(&path).unwrap_or_else(|err| {
        eprintln!("mdreader: cannot read {}: {err}", path.display());
        process::exit(1);
    });
    let is_dir = path.is_dir();

    // Fail before serving, rather than on the first request, if a file cannot be read or holds
    // something that cannot be embedded in the page.
    if let Err(err) = read_assets(&injected) {
        eprintln!("mdreader: {err}");
        process::exit(1);
    }

    let server = Server::http(&addr).unwrap_or_else(|err| {
        eprintln!("mdreader: cannot listen on {addr}: {err}");
        process::exit(1);
    });
    println!("Serving {} at http://{addr}/", path.display());

    for request in server.incoming_requests() {
        let response = if is_dir {
            serve_dir(&path, &injected, &request)
        } else {
            serve_file(&path, &injected, &request)
        };
        if let Err(err) = request.respond(response) {
            eprintln!("mdreader: failed to respond: {err}");
        }
    }
}

fn serve_file(
    path: &Path,
    injected: &[PathBuf],
    request: &Request,
) -> Response<io::Cursor<Vec<u8>>> {
    if url_path(request.url()) != "/" {
        return text_response("not found".to_string(), 404);
    }
    let markdown = match fs::read_to_string(path) {
        Ok(markdown) => markdown,
        Err(err) => return text_response(format!("cannot read {}: {err}", path.display()), 500),
    };
    // Read again for every page, so editing an injected file shows up like editing the markdown.
    let assets = match read_assets(injected) {
        Ok(assets) => assets,
        Err(err) => return text_response(err, 500),
    };
    let etag_of = |markdown: &str| etag((&assets, markdown));
    if *request.method() == Method::Post {
        return toggle_task_response(path, request, &markdown, etag_of);
    }
    page_response(request, etag_of(&markdown), |etag| {
        render_page(
            &display_name(path),
            &render_markdown(&markdown),
            None,
            etag,
            &assets,
        )
    })
}

fn serve_dir(
    root: &Path,
    injected: &[PathBuf],
    request: &Request,
) -> Response<io::Cursor<Vec<u8>>> {
    let tree = match read_tree(root, "") {
        Ok(tree) => tree,
        Err(err) => return text_response(format!("cannot read {}: {err}", root.display()), 500),
    };
    // Read again for every page, so editing an injected file shows up like editing the markdown.
    let assets = match read_assets(injected) {
        Ok(assets) => assets,
        Err(err) => return text_response(err, 500),
    };
    let title = display_name(root);

    let Some(current) = percent_decode(url_path(request.url())) else {
        return text_response("not found".to_string(), 404);
    };
    let current = current.strip_prefix('/').unwrap_or(&current);

    if current.is_empty() {
        return page_response(request, etag((&tree, &assets)), |etag| {
            let body = if tree.is_empty() {
                "<p>No markdown files found.</p>\n"
            } else {
                "<p>Select a file from the sidebar.</p>\n"
            };
            let sidebar = render_sidebar(&title, &tree, None);
            render_page(&title, body, Some(&sidebar), etag, &assets)
        });
    }

    // Only serve files discovered by the walk, so the URL can never escape the root.
    if !contains_file(&tree, current) {
        return text_response("not found".to_string(), 404);
    }

    let file = root.join(current);
    let markdown = match fs::read_to_string(&file) {
        Ok(markdown) => markdown,
        Err(err) => return text_response(format!("cannot read {}: {err}", file.display()), 500),
    };
    let etag_of = |markdown: &str| etag((&tree, &assets, markdown));
    if *request.method() == Method::Post {
        return toggle_task_response(&file, request, &markdown, etag_of);
    }
    page_response(request, etag_of(&markdown), |etag| {
        let sidebar = render_sidebar(&title, &tree, Some(current));
        render_page(
            &display_name(&file),
            &render_markdown(&markdown),
            Some(&sidebar),
            etag,
            &assets,
        )
    })
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

/// Checks or unchecks a task list item of the markdown file at `path`, as asked by a `POST` with
/// `?task=<index>&checked=<true|false>` from the page. `etag_of` gives the page's entity tag for a
/// version of the markdown. The request must name the page it was made on in `If-Match`, so a page
/// that is out of date cannot overwrite newer edits. Browsers only let another website send that
/// header after a CORS preflight, which this server never allows, so no other website can do this.
fn toggle_task_response(
    path: &Path,
    request: &Request,
    markdown: &str,
    etag_of: impl Fn(&str) -> String,
) -> Response<io::Cursor<Vec<u8>>> {
    let url = request.url();
    let index = query_param(url, "task").and_then(|index| index.parse().ok());
    let checked = match query_param(url, "checked") {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    let (Some(index), Some(checked)) = (index, checked) else {
        return text_response(
            "expected ?task=<index>&checked=<true|false>".to_string(),
            400,
        );
    };

    let Some(if_match) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("If-Match"))
    else {
        return text_response("missing If-Match header".to_string(), 428);
    };
    if if_match.value.as_str().trim() != etag_of(markdown) {
        return text_response("the page is out of date".to_string(), 412);
    }

    let Some(updated) = toggle_task(markdown, index, checked) else {
        return text_response(format!("no task {index}"), 400);
    };
    if updated != markdown
        && let Err(err) = fs::write(path, &updated)
    {
        return text_response(format!("cannot write {}: {err}", path.display()), 500);
    }
    Response::from_data(Vec::new())
        .with_status_code(204)
        .with_header(Header::from_bytes("ETag", etag_of(&updated)).expect("valid header"))
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

/// The CSS and JavaScript given with `--inject`, embedded in every page so the reader can be
/// extended — with custom element definitions, for example — without rebuilding.
#[derive(Debug, Default, Hash)]
struct Assets {
    styles: Vec<String>,
    scripts: Vec<String>,
}

impl Assets {
    /// Identifies this set of files, so a page can tell that they changed and reload itself.
    fn fingerprint(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// Reads the files given with `--inject`. A `.css` file becomes a `<style>` and a `.js` file a
/// `<script type="module">`, which lets injected code `import` the way `page.html` does. The files
/// are embedded in the page rather than served, so every URL this server answers stays a markdown
/// file, and their contents are part of the page's entity tag, so editing one reloads the page.
fn read_assets(paths: &[PathBuf]) -> Result<Assets, String> {
    let mut assets = Assets::default();
    for path in paths {
        let content = fs::read_to_string(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let (bucket, closing) = match extension.as_str() {
            "css" => (&mut assets.styles, "</style"),
            "js" => (&mut assets.scripts, "</script"),
            _ => return Err(format!("{}: expected a .css or .js file", path.display())),
        };
        // The file is embedded as written, so a closing tag inside it would end the element early.
        if content.to_lowercase().contains(closing) {
            return Err(format!(
                "{}: cannot embed a file containing `{closing}`",
                path.display()
            ));
        }
        bucket.push(content);
    }
    Ok(assets)
}

fn render_sidebar(title: &str, tree: &[Node], current: Option<&str>) -> String {
    // The hamburger button only shows on narrow screens, where the tree starts collapsed. The
    // resizer after the sidebar only shows on wide screens.
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
    out.push_str(
        "</div>\n</nav>\n\
         <div class=\"sidebar-resizer\" role=\"separator\" aria-orientation=\"vertical\"></div>\n",
    );
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

/// Renders markdown to HTML. Headings get an `id` and a `#` link to themselves. Fenced code blocks
/// whose info string names a supported language are syntax highlighted, `mermaid` blocks are left
/// for `page.html` to draw as diagrams, and other code blocks are rendered as plain text. Task list
/// checkboxes are numbered in document order, the way `toggle_task` counts them.
fn render_markdown(markdown: &str) -> String {
    let mut parser = Parser::new_ext(markdown, Options::all());
    let mut events = Vec::new();
    let mut ids = HashSet::new();
    let mut tasks = 0;
    while let Some(event) = parser.next() {
        if let Event::Start(Tag::Heading {
            level,
            id,
            classes,
            attrs,
        }) = event
        {
            // Headings hold only inline content, so the next heading end tag closes this one.
            let content: Vec<_> = parser
                .by_ref()
                .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
                .collect();
            // An explicit `{#id}` is kept as written; otherwise the id is derived from the text.
            let id = match id {
                Some(id) => {
                    ids.insert(id.to_string());
                    id.to_string()
                }
                None => unique_id(&mut ids, slugify(&content)),
            };
            let anchor = format!(
                "<a class=\"anchor\" href=\"#{}\" aria-label=\"Link to this section\">#</a>",
                escape_html(&id)
            );
            events.push(Event::Start(Tag::Heading {
                level,
                id: Some(id.into()),
                classes,
                attrs,
            }));
            events.extend(content);
            events.push(Event::InlineHtml(anchor.into()));
            events.push(Event::End(TagEnd::Heading(level)));
            continue;
        }

        // The checkbox stays disabled until `page.html` can save it.
        if let Event::TaskListMarker(checked) = event {
            let checked = if checked { " checked=\"\"" } else { "" };
            let html = format!(
                "<input type=\"checkbox\" data-task=\"{tasks}\" disabled=\"\"{checked}/>\n"
            );
            events.push(Event::InlineHtml(html.into()));
            tasks += 1;
            continue;
        }

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

/// Returns `markdown` with its `index`th task list item checked or unchecked, counting items in
/// document order, or `None` if there is no such item. Only the character between the brackets
/// changes, and an item already in the wanted state is left as written.
fn toggle_task(markdown: &str, index: usize, checked: bool) -> Option<String> {
    let (was_checked, range) = Parser::new_ext(markdown, Options::all())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::TaskListMarker(checked) => Some((checked, range)),
            _ => None,
        })
        .nth(index)?;
    let mut updated = markdown.to_string();
    if was_checked != checked {
        // The marker's range can include the whitespace before its `[`.
        let start = range.start;
        let mark = start + markdown[range].find('[')? + 1;
        updated.replace_range(mark..mark + 1, if checked { "x" } else { " " });
    }
    Some(updated)
}

/// Derives a heading id from its text the way GitHub does: lowercased, with ASCII punctuation other
/// than `-` and `_` removed and each space replaced by `-`.
fn slugify(content: &[Event]) -> String {
    let mut slug = String::new();
    for event in content {
        let (Event::Text(text) | Event::Code(text)) = event else {
            continue;
        };
        for c in text.chars() {
            if c == ' ' {
                slug.push('-');
            } else if c == '-' || c == '_' || !(c.is_ascii_punctuation() || c.is_whitespace()) {
                slug.extend(c.to_lowercase());
            }
        }
    }
    slug
}

/// Returns `slug`, or `slug-1`, `slug-2`, ... when it is already taken, and marks it as taken.
fn unique_id(ids: &mut HashSet<String>, slug: String) -> String {
    let base = if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    };
    let mut id = base.clone();
    let mut n = 0;
    while !ids.insert(id.clone()) {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

/// Fills the `{{title}}`, `{{etag}}`, `{{assets}}`, `{{sidebar}}`, `{{styles}}`, `{{scripts}}` and
/// `{{body}}` placeholders of `page.html` in a single pass, so placeholder-like text inside the
/// substituted values — injected JavaScript especially — is left untouched.
fn render_page(
    title: &str,
    body: &str,
    sidebar: Option<&str>,
    etag: &str,
    assets: &Assets,
) -> String {
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
            "assets" => out.push_str(&assets.fingerprint()),
            "sidebar" => out.push_str(sidebar.unwrap_or_default()),
            "styles" => {
                for style in &assets.styles {
                    let _ = write!(out, "<style>\n{style}</style>\n");
                }
            }
            "scripts" => {
                for script in &assets.scripts {
                    let _ = write!(out, "<script type=\"module\">\n{script}</script>\n");
                }
            }
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

/// Returns the undecoded value of the first `name` parameter in the query string of `url`.
fn query_param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    let query = query.split('#').next().unwrap_or_default();
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
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
    fn links_headings_to_themselves() {
        let html = render_markdown(
            "# Hello, `World`!\n## Hello World\n## Custom {#custom}\n### สวัสดี ครับ\n## ?\n",
        );
        assert_eq!(
            html,
            "<h1 id=\"hello-world\">Hello, <code>World</code>!\
             <a class=\"anchor\" href=\"#hello-world\" aria-label=\"Link to this section\">#</a></h1>\n\
             <h2 id=\"hello-world-1\">Hello World\
             <a class=\"anchor\" href=\"#hello-world-1\" aria-label=\"Link to this section\">#</a></h2>\n\
             <h2 id=\"custom\">Custom\
             <a class=\"anchor\" href=\"#custom\" aria-label=\"Link to this section\">#</a></h2>\n\
             <h3 id=\"สวัสดี-ครับ\">สวัสดี ครับ\
             <a class=\"anchor\" href=\"#สวัสดี-ครับ\" aria-label=\"Link to this section\">#</a></h3>\n\
             <h2 id=\"section\">?\
             <a class=\"anchor\" href=\"#section\" aria-label=\"Link to this section\">#</a></h2>\n"
        );
    }

    #[test]
    fn numbers_task_list_checkboxes() {
        let html = render_markdown("- [ ] one\n- [x] two\n");
        assert_eq!(
            html,
            "<ul>\n\
             <li><input type=\"checkbox\" data-task=\"0\" disabled=\"\"/>\none</li>\n\
             <li><input type=\"checkbox\" data-task=\"1\" disabled=\"\" checked=\"\"/>\ntwo</li>\n\
             </ul>\n"
        );
    }

    #[test]
    fn toggles_task_list_items() {
        let markdown = "- [ ] one\n  - [X] two\n\n  text\n\n1. [x] three\n";
        assert_eq!(
            toggle_task(markdown, 0, true).as_deref(),
            Some("- [x] one\n  - [X] two\n\n  text\n\n1. [x] three\n")
        );
        assert_eq!(
            toggle_task(markdown, 1, false).as_deref(),
            Some("- [ ] one\n  - [ ] two\n\n  text\n\n1. [x] three\n")
        );
        assert_eq!(toggle_task(markdown, 1, true).as_deref(), Some(markdown));
        assert_eq!(
            toggle_task(markdown, 2, false).as_deref(),
            Some("- [ ] one\n  - [X] two\n\n  text\n\n1. [ ] three\n")
        );
        assert_eq!(toggle_task(markdown, 3, true), None);
    }

    #[test]
    fn embeds_injected_files_in_the_page() {
        let assets = Assets {
            styles: vec!["my-callout { color: red }\n".to_string()],
            scripts: vec!["customElements.define(\"my-callout\", Callout);\n".to_string()],
        };
        let page = render_page("Doc", "<p>body</p>\n", None, "\"00ff\"", &assets);
        assert!(page.contains("<style>\nmy-callout { color: red }\n</style>"));
        assert!(page.contains(
            "<script type=\"module\">\ncustomElements.define(\"my-callout\", Callout);\n</script>"
        ));
        assert!(page.contains(&format!(
            "<meta name=\"mdreader-assets\" content=\"{}\" />",
            assets.fingerprint()
        )));
    }

    /// Injected JavaScript is substituted in, not scanned, so braces in it are left as written.
    fn injected(script: &str) -> String {
        let assets = Assets {
            styles: Vec::new(),
            scripts: vec![script.to_string()],
        };
        render_page("Doc", "", None, "\"00ff\"", &assets)
    }

    #[test]
    fn leaves_placeholder_like_text_in_injected_code_alone() {
        assert!(injected("const a = {{ x: 1 }};\n").contains("const a = {{ x: 1 }};"));
        assert!(injected("const b = `{{body}}`;\n").contains("const b = `{{body}}`;"));
    }

    #[test]
    fn fingerprints_change_with_the_injected_files() {
        let one = Assets {
            styles: vec!["a{}".to_string()],
            scripts: Vec::new(),
        };
        let two = Assets {
            styles: vec!["b{}".to_string()],
            scripts: Vec::new(),
        };
        assert_ne!(one.fingerprint(), two.fingerprint());
        assert_eq!(one.fingerprint(), one.fingerprint());
    }

    #[test]
    fn rejects_files_it_cannot_embed() {
        let dir = env::temp_dir().join(format!("mdreader-assets-{}", process::id()));
        fs::create_dir_all(&dir).expect("temp dir");

        let other = dir.join("thing.txt");
        fs::write(&other, "x").expect("write");
        assert!(
            read_assets(&[other])
                .unwrap_err()
                .contains("expected a .css or .js file")
        );

        // A closing tag in the file would end the element it is embedded in.
        let breaks_out = dir.join("bad.js");
        fs::write(&breaks_out, "const s = \"</SCRIPT>\";\n").expect("write");
        assert!(read_assets(&[breaks_out]).unwrap_err().contains("</script"));

        let style = dir.join("good.css");
        fs::write(&style, "a { color: red }\n").expect("write");
        let script = dir.join("good.js");
        fs::write(&script, "export const x = 1;\n").expect("write");
        let assets = read_assets(&[style, script]).expect("assets");
        assert_eq!(assets.styles, ["a { color: red }\n"]);
        assert_eq!(assets.scripts, ["export const x = 1;\n"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_query_params() {
        let url = "/a.md?task=3&checked=true#top";
        assert_eq!(query_param(url, "task"), Some("3"));
        assert_eq!(query_param(url, "checked"), Some("true"));
        assert_eq!(query_param(url, "check"), None);
        assert_eq!(query_param("/a.md", "task"), None);
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
