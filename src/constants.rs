#![allow(dead_code)]

pub const VERSION: &str = "4.1.0";
pub const VERSION_MAJOR: u32 = 4;
pub const VERSION_MINOR: u32 = 1;
pub const VERSION_POINT: u32 = 0;
pub const CODE_STATUS: &str = "stable";
pub const HOME_PAGE_URL: &str = "https://www.privoxy.org/";

pub const BUFFER_SIZE: usize = 5000;
pub const CGI_PARAM_LEN_MAX: usize = 500;
pub const HOSTENT_BUFFER_SIZE: usize = 2048;
pub const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8118";

pub const HTTP_OK: &str = "HTTP/1.1 200 OK\r\n";
pub const HTTP_BAD_REQUEST: &str = "HTTP/1.1 400 Bad Request\r\n";
pub const HTTP_NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\n";
pub const HTTP_CONNECT_OK: &str = "HTTP/1.1 200 Connection established\r\n\r\n";

pub const DEFAULT_CONFIG_FILE: &str = if cfg!(windows) {
    "config.txt"
} else {
    "config"
};

pub const CONNECTION_TIMEOUT_SECS: u64 = 300;
pub const KEEP_ALIVE_TIMEOUT_SECS: u64 = 55;

pub const MAX_CONNECTIONS: usize = 1000;

pub use crate::errlog::{
    LOG_LEVEL_REQUEST, LOG_LEVEL_CONNECT, LOG_LEVEL_TAGGING,
    LOG_LEVEL_HEADER, LOG_LEVEL_WRITING, LOG_LEVEL_FORCE,
    LOG_LEVEL_RE_FILTER, LOG_LEVEL_REDIRECTS, LOG_LEVEL_DEANIMATE,
    LOG_LEVEL_CLF, LOG_LEVEL_CRUNCH, LOG_LEVEL_CGI, LOG_LEVEL_INFO,
    LOG_LEVEL_ERROR, LOG_LEVEL_FATAL, LOG_LEVEL_RECEIVED, LOG_LEVEL_ACTIONS,
};

pub const ANCHOR_LEFT: u32 = 1;
pub const ANCHOR_RIGHT: u32 = 2;

pub const CSP_FLAG_CHUNKED: u32 = 0x0001;
pub const CSP_FLAG_CONTENT_LENGTH_SET: u32 = 0x0002;
pub const CSP_FLAG_MODIFIED: u32 = 0x0004;
pub const CSP_FLAG_CLIENT_CONNECTION_KEEP_ALIVE: u32 = 0x0008;
pub const CSP_FLAG_SERVER_CONNECTION_KEEP_ALIVE: u32 = 0x0010;
pub const CSP_FLAG_SSL_TUNNEL: u32 = 0x0020;
