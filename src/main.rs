use std::{env, fs, path::PathBuf, process};

use pulldown_cmark::{Options, Parser, html};
use tiny_http::{Header, Response, Server};

const DEFAULT_ADDR: &str = "127.0.0.1:8080";

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: mdreader <file.md> [addr]");
        process::exit(2);
    };
    let addr = args.next().unwrap_or_else(|| DEFAULT_ADDR.to_string());

    if let Err(err) = fs::metadata(&path) {
        eprintln!("mdreader: cannot read {}: {err}", path.display());
        process::exit(1);
    }

    let server = Server::http(&addr).unwrap_or_else(|err| {
        eprintln!("mdreader: cannot listen on {addr}: {err}");
        process::exit(1);
    });
    println!("Serving {} at http://{addr}/", path.display());

    for request in server.incoming_requests() {
        let response = match request.url() {
            "/" => match fs::read_to_string(&path) {
                Ok(markdown) => html_response(render_page(&path, &markdown), 200),
                Err(err) => text_response(format!("cannot read {}: {err}", path.display()), 500),
            },
            _ => text_response("not found".to_string(), 404),
        };
        if let Err(err) = request.respond(response) {
            eprintln!("mdreader: failed to respond: {err}");
        }
    }
}

fn render_page(path: &PathBuf, markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut body = String::new();
    html::push_html(&mut body, parser);

    let title = path
        .file_name()
        .map(|name| escape_html(&name.to_string_lossy()))
        .unwrap_or_default();

    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
body {{ max-width: 48rem; margin: 2rem auto; padding: 0 1rem; font-family: system-ui, sans-serif; line-height: 1.6; }}
pre {{ background: #f5f5f5; padding: 1rem; overflow-x: auto; }}
code {{ font-family: ui-monospace, monospace; }}
table {{ border-collapse: collapse; }}
th, td {{ border: 1px solid #ccc; padding: 0.25rem 0.5rem; }}
img {{ max-width: 100%; }}
</style>
</head>
<body>
{body}</body>
</html>
"#
    )
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
