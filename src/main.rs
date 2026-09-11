use std::{
    env,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
    process,
};

use pulldown_cmark::{Options, Parser, html};
use tiny_http::{Header, Response, Server};

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
            serve_dir(&path, request.url())
        } else {
            serve_file(&path, request.url())
        };
        if let Err(err) = request.respond(response) {
            eprintln!("mdreader: failed to respond: {err}");
        }
    }
}

fn serve_file(path: &Path, url: &str) -> Response<io::Cursor<Vec<u8>>> {
    match url_path(url) {
        "/" => match fs::read_to_string(path) {
            Ok(markdown) => html_response(
                render_page(&display_name(path), &render_markdown(&markdown), None),
                200,
            ),
            Err(err) => text_response(format!("cannot read {}: {err}", path.display()), 500),
        },
        _ => text_response("not found".to_string(), 404),
    }
}

fn serve_dir(root: &Path, url: &str) -> Response<io::Cursor<Vec<u8>>> {
    let tree = match read_tree(root, "") {
        Ok(tree) => tree,
        Err(err) => return text_response(format!("cannot read {}: {err}", root.display()), 500),
    };
    let title = display_name(root);

    let Some(current) = percent_decode(url_path(url)) else {
        return text_response("not found".to_string(), 404);
    };
    let current = current.strip_prefix('/').unwrap_or(&current);

    if current.is_empty() {
        let body = if tree.is_empty() {
            "<p>No markdown files found.</p>\n"
        } else {
            "<p>Select a file from the sidebar.</p>\n"
        };
        let sidebar = render_sidebar(&title, &tree, None);
        return html_response(render_page(&title, body, Some(&sidebar)), 200);
    }

    // Only serve files discovered by the walk, so the URL can never escape the root.
    if !contains_file(&tree, current) {
        return text_response("not found".to_string(), 404);
    }

    let file = root.join(current);
    match fs::read_to_string(&file) {
        Ok(markdown) => {
            let sidebar = render_sidebar(&title, &tree, Some(current));
            html_response(
                render_page(
                    &display_name(&file),
                    &render_markdown(&markdown),
                    Some(&sidebar),
                ),
                200,
            )
        }
        Err(err) => text_response(format!("cannot read {}: {err}", file.display()), 500),
    }
}

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
    Path::new(name).extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
    })
}

fn contains_file(nodes: &[Node], path: &str) -> bool {
    nodes.iter().any(|node| match node {
        Node::Dir { children, .. } => contains_file(children, path),
        Node::File { path: file, .. } => file == path,
    })
}

fn render_sidebar(title: &str, tree: &[Node], current: Option<&str>) -> String {
    let mut out = format!(
        "<nav class=\"sidebar\">\n<a class=\"root\" href=\"/\">{}</a>\n",
        escape_html(title)
    );
    push_tree(&mut out, tree, current);
    out.push_str("</nav>\n");
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

fn render_markdown(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut body = String::new();
    html::push_html(&mut body, parser);
    body
}

/// Fills the `{{title}}`, `{{sidebar}}` and `{{body}}` placeholders of `page.html` in a single
/// pass, so placeholder-like text inside the substituted values is left untouched.
fn render_page(title: &str, body: &str, sidebar: Option<&str>) -> String {
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
