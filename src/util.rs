#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};
use base64::Engine;

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn format_timestamp(timestamp: u64) -> String {
    let datetime = chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .unwrap_or_default();
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn is_valid_ip(addr: &str) -> bool {
    addr.parse::<std::net::IpAddr>().is_ok()
}

pub fn is_private_ip(addr: &str) -> bool {
    match addr.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ipv4)) => {
            let octets = ipv4.octets();
            // 10.0.0.0/8
            octets[0] == 10 ||
            // 172.16.0.0/12
            (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31) ||
            // 192.168.0.0/16
            (octets[0] == 192 && octets[1] == 168) ||
            // 127.0.0.0/8 (loopback)
            octets[0] == 127
        }
        Ok(std::net::IpAddr::V6(ipv6)) => {
            // Check for loopback (::1) or unique local addresses (fc00::/7)
            ipv6.is_loopback() || (ipv6.segments()[0] & 0xfe00) == 0xfc00
        }
        Err(_) => false,
    }
}

pub fn sanitize_header_value(value: &str) -> String {
    value
        .replace('\r', "")
        .replace('\n', "")
        .replace('\0', "")
}

pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

pub fn parse_content_type(content_type: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = content_type.split(';').collect();
    let mime_type = parts[0].trim().to_lowercase();

    let charset = parts.get(1).and_then(|p| {
        p.trim().strip_prefix("charset=").map(|s| s.trim().to_string())
    });

    (mime_type, charset)
}

pub fn is_text_content_type(content_type: &str) -> bool {
    let (mime_type, _) = parse_content_type(content_type);
    mime_type.starts_with("text/") ||
    mime_type == "application/json" ||
    mime_type == "application/javascript" ||
    mime_type == "application/xml" ||
    mime_type == "application/xhtml+xml"
}

pub fn is_html_content_type(content_type: &str) -> bool {
    let (mime_type, _) = parse_content_type(content_type);
    mime_type == "text/html" || mime_type == "application/xhtml+xml"
}

pub fn is_image_content_type(content_type: &str) -> bool {
    let (mime_type, _) = parse_content_type(content_type);
    mime_type.starts_with("image/")
}

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join("")
}

pub fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

pub fn base64_encode(data: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    STANDARD.encode(data)
}

pub fn base64_decode(data: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    STANDARD.decode(data).ok()
}

pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut star_idx = None;
    let mut match_idx = 0;

    while t_idx < text_chars.len() {
        if p_idx < pattern_chars.len() &&
            (pattern_chars[p_idx] == '?' || pattern_chars[p_idx] == text_chars[t_idx]) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < pattern_chars.len() && pattern_chars[p_idx] == '*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
        } else if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < pattern_chars.len() && pattern_chars[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == pattern_chars.len()
}

pub fn parse_host_port(addr: &str) -> Option<(String, u16)> {
    if let Some(colon_pos) = addr.rfind(':') {
        let host = &addr[..colon_pos];
        let port = addr[colon_pos + 1..].parse::<u16>().ok()?;
        Some((host.to_string(), port))
    } else {
        Some((addr.to_string(), 80))
    }
}

pub fn join_path(base: &str, path: &str) -> String {
    if base.ends_with('/') {
        format!("{}{}", base, path)
    } else {
        format!("{}/{}", base, path)
    }
}

pub fn get_file_extension(path: &str) -> Option<&str> {
    path.rfind('.').map(|pos| &path[pos + 1..])
}

pub fn is_safe_path(path: &str) -> bool {
    !path.contains("..") && !path.starts_with('/') && !path.starts_with('\\')
}

/// Hash a string to compute a (hopefully) unique numeric integer value.
/// Ported from hash_string in miscutil.c
pub fn hash_string(s: &str) -> u32 {
    let mut h: u32 = 0;
    for byte in s.bytes() {
        h = 5 * h + byte as u32;
    }
    h
}

/// Case insensitive string comparison
/// Ported from strcmpic in miscutil.c
/// Returns: 0 if s1==s2, Negative if s1<s2, Positive if s1>s2
pub fn strcmpic(s1: &str, s2: &str) -> i32 {
    let mut iter1 = s1.chars();
    let mut iter2 = s2.chars();
    
    loop {
        let c1 = iter1.next();
        let c2 = iter2.next();
        
        match (c1, c2) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(ch1), Some(ch2)) => {
                let lower1 = ch1.to_ascii_lowercase();
                let lower2 = ch2.to_ascii_lowercase();
                
                if lower1 != lower2 {
                    return (lower1 as i32) - (lower2 as i32);
                }
            }
        }
    }
}

/// Case insensitive string comparison (up to n characters)
/// Ported from strncmpic in miscutil.c
/// Returns: 0 if s1==s2, Negative if s1<s2, Positive if s1>s2
pub fn strncmpic(s1: &str, s2: &str, n: usize) -> i32 {
    let mut iter1 = s1.chars();
    let mut iter2 = s2.chars();
    
    for _ in 0..n {
        let c1 = iter1.next();
        let c2 = iter2.next();
        
        match (c1, c2) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(ch1), Some(ch2)) => {
                let lower1 = ch1.to_ascii_lowercase();
                let lower2 = ch2.to_ascii_lowercase();
                
                if lower1 != lower2 {
                    return (lower1 as i32) - (lower2 as i32);
                }
            }
        }
    }
    
    0
}

/// In-situ-eliminate all leading and trailing whitespace from a string.
/// Ported from chomp in miscutil.c
pub fn chomp(s: &str) -> String {
    s.trim().to_string()
}

/// Convert a string to uppercase
/// Ported from string_toupper in miscutil.c
pub fn string_toupper(s: &str) -> String {
    s.to_uppercase()
}

/// Convert a string to lowercase
/// Ported from string_tolower in miscutil.c
pub fn string_tolower(s: &str) -> String {
    s.to_lowercase()
}

/// Duplicate a string slice with specified length
/// Ported from bindup in miscutil.c
pub fn bindup(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Make a path from directory and file
/// Ported from make_path in miscutil.c
pub fn make_path(dir: &str, file: &str) -> String {
    let dir = dir.trim_end_matches('/').trim_end_matches('\\');
    let file = file.trim_start_matches('/').trim_start_matches('\\');
    
    #[cfg(windows)]
    return format!("{}\\{}", dir, file);
    
    #[cfg(not(windows))]
    return format!("{}/{}", dir, file);
}

/// Pick a random number from a range
/// Ported from pick_from_range in miscutil.c
pub fn pick_from_range(range: i64) -> i64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(0..range)
}

/// Check if a host is an IP address
/// Ported from host_is_ip_address in miscutil.c
pub fn host_is_ip_address(host: &str) -> bool {
    // Try to parse as IPv4
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    
    // Try to parse as IPv6
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    
    false
}

/// Split a string using delimiters.
/// Ported from ssplit in ssplit.c
/// 
/// Parameters:
/// - str: string to split
/// - delim: delimiter characters (if None, uses " \t")
/// 
/// Returns: A vector of string slices
pub fn ssplit<'a>(str: &'a str, delim: Option<&str>) -> Vec<&'a str> {
    let delimiters = delim.unwrap_or(" \t");
    
    str.split(|c| delimiters.contains(c))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split a string with a maximum number of results.
/// Similar to ssplit but limits the number of results.
pub fn ssplit_n<'a>(str: &'a str, delim: Option<&str>, max_results: usize) -> Vec<&'a str> {
    let delimiters = delim.unwrap_or(" \t");
    
    str.split(|c| delimiters.contains(c))
        .filter(|s| !s.is_empty())
        .take(max_results)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_string() {
        let hash1 = hash_string("test");
        let hash2 = hash_string("test");
        let hash3 = hash_string("different");
        
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_strcmpic() {
        assert_eq!(strcmpic("test", "test"), 0);
        assert_eq!(strcmpic("TEST", "test"), 0);
        assert!(strcmpic("abc", "def") < 0);
        assert!(strcmpic("def", "abc") > 0);
    }

    #[test]
    fn test_strncmpic() {
        assert_eq!(strncmpic("test", "test", 4), 0);
        assert_eq!(strncmpic("TEST", "test", 4), 0);
        assert_eq!(strncmpic("abc", "def", 3), -3);
        assert_eq!(strncmpic("abc", "abcdef", 3), 0);
    }

    #[test]
    fn test_chomp() {
        assert_eq!(chomp("  test  "), "test");
        assert_eq!(chomp("test"), "test");
        assert_eq!(chomp("  test"), "test");
        assert_eq!(chomp("test  "), "test");
    }

    #[test]
    fn test_string_toupper() {
        assert_eq!(string_toupper("test"), "TEST");
        assert_eq!(string_toupper("Test"), "TEST");
    }

    #[test]
    fn test_string_tolower() {
        assert_eq!(string_tolower("TEST"), "test");
        assert_eq!(string_tolower("Test"), "test");
    }

    #[test]
    fn test_bindup() {
        assert_eq!(bindup("hello", 3), "hel");
        assert_eq!(bindup("hello", 10), "hello");
    }

    #[test]
    fn test_make_path() {
        #[cfg(windows)]
        {
            assert_eq!(make_path("C:\\dir", "file.txt"), "C:\\dir\\file.txt");
        }
        
        #[cfg(not(windows))]
        {
            assert_eq!(make_path("/dir", "file.txt"), "/dir/file.txt");
        }
    }

    #[test]
    fn test_host_is_ip_address() {
        assert!(host_is_ip_address("192.168.1.1"));
        assert!(host_is_ip_address("127.0.0.1"));
        assert!(host_is_ip_address("::1"));
        assert!(!host_is_ip_address("example.com"));
        assert!(!host_is_ip_address("localhost"));
    }

    #[test]
    fn test_ssplit() {
        let result = ssplit("hello world test", None);
        assert_eq!(result, vec!["hello", "world", "test"]);
        
        let result = ssplit("hello,world,test", Some(","));
        assert_eq!(result, vec!["hello", "world", "test"]);
        
        let result = ssplit("  hello   world  ", None);
        assert_eq!(result, vec!["hello", "world"]);
        
        let result = ssplit("", None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ssplit_n() {
        let result = ssplit_n("hello world test foo bar", None, 3);
        assert_eq!(result, vec!["hello", "world", "test"]);
        
        let result = ssplit_n("a,b,c,d,e", Some(","), 2);
        assert_eq!(result, vec!["a", "b"]);
    }
}
