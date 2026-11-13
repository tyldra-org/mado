//! A tiny HTTP server on 127.0.0.1 for fixture pages, so experiments load
//! real http:// documents without the network. It logs every request line,
//! which experiments use to see what reached the server.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Fixtures {
    port: u16,
    log: Arc<Mutex<Vec<Request>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Request {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Fixtures {
    /// Serves `page(path)` for each request; `None` is a 404.
    pub fn start(page: fn(&str) -> Option<String>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&log);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let _ = serve(stream, page, &log);
                });
            }
        });
        Ok(Fixtures { port, log })
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

fn serve(
    stream: TcpStream,
    page: fn(&str) -> Option<String>,
    log: &Mutex<Vec<Request>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.trim_end().split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
            }
        }
        if let Ok(mut l) = log.lock() {
            l.push(Request {
                path: path.clone(),
                headers,
            });
        }
        let (status, body) = match page(&path) {
            Some(body) => ("200 OK", body),
            None => ("404 Not Found", "not found".to_owned()),
        };
        write!(
            writer,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
    }
}
