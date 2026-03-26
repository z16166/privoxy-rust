#![allow(dead_code)]

use std::collections::HashMap;

use bytes::{Bytes, BytesMut};
use httparse;
use url::Url;

use crate::compression::COMPRESSION_LEVEL_DEFAULT;
use crate::error::{PrivoxyError, PrivoxyResult};

#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// Original command line
    pub cmd: String,
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Original URL string
    pub url: String,
    /// Parsed URL
    pub parsed_url: Option<Url>,
    /// HTTP version
    pub version: String,
    /// Headers
    pub headers: HashMap<String, String>,
    /// Request body
    pub body: Option<Bytes>,
    /// Target host
    pub host: String,
    /// Target port
    pub port: u16,
    /// Is HTTPS
    pub is_ssl: bool,
    /// Path
    pub path: String,
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            cmd: String::new(),
            method: String::new(),
            url: String::new(),
            parsed_url: None,
            version: String::new(),
            headers: HashMap::new(),
            body: None,
            host: String::new(),
            port: 80,
            is_ssl: false,
            path: String::new(),
        }
    }

    pub fn parse(data: &[u8]) -> PrivoxyResult<Self> {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);

        let status = req.parse(data)
            .map_err(|e| PrivoxyError::Http(format!("Failed to parse HTTP request: {:?}", e)))?;

        if status.is_partial() {
            return Err(PrivoxyError::Http("Incomplete HTTP request".to_string()));
        }

        let mut request = Self::new();

        if let Some(method) = req.method {
            request.method = method.to_string();
        }

        if let Some(path) = req.path {
            request.path = path.to_string();
            request.url = path.to_string();
        }

        if let Some(version) = req.version {
            request.version = format!("HTTP/1.{}", version);
        }

        // Parse headers
        for header in req.headers.iter() {
            if let Ok(name) = std::str::from_utf8(header.name.as_bytes()) {
                if let Ok(value) = std::str::from_utf8(header.value) {
                    request.headers.insert(name.to_string(), value.to_string());

                    // Extract host from Host header
                    if name.eq_ignore_ascii_case("Host") {
                        request.host = value.to_string();
                        if let Some(colon_pos) = value.find(':') {
                            request.host = value[..colon_pos].to_string();
                            request.port = value[colon_pos + 1..].parse().unwrap_or(80);
                        }
                    }
                }
            }
        }

        // Check if it's a CONNECT request (HTTPS)
        request.is_ssl = request.method == "CONNECT";
        if request.is_ssl {
            // For CONNECT, the request-URI contains host:port (e.g., "youtube.com:443").
            // This is the authoritative source, following the C code's parse_http_url().
            // httparse stores the request-URI in path.
            let connect_target = request.path.clone();
            if let Some(colon_pos) = connect_target.rfind(':') {
                request.host = connect_target[..colon_pos].to_string();
                request.port = connect_target[colon_pos + 1..].parse().unwrap_or(443);
            } else {
                request.host = connect_target;
                request.port = 443;
            }
        }

        // Parse body if present
        let header_len = status.unwrap();
        if data.len() > header_len {
            request.body = Some(Bytes::copy_from_slice(&data[header_len..]));
        }

        Ok(request)
    }

    pub fn get_header(&self, name: &str) -> Option<&String> {
        self.headers.get(name)
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.insert(name.to_string(), value.to_string());
    }

    pub fn remove_header(&mut self, name: &str) {
        self.headers.remove(name);
    }

    pub fn is_get(&self) -> bool {
        self.method == "GET"
    }

    pub fn is_post(&self) -> bool {
        self.method == "POST"
    }

    pub fn to_bytes(&self) -> Bytes {
        let mut result = format!("{} {} {}\r\n", self.method, self.path, self.version);

        for (name, value) in &self.headers {
            result.push_str(&format!("{}: {}\r\n", name, value));
        }

        result.push_str("\r\n");

        let mut bytes = BytesMut::from(result.as_bytes());

        if let Some(ref body) = self.body {
            bytes.extend_from_slice(body);
        }

        bytes.freeze()
    }
}

impl Default for HttpRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP version
    pub version: String,
    /// Status code
    pub status_code: u16,
    /// Status text
    pub status_text: String,
    /// Headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: Vec<u8>,
    /// Compression level
    pub compression_level: u8,
}

impl HttpResponse {
    pub fn new() -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status_code: 200,
            status_text: "OK".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
            compression_level: COMPRESSION_LEVEL_DEFAULT,
        }
    }

    pub fn with_status(status_code: u16, status_text: &str) -> Self {
        let mut response = Self::new();
        response.status_code = status_code;
        response.status_text = status_text.to_string();
        response
    }

    pub fn parse(data: &[u8]) -> PrivoxyResult<Self> {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut resp = httparse::Response::new(&mut headers);

        let status = resp.parse(data)
            .map_err(|e| PrivoxyError::Http(format!("Failed to parse HTTP response: {:?}", e)))?;

        if status.is_partial() {
            return Err(PrivoxyError::Http("Incomplete HTTP response".to_string()));
        }

        let mut response = Self::new();

        if let Some(code) = resp.code {
            response.status_code = code;
        }

        if let Some(reason) = resp.reason {
            response.status_text = reason.to_string();
        }

        if let Some(version) = resp.version {
            response.version = format!("HTTP/1.{}", version);
        }

        // Parse headers
        for header in resp.headers.iter() {
            if let Ok(name) = std::str::from_utf8(header.name.as_bytes()) {
                if let Ok(value) = std::str::from_utf8(header.value) {
                    response.headers.insert(name.to_string(), value.to_string());
                }
            }
        }

        // Parse body if present
        let header_len = status.unwrap();
        if data.len() > header_len {
            response.body = data[header_len..].to_vec();
        }

        Ok(response)
    }

    pub fn set_body(&mut self, body: Vec<u8>) {
        self.body = body;
        self.headers.insert("Content-Length".to_string(), self.body.len().to_string());
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.insert(name.to_string(), value.to_string());
    }

    pub fn to_bytes(&self) -> Bytes {
        let mut result = format!("{} {} {}\r\n", self.version, self.status_code, self.status_text);

        for (name, value) in &self.headers {
            result.push_str(&format!("{}: {}\r\n", name, value));
        }

        result.push_str("\r\n");

        let mut bytes = BytesMut::from(result.as_bytes());
        bytes.extend_from_slice(&self.body);
        bytes.freeze()
    }
}

impl Default for HttpResponse {
    fn default() -> Self {
        Self::new()
    }
}

pub fn create_error_response(status_code: u16, message: &str) -> HttpResponse {
    let status_text = match status_code {
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };

    let body = format!(
        "<html>\n<head><title>{} {}</title></head>\n<body>\n<h1>{} {}</h1>\n<p>{}</p>\n</body>\n</html>",
        status_code, status_text, status_code, status_text, message
    );

    let mut response = HttpResponse::with_status(status_code, status_text);
    response.set_header("Content-Type", "text/html; charset=utf-8");
    response.set_body(body.into_bytes());
    response
}

pub fn create_connect_response() -> HttpResponse {
    HttpResponse::with_status(200, "Connection established")
}
