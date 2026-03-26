use thiserror::Error;

#[derive(Error, Debug)]
pub enum PrivoxyError {
    #[error("Memory allocation failed")]
    Memory,

    #[error("CGI parameters error: {0}")]
    CgiParams(String),

    #[error("File error: {0}")]
    File(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Modified error")]
    Modified,

    #[error("Compression error: {0}")]
    Compress(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network error: {0}")]
    Network(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Filter error: {0}")]
    Filter(String),

    #[error("SSL/TLS error: {0}")]
    Ssl(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Timeout error")]
    Timeout,

    #[error("Too many connections")]
    TooManyConnections,

    #[error("Unknown error")]
    Unknown,

    #[error("{0}")]
    Other(String),
}

pub type PrivoxyResult<T> = Result<T, PrivoxyError>;

impl From<PrivoxyError> for std::io::Error {
    fn from(err: PrivoxyError) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, err.to_string())
    }
}

impl From<regex::Error> for PrivoxyError {
    fn from(err: regex::Error) -> Self {
        PrivoxyError::Filter(format!("Regex error: {}", err))
    }
}

impl From<url::ParseError> for PrivoxyError {
    fn from(err: url::ParseError) -> Self {
        PrivoxyError::InvalidUrl(err.to_string())
    }
}

impl From<httparse::Error> for PrivoxyError {
    fn from(err: httparse::Error) -> Self {
        PrivoxyError::Http(format!("HTTP parse error: {:?}", err))
    }
}
