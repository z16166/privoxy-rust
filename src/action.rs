use std::collections::HashSet;
use regex::Regex;
use tracing::debug;

use crate::config::{Action, Config};
use crate::error::PrivoxyResult;
use crate::filter::{Filter, FilterType, FilterVariables};

#[cfg(feature = "image-blocking")]
use crate::deanimate;

pub struct ActionContext {
    pub tags: HashSet<String>,
    pub suppress_tags: HashSet<String>,
}

impl Default for ActionContext {
    fn default() -> Self {
        Self {
            tags: HashSet::new(),
            suppress_tags: HashSet::new(),
        }
    }
}

pub fn find_action_for_url(url: &str, config: &Config) -> Option<Action> {
    for url_action in &config.url_actions {
        for pattern in &url_action.patterns {
            if url_matches_pattern(url, pattern) {
                return Some(url_action.action.clone());
            }
        }
    }
    None
}

#[allow(dead_code)]
pub fn find_action_for_url_with_tags(url: &str, config: &Config, ctx: &ActionContext) -> Option<Action> {
    let mut action = find_action_for_url(url, config)?;
    
    for tag in &ctx.tags {
        if !ctx.suppress_tags.contains(tag) {
            if let Some(tag_action) = find_action_for_tag(tag, config) {
                merge_actions(&mut action, &tag_action);
            }
        }
    }
    
    Some(action)
}

fn find_action_for_tag(tag: &str, config: &Config) -> Option<Action> {
    for url_action in &config.url_actions {
        for pattern in &url_action.patterns {
            if pattern.starts_with("TAG:") {
                let tag_pattern = &pattern[4..];
                if let Ok(re) = Regex::new(tag_pattern) {
                    if re.is_match(tag) {
                        return Some(url_action.action.clone());
                    }
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn merge_actions(target: &mut Action, source: &Action) {
    if source.block {
        target.block = true;
        if source.block_reason.is_some() {
            target.block_reason = source.block_reason.clone();
        }
    }
    
    if source.redirect.is_some() {
        target.redirect = source.redirect.clone();
    }
    
    for filter_name in &source.filter_names {
        if !target.filter_names.contains(filter_name) {
            target.filter_names.push(filter_name.clone());
        }
    }
    
    for header in &source.add_headers {
        if !target.add_headers.contains(header) {
            target.add_headers.push(header.clone());
        }
    }
    
    for header in &source.crunch_client_headers {
        if !target.crunch_client_headers.contains(header) {
            target.crunch_client_headers.push(header.clone());
        }
    }
    
    for header in &source.crunch_server_headers {
        if !target.crunch_server_headers.contains(header) {
            target.crunch_server_headers.push(header.clone());
        }
    }
}

pub fn url_matches_pattern(url: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern == "." || pattern == "*" {
        return true;
    }
    
    if pattern.starts_with("TAG:") {
        return false;
    }
    
    if pattern.starts_with("NO-CLIENT-TAG:") || pattern.starts_with("NO-SERVER-TAG:") {
        return false;
    }
    
    // Following the C code's compile_url_pattern() / url_match() architecture:
    // 1. Split pattern at '/' into host_pattern and path_pattern
    // 2. Split URL into host, port, path components
    // 3. Match host and path independently
    
    let (host_pattern, path_pattern) = if let Some(slash_pos) = pattern.find('/') {
        (&pattern[..slash_pos], Some(&pattern[slash_pos..]))
    } else {
        (pattern, None)
    };
    
    // Parse URL into components: host[:port][/path]
    let (url_host, _url_port, url_path) = parse_url_components(url);
    
    // 1. Host matching (following C code's domain_match with anchoring)
    if !host_pattern.is_empty() && !host_pattern_matches(&url_host, host_pattern) {
        return false;
    }
    
    // 2. Path matching - "/" alone matches everything, otherwise left-anchored
    if let Some(pp) = path_pattern {
        if pp.len() > 1 {
            // Left-anchored path match (following C code's compile_pattern with LEFT_ANCHORED)
            if !path_pattern_matches(&url_path, pp) {
                return false;
            }
        }
    }
    
    true
}

/// Parse a URL string into (host, port, path) components.
/// URL can be: "host", "host:port", "host/path", "host:port/path",
/// or a full URL "http://host/path".
fn parse_url_components(url: &str) -> (String, Option<u16>, String) {
    // Strip scheme if present
    let url = if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else {
        url
    };
    
    // Split at first '/' to separate host[:port] from path
    let (hostport, path) = if let Some(slash_pos) = url.find('/') {
        (&url[..slash_pos], &url[slash_pos..])
    } else {
        (url, "/")
    };
    
    // Split host and port
    let (host, port) = if let Some(colon_pos) = hostport.rfind(':') {
        let port_str = &hostport[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            (hostport[..colon_pos].to_lowercase(), Some(port))
        } else {
            (hostport.to_lowercase(), None)
        }
    } else {
        (hostport.to_lowercase(), None)
    };
    
    (host, port, path.to_string())
}

/// Match a host string against a host pattern, following the C code's
/// compile_vanilla_host_pattern() / domain_match() logic.
///
/// Anchoring rules (from C code):
/// - Leading '.' in pattern => ANCHOR_LEFT (right-side of domain is unanchored)
///   ".youtube.com" matches "youtube.com", "www.youtube.com", etc.
/// - Trailing '.' in pattern => ANCHOR_RIGHT (left-side is unanchored)
///   "ad." matches "ad.example.com", etc.
/// - Both => fully unanchored
/// - Neither => fully anchored (exact match only)
/// - '*' in domain components => wildcard matching
fn host_pattern_matches(host: &str, pattern: &str) -> bool {
    let host = host.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    
    let left_unanchored = pattern_lower.starts_with('.');
    let right_unanchored = pattern_lower.ends_with('.');
    
    // Strip anchoring dots from pattern to get the core domain components
    let core_pattern = pattern_lower
        .trim_start_matches('.')
        .trim_end_matches('.');
    
    if core_pattern.is_empty() {
        // Pattern is just "." or ".." �?matches everything
        return true;
    }
    
    // Split both host and pattern into domain components
    let host_parts: Vec<&str> = host.split('.').collect();
    let pattern_parts: Vec<&str> = core_pattern.split('.').collect();
    
    let hlen = host_parts.len();
    let plen = pattern_parts.len();
    
    if hlen < plen {
        return false;
    }
    
    if left_unanchored && !right_unanchored {
        // Right-anchored: pattern must match the rightmost components of host
        // ".youtube.com" matches "youtube.com", "www.youtube.com"
        let offset = hlen - plen;
        domain_components_match(&pattern_parts, &host_parts[offset..])
    } else if !left_unanchored && right_unanchored {
        // Left-anchored: pattern must match the leftmost components of host
        // "ad." matches "ad.example.com"
        domain_components_match(&pattern_parts, &host_parts[..plen])
    } else if left_unanchored && right_unanchored {
        // Fully unanchored: pattern can match anywhere in the host
        for n in 0..=(hlen - plen) {
            if domain_components_match(&pattern_parts, &host_parts[n..n + plen]) {
                return true;
            }
        }
        false
    } else {
        // Fully anchored: exact match (same number of components)
        if hlen != plen {
            return false;
        }
        domain_components_match(&pattern_parts, &host_parts)
    }
}

/// Compare domain components with wildcard support.
/// Following C code's simple_domaincmp() / simplematch().
fn domain_components_match(pattern_parts: &[&str], host_parts: &[&str]) -> bool {
    if pattern_parts.len() != host_parts.len() {
        return false;
    }
    for (p, h) in pattern_parts.iter().zip(host_parts.iter()) {
        if !simplematch(p, h) {
            return false;
        }
    }
    true
}

/// Simple string matching with '*' wildcard support.
/// Following C code's simplematch() (simplified: only '*' and '?' wildcards).
fn simplematch(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    simplematch_recursive(&pat, &txt, 0, 0)
}

fn simplematch_recursive(pat: &[char], txt: &[char], mut pi: usize, mut ti: usize) -> bool {
    while pi < pat.len() && ti < txt.len() {
        if pat[pi] == '*' {
            pi += 1;
            if pi >= pat.len() {
                return true; // trailing * matches everything
            }
            // Try matching the rest of the pattern at every position
            for start in ti..=txt.len() {
                if simplematch_recursive(pat, txt, pi, start) {
                    return true;
                }
            }
            return false;
        } else if pat[pi] == '?' || pat[pi] == txt[ti] {
            pi += 1;
            ti += 1;
        } else {
            return false;
        }
    }
    // Skip trailing wildcards
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi >= pat.len() && ti >= txt.len()
}

/// Match a URL path against a path pattern.
/// Following C code's compile_pattern with LEFT_ANCHORED: "^pattern"
fn path_pattern_matches(path: &str, pattern: &str) -> bool {
    // The C code compiles the path pattern as a left-anchored regex
    // For simple patterns without regex metacharacters, we can use prefix matching
    // For patterns with regex characters, use regex
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') 
        || pattern.contains('(') || pattern.contains('+') || pattern.contains('\\') {
        // Use regex: left-anchored
        let regex_pattern = format!("^{}", pattern);
        if let Ok(re) = regex::Regex::new(&regex_pattern) {
            return re.is_match(path);
        }
        false
    } else {
        // Simple prefix match
        path.starts_with(pattern)
    }
}

pub fn apply_client_header_taggers(
    headers: &crate::http::Headers,
    config: &Config,
    action: &Action,
    ctx: &mut ActionContext,
    variables: &FilterVariables,
) {
    for tagger_name in &action.client_header_tagger_names {
        if let Some(tagger) = find_filter(config, tagger_name, FilterType::ClientHeaderTagger) {
            for (header_name, header_value) in headers {
                let header_line = format!("{}: {}", header_name, header_value);
                if let Some(tag) = apply_tagger(&tagger, &header_line, variables) {
                    add_tag(ctx, &tag, config);
                }
            }
        }
    }
}

pub fn apply_server_header_taggers(
    headers: &crate::http::Headers,
    config: &Config,
    action: &Action,
    ctx: &mut ActionContext,
    variables: &FilterVariables,
) {
    for tagger_name in &action.server_header_tagger_names {
        if let Some(tagger) = find_filter(config, tagger_name, FilterType::ServerHeaderTagger) {
            for (header_name, header_value) in headers {
                let header_line = format!("{}: {}", header_name, header_value);
                if let Some(tag) = apply_tagger(&tagger, &header_line, variables) {
                    add_tag(ctx, &tag, config);
                }
            }
        }
    }
}

pub fn apply_client_body_taggers(
    body: &[u8],
    config: &Config,
    action: &Action,
    ctx: &mut ActionContext,
    variables: &FilterVariables,
) {
    if body.is_empty() {
        return;
    }
    
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return,
    };
    
    for tagger_name in &action.client_body_tagger_names {
        if let Some(tagger) = find_filter(config, tagger_name, FilterType::ClientBodyTagger) {
            if let Some(tag) = apply_tagger(&tagger, body_str, variables) {
                add_tag(ctx, &tag, config);
            }
        }
    }
}

fn apply_tagger(tagger: &Filter, input: &str, variables: &FilterVariables) -> Option<String> {
    if !tagger.enabled {
        return None;
    }
    
    let result = if tagger.dynamic {
        tagger.apply_with_variables(input, Some(variables))
    } else {
        tagger.apply(input)
    };
    
    if result != input && !result.is_empty() {
        Some(result.into_owned())
    } else {
        None
    }
}

fn add_tag(ctx: &mut ActionContext, tag: &str, config: &Config) {
    if ctx.suppress_tags.contains(tag) {
        debug!("Tag '{}' suppressed", tag);
        return;
    }
    
    if ctx.tags.contains(tag) {
        debug!("Tag '{}' already present", tag);
        return;
    }
    
    ctx.tags.insert(tag.to_string());
    debug!("Tag '{}' added", tag);
    
    if let Some(tag_action) = find_action_for_tag(tag, config) {
        for suppress_tag in &tag_action.suppress_tags {
            ctx.suppress_tags.insert(suppress_tag.clone());
        }
    }
}

fn find_filter<'a>(config: &'a Config, name: &str, filter_type: FilterType) -> Option<&'a Filter> {
    config.filters.iter().find(|f| f.name == name && f.filter_type == filter_type)
}
pub fn apply_client_body_filter(
    body: &mut Vec<u8>,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
) -> PrivoxyResult<()> {
    if body.is_empty() {
        return Ok(());
    }
    
    // Take ownership of body vector to avoid cloning into String
    let mut body_str = match String::from_utf8(std::mem::take(body)) {
        Ok(s) => s,
        Err(e) => {
            // Restore body on error
            *body = e.into_bytes();
            return Ok(());
        }
    };
    
    let mut changed = false;
    for filter_name in &action.client_body_filter_names {
        if let Some(filter) = find_filter(config, filter_name, FilterType::ClientBody) {
            if !filter.enabled {
                continue;
            }
            
            let before_len = body_str.len();
            let next = if filter.dynamic {
                filter.apply_with_variables(&body_str, Some(variables))
            } else {
                filter.apply(&body_str)
            };
            
            if let std::borrow::Cow::Owned(s) = next {
                body_str = s;
                changed = true;
                debug!("Applied client-body-filter '{}' ({} -> {} bytes)", 
                    filter_name, before_len, body_str.len());
            }
        }
    }
    
    // Only convert back and assign if something changed
    if changed {
        *body = body_str.into_bytes();
    } else {
        *body = body_str.into_bytes(); // Still need to restore taken body
    }
    Ok(())
}


/// Apply content filters to response body
/// Ported from execute_content_filters in filters.c
pub fn apply_content_filters(
    body: &mut Vec<u8>,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
) -> PrivoxyResult<()> {
    if body.is_empty() {
        return Ok(());
    }
    
    // Take ownership of body vector to avoid cloning into String
    let mut body_str = match String::from_utf8(std::mem::take(body)) {
        Ok(s) => s,
        Err(e) => {
            // Restore body on error
            *body = e.into_bytes();
            return Ok(());
        }
    };
    
    let mut changed = false;
    for filter_name in &action.filter_names {
        if let Some(filter) = find_filter(config, filter_name, FilterType::Content) {
            if !filter.enabled {
                continue;
            }
            
            let before_len = body_str.len();
            let next_cow = if filter.dynamic {
                filter.apply_with_variables(&body_str, Some(variables))
            } else {
                filter.apply(&body_str)
            };
            
            if let std::borrow::Cow::Owned(s) = next_cow {
                body_str = s;
                changed = true;
                debug!("Applied content filter '{}' ({} -> {} bytes)", 
                    filter_name, before_len, body_str.len());
            }
        }
    }
    
    if changed {
        *body = body_str.into_bytes();
    } else {
        *body = body_str.into_bytes(); // Restore taken body
    }
    Ok(())
}

/// Apply client header filters
/// Ported from filter_header in parsers.c
pub fn apply_client_header_filters(
    headers: &mut crate::http::Headers,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
) {
    for filter_name in &action.client_header_filter_names {
        if let Some(filter) = find_filter(config, filter_name, FilterType::ClientHeader) {
            if !filter.enabled {
                continue;
            }
            
            let old_headers = std::mem::take(headers);
            for (name, value) in old_headers {
                let header_line = format!("{}: {}", name, value);
                let filtered = if filter.dynamic {
                    filter.apply_with_variables(&header_line, Some(variables))
                 } else {
                    filter.apply(&header_line)
                 };
                
                if filtered.is_empty() {
                    debug!("Header filter {} removed header {}: {}", filter_name, name, value);
                    continue;
                }
                
                if let Some((n, v)) = filtered.split_once(':') {
                    headers.push((n.trim().to_string(), v.trim().to_string()));
                } else {
                    // Fallback: keep original if parsing fails but not empty
                    headers.push((name, value));
                }
            }
        }
    }
}

/// Apply server header filters
pub fn apply_server_header_filters(
    headers: &mut crate::http::Headers,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
) {
    for filter_name in &action.server_header_filter_names {
        if let Some(filter) = find_filter(config, filter_name, FilterType::ServerHeader) {
            if !filter.enabled {
                continue;
            }
            
            let old_headers = std::mem::take(headers);
            for (name, value) in old_headers {
                let header_line = format!("{}: {}", name, value);
                let filtered = if filter.dynamic {
                    filter.apply_with_variables(&header_line, Some(variables))
                } else {
                    filter.apply(&header_line)
                };
                
                if filtered.is_empty() {
                    debug!("Header filter {} removed header {}: {}", filter_name, name, value);
                    continue;
                }
                
                if let Some((n, v)) = filtered.split_once(':') {
                    headers.push((n.trim().to_string(), v.trim().to_string()));
                } else {
                    // Fallback: keep original if parsing fails but not empty
                    headers.push((name, value));
                }
            }
        }
    }
}

pub fn apply_add_header(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    for header in &action.add_headers {
        if let Some((name, value)) = header.split_once(':') {
            crate::http::set_header(headers, name.trim(), value.trim());
            debug!("Added header: {}: {}", name.trim(), value.trim());
        }
    }
}

pub fn apply_crunch_client_header(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    for pattern in &action.crunch_client_headers {
        let pattern_lower = pattern.to_lowercase();
        let headers_to_remove: Vec<String> = headers.iter().map(|(n, _)| n.clone())
            .filter(|k| k.to_lowercase().contains(&pattern_lower))
            
            .collect();
        for header in headers_to_remove {
            debug!("Crunching client header: {}", header);
            crate::http::remove_header(headers, &header);
        }
    }
}

pub fn apply_client_header_actions(
    headers: &mut crate::http::Headers,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
    client_addr: &str,
) {
    // 1. Header Filters (parsers.c: filter_header)
    apply_client_header_filters(headers, config, action, variables);

    // 2. Crunching (parsers.c: crunch_client_header)
    apply_crunch_client_header(headers, action);
    
    if action.crunch_outgoing_cookies {
        crate::http::remove_header(headers, "Cookie");
        debug!("Crunched outgoing cookies");
    }

    if action.crunch_if_none_match {
        crate::http::remove_header(headers, "If-None-Match");
        debug!("Crunched If-None-Match");
    }

    // 3. Modifying specific headers
    apply_hide_referrer(headers, action);
    apply_hide_user_agent(headers, action);
    apply_send_user_agent(headers, action);
    apply_hide_from_header(headers, action);
    apply_hide_accept_language(headers, action);
    apply_hide_if_modified_since(headers, action);
    apply_change_x_forwarded_for(headers, action, client_addr);
    apply_prevent_compression(headers, action);
    apply_session_cookies_only(headers, action);

    // 4. Adding headers
    apply_add_header(headers, action);
}

pub fn apply_server_header_actions(
    headers: &mut crate::http::Headers,
    config: &Config,
    action: &Action,
    variables: &FilterVariables,
) {
    // 1. Header Filters
    apply_server_header_filters(headers, config, action, variables);

    // 2. Crunching
    apply_crunch_server_header(headers, action);
    
    if action.crunch_incoming_cookies {
        crate::http::remove_header(headers, "Set-Cookie");
        debug!("Crunched incoming cookies");
    }

    // 3. Modifying specific headers
    apply_hide_content_disposition(headers, action);
    apply_overwrite_last_modified(headers, action);
    apply_content_type_overwrite(headers, action);
}

pub fn apply_crunch_server_header(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    for pattern in &action.crunch_server_headers {
        let pattern_lower = pattern.to_lowercase();
        let headers_to_remove: Vec<String> = headers.iter().map(|(n, _)| n.clone())
            .filter(|k| k.to_lowercase().contains(&pattern_lower))
            
            .collect();
        for header in headers_to_remove {
            debug!("Crunching server header: {}", header);
            crate::http::remove_header(headers, &header);
        }
    }
}

pub fn apply_change_x_forwarded_for(
    headers: &mut crate::http::Headers,
    action: &Action,
    client_addr: &str,
) {
    if let Some(ref mode) = action.change_x_forwarded_for {
        if mode == "block" {
            crate::http::remove_header(headers, "X-Forwarded-For");
            debug!("Blocking X-Forwarded-For header");
        } else if mode == "add" {
            if let Some(client_ip) = client_addr.split(':').next() {
                if let Some(existing) = crate::http::get_header(headers, "X-Forwarded-For") {
                    crate::http::set_header(headers, "X-Forwarded-For".to_string(), 
                        format!("{}, {}", existing, client_ip));
                } else {
                    crate::http::set_header(headers, "X-Forwarded-For", client_ip);
                }
                debug!("Added client IP to X-Forwarded-For: {}", client_ip);
            }
        }
    }
}

pub fn apply_hide_referrer(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref referrer) = action.hide_referrer {
        if referrer == "conditional-block" || referrer == "block" {
            crate::http::remove_header(headers, "Referer");
            debug!("Blocking Referer header");
        } else if referrer == "conditional-forge" || referrer == "forge" {
            if let Some(host) = crate::http::get_header(headers, "Host") {
                let scheme = if let Some(port) = host.split(':').last() {
                    if port == "443" { "https" } else { "http" }
                } else {
                    "http"
                };
                crate::http::set_header(headers, "Referer".to_string(), format!("{}://{}/", scheme, host));
                debug!("Forged Referer header");
            }
        } else {
            crate::http::set_header(headers, "Referer".to_string(), referrer);
            debug!("Set Referer to: {}", referrer);
        }
    }
}

pub fn apply_hide_user_agent(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref ua) = action.hide_user_agent {
        if ua == "block" {
            crate::http::remove_header(headers, "User-Agent");
            debug!("Blocking User-Agent header");
        } else {
            crate::http::set_header(headers, "User-Agent".to_string(), ua);
            debug!("Hiding User-Agent, setting to: {}", ua);
        }
    }
}

pub fn apply_send_user_agent(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref ua) = action.send_user_agent {
        crate::http::set_header(headers, "User-Agent".to_string(), ua);
        debug!("Setting User-Agent to: {}", ua);
    }
}

pub fn apply_hide_from_header(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref mode) = action.hide_from_header {
        if mode == "block" {
            crate::http::remove_header(headers, "From");
            debug!("Blocking From header");
        } else {
            crate::http::set_header(headers, "From".to_string(), mode);
            debug!("Setting From header to: {}", mode);
        }
    }
}

pub fn apply_hide_accept_language(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref mode) = action.hide_accept_language {
        if mode == "block" {
            crate::http::remove_header(headers, "Accept-Language");
            debug!("Blocking Accept-Language header");
        } else {
            crate::http::set_header(headers, "Accept-Language".to_string(), mode);
            debug!("Setting Accept-Language to: {}", mode);
        }
    }
}

pub fn apply_hide_if_modified_since(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref mode) = action.hide_if_modified_since {
        if mode == "block" {
            crate::http::remove_header(headers, "If-Modified-Since");
            debug!("Blocking If-Modified-Since header");
        } else if let Ok(offset) = mode.parse::<i64>() {
            use std::time::{SystemTime, UNIX_EPOCH};
            if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
                let timestamp = now.as_secs() as i64 + offset;
                if let Some(dt) = chrono::DateTime::from_timestamp(timestamp, 0) {
                    let date_str = dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
                    crate::http::set_header(headers, "If-Modified-Since".to_string(), date_str);
                    debug!("Setting If-Modified-Since with offset {}", offset);
                }
            }
        }
    }
}

pub fn apply_hide_content_disposition(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref mode) = action.hide_content_disposition {
        if mode == "block" {
            crate::http::remove_header(headers, "Content-Disposition");
            debug!("Blocking Content-Disposition header");
        } else {
            crate::http::set_header(headers, "Content-Disposition".to_string(), mode);
            debug!("Setting Content-Disposition to: {}", mode);
        }
    }
}

pub fn apply_prevent_compression(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if action.prevent_compression {
        crate::http::remove_header(headers, "Accept-Encoding");
        crate::http::set_header(headers, "Accept-Encoding", "identity");
        debug!("Preventing compression");
    }
}

pub fn apply_downgrade_http_version(version: &mut String, action: &Action) {
    if action.downgrade_http_version {
        *version = "HTTP/1.0".to_string();
        debug!("Downgraded HTTP version to 1.0");
    }
}

pub fn apply_session_cookies_only(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if action.session_cookies_only {
        if let Some(cookie) = crate::http::get_header(headers, "Cookie") {
            let filtered_cookie: String = cookie
                .split(';')
                .filter(|c| {
                    let c_lower = c.to_lowercase();
                    !c_lower.contains("secure") && !c_lower.contains("httponly")
                })
                .collect::<Vec<_>>()
                .join(";");
            crate::http::set_header(headers, "Cookie".to_string(), filtered_cookie);
            debug!("Converted cookies to session-only");
        }
    }
}

pub fn apply_content_type_overwrite(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref content_type) = action.content_type_overwrite {
        crate::http::set_header(headers, "Content-Type".to_string(), content_type);
        debug!("Overwriting Content-Type to: {}", content_type);
    }
}

pub fn apply_overwrite_last_modified(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(ref mode) = action.overwrite_last_modified {
        if mode == "block" {
            crate::http::remove_header(headers, "Last-Modified");
            debug!("Blocking Last-Modified header");
        } else if mode == "reset-to-request-time" {
            use std::time::{SystemTime, UNIX_EPOCH};
            if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
                if let Some(dt) = chrono::DateTime::from_timestamp(duration.as_secs() as i64, 0) {
                    let timestamp = dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
                    crate::http::set_header(headers, "Last-Modified".to_string(), timestamp);
                    debug!("Reset Last-Modified to request time");
                }
            }
        } else if mode == "randomize" {
            use std::time::{SystemTime, UNIX_EPOCH};
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            let random_offset = (now % 86400) as i64;
            if let Some(dt) = chrono::DateTime::from_timestamp(now as i64 - random_offset, 0) {
                let timestamp = dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
                crate::http::set_header(headers, "Last-Modified".to_string(), timestamp);
                debug!("Randomized Last-Modified");
            }
        }
    }
}

pub fn apply_limit_cookie_lifetime(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if let Some(lifetime_secs) = action.limit_cookie_lifetime {
        if let Some(cookie) = crate::http::get_header(headers, "Set-Cookie") {
            let max_age_attr = format!("Max-Age={}", lifetime_secs);
            let new_cookie = if cookie.to_lowercase().contains("max-age=") {
                let re = Regex::new(r"(?i)Max-Age=\d+").unwrap();
                re.replace(&cookie, &max_age_attr).to_string()
            } else {
                format!("{}; {}", cookie, max_age_attr)
            };
            crate::http::set_header(headers, "Set-Cookie".to_string(), new_cookie);
            debug!("Limited cookie lifetime to {} seconds", lifetime_secs);
        }
    }
}

pub fn apply_force_text_mode(
    headers: &mut crate::http::Headers,
    action: &Action,
) {
    if action.force_text_mode {
        crate::http::set_header(headers, "Content-Type", "text/plain");
        debug!("Forcing text mode");
    }
}

pub fn check_limit_connect(action: &Action, port: u16) -> bool {
    if let Some(ref portlist) = action.limit_connect {
        for part in portlist.split(',') {
            let part = part.trim();
            if part.contains('-') {
                if let Some((start, end)) = part.split_once('-') {
                    if let (Ok(start_port), Ok(end_port)) = (start.parse::<u16>(), end.parse::<u16>()) {
                        if port >= start_port && port <= end_port {
                            return true;
                        }
                    }
                }
            } else {
                if let Ok(allowed_port) = part.parse::<u16>() {
                    if port == allowed_port {
                        return true;
                    }
                }
            }
        }
        return false;
    }
    true
}

#[cfg(feature = "image-blocking")]
pub use deanimate::deanimate_gif;

#[cfg(not(feature = "image-blocking"))]
pub fn deanimate_gif(_body: &[u8], _mode: &str) -> Option<Vec<u8>> {
    None
}

pub fn apply_fast_redirects(
    status_code: u16,
    location: Option<&String>,
    mode: &str,
) -> Option<String> {
    if status_code >= 300 && status_code < 400 {
        if let Some(loc) = location {
            debug!("Fast redirect detected: {} -> {}", status_code, loc);
            
            if mode == "check-decoded-url" {
                if let Ok(decoded) = urlencoding_decode(loc) {
                    debug!("Decoded redirect location: {}", decoded);
                    return Some(decoded);
                }
            }
            return Some(loc.clone());
        }
    }
    None
}

fn urlencoding_decode(s: &str) -> Result<String, std::string::FromUtf8Error> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte);
            } else {
                result.extend_from_slice(b"%");
                result.extend_from_slice(hex.as_bytes());
            }
        } else if c == '+' {
            result.push(b' ');
        } else {
            result.extend_from_slice(c.to_string().as_bytes());
        }
    }
    
    String::from_utf8(result)
}

pub fn apply_suppress_tags(action: &Action, ctx: &mut ActionContext) {
    for tag in &action.suppress_tags {
        ctx.suppress_tags.insert(tag.clone());
        debug!("Suppressing tag: {}", tag);
    }
}

pub fn create_blocked_page_response(url: &str, reason: &str) -> crate::http::HttpResponse {
    let body = format!(r#"<!DOCTYPE html>
<html>
<head>
<title>Request blocked by Privoxy</title>
<style>
body {{ font-family: Arial, sans-serif; margin: 40px; background: #f0f0f0; }}
.container {{ background: white; padding: 20px; border-radius: 8px; max-width: 600px; margin: 0 auto; }}
h1 {{ color: #c00; }}
.url {{ word-break: break-all; background: #f5f5f5; padding: 10px; border-radius: 4px; }}
</style>
</head>
<body>
<div class="container">
<h1>Request Blocked</h1>
<p>The requested URL has been blocked by Privoxy.</p>
<p><strong>URL:</strong> <div class="url">{}</div></p>
<p><strong>Reason:</strong> {}</p>
</div>
</body>
</html>"#, url, reason);

    crate::http::HttpResponse {
        version: "HTTP/1.1".to_string(),
        status_code: 403,
        status_text: "Forbidden".to_string(),
        headers: vec![
            ("Content-Type".to_string(), "text/html; charset=utf-8".to_string()),
            ("Content-Length".to_string(), body.len().to_string()),
        ].into_iter().collect(),
        body: body.into_bytes(),
        compression_level: 0,
        header_len: 0,
    }
}

pub fn create_blocked_image_response() -> crate::http::HttpResponse {
    let blank_gif: [u8; 43] = [
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00,
        0x80, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x21,
        0xf9, 0x04, 0x01, 0x0a, 0x00, 0x01, 0x00, 0x2c, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x4c,
        0x01, 0x00, 0x3b
    ];

    crate::http::HttpResponse {
        version: "HTTP/1.1".to_string(),
        status_code: 200,
        status_text: "OK".to_string(),
        headers: vec![
            ("Content-Type".to_string(), "image/gif".to_string()),
            ("Content-Length".to_string(), blank_gif.len().to_string()),
            ("Cache-Control".to_string(), "max-age=86400".to_string()),
        ].into_iter().collect(),
        body: blank_gif.to_vec(),
        compression_level: 0,
        header_len: 0,
    }
}

pub fn create_empty_document_response(status_code: u16, status_text: &str) -> crate::http::HttpResponse {
    crate::http::HttpResponse {
        version: "HTTP/1.1".to_string(),
        status_code,
        status_text: status_text.to_string(),
        headers: vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            ("Content-Length".to_string(), "0".to_string()),
        ].into_iter().collect(),
        body: Vec::new(),
        compression_level: 0,
        header_len: 0,
    }
}

pub fn create_redirect_response(redirect_url: &str) -> crate::http::HttpResponse {
    crate::http::HttpResponse {
        version: "HTTP/1.1".to_string(),
        status_code: 302,
        status_text: "Found".to_string(),
        headers: vec![
            ("Location".to_string(), redirect_url.to_string()),
            ("Content-Length".to_string(), "0".to_string()),
        ].into_iter().collect(),
        body: Vec::new(),
        compression_level: 0,
        header_len: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_matches_pattern() {
        assert!(url_matches_pattern("example.com", "."));
        assert!(url_matches_pattern("example.com", "*"));
        assert!(url_matches_pattern("www.example.com", ".example.com"));
        assert!(url_matches_pattern("example.com", ".example.com"));
        assert!(url_matches_pattern("sub.example.com", "*.example.com"));
        assert!(url_matches_pattern("example.com", "example.com"));
        assert!(url_matches_pattern("test.example.com", "*.example.com"));
        assert!(!url_matches_pattern("example.org", ".example.com"));
    }

    #[test]
    fn test_check_limit_connect() {
        let mut action = Action::default();
        
        action.limit_connect = Some("443".to_string());
        assert!(check_limit_connect(&action, 443));
        assert!(!check_limit_connect(&action, 80));
        
        action.limit_connect = Some("80,443".to_string());
        assert!(check_limit_connect(&action, 80));
        assert!(check_limit_connect(&action, 443));
        assert!(!check_limit_connect(&action, 8080));
        
        action.limit_connect = Some("8080-8090".to_string());
        assert!(check_limit_connect(&action, 8085));
        assert!(!check_limit_connect(&action, 80));
    }

    #[test]
    fn test_apply_header_actions() {
        use crate::config::Action;
        use crate::http::Headers;
        
        let mut headers: Headers = vec![
            ("User-Agent".to_string(), "Mozilla/5.0".to_string()),
            ("Referer".to_string(), "http://google.com/".to_string()),
            ("Host".to_string(), "example.com".to_string()),
        ];
        
        let mut action = Action::default();
        action.hide_user_agent = Some("Privoxy/1.0".to_string());
        action.hide_referrer = Some("http://forge.com/".to_string());
        
        let config = crate::config::Config::default();
        let variables = FilterVariables::default();
        
        apply_client_header_actions(&mut headers, &config, &action, &variables, "127.0.0.1");
        
        assert_eq!(crate::http::get_header(&headers, "User-Agent"), Some(&"Privoxy/1.0".to_string()));
        assert_eq!(crate::http::get_header(&headers, "Referer"), Some(&"http://forge.com/".to_string()));
    }
}
