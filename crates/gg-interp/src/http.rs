// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `http` module: a blocking HTTP client, via the
//! `ureq` crate (3.x) from crates.io.
//!
//! `import http` gives `http.get(url)` /
//! `http.get(url, headers)`, `http.post(url, body)` /
//! `http.post(url, body, content_type)` and
//! `http.request(method, url, body, headers)`.
//!
//! Every call returns a Dict with three keys: `status` (the
//! HTTP status code as an Int), `headers` (a Dict of the
//! response headers, keys lowercased, values Str; a repeated
//! header keeps the last value) and `body` (the response body
//! as a Str). An HTTP error status — 404, 500, ... — is NOT
//! an error: it comes back in the Dict with its status and
//! body, like any serious HTTP library. Only failures to
//! perform the request are runtime errors: an invalid URL, a
//! host that cannot be reached, a refused connection, a
//! timeout (30 seconds for the whole exchange, connect and
//! read included).
//!
//! The body is bytes on the wire and a Str in GG, so it is
//! decoded with `from_utf8_lossy`: bytes that are not valid
//! UTF-8 are replaced with U+FFFD instead of failing the
//! request. `post` sends its body as UTF-8 bytes; its
//! `content_type` defaults to `"text/plain"`.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::time::Duration;

use crate::error::RuntimeError;
use crate::value::{DictData, DictKey, Value};

/// Timeout for a whole exchange — DNS, connect, TLS, sending
/// the request and reading the response body — 30 seconds.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The default Content-Type for `post` when none is given.
const DEFAULT_CONTENT_TYPE: &str = "text/plain";

/// The agent for every request: a 30-second global timeout,
/// error statuses returned as responses instead of errors
/// (`http_status_as_error(false)`), and TLS verified against
/// the platform trust store (`PlatformVerifier`) — not just
/// the bundled Mozilla roots — so it also works behind a
/// proxy whose CA is installed in the system store. A fresh
/// agent per call: agents are cheap, and sharing one would
/// share a connection pool across unrelated GG programs.
fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

/// A Str argument, or a clear error naming the function.
fn as_str(name: &str, value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        other => Err(RuntimeError::new(format!(
            "{name}() expects a Str, not {}",
            other.type_name()
        ))),
    }
}

/// A Dict of Str to Str as request headers, or a clear error
/// naming the function. Keys and values must both be Str.
fn as_headers(name: &str, value: &Value) -> Result<Vec<(String, String)>, RuntimeError> {
    let Value::Dict(dict) = value else {
        return Err(RuntimeError::new(format!(
            "{name}() headers must be a Dict, not {}",
            value.type_name()
        )));
    };
    let mut headers = Vec::new();
    for (key, item) in dict.borrow().entries.iter() {
        let DictKey::Str(key) = key else {
            return Err(RuntimeError::new(format!(
                "{name}() header names must be Str, not {}",
                key.to_value().type_name()
            )));
        };
        let Value::Str(item) = item else {
            return Err(RuntimeError::new(format!(
                "{name}() header values must be Str, not {}",
                item.type_name()
            )));
        };
        headers.push((key.clone(), item.clone()));
    }
    Ok(headers)
}

/// A ureq failure as a GG runtime error, naming the function
/// and, for the common cases, the situation in plain words.
fn map_error(name: &str, err: ureq::Error) -> RuntimeError {
    let detail = match &err {
        ureq::Error::Timeout(_) => "request timed out (30s limit)".to_string(),
        ureq::Error::HostNotFound => "host not found".to_string(),
        ureq::Error::BadUri(message) => format!("invalid URL: {message}"),
        _ => {
            // `Error::Http` prints with an "http: " prefix from
            // the `http` crate; drop it, it reads as a stutter
            // after the function name.
            let text = err.to_string();
            text.strip_prefix("http: ")
                .map(str::to_string)
                .unwrap_or(text)
        }
    };
    RuntimeError::new(format!("{name}(): {detail}"))
}

/// A ureq response as a GG Dict: `status` (Int), `headers`
/// (Dict, keys lowercased, a repeated header keeps the last
/// value) and `body` (Str, decoded lossily from UTF-8).
fn response_to_value(
    mut response: ureq::http::Response<ureq::Body>,
    name: &str,
) -> Result<Value, RuntimeError> {
    let status = response.status().as_u16() as i64;
    let mut headers = DictData::new();
    for (key, value) in response.headers().iter() {
        headers.insert(
            DictKey::Str(key.as_str().to_lowercase()),
            Value::Str(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        );
    }
    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|err| map_error(name, err))?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let mut dict = DictData::new();
    dict.insert(DictKey::Str("status".to_string()), Value::Int(status));
    dict.insert(
        DictKey::Str("headers".to_string()),
        Value::Dict(Rc::new(RefCell::new(headers))),
    );
    dict.insert(DictKey::Str("body".to_string()), Value::Str(body));
    Ok(Value::Dict(Rc::new(RefCell::new(dict))))
}

/// Set every header on a request builder, in order.
fn with_headers<B>(
    mut builder: ureq::RequestBuilder<B>,
    headers: &[(String, String)],
) -> ureq::RequestBuilder<B> {
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
}

/// `http.get(url)` / `http.get(url, headers)` — a GET, with
/// optional request headers as a Dict of Str to Str.
pub fn get(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let url = as_str("http.get", &args[0])?;
    let headers = match args.get(1) {
        None => Vec::new(),
        Some(value) => as_headers("http.get", value)?,
    };
    let response = with_headers(agent().get(url.as_str()), &headers)
        .call()
        .map_err(|err| map_error("http.get", err))?;
    response_to_value(response, "http.get")
}

/// `http.post(url, body)` / `http.post(url, body,
/// content_type)` — a POST of the body as UTF-8 bytes; the
/// Content-Type defaults to `"text/plain"`.
pub fn post(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let url = as_str("http.post", &args[0])?;
    let body = as_str("http.post", &args[1])?;
    let content_type = match args.get(2) {
        None => DEFAULT_CONTENT_TYPE.to_string(),
        Some(value) => as_str("http.post", value)?,
    };
    let response = agent()
        .post(url.as_str())
        .header("Content-Type", content_type.as_str())
        .send(body.as_bytes())
        .map_err(|err| map_error("http.post", err))?;
    response_to_value(response, "http.post")
}

/// `http.request(method, url, body, headers)` — any method
/// (`"GET"`, `"PUT"`, `"DELETE"`, ...; the case does not
/// matter). `body` is a Str or Nil (Nil means no body) and
/// `headers` is a Dict of Str to Str or Nil. Only POST, PUT
/// and PATCH take a body; a body with any other method is an
/// error, never silently dropped.
pub fn request(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let method = as_str("http.request", &args[0])?.to_ascii_uppercase();
    let url = as_str("http.request", &args[1])?;
    let body = match &args[2] {
        Value::Nil => None,
        Value::Str(s) => Some(s.clone()),
        other => {
            return Err(RuntimeError::new(format!(
                "http.request() body must be a Str or Nil, not {}",
                other.type_name()
            )));
        }
    };
    let headers = match &args[3] {
        Value::Nil => Vec::new(),
        value => as_headers("http.request", value)?,
    };
    if body.is_some() && !matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
        return Err(RuntimeError::new(format!(
            "http.request(): {method} does not take a body"
        )));
    }
    let agent = agent();
    let url = url.as_str();
    let empty: &[u8] = &[];
    let body_bytes: &[u8] = body.as_deref().map(str::as_bytes).unwrap_or(empty);
    let response = match method.as_str() {
        "GET" => with_headers(agent.get(url), &headers).call(),
        "DELETE" => with_headers(agent.delete(url), &headers).call(),
        "HEAD" => with_headers(agent.head(url), &headers).call(),
        "OPTIONS" => with_headers(agent.options(url), &headers).call(),
        "TRACE" => with_headers(agent.trace(url), &headers).call(),
        "POST" => with_headers(agent.post(url), &headers).send(body_bytes),
        "PUT" => with_headers(agent.put(url), &headers).send(body_bytes),
        "PATCH" => with_headers(agent.patch(url), &headers).send(body_bytes),
        _ => {
            return Err(RuntimeError::new(format!(
                "http.request(): unsupported method \"{method}\""
            )));
        }
    }
    .map_err(|err| map_error("http.request", err))?;
    response_to_value(response, "http.request")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    /// Run GG `source` and return its printed output, or the
    /// error text. (Same shape as the helpers in `lib.rs`.)
    fn run_ok(source: &str) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        crate::run_source(source, &mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_err(source: &str) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        let err = crate::run_source(source, &mut input, &mut out).unwrap_err();
        err.to_string()
    }

    /// A tiny HTTP server on 127.0.0.1, serving fixed
    /// responses on a background thread until the process
    /// exits. No internet involved: the tests stay fast and
    /// deterministic.
    struct TestServer {
        base: String,
    }

    fn start_server() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                handle(stream);
            }
        });
        TestServer {
            base: format!("http://127.0.0.1:{port}"),
        }
    }

    /// One request: read the method, path, headers and body,
    /// then write a fixed response for the path.
    fn handle(mut stream: TcpStream) {
        let mut reader = match stream.try_clone() {
            Ok(clone) => BufReader::new(clone),
            Err(_) => return,
        };
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(_) if line == "\r\n" || line.is_empty() => break,
                Ok(_) => {}
                Err(_) => return,
            }
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_lowercase();
                let value = value.trim().to_string();
                if name == "content-length" {
                    content_length = value.parse().unwrap_or(0);
                }
                headers.push((name, value));
            }
        }
        let mut body = vec![0u8; content_length];
        if content_length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        let header = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        let (status, response_headers, response_body): (u16, Vec<(String, String)>, Vec<u8>) =
            match (method.as_str(), path.as_str()) {
                ("GET", "/ok") => (
                    200,
                    vec![
                        ("Content-Type".to_string(), "application/json".to_string()),
                        ("X-Custom".to_string(), "hello".to_string()),
                        ("X-Dup".to_string(), "one".to_string()),
                        ("X-Dup".to_string(), "two".to_string()),
                    ],
                    b"{\"ok\": true}".to_vec(),
                ),
                ("GET", "/notfound") => (404, vec![], b"not here".to_vec()),
                ("POST", "/echo") => (
                    200,
                    vec![(
                        "X-Echo-Content-Type".to_string(),
                        header("content-type").unwrap_or_else(|| "none".to_string()),
                    )],
                    body,
                ),
                ("GET", "/headers") => (
                    200,
                    vec![],
                    header("x-test")
                        .unwrap_or_else(|| "missing".to_string())
                        .into_bytes(),
                ),
                ("GET", "/bytes") => (200, vec![], vec![b'h', b'i', 0xFF, b'!']),
                ("GET", "/method") | ("PUT", "/method") | ("DELETE", "/method") => {
                    (200, vec![], method.into_bytes())
                }
                _ => (404, vec![], b"unknown path".to_vec()),
            };
        let mut out = format!("HTTP/1.1 {status} {}\r\n", reason(status));
        for (name, value) in response_headers {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str(&format!("Content-Length: {}\r\n", response_body.len()));
        out.push_str("Connection: close\r\n\r\n");
        if stream.write_all(out.as_bytes()).is_err() {
            return;
        }
        let _ = stream.write_all(&response_body);
    }

    fn reason(status: u16) -> &'static str {
        match status {
            200 => "OK",
            404 => "Not Found",
            _ => "Unknown",
        }
    }

    #[test]
    fn get_returns_status_headers_and_body() {
        let server = start_server();
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.get(\"{}/ok\")\nprint r[\"status\"]\nprint r[\"body\"]\nprint r[\"headers\"][\"content-type\"]\nprint r[\"headers\"][\"x-custom\"]\nprint r[\"headers\"][\"x-dup\"]\n",
                server.base
            )),
            "200\n{\"ok\": true}\napplication/json\nhello\ntwo\n"
        );
    }

    #[test]
    fn get_with_request_headers() {
        let server = start_server();
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.get(\"{}/headers\", {{\"X-Test\": \"ciao\"}})\nprint r[\"status\"]\nprint r[\"body\"]\n",
                server.base
            )),
            "200\nciao\n"
        );
        // Without the header the server reports "missing".
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.get(\"{}/headers\")\nprint r[\"body\"]\n",
                server.base
            )),
            "missing\n"
        );
    }

    #[test]
    fn error_status_is_returned_not_raised() {
        let server = start_server();
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.get(\"{}/notfound\")\nprint r[\"status\"]\nprint r[\"body\"]\n",
                server.base
            )),
            "404\nnot here\n"
        );
    }

    #[test]
    fn post_echoes_body_and_content_type() {
        let server = start_server();
        // An explicit content type reaches the server.
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.post(\"{}/echo\", \"{{\\\"a\\\": 1}}\", \"application/json\")\nprint r[\"status\"]\nprint r[\"body\"]\nprint r[\"headers\"][\"x-echo-content-type\"]\n",
                server.base
            )),
            "200\n{\"a\": 1}\napplication/json\n"
        );
        // Without one it defaults to text/plain.
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.post(\"{}/echo\", \"ciao\")\nprint r[\"headers\"][\"x-echo-content-type\"]\n",
                server.base
            )),
            "text/plain\n"
        );
    }

    #[test]
    fn request_put_delete_and_lowercase_method() {
        let server = start_server();
        // GG has no `nil` literal: Nil comes from a call that
        // returns it, here `json.parse("null")`.
        assert_eq!(
            run_ok(&format!(
                "import http\nimport json\nlet nil = json.parse(\"null\")\nlet r = http.request(\"PUT\", \"{}/method\", \"dati\", nil)\nprint r[\"status\"]\nprint r[\"body\"]\nlet d = http.request(\"DELETE\", \"{}/method\", nil, nil)\nprint d[\"body\"]\nlet g = http.request(\"get\", \"{}/ok\", nil, nil)\nprint g[\"status\"]\n",
                server.base, server.base, server.base
            )),
            "200\nPUT\nDELETE\n200\n"
        );
    }

    #[test]
    fn request_body_on_get_is_an_error() {
        let server = start_server();
        let err = run_err(&format!(
            "import http\nimport json\nlet nil = json.parse(\"null\")\nprint http.request(\"GET\", \"{}/ok\", \"corpo\", nil)\n",
            server.base
        ));
        assert!(
            err.contains("http.request(): GET does not take a body"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn request_unsupported_method_is_an_error() {
        let server = start_server();
        let err = run_err(&format!(
            "import http\nimport json\nlet nil = json.parse(\"null\")\nprint http.request(\"FOO BAR\", \"{}/ok\", nil, nil)\n",
            server.base
        ));
        assert!(
            err.contains("http.request(): unsupported method"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn invalid_url_is_an_error() {
        assert!(
            run_err("import http\nprint http.get(\"not a url\")\n")
                .contains("http.get(): invalid uri character"),
            "expected an invalid-uri error"
        );
        assert!(
            run_err("import http\nprint http.get(\"ftp://example.com/x\")\n")
                .contains("http.get(): invalid URL: unknown scheme: ftp"),
            "expected an unknown-scheme error"
        );
    }

    #[test]
    fn connection_refused_is_an_error() {
        // Bind a port, then close it: nothing listens there.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = run_err(&format!(
            "import http\nprint http.get(\"http://127.0.0.1:{port}/\")\n"
        ));
        assert!(err.contains("refused"), "unexpected error: {err}");
    }

    #[test]
    fn non_utf8_body_is_lossy() {
        let server = start_server();
        // The 0xFF byte becomes U+FFFD instead of failing.
        assert_eq!(
            run_ok(&format!(
                "import http\nlet r = http.get(\"{}/bytes\")\nprint r[\"body\"]\n",
                server.base
            )),
            "hi\u{FFFD}!\n"
        );
    }

    #[test]
    fn wrong_argument_types_are_errors() {
        let server = start_server();
        // GG has no `nil` literal: Nil comes from a call that
        // returns it, here `json.parse("null")`.
        let nil = "import http\nimport json\nlet nil = json.parse(\"null\")\n";
        assert!(
            run_err("import http\nprint http.get(42)\n")
                .contains("http.get() expects a Str, not Int"),
            "expected a type error for get(42)"
        );
        assert!(
            run_err(&format!(
                "import http\nprint http.get(\"{}/ok\", \"headers\")\n",
                server.base
            ))
            .contains("http.get() headers must be a Dict, not Str"),
            "expected a type error for a non-Dict headers"
        );
        assert!(
            run_err(&format!(
                "import http\nprint http.post(\"{}/echo\", 5)\n",
                server.base
            ))
            .contains("http.post() expects a Str, not Int"),
            "expected a type error for a non-Str post body"
        );
        assert!(
            run_err(&format!(
                "{nil}print http.request(\"GET\", \"{}/ok\", 5, nil)\n",
                server.base
            ))
            .contains("http.request() body must be a Str or Nil, not Int"),
            "expected a type error for a non-Str request body"
        );
        assert!(
            run_err(&format!(
                "{nil}print http.request(\"GET\", \"{}/ok\", nil, {{\"a\": 1}})\n",
                server.base
            ))
            .contains("http.request() header values must be Str, not Int"),
            "expected a type error for a non-Str header value"
        );
        assert!(
            run_err(&format!(
                "{nil}print http.request(\"GET\", \"{}/ok\", nil, {{1: \"x\"}})\n",
                server.base
            ))
            .contains("http.request() header names must be Str, not Int"),
            "expected a type error for a non-Str header name"
        );
    }
}
