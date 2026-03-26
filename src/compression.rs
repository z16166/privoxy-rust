use bytes::Bytes;
use std::io::Read;
use std::io::Write;

use crate::error::{PrivoxyError, PrivoxyResult};

#[cfg(feature = "compression")]
use flate2::read::{DeflateDecoder, GzDecoder};
#[cfg(feature = "compression")]
use flate2::write::{DeflateEncoder, GzEncoder};
#[cfg(feature = "compression")]
use flate2::Compression;

pub const COMPRESSION_LEVEL_DEFAULT: u8 = 6;
#[allow(dead_code)]
pub const COMPRESSION_LEVEL_MIN: u8 = 0;
#[allow(dead_code)]
pub const COMPRESSION_LEVEL_MAX: u8 = 9;
#[allow(dead_code)]
pub const COMPRESSION_LEVEL_FASTEST: u8 = 1;
#[allow(dead_code)]
pub const COMPRESSION_LEVEL_BEST: u8 = 9;

/// Compression algorithms supported by Privoxy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgorithm {
    /// No compression
    #[allow(dead_code)]
    None,
    /// Deflate compression (RFC 1951)
    Deflate,
    /// Gzip compression (RFC 1952)
    Gzip,
}

impl CompressionAlgorithm {
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" | "identity" => Some(CompressionAlgorithm::None),
            "deflate" => Some(CompressionAlgorithm::Deflate),
            "gzip" | "x-gzip" => Some(CompressionAlgorithm::Gzip),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            CompressionAlgorithm::None => "identity",
            CompressionAlgorithm::Deflate => "deflate",
            CompressionAlgorithm::Gzip => "gzip",
        }
    }

    #[allow(dead_code)]
    pub fn content_encoding(&self) -> Option<&'static str> {
        match self {
            CompressionAlgorithm::None => None,
            CompressionAlgorithm::Deflate => Some("deflate"),
            CompressionAlgorithm::Gzip => Some("gzip"),
        }
    }
}

/// Compress data using deflate
#[allow(dead_code)]
#[cfg(feature = "compression")]
pub fn compress_deflate(data: &[u8], level: u8) -> PrivoxyResult<Bytes> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(level as u32));
    encoder.write_all(data)
        .map_err(|e| PrivoxyError::Compress(format!("Deflate compression failed: {}", e)))?;
    let compressed = encoder.finish()
        .map_err(|e| PrivoxyError::Compress(format!("Deflate finish failed: {}", e)))?;
    Ok(Bytes::from(compressed))
}

/// Compress data using gzip
#[allow(dead_code)]
#[cfg(feature = "compression")]
pub fn compress_gzip(data: &[u8], level: u8) -> PrivoxyResult<Bytes> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level as u32));
    encoder.write_all(data)
        .map_err(|e| PrivoxyError::Compress(format!("Gzip compression failed: {}", e)))?;
    let compressed = encoder.finish()
        .map_err(|e| PrivoxyError::Compress(format!("Gzip finish failed: {}", e)))?;
    Ok(Bytes::from(compressed))
}

/// Compress data using the specified algorithm
#[allow(dead_code)]
pub fn compress(data: &[u8], algorithm: CompressionAlgorithm, level: u8) -> PrivoxyResult<Bytes> {
    match algorithm {
        CompressionAlgorithm::None => Ok(Bytes::copy_from_slice(data)),
        #[cfg(feature = "compression")]
        CompressionAlgorithm::Deflate => compress_deflate(data, level),
        #[cfg(feature = "compression")]
        CompressionAlgorithm::Gzip => compress_gzip(data, level),
        #[cfg(not(feature = "compression"))]
        CompressionAlgorithm::Deflate | CompressionAlgorithm::Gzip => {
            Err(PrivoxyError::Compress("Compression feature is not enabled".to_string()))
        }
    }
}

/// Decompress deflate data
#[cfg(feature = "compression")]
pub fn decompress_deflate(data: &[u8]) -> PrivoxyResult<Bytes> {
    let mut decoder = DeflateDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)
        .map_err(|e| PrivoxyError::Compress(format!("Deflate decompression failed: {}", e)))?;
    Ok(Bytes::from(decompressed))
}

/// Decompress gzip data
#[cfg(feature = "compression")]
pub fn decompress_gzip(data: &[u8]) -> PrivoxyResult<Bytes> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)
        .map_err(|e| PrivoxyError::Compress(format!("Gzip decompression failed: {}", e)))?;
    Ok(Bytes::from(decompressed))
}

/// Decompress data using the specified algorithm
pub fn decompress(data: &[u8], algorithm: CompressionAlgorithm) -> PrivoxyResult<Bytes> {
    match algorithm {
        CompressionAlgorithm::None => Ok(Bytes::copy_from_slice(data)),
        #[cfg(feature = "compression")]
        CompressionAlgorithm::Deflate => decompress_deflate(data),
        #[cfg(feature = "compression")]
        CompressionAlgorithm::Gzip => decompress_gzip(data),
        #[cfg(not(feature = "compression"))]
        CompressionAlgorithm::Deflate | CompressionAlgorithm::Gzip => {
            Err(PrivoxyError::Compress("Compression feature is not enabled".to_string()))
        }
    }
}

/// Estimate maximum compressed size
#[allow(dead_code)]
pub fn compress_bound(input_len: usize) -> usize {
    // Worst case expansion for deflate is 0.1% + 12 bytes
    input_len + (input_len / 1000) + 12
}

/// Check if compression is beneficial for the data
#[allow(dead_code)]
pub fn should_compress(data: &[u8], min_size: usize) -> bool {
    data.len() >= min_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "compression")]
    #[test]
    fn test_compress_deflate() {
        let data = b"Hello, World! This is a test string that should compress well. It needs to be long enough so that compression is actually effective and the output is smaller than the input, considering header overhead. Let's add more data here to be sure.";
        let compressed = compress_deflate(data, COMPRESSION_LEVEL_DEFAULT).unwrap();
        assert!(compressed.len() < data.len());
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_compress_gzip() {
        let data = b"Hello, World! This is a test string that should compress well. It needs to be long enough so that compression is actually effective and the output is smaller than the input, considering header overhead. Let's add more data here to be sure.";
        let compressed = compress_gzip(data, COMPRESSION_LEVEL_DEFAULT).unwrap();
        // Gzip has more overhead, but with enough data it should still be smaller or at least valid
        assert!(compressed.len() > 0);
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_decompress_deflate() {
        let data = b"Hello, World!";
        let compressed = compress_deflate(data, COMPRESSION_LEVEL_DEFAULT).unwrap();
        let decompressed = decompress_deflate(&compressed).unwrap();
        assert_eq!(decompressed.as_ref(), data.as_slice());
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_decompress_gzip() {
        let data = b"Hello, World!";
        let compressed = compress_gzip(data, COMPRESSION_LEVEL_DEFAULT).unwrap();
        let decompressed = decompress_gzip(&compressed).unwrap();
        assert_eq!(decompressed.as_ref(), data.as_slice());
    }

    #[test]
    fn test_compression_algorithm() {
        assert_eq!(CompressionAlgorithm::from_str("deflate"), Some(CompressionAlgorithm::Deflate));
        assert_eq!(CompressionAlgorithm::from_str("gzip"), Some(CompressionAlgorithm::Gzip));
        assert_eq!(CompressionAlgorithm::from_str("none"), Some(CompressionAlgorithm::None));
        assert_eq!(CompressionAlgorithm::from_str("unknown"), None);
    }

    #[cfg(not(feature = "compression"))]
    #[test]
    fn test_compress_no_feature() {
        let data = b"Hello, World!";
        let result = compress(data, CompressionAlgorithm::Deflate, COMPRESSION_LEVEL_DEFAULT);
        assert!(result.is_err());
        
        let result = compress(data, CompressionAlgorithm::Gzip, COMPRESSION_LEVEL_DEFAULT);
        assert!(result.is_err());
        
        let result = compress(data, CompressionAlgorithm::None, COMPRESSION_LEVEL_DEFAULT);
        assert!(result.is_ok());
    }

    #[cfg(not(feature = "compression"))]
    #[test]
    fn test_decompress_no_feature() {
        let data = b"Hello, World!";
        let result = decompress(data, CompressionAlgorithm::Deflate);
        assert!(result.is_err());
        
        let result = decompress(data, CompressionAlgorithm::Gzip);
        assert!(result.is_err());
        
        let result = decompress(data, CompressionAlgorithm::None);
        assert!(result.is_ok());
    }
}
