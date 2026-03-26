#![allow(dead_code)]

//! URL and HTML encoding module - Ported from encode.c
//! 
//! This module provides functions to encode and decode URLs,
//! and also to encode cookies and HTML text.

use crate::error::{PrivoxyError, PrivoxyResult};

/// Maps special characters in a URL to their equivalent % codes
static URL_CODE_MAP: &[u8; 256] = &{
    let mut map = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        map[i] = match i as u8 {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => 0,
            b'-' | b'_' | b'.' | b'~' => 0,
            _ => 1,
        };
        i += 1;
    }
    map
};

/// Maps special characters in HTML to their equivalent entities
static HTML_CODE_MAP: &[Option<&str>; 256] = &{
    let mut map = [None; 256];
    map[b'"' as usize] = Some("&quot;");
    map[b'&' as usize] = Some("&amp;");
    map[39] = Some("&#39;");  // ASCII code for single quote
    map[b'<' as usize] = Some("&lt;");
    map[b'>' as usize] = Some("&gt;");
    map
};

/// Allowed characters for RFC 3986 URLs
static RFC3986_ALLOWED: &[bool; 128] = &{
    let mut allowed = [false; 128];
    allowed[b'!' as usize] = true;
    allowed[b'#' as usize] = true;
    allowed[b'$' as usize] = true;
    allowed[b'%' as usize] = true;
    allowed[b'&' as usize] = true;
    allowed[b'\'' as usize] = true;
    allowed[b'(' as usize] = true;
    allowed[b')' as usize] = true;
    allowed[b'*' as usize] = true;
    allowed[b'+' as usize] = true;
    allowed[b',' as usize] = true;
    allowed[b'-' as usize] = true;
    allowed[b'.' as usize] = true;
    allowed[b'/' as usize] = true;
    allowed[b':' as usize] = true;
    allowed[b';' as usize] = true;
    allowed[b'=' as usize] = true;
    allowed[b'?' as usize] = true;
    allowed[b'@' as usize] = true;
    allowed[b'[' as usize] = true;
    allowed[b']' as usize] = true;
    allowed[b'_' as usize] = true;
    allowed[b'~' as usize] = true;
    let mut i = b'0' as usize;
    while i <= b'9' as usize {
        allowed[i] = true;
        i += 1;
    }
    let mut i = b'A' as usize;
    while i <= b'Z' as usize {
        allowed[i] = true;
        i += 1;
    }
    let mut i = b'a' as usize;
    while i <= b'z' as usize {
        allowed[i] = true;
        i += 1;
    }
    allowed
};

/// Encode a string for use in HTML (replaces <, >, &, ", ')
pub fn html_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    
    for c in s.chars() {
        if (c as u32) < 256 {
            if let Some(&Some(entity)) = HTML_CODE_MAP.get(c as usize) {
                result.push_str(entity);
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    
    result
}

/// URL encode a string (replaces special characters with %xx codes)
pub fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    
    for c in s.bytes() {
        if URL_CODE_MAP[c as usize] == 1 {
            result.push_str(&format!("%{:02X}", c));
        } else {
            result.push(c as char);
        }
    }
    
    result
}

/// URL decode a string (replaces %xx codes with their decoded form)
pub fn url_decode(s: &str) -> PrivoxyResult<String> {
    let mut result = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    
    while let Some(b) = bytes.next() {
        match b {
            b'+' => result.push(b' '),
            b'%' => {
                let hex: String = bytes.by_ref().take(2).map(|b| b as char).collect();
                if hex.len() == 2 {
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        result.push(byte);
                    } else {
                        // Malformed, use as-is
                        result.push(b'%');
                        result.extend(hex.bytes());
                    }
                } else {
                    // Incomplete hex, use as-is
                    result.push(b'%');
                    result.extend(hex.bytes());
                }
            }
            _ => result.push(b),
        }
    }
    
    String::from_utf8(result)
        .map_err(|_| PrivoxyError::Parse("Invalid UTF-8 in decoded string".to_string()))
}

/// Convert a hex string (2 digits) to integer
pub fn xtoi(s: &str) -> u8 {
    let chars: Vec<char> = s.chars().take(2).collect();
    if chars.len() < 2 {
        return 0;
    }
    
    let high = char_to_hex(chars[0]);
    let low = char_to_hex(chars[1]);
    
    if high.is_none() || low.is_none() {
        return 0;
    }
    
    (high.unwrap() << 4) | low.unwrap()
}

/// Convert a single hex character to its integer value
fn char_to_hex(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        'A'..='F' => Some(c as u8 - b'A' + 10),
        'a'..='f' => Some(c as u8 - b'a' + 10),
        _ => None,
    }
}

/// Percent-encode a string according to RFC 3986
pub fn percent_encode_url(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    
    for c in s.bytes() {
        if (c < 128 && RFC3986_ALLOWED[c as usize]) || c >= 128 {
            result.push(c as char);
        } else {
            result.push_str(&format!("%{:02X}", c));
        }
    }
    
    result
}

/// URL encode a query parameter (for use in query strings)
pub fn url_encode_query_param(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    
    for c in s.bytes() {
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(c as char);
            }
            b' ' => {
                result.push('+');
            }
            _ => {
                result.push_str(&format!("%{:02X}", c));
            }
        }
    }
    
    result
}

/// HTML encode and return the result (convenience function)
pub fn html_encode_string(s: impl AsRef<str>) -> String {
    html_encode(s.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_encode() {
        assert_eq!(html_encode("<script>"), "&lt;script&gt;");
        assert_eq!(html_encode("a & b"), "a &amp; b");
        assert_eq!(html_encode("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(html_encode("it's"), "it&#39;s");
        assert_eq!(html_encode("normal"), "normal");
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("a+b"), "a%2Bb");
        assert_eq!(url_encode("test@email.com"), "test%40email.com");
        assert_eq!(url_encode("normal-text"), "normal-text");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello%20world").unwrap(), "hello world");
        assert_eq!(url_decode("a+b").unwrap(), "a b");
        assert_eq!(url_decode("test%40email.com").unwrap(), "test@email.com");
        assert_eq!(url_decode("normal-text").unwrap(), "normal-text");
    }

    #[test]
    fn test_percent_encode_url() {
        assert_eq!(percent_encode_url("hello world"), "hello%20world");
        assert_eq!(percent_encode_url("http://example.com"), "http://example.com");
        assert_eq!(percent_encode_url("path/to/file"), "path/to/file");
        assert_eq!(percent_encode_url("file name.txt"), "file%20name.txt");
    }

    #[test]
    fn test_xtoi() {
        assert_eq!(xtoi("00"), 0);
        assert_eq!(xtoi("0F"), 15);
        assert_eq!(xtoi("1f"), 31);
        assert_eq!(xtoi("FF"), 255);
        assert_eq!(xtoi("ff"), 255);
        assert_eq!(xtoi("GG"), 0); // Invalid
    }

    #[test]
    fn test_url_encode_query_param() {
        assert_eq!(url_encode_query_param("hello world"), "hello+world");
        assert_eq!(url_encode_query_param("a&b"), "a%26b");
        assert_eq!(url_encode_query_param("test=value"), "test%3Dvalue");
    }
}
