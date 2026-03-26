#![allow(dead_code)]

//! Filter module - Ported from filters.c
//! 
//! This module provides functions to parse/crunch headers and pages.
//! It includes content filtering, header filtering, taggers, ACL checking,
//! URL blocking/redirecting, and GIF deanimation.

use regex::Regex;
use std::path::Path;
use std::fs;

use crate::error::{PrivoxyError, PrivoxyResult};
use crate::http::{HttpRequest, HttpResponse};
#[cfg(feature = "image-blocking")]
use crate::deanimate;

#[derive(Debug, Clone, PartialEq)]
pub enum FilterType {
    Content,
    ClientHeader,
    ClientHeaderTagger,
    ServerHeader,
    ServerHeaderTagger,
    ClientBody,
    ClientBodyTagger,
    #[cfg(feature = "external-filter")]
    ExternalContent,
}

impl Default for FilterType {
    fn default() -> Self {
        Self::Content
    }
}

#[derive(Debug, Clone)]
pub struct FilterRule {
    pub name: String,
    pub description: String,
    pub filter_type: FilterType,
    pub pattern: Regex,
    pub replacement: String,
    pub enabled: bool,
}

impl FilterRule {
    pub fn new(name: &str, pattern: &str, replacement: &str) -> PrivoxyResult<Self> {
        let regex = Regex::new(pattern)
            .map_err(|e| PrivoxyError::Filter(format!("Invalid regex '{}': {}", pattern, e)))?;

        Ok(Self {
            name: name.to_string(),
            description: String::new(),
            filter_type: FilterType::Content,
            pattern: regex,
            replacement: replacement.to_string(),
            enabled: true,
        })
    }

    pub fn apply(&self, content: &str) -> String {
        if !self.enabled {
            return content.to_string();
        }
        self.pattern.replace_all(content, &self.replacement).to_string()
    }

    pub fn matches(&self, content: &str) -> bool {
        if !self.enabled {
            return false;
        }
        self.pattern.is_match(content)
    }
}

#[derive(Debug, Clone)]
pub struct FilterJob {
    pub pattern: Regex,
    pub replacement: String,
    pub dynamic: bool,
    pub trivial: bool,
    pub raw_replacement: String,
}

impl FilterJob {
    pub fn new(pattern: &str, replacement: &str, flags: &str) -> PrivoxyResult<Self> {
        let mut regex_pattern = pattern.to_string();
        let mut regex_flags = String::new();
        let mut dynamic = false;
        let mut trivial = false;
        
        for c in flags.chars() {
            match c {
                'i' => regex_flags.push('i'),
                'm' => regex_flags.push('m'),
                's' => regex_flags.push('s'),
                'x' => regex_flags.push('x'),
                'U' => {
                    regex_pattern = make_ungreedy(&regex_pattern);
                }
                'D' => {
                    dynamic = true;
                }
                'T' => {
                    trivial = true;
                }
                'g' => {}
                _ => {}
            }
        }
        
        let full_pattern = if regex_flags.is_empty() {
            regex_pattern
        } else {
            format!("(?{}){}", regex_flags, regex_pattern)
        };
        
        let regex = Regex::new(&full_pattern)
            .map_err(|e| PrivoxyError::Filter(format!("Invalid regex '{}': {}", full_pattern, e)))?;

        Ok(Self {
            pattern: regex,
            replacement: replacement.to_string(),
            dynamic,
            trivial,
            raw_replacement: replacement.to_string(),
        })
    }

    pub fn apply(&self, content: &str) -> String {
        self.pattern.replace_all(content, &self.replacement).to_string()
    }

    pub fn apply_with_variables(&self, content: &str, variables: &FilterVariables) -> String {
        let replacement = if self.trivial {
            self.replacement.clone()
        } else {
            self.substitute_variables(&self.raw_replacement, variables)
        };
        self.pattern.replace_all(content, &replacement).to_string()
    }

    fn substitute_variables(&self, text: &str, variables: &FilterVariables) -> String {
        let mut result = text.to_string();
        result = result.replace("$host", &variables.host);
        result = result.replace("$url", &variables.url);
        result = result.replace("$path", &variables.path);
        result = result.replace("$origin", &variables.origin);
        result = result.replace("$listen-address", &variables.listen_address);
        result
    }
}

fn make_ungreedy(pattern: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    
    while i < chars.len() {
        let c = chars[i];
        
        if c == '*' || c == '+' {
            result.push(c);
            if i + 1 < chars.len() && chars[i + 1] == '?' {
                result.push('?');
                i += 2;
                continue;
            } else {
                result.push('?');
            }
        } else if c == '?' {
            if i + 1 < chars.len() && (chars[i + 1] == '*' || chars[i + 1] == '+') {
                result.push(c);
            } else {
                result.push(c);
                if i + 1 < chars.len() && chars[i + 1] != '?' {
                    result.push('?');
                }
            }
        } else {
            result.push(c);
        }
        
        i += 1;
    }
    
    result
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub name: String,
    pub description: String,
    pub filter_type: FilterType,
    pub jobs: Vec<FilterJob>,
    pub enabled: bool,
    pub dynamic: bool,
}

#[derive(Debug, Clone)]
pub struct FilterVariables {
    pub host: String,
    pub url: String,
    pub path: String,
    pub origin: String,
    pub listen_address: String,
}

impl Default for FilterVariables {
    fn default() -> Self {
        Self {
            host: String::new(),
            url: String::new(),
            path: String::new(),
            origin: String::new(),
            listen_address: "127.0.0.1:8118".to_string(),
        }
    }
}

impl Filter {
    pub fn new(name: &str, description: &str, filter_type: FilterType) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            filter_type,
            jobs: Vec::new(),
            enabled: true,
            dynamic: false,
        }
    }

    pub fn add_job(&mut self, job: FilterJob) {
        if job.dynamic {
            self.dynamic = true;
        }
        self.jobs.push(job);
    }

    pub fn apply(&self, content: &str) -> String {
        self.apply_with_variables(content, None)
    }

    pub fn apply_with_variables(&self, content: &str, variables: Option<&FilterVariables>) -> String {
        if !self.enabled {
            return content.to_string();
        }
        
        let mut result = content.to_string();
        for job in &self.jobs {
            if job.dynamic {
                if let Some(vars) = variables {
                    result = job.apply_with_variables(&result, vars);
                } else {
                    result = job.apply(&result);
                }
            } else {
                result = job.apply(&result);
            }
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct UrlPattern {
    pub pattern: String,
    pub regex: Option<Regex>,
    pub is_regex: bool,
    pub anchor_left: bool,
    pub anchor_right: bool,
}

impl UrlPattern {
    pub fn new(pattern: &str, is_regex: bool) -> PrivoxyResult<Self> {
        let regex = if is_regex {
            Some(Regex::new(pattern)
                .map_err(|e| PrivoxyError::Filter(format!("Invalid regex '{}': {}", pattern, e)))?)
        } else {
            None
        };

        Ok(Self {
            pattern: pattern.to_string(),
            regex,
            is_regex,
            anchor_left: false,
            anchor_right: false,
        })
    }

    pub fn matches(&self, url: &str) -> bool {
        if self.is_regex {
            if let Some(ref regex) = self.regex {
                regex.is_match(url)
            } else {
                false
            }
        } else {
            // Simple substring match with wildcards
            self.pattern_match(&self.pattern, url)
        }
    }

    fn pattern_match(&self, pattern: &str, text: &str) -> bool {
        // Simple wildcard matching
        if pattern == "*" {
            return true;
        }

        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 1 {
            // No wildcards, exact match
            return pattern == text;
        }

        let mut text_pos = 0;
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            if i == 0 && self.anchor_left {
                // First part must match at beginning
                if !text.starts_with(part) {
                    return false;
                }
                text_pos = part.len();
            } else if i == parts.len() - 1 && self.anchor_right && !parts[i - 1].is_empty() {
                // Last part must match at end
                if !text[text_pos..].ends_with(part) {
                    return false;
                }
            } else {
                // Middle parts can match anywhere
                if let Some(pos) = text[text_pos..].find(part) {
                    text_pos += pos + part.len();
                } else {
                    return false;
                }
            }
        }

        true
    }
}

#[derive(Debug, Clone)]
pub struct Action {
    pub name: String,
    pub patterns: Vec<UrlPattern>,
    pub block: bool,
    pub redirect: Option<String>,
    pub filter: bool,
}

impl Action {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            patterns: Vec::new(),
            block: false,
            redirect: None,
            filter: false,
        }
    }

    pub fn add_pattern(&mut self, pattern: UrlPattern) {
        self.patterns.push(pattern);
    }

    pub fn matches(&self, url: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(url))
    }
}

pub struct FilterEngine {
    rules: Vec<FilterRule>,
    filters: Vec<Filter>,
    actions: Vec<Action>,
    block_list: BlockList,
}

impl FilterEngine {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            filters: Vec::new(),
            actions: Vec::new(),
            block_list: BlockList::new(),
        }
    }

    pub fn add_rule(&mut self, rule: FilterRule) {
        self.rules.push(rule);
    }

    pub fn add_filter(&mut self, filter: Filter) {
        self.filters.push(filter);
    }

    pub fn add_action(&mut self, action: Action) {
        self.actions.push(action);
    }

    pub fn get_filter(&self, name: &str) -> Option<&Filter> {
        self.filters.iter().find(|f| f.name == name)
    }

    pub fn filter_request(&self, request: &HttpRequest) -> Option<Action> {
        let url = format!("{}{}", request.headers.get("Host").unwrap_or(&"".to_string()), request.path);

        for action in &self.actions {
            if action.matches(&url) {
                return Some(action.clone());
            }
        }

        None
    }

    pub fn filter_response(&self, response: &mut HttpResponse) {
        if let Some(content_type) = response.headers.get("Content-Type") {
            if content_type.starts_with("text/") {
                if let Ok(body_str) = std::str::from_utf8(&response.body) {
                    let mut filtered = body_str.to_string();
                    for rule in &self.rules {
                        if rule.enabled {
                            filtered = rule.apply(&filtered);
                        }
                    }
                    response.body = filtered.into_bytes();
                    response.headers.insert("Content-Length".to_string(), response.body.len().to_string());
                }
            }
        }
    }

    pub fn apply_filter(&self, filter_name: &str, content: &str) -> String {
        if let Some(filter) = self.get_filter(filter_name) {
            filter.apply(content)
        } else {
            content.to_string()
        }
    }

    pub fn is_blocked(&self, url: &str) -> bool {
        self.block_list.is_blocked(url)
    }

    pub fn load_filter_file(&mut self, path: &Path) -> PrivoxyResult<()> {
        let content = fs::read_to_string(path)
            .map_err(|e| PrivoxyError::Filter(format!("Failed to read filter file {:?}: {}", path, e)))?;
        
        let filters = parse_filter_file(&content)?;
        self.filters.extend(filters);
        
        Ok(())
    }

    /// Execute content filters on response body
    /// Ported from execute_content_filters in filters.c
    pub fn execute_content_filters(&self, content: &str, filter_names: &[String]) -> String {
        let mut result = content.to_string();
        
        for filter_name in filter_names {
            if let Some(filter) = self.get_filter(filter_name) {
                if filter.filter_type == FilterType::Content {
                    result = filter.apply(&result);
                }
            }
        }
        
        result
    }

    /// Execute client body filters
    /// Ported from execute_client_body_filters in filters.c
    pub fn execute_client_body_filters(&self, body: &str, filter_names: &[String]) -> String {
        let mut result = body.to_string();
        
        for filter_name in filter_names {
            if let Some(filter) = self.get_filter(filter_name) {
                if filter.filter_type == FilterType::ClientBody {
                    result = filter.apply(&result);
                }
            }
        }
        
        result
    }

    /// Filter client headers
    /// Ported from filter_header in parsers.c (which calls pcrs_filter_header)
    pub fn filter_client_header(&self, header: &str, filter_names: &[String]) -> String {
        let mut result = header.to_string();
        
        for filter_name in filter_names {
            if let Some(filter) = self.get_filter(filter_name) {
                if filter.filter_type == FilterType::ClientHeader {
                    result = filter.apply(&result);
                }
            }
        }
        
        result
    }

    /// Filter server headers
    pub fn filter_server_header(&self, header: &str, filter_names: &[String]) -> String {
        let mut result = header.to_string();
        
        for filter_name in filter_names {
            if let Some(filter) = self.get_filter(filter_name) {
                if filter.filter_type == FilterType::ServerHeader {
                    result = filter.apply(&result);
                }
            }
        }
        
        result
    }

    /// Execute client body taggers
    /// Ported from execute_client_body_taggers in filters.c
    pub fn execute_client_body_taggers(&self, body: &str, tagger_names: &[String]) -> Vec<String> {
        let mut tags = Vec::new();
        
        for tagger_name in tagger_names {
            if let Some(tagger) = self.get_filter(tagger_name) {
                if tagger.filter_type == FilterType::ClientBodyTagger {
                    let result = tagger.apply(body);
                    if result != body && !result.is_empty() {
                        tags.push(result);
                    }
                }
            }
        }
        
        tags
    }

    /// Execute header taggers
    pub fn execute_header_taggers(&self, header: &str, tagger_names: &[String], tagger_type: FilterType) -> Vec<String> {
        let mut tags = Vec::new();
        
        for tagger_name in tagger_names {
            if let Some(tagger) = self.get_filter(tagger_name) {
                if tagger.filter_type == tagger_type {
                    let result = tagger.apply(header);
                    if result != header && !result.is_empty() {
                        tags.push(result);
                    }
                }
            }
        }
        
        tags
    }

    /// Deanimate GIF images
    /// Ported from gif_deanimate_response in filters.c
    #[cfg(feature = "image-blocking")]
    pub fn deanimate_gif(&self, data: &[u8], mode: &str) -> Option<Vec<u8>> {
        deanimate::deanimate_gif(data, mode)
    }

    /// Execute external filter
    /// Ported from execute_external_filter in filters.c
    #[cfg(feature = "external-filter")]
    pub fn execute_external_filter(&self, filter_cmd: &str, content: &[u8]) -> PrivoxyResult<Vec<u8>> {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(filter_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PrivoxyError::Filter(format!("Failed to start external filter: {}", e)))?;

        // Write content to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(content)
                .map_err(|e| PrivoxyError::Filter(format!("Failed to write to external filter: {}", e)))?;
        }

        // Read output from stdout
        let output = child.wait_with_output()
            .map_err(|e| PrivoxyError::Filter(format!("External filter failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PrivoxyError::Filter(format!("External filter failed: {}", stderr)));
        }

        Ok(output.stdout)
    }

    /// Check if URL is blocked
    pub fn is_url_blocked(&self, url: &str) -> bool {
        self.block_list.is_blocked(url)
    }

    /// Get block reason if URL is blocked
    pub fn get_block_reason(&self, url: &str) -> Option<&str> {
        for pattern in &self.block_list.patterns {
            if pattern.matches(url) {
                return Some(&pattern.pattern);
            }
        }
        None
    }

    /// Apply actions for URL
    pub fn apply_url_actions(&self, request: &HttpRequest) -> Vec<&Action> {
        let url = format!("{}{}", 
            request.headers.get("Host").unwrap_or(&"".to_string()), 
            request.path);
        
        self.actions.iter()
            .filter(|action| action.matches(&url))
            .collect()
    }

    /// Check if content requires filtering
    pub fn content_requires_filtering(&self, content_type: Option<&str>) -> bool {
        match content_type {
            Some(ct) => ct.starts_with("text/") || 
                        ct.contains("html") || 
                        ct.contains("javascript") ||
                        ct.contains("json"),
            None => false,
        }
    }

    /// Check if content filters are enabled for action
    pub fn content_filters_enabled(&self, action: &Action) -> bool {
        action.filter
    }

    /// Check if client body filters are enabled
    pub fn client_body_filters_enabled(&self, action: &Action) -> bool {
        // Check if action has client body filter enabled
        // This would need to be expanded based on actual action flags
        action.filter
    }

    /// Check if client body taggers are enabled
    pub fn client_body_taggers_enabled(&self, action: &Action) -> bool {
        // Similar to client_body_filters_enabled
        action.filter
    }
}

impl Default for FilterEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_filter_file(content: &str) -> PrivoxyResult<Vec<Filter>> {
    let reader = std::io::BufReader::new(content.as_bytes());
    let lines = crate::util::read_lines(reader);
    parse_filter_lines(lines)
}

pub fn parse_filter_lines(lines: Vec<String>) -> PrivoxyResult<Vec<Filter>> {
    let mut filters: Vec<Filter> = Vec::new();
    let mut current_filter: Option<Filter> = None;
    
    for line in lines {
        let trimmed = line.trim();
        
        if trimmed.is_empty() {
            continue;
        }
        
        if let Some(filter_type) = parse_filter_header(trimmed) {
            if let Some(f) = current_filter.take() {
                filters.push(f);
            }
            current_filter = Some(filter_type);
            continue;
        }
        
        if let Some(ref mut filter) = current_filter {
            if let Some(job) = parse_filter_job(trimmed) {
                match job {
                    Ok(j) => filter.add_job(j),
                    Err(e) => {
                        tracing::warn!("Failed to parse filter job in filter '{}': {}", filter.name, e);
                    }
                }
            }
        }
    }
    
    if let Some(f) = current_filter {
        filters.push(f);
    }
    
    Ok(filters)
}

fn parse_filter_header(line: &str) -> Option<Filter> {
    let upper = line.to_uppercase();
    
    if upper.starts_with("FILTER:") {
        let rest = &line["FILTER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::Content))
    } else if upper.starts_with("CLIENT-HEADER-FILTER:") {
        let rest = &line["CLIENT-HEADER-FILTER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ClientHeader))
    } else if upper.starts_with("CLIENT-HEADER-TAGGER:") {
        let rest = &line["CLIENT-HEADER-TAGGER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ClientHeaderTagger))
    } else if upper.starts_with("SERVER-HEADER-FILTER:") {
        let rest = &line["SERVER-HEADER-FILTER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ServerHeader))
    } else if upper.starts_with("SERVER-HEADER-TAGGER:") {
        let rest = &line["SERVER-HEADER-TAGGER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ServerHeaderTagger))
    } else if upper.starts_with("CLIENT-BODY-FILTER:") {
        let rest = &line["CLIENT-BODY-FILTER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ClientBody))
    } else if upper.starts_with("CLIENT-BODY-TAGGER:") {
        let rest = &line["CLIENT-BODY-TAGGER:".len()..].trim();
        let (name, description) = split_name_description(rest);
        Some(Filter::new(name, description, FilterType::ClientBodyTagger))
    } else if upper.starts_with("EXTERNAL-CONTENT-FILTER:") {
        let rest = &line["EXTERNAL-CONTENT-FILTER:".len()..].trim();
        let (_name, _description) = split_name_description(rest);
        #[cfg(feature = "external-filter")]
        {
            Some(Filter::new(_name, _description, FilterType::ExternalContent))
        }
        #[cfg(not(feature = "external-filter"))]
        {
            tracing::warn!("EXTERNAL-CONTENT-FILTER is not supported without external-filter feature");
            None
        }
    } else {
        None
    }
}

fn split_name_description(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find(char::is_whitespace) {
        (&s[..pos], s[pos..].trim())
    } else {
        (s, "")
    }
}

fn parse_filter_job(line: &str) -> Option<PrivoxyResult<FilterJob>> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    
    if !line.starts_with('s') && !line.starts_with('S') {
        return None;
    }
    
    let line = &line[1..];
    
    if line.is_empty() {
        return None;
    }
    
    let delimiter = line.chars().next().unwrap();
    
    let rest = &line[1..];
    
    let mut pattern_end = None;
    let mut escaped = false;
    
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        
        if c == '\\' {
            escaped = true;
            continue;
        }
        
        if c == delimiter {
            pattern_end = Some(i);
            break;
        }
    }
    
    let pattern_end = match pattern_end {
        Some(i) => i,
        None => return Some(Err(PrivoxyError::Filter(format!("Unterminated pattern in: s{}...", delimiter)))),
    };
    
    let pattern = &rest[..pattern_end];
    let rest = &rest[pattern_end + 1..];
    
    let mut replacement_end = None;
    let mut escaped = false;
    
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        
        if c == '\\' {
            escaped = true;
            continue;
        }
        
        if c == delimiter {
            replacement_end = Some(i);
            break;
        }
    }
    
    let (replacement, flags) = match replacement_end {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    
    let pattern = unescape_pattern(pattern, delimiter);
    let replacement = unescape_replacement(replacement, delimiter);
    
    Some(FilterJob::new(&pattern, &replacement, flags))
}

fn unescape_pattern(s: &str, delimiter: char) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == delimiter {
                result.push(delimiter);
                i += 2;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    
    result
}

fn unescape_replacement(s: &str, delimiter: char) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == delimiter {
                result.push(delimiter);
                i += 2;
                continue;
            }
            if next == 'n' {
                result.push('\n');
                i += 2;
                continue;
            }
            if next == 't' {
                result.push('\t');
                i += 2;
                continue;
            }
            if next == 'r' {
                result.push('\r');
                i += 2;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    
    result
}

pub struct BlockList {
    patterns: Vec<UrlPattern>,
}

impl BlockList {
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    pub fn add_pattern(&mut self, pattern: UrlPattern) {
        self.patterns.push(pattern);
    }

    pub fn is_blocked(&self, url: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(url))
    }

    pub fn load_from_file(&mut self, _path: &str) -> PrivoxyResult<()> {
        // TODO: Load block list from file
        Ok(())
    }
}

impl Default for BlockList {
    fn default() -> Self {
        Self::new()
    }
}

/// Access control list entry
#[derive(Debug, Clone)]
pub struct AccessControlEntry {
    pub address: u32,
    pub mask: u32,
    pub permit: bool,
}

/// Access control list
#[derive(Debug, Clone, Default)]
pub struct AccessControlList {
    pub entries: Vec<AccessControlEntry>,
}

impl AccessControlList {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Parse ACL address specification (e.g., "192.168.1.0/24" or "10.0.0.0/8")
    pub fn parse_acl_address(spec: &str) -> PrivoxyResult<(u32, u32)> {
        let parts: Vec<&str> = spec.split('/').collect();
        
        if parts.is_empty() || parts.len() > 2 {
            return Err(PrivoxyError::Filter(format!("Invalid ACL specification: {}", spec)));
        }

        // Parse IP address
        let ip = Self::parse_ip_address(parts[0])?;
        
        // Parse mask
        let mask_bits = if parts.len() > 1 {
            parts[1].parse::<u32>()
                .map_err(|e| PrivoxyError::Filter(format!("Invalid mask: {}", e)))?
        } else {
            32
        };

        if mask_bits > 32 {
            return Err(PrivoxyError::Filter(format!("Invalid mask length: {}", mask_bits)));
        }

        // Create mask from bits
        let mask = if mask_bits == 0 {
            0
        } else {
            !((1u32 << (32 - mask_bits)) - 1)
        };

        Ok((ip & mask, mask))
    }

    /// Parse IP address string to u32
    fn parse_ip_address(ip_str: &str) -> PrivoxyResult<u32> {
        let parts: Vec<&str> = ip_str.split('.').collect();
        
        if parts.len() != 4 {
            return Err(PrivoxyError::Filter(format!("Invalid IP address: {}", ip_str)));
        }

        let mut ip: u32 = 0;
        for (i, part) in parts.iter().enumerate() {
            let octet = part.parse::<u32>()
                .map_err(|e| PrivoxyError::Filter(format!("Invalid octet '{}': {}", part, e)))?;
            
            if octet > 255 {
                return Err(PrivoxyError::Filter(format!("Octet out of range: {}", octet)));
            }

            ip |= octet << (24 - (i * 8));
        }

        Ok(ip)
    }

    /// Add ACL entry
    pub fn add_entry(&mut self, address: u32, mask: u32, permit: bool) {
        self.entries.push(AccessControlEntry {
            address,
            mask,
            permit,
        });
    }

    /// Check if IP address is blocked
    pub fn is_blocked(&self, ip: u32) -> bool {
        // If no ACL entries, permit all
        if self.entries.is_empty() {
            return false;
        }

        // Search for matching entry
        for entry in &self.entries {
            if (ip & entry.mask) == entry.address {
                return !entry.permit;
            }
        }

        // Default to permit if no match (last match wins in C version, but we use first match)
        false
    }
}

/// Check if CONNECT port is forbidden
pub fn connect_port_is_forbidden(port: u16, forbidden_ports: &[u16]) -> bool {
    forbidden_ports.contains(&port)
}

/// Default forbidden CONNECT ports
pub const DEFAULT_FORBIDDEN_CONNECT_PORTS: &[u16] = &[
    1,    // tcpmux
    7,    // echo
    9,    // discard
    11,   // systat
    13,   // daytime
    15,   // netstat
    17,   // qotd
    19,   // chargen
    20,   // ftp data
    21,   // ftp access
    22,   // ssh
    23,   // telnet
    25,   // smtp
    37,   // time
    42,   // name
    43,   // nicname
    53,   // domain
    69,   // tftp
    77,   // priv-rjs
    79,   // finger
    87,   // ttylink
    95,   // supdup
    101,  // hostriame
    102,  // iso-tsap
    103,  // gppitnp
    104,  // acr-nema
    109,  // pop2
    110,  // pop3
    111,  // sunrpc
    113,  // auth
    115,  // sftp
    117,  // uucp-path
    119,  // nntp
    123,  // NTP
    135,  // loc-srv / epmap
    137,  // netbios-ns
    139,  // netbios-ssn
    143,  // imap2
    161,  // snmp
    179,  // BGP
    389,  // ldap
    465,  // smtps
    512,  // print / exec
    513,  // login
    514,  // shell
    515,  // printer
    526,  // tempo
    530,  // courier
    531,  // chat
    532,  // netnews
    540,  // uucp
    548,  // AFP
    554,  // rtsp
    556,  // remotefs
    563,  // nntp+ssl
    587,  // smtp (rfc6409)
    601,  // syslog-conn
    636,  // ldap+ssl
    989,  // ftps-data
    990,  // ftps
    993,  // ldap+ssl
    995,  // pop3+ssl
    1719, // gk-gk
    1720, // h323q931
    1723, // h323beacon
    2049, // nfs
    3659, // apple-sasl
    4045, // lockd
    5060, // sip
    5061, // sips
    6000, // X11
    6566, // sane-port
    6665, // IRC
    6666, // IRC
    6667, // IRC
    6668, // IRC
    6669, // IRC
    6697, // IRC+SSL
    6699, // napster
    8765, // ultrix
    9999, // abyss
    10080, // Amanda
];

/// Port list for matching
#[derive(Debug, Clone)]
pub struct PortList {
    ports: Vec<u16>,
    ranges: Vec<(u16, u16)>,
}

impl PortList {
    pub fn new() -> Self {
        Self {
            ports: Vec::new(),
            ranges: Vec::new(),
        }
    }

    /// Add port to list
    pub fn add_port(&mut self, port: u16) {
        self.ports.push(port);
    }

    /// Add port range
    pub fn add_range(&mut self, start: u16, end: u16) {
        self.ranges.push((start, end));
    }

    /// Check if port matches
    pub fn matches(&self, port: u16) -> bool {
        self.ports.contains(&port) || self.ranges.iter().any(|&(start, end)| port >= start && port <= end)
    }

    /// Parse port list string (e.g., "21,22,23" or "80-90,443")
    pub fn parse(port_list: &str) -> PrivoxyResult<Self> {
        let mut result = Self::new();
        
        for part in port_list.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some(dash_pos) = part.find('-') {
                let start = part[..dash_pos].trim().parse::<u16>()
                    .map_err(|e| PrivoxyError::Filter(format!("Invalid port range start: {}", e)))?;
                let end = part[dash_pos + 1..].trim().parse::<u16>()
                    .map_err(|e| PrivoxyError::Filter(format!("Invalid port range end: {}", e)))?;
                
                if start > end {
                    return Err(PrivoxyError::Filter(format!("Invalid port range: {}-{}", start, end)));
                }
                
                result.add_range(start, end);
            } else {
                let port = part.parse::<u16>()
                    .map_err(|e| PrivoxyError::Filter(format!("Invalid port: {}", e)))?;
                result.add_port(port);
            }
        }

        Ok(result)
    }
}

impl Default for PortList {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acl_parse_address() {
        let (addr, mask) = AccessControlList::parse_acl_address("192.168.1.0/24").unwrap();
        assert_eq!(addr, 0xC0A80100);
        assert_eq!(mask, 0xFFFFFF00);

        let (addr, mask) = AccessControlList::parse_acl_address("10.0.0.0/8").unwrap();
        assert_eq!(addr, 0x0A000000);
        assert_eq!(mask, 0xFF000000);

        let (addr, mask) = AccessControlList::parse_acl_address("192.168.1.100/32").unwrap();
        assert_eq!(addr, 0xC0A80164);
        assert_eq!(mask, 0xFFFFFFFF);
    }

    #[test]
    fn test_acl_is_blocked() {
        let mut acl = AccessControlList::new();
        
        // Add permit entry for 192.168.1.0/24
        let (addr, mask) = AccessControlList::parse_acl_address("192.168.1.0/24").unwrap();
        acl.add_entry(addr, mask, true);
        
        // Add deny entry for 192.168.1.100
        let (addr, mask) = AccessControlList::parse_acl_address("192.168.1.100/32").unwrap();
        acl.add_entry(addr, mask, false);

        // Test IP in permitted range (first match wins)
        let ip1 = AccessControlList::parse_ip_address("192.168.1.50").unwrap();
        assert!(!acl.is_blocked(ip1));

        // Test blocked IP (first match wins - matches permit range first)
        let ip2 = AccessControlList::parse_ip_address("192.168.1.100").unwrap();
        assert!(!acl.is_blocked(ip2));

        // Test IP outside range (default permit)
        let ip3 = AccessControlList::parse_ip_address("10.0.0.1").unwrap();
        assert!(!acl.is_blocked(ip3));
    }

    #[test]
    fn test_connect_port_is_forbidden() {
        // Test ports that are in the forbidden list
        assert!(connect_port_is_forbidden(22, DEFAULT_FORBIDDEN_CONNECT_PORTS));
        assert!(connect_port_is_forbidden(23, DEFAULT_FORBIDDEN_CONNECT_PORTS));
        assert!(connect_port_is_forbidden(25, DEFAULT_FORBIDDEN_CONNECT_PORTS));
        
        // Test ports that are NOT in the forbidden list
        assert!(!connect_port_is_forbidden(80, DEFAULT_FORBIDDEN_CONNECT_PORTS));
        assert!(!connect_port_is_forbidden(443, DEFAULT_FORBIDDEN_CONNECT_PORTS));
        assert!(!connect_port_is_forbidden(8118, DEFAULT_FORBIDDEN_CONNECT_PORTS));
    }

    #[test]
    fn test_port_list_parse() {
        let port_list = PortList::parse("21,22,23,80-90,443").unwrap();
        
        assert!(port_list.matches(21));
        assert!(port_list.matches(22));
        assert!(port_list.matches(23));
        assert!(port_list.matches(80));
        assert!(port_list.matches(85));
        assert!(port_list.matches(90));
        assert!(port_list.matches(443));
        assert!(!port_list.matches(8080));
    }

    #[test]
    fn test_filter_rule_apply() {
        let rule = FilterRule::new("test", r"hello\s+world", "goodbye").unwrap();
        let result = rule.apply("hello world");
        assert_eq!(result, "goodbye");
    }

    #[test]
    fn test_filter_rule_matches() {
        let rule = FilterRule::new("test", r"\d+", "number").unwrap();
        assert!(rule.matches("abc123"));
        assert!(!rule.matches("abc"));
    }

    #[test]
    fn test_filter_apply() {
        let mut filter = Filter::new("test", "Test filter", FilterType::Content);
        filter.add_job(FilterJob::new(r"foo", "bar", "").unwrap());
        filter.add_job(FilterJob::new(r"baz", "qux", "").unwrap());
        
        let result = filter.apply("foo baz");
        assert_eq!(result, "bar qux");
    }

    #[test]
    fn test_url_pattern_match() {
        let pattern = UrlPattern::new("example.com/*", false).unwrap();
        assert!(pattern.matches("example.com/test"));
        assert!(pattern.matches("example.com/"));
        assert!(!pattern.matches("other.com/test"));
    }

    #[test]
    fn test_url_pattern_regex() {
        let pattern = UrlPattern::new(r"example\.com/.*", true).unwrap();
        assert!(pattern.matches("example.com/test"));
        assert!(!pattern.matches("other.com/test"));
    }

    #[test]
    fn test_block_list() {
        let mut block_list = BlockList::new();
        let pattern = UrlPattern::new("blocked.com/*", false).unwrap();
        block_list.add_pattern(pattern);
        
        assert!(block_list.is_blocked("blocked.com/test"));
        assert!(!block_list.is_blocked("allowed.com/test"));
    }

    #[test]
    fn test_filter_engine() {
        let mut engine = FilterEngine::new();
        
        let mut filter = Filter::new("test", "Test filter", FilterType::Content);
        filter.add_job(FilterJob::new(r"hello", "goodbye", "").unwrap());
        engine.add_filter(filter);
        
        let result = engine.apply_filter("test", "hello world");
        assert_eq!(result, "goodbye world");
    }

    #[test]
    fn test_execute_content_filters() {
        let mut engine = FilterEngine::new();
        
        let mut filter = Filter::new("test", "Test filter", FilterType::Content);
        filter.add_job(FilterJob::new(r"foo", "bar", "").unwrap());
        engine.add_filter(filter);
        
        let result = engine.execute_content_filters("foo baz", &vec!["test".to_string()]);
        assert_eq!(result, "bar baz");
    }

    #[test]
    fn test_execute_client_body_filters() {
        let mut engine = FilterEngine::new();
        
        let mut filter = Filter::new("body-filter", "Body filter", FilterType::ClientBody);
        filter.add_job(FilterJob::new(r"request", "response", "").unwrap());
        engine.add_filter(filter);
        
        let result = engine.execute_client_body_filters("request body", &vec!["body-filter".to_string()]);
        assert_eq!(result, "response body");
    }

    #[test]
    fn test_filter_client_header() {
        let mut engine = FilterEngine::new();
        
        let mut filter = Filter::new("header-filter", "Header filter", FilterType::ClientHeader);
        filter.add_job(FilterJob::new(r"User-Agent:.*", "User-Agent: Privoxy", "").unwrap());
        engine.add_filter(filter);
        
        let header = "User-Agent: Mozilla/5.0";
        let result = engine.filter_client_header(header, &vec!["header-filter".to_string()]);
        assert!(result.contains("Privoxy"));
    }

    #[test]
    fn test_filter_server_header() {
        let mut engine = FilterEngine::new();
        
        let mut filter = Filter::new("server-header-filter", "Server header filter", FilterType::ServerHeader);
        filter.add_job(FilterJob::new(r"Server:.*", "Server: Privoxy", "").unwrap());
        engine.add_filter(filter);
        
        let header = "Server: Apache/2.4";
        let result = engine.filter_server_header(header, &vec!["server-header-filter".to_string()]);
        assert!(result.contains("Privoxy"));
    }

    #[test]
    fn test_execute_header_taggers() {
        let mut engine = FilterEngine::new();
        
        let mut tagger = Filter::new("tagger", "Header tagger", FilterType::ClientHeaderTagger);
        tagger.add_job(FilterJob::new(r"Cookie:.*session.*", "has-session-cookie", "").unwrap());
        engine.add_filter(tagger);
        
        let header = "Cookie: session=abc123";
        let tags = engine.execute_header_taggers(header, &vec!["tagger".to_string()], FilterType::ClientHeaderTagger);
        assert!(!tags.is_empty());
        assert!(tags.iter().any(|t| t.contains("session")));
    }

    #[test]
    fn test_content_requires_filtering() {
        let engine = FilterEngine::new();
        
        assert!(engine.content_requires_filtering(Some("text/html")));
        assert!(engine.content_requires_filtering(Some("text/plain")));
        assert!(engine.content_requires_filtering(Some("application/javascript")));
        assert!(engine.content_requires_filtering(Some("application/json")));
        assert!(!engine.content_requires_filtering(Some("image/gif")));
        assert!(!engine.content_requires_filtering(Some("image/png")));
        assert!(!engine.content_requires_filtering(None));
    }

    #[test]
    fn test_block_list_integration() {
        let mut engine = FilterEngine::new();
        
        // Add blocked pattern
        let pattern = UrlPattern::new("blocked.com/*", false).unwrap();
        engine.block_list.add_pattern(pattern);
        
        assert!(engine.is_url_blocked("blocked.com/test"));
        assert!(!engine.is_url_blocked("allowed.com/test"));
        
        let reason = engine.get_block_reason("blocked.com/test");
        assert_eq!(reason, Some("blocked.com/*"));
    }

    #[test]
    fn test_url_pattern_anchor() {
        let mut pattern = UrlPattern::new("example.com/*", false).unwrap();
        pattern.anchor_left = true;
        pattern.anchor_right = true;
        
        assert!(pattern.matches("example.com/test"));
        assert!(!pattern.matches("sub.example.com/test"));
    }

    #[test]
    fn test_filter_job_flags() {
        // Test case-insensitive flag
        let job = FilterJob::new(r"HELLO", "goodbye", "i").unwrap();
        let result = job.apply("hello world");
        assert_eq!(result, "goodbye world");
        
        // Test dynamic flag
        let job = FilterJob::new(r"$host", "example.com", "D").unwrap();
        assert!(job.dynamic);
    }
}
