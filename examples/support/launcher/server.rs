//! Loopback-only asset server owned by the browser launcher.
use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "browser root must be a directory"));
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}/", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::clone(&stop);
        let worker = thread::Builder::new().name("example-assets".into()).spawn(move || {
            while !shutdown.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // A malformed/disconnected request must not stop the listener.
                        let _ = serve(&mut stream, &root);
                        // Deliver errors before closing a socket with unread request data.
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(10)));
                        let deadline = Instant::now() + Duration::from_millis(50);
                        let mut discard = [0; 1024];
                        for _ in 0..64 {
                            if Instant::now() >= deadline {
                                break;
                            }
                            match stream.read(&mut discard) {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {}
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { url, stop, worker: Some(worker) })
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8], head: bool) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    if !head {
        stream.write_all(body)?;
    }
    Ok(())
}

fn serve(stream: &mut TcpStream, root: &Path) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let mut headers = Vec::new();
    let started = Instant::now();
    let end = loop {
        if let Some(end) = headers.windows(4).position(|part| part == b"\r\n\r\n") {
            break end;
        }
        if headers.len() == 8192 {
            return respond(stream, "431 Request Header Fields Too Large", "text/plain", b"Headers too large", false);
        }
        if started.elapsed() > Duration::from_millis(750) {
            return respond(stream, "408 Request Timeout", "text/plain", b"Request timeout", false);
        }
        let mut buffer = [0; 1024];
        let available = buffer.len().min(8192 - headers.len());
        match stream.read(&mut buffer[..available]) {
            Ok(0) => return Ok(()),
            Ok(count) => headers.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    };
    let request = std::str::from_utf8(&headers[..end]).ok();
    let parts: Vec<_> = request.and_then(|value| value.split("\r\n").next()).unwrap_or("").split_whitespace().collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return respond(stream, "400 Bad Request", "text/plain", b"Bad request", false);
    }
    let head = parts[0] == "HEAD";
    if !matches!(parts[0], "GET" | "HEAD") {
        return respond(stream, "405 Method Not Allowed", "text/plain", b"Use GET or HEAD", false);
    }
    let Some(path) = asset_path(root, parts[1]) else {
        return respond(stream, "404 Not Found", "text/plain", b"Not found", head);
    };
    let mime = match path.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => return respond(stream, "404 Not Found", "text/plain", b"Not found", head),
    };
    match fs::read(path) {
        Ok(body) => respond(stream, "200 OK", mime, &body, head),
        Err(_) => respond(stream, "404 Not Found", "text/plain", b"Not found", head),
    }
}

fn asset_path(root: &Path, target: &str) -> Option<PathBuf> {
    let raw = target.split('?').next()?;
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    let decoded = std::str::from_utf8(&decoded).ok()?;
    if !decoded.starts_with('/') || decoded.contains(['\\', '\0', '%']) {
        return None;
    }
    let relative = if decoded == "/" { "index.html" } else { decoded.strip_prefix('/')? };
    if relative.split('/').any(|segment| matches!(segment, "" | "." | "..")) {
        return None;
    }
    if relative != "index.html" && !relative.starts_with("ui/") && !relative.starts_with("pkg/") {
        return None;
    }
    let path = root.join(relative).canonicalize().ok()?;
    let resolved = path.strip_prefix(root).ok()?;
    let public = resolved == Path::new("index.html") || resolved.starts_with("ui") || resolved.starts_with("pkg");
    if public && path.is_file() {
        Some(path)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        net::{Shutdown, TcpStream},
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("contime-server-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(root.join("ui")).unwrap();
            fs::create_dir_all(root.join("pkg")).unwrap();
            for (path, value) in [
                ("index.html", "dashboard"),
                ("ui/app.js", "javascript"),
                ("ui/style.css", "styles"),
                ("pkg/demo.wasm", "wasm"),
                ("Cargo.toml", "private build"),
                ("secret.rs", "private source"),
                ("secret.js", "private javascript"),
            ] {
                fs::write(root.join(path), value).unwrap();
            }
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn request(server: &Server, request: &str) -> String {
        let mut stream = TcpStream::connect(server.url().trim_start_matches("http://").trim_end_matches('/')).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    #[test]
    fn serves_only_public_files_with_browser_content_types() {
        let fixture = Fixture::new();
        let server = Server::start(&fixture.0).unwrap();
        for (path, mime, body) in [
            ("/", "text/html", "dashboard"),
            ("/index.html?q=1", "text/html", "dashboard"),
            ("/ui/app.js", "text/javascript", "javascript"),
            ("/ui/style.css", "text/css", "styles"),
            ("/pkg/demo.wasm", "application/wasm", "wasm"),
        ] {
            let response = request(&server, &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"));
            assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
            assert!(response.contains(&format!("Content-Type: {mime}")), "{response}");
            assert!(response.ends_with(body), "{response}");
        }
        for path in [
            "/Cargo.toml",
            "/secret.rs",
            "/ui/missing.js",
            "/../Cargo.toml",
            "/ui/../../Cargo.toml",
            "/ui/%2e%2e/%2e%2e/Cargo.toml",
            "/ui/%2E%2E/app.js",
            "/ui%2f..%2fCargo.toml",
            "/ui/%252e%252e/secret.rs",
        ] {
            let response = request(&server, &format!("GET {path} HTTP/1.1\r\n\r\n"));
            assert!(!response.starts_with("HTTP/1.1 200"), "{path}: {response}");
            assert!(!response.contains("private"));
        }
    }
    #[test]
    fn head_has_length_without_body_and_bad_requests_do_not_stop_server() {
        let fixture = Fixture::new();
        let server = Server::start(&fixture.0).unwrap();
        let response = request(&server, "HEAD / HTTP/1.1\r\n\r\n");
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Content-Length: 9\r\n"));
        assert!(response.ends_with("\r\n\r\n"));
        assert!(request(&server, "POST / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 405"));
        assert!(request(&server, "invalid\r\n\r\n").starts_with("HTTP/1.1 400"));
        let mut dropped = TcpStream::connect(server.url().trim_start_matches("http://").trim_end_matches('/')).unwrap();
        dropped.write_all(b"GET /").unwrap();
        dropped.shutdown(Shutdown::Both).unwrap();
        let oversized = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "x".repeat(9000));
        let response = request(&server, &oversized);
        assert!(response.starts_with("HTTP/1.1 431"), "{response:?}");
        assert!(request(&server, "GET / HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 200"));
    }
    #[test]
    fn accepts_headers_split_across_tcp_writes() {
        let fixture = Fixture::new();
        let server = Server::start(&fixture.0).unwrap();
        let mut stream = TcpStream::connect(server.url().trim_start_matches("http://").trim_end_matches('/')).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(b"GET /ui/app.js HTTP/1.1\r\nHost: local").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        stream.write_all(b"host\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("javascript"), "{response}");
    }
    fn drop_and_check_worker(server: Server) {
        // Only Server and its worker own strong references to this flag. Observe
        // their lifetime without probing a port which another listener may reuse.
        let stopped = Arc::downgrade(&server.stop);
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(stopped.upgrade().is_none(), "server worker still owns its shutdown state");
    }
    #[test]
    fn dropping_server_releases_worker_and_idle_connection_promptly() {
        let fixture = Fixture::new();
        let server = Server::start(&fixture.0).unwrap();
        let address = server.url().trim_start_matches("http://").trim_end_matches('/').to_owned();
        let mut idle = TcpStream::connect(&address).unwrap();
        idle.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        drop_and_check_worker(server);
        // This connection belongs to the original server even if its listening
        // port has since been reused. Accepted clients see EOF; an unaccepted
        // connection may be reset when the listening socket is dropped.
        let mut byte = [0];
        match idle.read(&mut byte) {
            Ok(0) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            result => panic!("idle connection survived server shutdown: {result:?}"),
        }
    }
    #[test]
    fn dropping_server_without_clients_releases_worker_promptly() {
        let fixture = Fixture::new();
        drop_and_check_worker(Server::start(&fixture.0).unwrap());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_to_private_files_inside_or_outside_root() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::os::unix::fs::symlink(outside.0.join("secret.rs"), fixture.0.join("ui/escape.js")).unwrap();
        std::os::unix::fs::symlink(fixture.0.join("secret.js"), fixture.0.join("ui/internal.js")).unwrap();
        let server = Server::start(&fixture.0).unwrap();
        assert!(request(&server, "GET /ui/escape.js HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(request(&server, "GET /ui/internal.js HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }
}
