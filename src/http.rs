#![allow(dead_code)]


use bytes::{Bytes, BytesMut};
use httparse;
use url::Url;

use crate::compression::COMPRESSION_LEVEL_DEFAULT;
use crate::error::{PrivoxyError, PrivoxyResult};

pub type Headers = Vec<(String, String)>;

/// Get the first header value matching the name (case-insensitive)
pub fn get_header<'a, S: AsRef<str>>(headers: &'a Headers, name: S) -> Option<&'a String> {
    headers.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name.as_ref()))
        .map(|(_, v)| v)
}

/// Get all header values matching the name (case-insensitive)
pub fn get_all_headers<'a, S: AsRef<str>>(headers: &'a Headers, name: S) -> Vec<&'a String> {
    headers.iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name.as_ref()))
        .map(|(_, v)| v)
        .collect()
}

/// Set a header, replacing all existing headers with that name
pub fn set_header<N: AsRef<str>, V: AsRef<str>>(headers: &mut Headers, name: N, value: V) {
    remove_header(headers, name.as_ref());
    headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
}

/// Add a header without removing existing ones
pub fn add_header<N: AsRef<str>, V: AsRef<str>>(headers: &mut Headers, name: N, value: V) {
    headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
}

/// Remove all headers matching the name (case-insensitive)
pub fn remove_header<S: AsRef<str>>(headers: &mut Headers, name: S) {
    headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name.as_ref()));
}

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
    /// Headers (preserved order, supports multiples)
    pub headers: Vec<(String, String)>,
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
            headers: Vec::new(),
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

        // Extract the full request line for logging/tracing (the "cmd")
        if let Some(pos) = data.iter().position(|&b| b == b'\r' || b == b'\n') {
            request.cmd = String::from_utf8_lossy(&data[..pos]).to_string();
        }

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
                    request.headers.push((name.to_string(), value.to_string()));

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

    pub fn get_header<'a, S: AsRef<str>>(&'a self, name: S) -> Option<&'a String> {
        self.headers.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name.as_ref()))
            .map(|(_, v)| v)
    }

    pub fn get_all_headers<'a, S: AsRef<str>>(&'a self, name: S) -> Vec<&'a String> {
        self.headers.iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name.as_ref()))
            .map(|(_, v)| v)
            .collect()
    }

    pub fn set_header<N: AsRef<str>, V: AsRef<str>>(&mut self, name: N, value: V) {
        // Replace all existing headers with this name
        self.remove_header(name.as_ref());
        self.headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
    }

    pub fn add_header<N: AsRef<str>, V: AsRef<str>>(&mut self, name: N, value: V) {
        self.headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
    }

    pub fn remove_header<S: AsRef<str>>(&mut self, name: S) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name.as_ref()));
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
    /// Headers (preserved order, supports multiples)
    pub headers: Vec<(String, String)>,
    /// Response body
    pub body: Vec<u8>,
    /// Compression level
    pub compression_level: u8,
    /// Length of the header section in the raw response
    pub header_len: usize,
}

impl HttpResponse {
    pub fn new() -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status_code: 200,
            status_text: "OK".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            compression_level: COMPRESSION_LEVEL_DEFAULT,
            header_len: 0,
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

        let header_len = status.unwrap();
        let mut response = Self::new();
        response.header_len = header_len;

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
                    response.headers.push((name.to_string(), value.to_string()));
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
        self.set_header("Content-Length", &self.body.len().to_string());
    }

    pub fn header_length(&self) -> usize {
        self.header_len
    }

    pub fn get_header<'a, S: AsRef<str>>(&'a self, name: S) -> Option<&'a String> {
        self.headers.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name.as_ref()))
            .map(|(_, v)| v)
    }

    pub fn set_header<N: AsRef<str>, V: AsRef<str>>(&mut self, name: N, value: V) {
        self.remove_header(name.as_ref());
        self.headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
    }

    pub fn add_header<N: AsRef<str>, V: AsRef<str>>(&mut self, name: N, value: V) {
        self.headers.push((name.as_ref().to_string(), value.as_ref().to_string()));
    }

    pub fn remove_header<S: AsRef<str>>(&mut self, name: S) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name.as_ref()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_order_and_multiples() {
        let mut resp = HttpResponse::new();
        resp.add_header("Set-Cookie", "a=1");
        resp.add_header("X-Custom", "first");
        resp.add_header("Set-Cookie", "b=2");
        resp.add_header("X-Custom", "second");

        assert_eq!(resp.headers.len(), 4);
        
        // Verify order
        assert_eq!(resp.headers[0].0, "Set-Cookie");
        assert_eq!(resp.headers[0].1, "a=1");
        assert_eq!(resp.headers[1].0, "X-Custom");
        assert_eq!(resp.headers[2].0, "Set-Cookie");
        assert_eq!(resp.headers[2].1, "b=2");
        
        // Verify get_all_headers using standalone helper
        let cookies = get_all_headers(&resp.headers, "Set-Cookie");
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0], "a=1");
        assert_eq!(cookies[1], "b=2");

        // Verify set_header replaces all
        set_header(&mut resp.headers, "X-Custom", "replaced");
        assert_eq!(get_all_headers(&resp.headers, "X-Custom").len(), 1);
        assert_eq!(get_header(&resp.headers, "X-Custom").unwrap(), "replaced");
        
        // Verify to_bytes preserves order in output
        let bytes = resp.to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Set-Cookie: a=1\r\n"));
        assert!(s.contains("Set-Cookie: b=2\r\n"));
        
        // Find positions to ensure order
        let pos1 = s.find("Set-Cookie: a=1").unwrap();
        let pos2 = s.find("Set-Cookie: b=2").unwrap();
        assert!(pos1 < pos2);
    }
    
    #[test]
    fn test_request_parsing_preserves_order() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nX-A: 1\r\nX-B: 2\r\nX-A: 3\r\n\r\n";
        let req = HttpRequest::parse(raw).unwrap();
        
        assert_eq!(req.headers.len(), 4);
        assert_eq!(req.headers[0].0, "Host");
        assert_eq!(req.headers[1].0, "X-A");
        assert_eq!(req.headers[1].1, "1");
        assert_eq!(req.headers[2].0, "X-B");
        assert_eq!(req.headers[3].0, "X-A");
        assert_eq!(req.headers[3].1, "3");
    }

    #[test]
    fn test_request_parsing_populates_cmd() {
        let raw = b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let req = HttpRequest::parse(raw).unwrap();
        
        assert_eq!(req.cmd, "GET http://example.com/ HTTP/1.1");
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "http://example.com/");
    }
}
