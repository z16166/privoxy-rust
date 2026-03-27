#![allow(dead_code)]
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tracing::{debug, info};

use crate::config::{Action, ForwardSpec, ForwardType, UrlAction};
use crate::error::{PrivoxyError, PrivoxyResult};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileList {
    pub filename: PathBuf,
    pub last_modified: SystemTime,
    pub active: bool,
    pub file_type: FileType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    Actions,
    Filters,
    Trust,
    Config,
}

impl FileList {
    pub fn new(filename: PathBuf, file_type: FileType) -> PrivoxyResult<Self> {
        let metadata = std::fs::metadata(&filename)
            .map_err(|e| PrivoxyError::Config(format!("Failed to stat {}: {}", filename.display(), e)))?;
        
        let last_modified = metadata.modified()
            .map_err(|e| PrivoxyError::Config(format!("Failed to get mtime for {}: {}", filename.display(), e)))?;

        Ok(Self {
            filename,
            last_modified,
            active: true,
            file_type,
        })
    }

    pub fn has_been_modified(&self) -> bool {
        std::fs::metadata(&self.filename)
            .and_then(|m| m.modified())
            .map(|t| t != self.last_modified)
            .unwrap_or(true)
    }

    pub fn check_file_changed(current: Option<&Self>, filename: &Path) -> PrivoxyResult<Option<Self>> {
        let statbuf = std::fs::metadata(filename)
            .map_err(|e| PrivoxyError::Config(format!("Failed to stat {}: {}", filename.display(), e)))?;

        if let Some(current) = current {
            if current.last_modified == statbuf.modified().unwrap_or(SystemTime::UNIX_EPOCH)
                && current.filename == filename
            {
                return Ok(None);
            }
        }

        let last_modified = statbuf.modified()
            .map_err(|e| PrivoxyError::Config(format!("Failed to get mtime for {}: {}", filename.display(), e)))?;

        Ok(Some(Self {
            filename: filename.to_path_buf(),
            last_modified,
            active: true,
            file_type: current.map(|c| c.file_type.clone()).unwrap_or(FileType::Actions),
        }))
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ActionsFile {
    pub file_list: FileList,
    pub url_actions: Vec<UrlAction>,
}

impl ActionsFile {
    pub fn load(filename: &Path) -> PrivoxyResult<Self> {
        info!("Loading actions file: {:?}", filename);
        
        let file_list = FileList::new(filename.to_path_buf(), FileType::Actions)?;
        let mut url_actions = Vec::new();

        let file = File::open(filename)
            .map_err(|e| PrivoxyError::Config(format!("Failed to open actions file: {}", e)))?;
        
        let reader = BufReader::new(file);
        let mut current_action = Action::default();
        let mut in_action_block = false;

        let mut line_number = 0;
        let mut section_start_line = 0;
        let lines = crate::util::read_lines(reader);
        for line in lines {
            line_number += 1;
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Check for action block start
            if line.starts_with('{') && line.ends_with('}') {
                current_action = Action::default();
                in_action_block = true;
                section_start_line = line_number;

                // Parse actions in the block
                let actions_str = &line[1..line.len() - 1];
                parse_action_string(actions_str, &mut current_action)?;
                continue;
            }

            // Check for URL patterns
            if in_action_block {
                url_actions.push(UrlAction {
                    patterns: vec![line.to_string()],
                    action: current_action.clone(),
                    file_name: filename.to_string_lossy().to_string(),
                    line_number: section_start_line,
                });
            }
        }


        info!("Loaded {} action rules from {:?}", url_actions.len(), filename);

        Ok(Self {
            file_list,
            url_actions,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FiltersFile {
    pub file_list: FileList,
    pub filters: Vec<crate::filter::Filter>,
}

impl FiltersFile {
    #[allow(dead_code)]
    pub fn load(filename: &Path) -> PrivoxyResult<Self> {
        info!("Loading filter file: {:?}", filename);
        
        let file_list = FileList::new(filename.to_path_buf(), FileType::Filters)?;
        
        let content = std::fs::read_to_string(filename)
            .map_err(|e| PrivoxyError::Config(format!("Failed to read filter file: {}", e)))?;

        let filters = crate::filter::parse_filter_file(&content)
            .map_err(|e| PrivoxyError::Config(format!("Failed to parse filters: {}", e)))?;

        info!("Loaded {} filters from {:?}", filters.len(), filename);

        Ok(Self {
            file_list,
            filters,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TrustFile {
    pub file_list: FileList,
    pub trust_patterns: Vec<String>,
}

impl TrustFile {
    pub fn load(filename: &Path) -> PrivoxyResult<Self> {
        info!("Loading trust file: {:?}", filename);
        
        let file_list = FileList::new(filename.to_path_buf(), FileType::Trust)?;
        let mut trust_patterns = Vec::new();

        let file = File::open(filename)
            .map_err(|e| PrivoxyError::Config(format!("Failed to open trust file: {}", e)))?;
        
        let reader = BufReader::new(file);

        let lines = crate::util::read_lines(reader);
        for line in lines {
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            trust_patterns.push(line.to_string());
        }

        info!("Loaded {} trust patterns from {:?}", trust_patterns.len(), filename);

        Ok(Self {
            file_list,
            trust_patterns,
        })
    }
}

pub(crate) fn parse_action_string(actions_str: &str, action: &mut Action) -> PrivoxyResult<()> {
    // Don't remove outer braces - the input may or may not have them
    let content = actions_str.trim();
    
    // Split by whitespace, but handle nested braces properly
    let mut tokens = Vec::new();
    let mut current_token = String::new();
    let mut brace_depth = 0;
    
    for ch in content.chars() {
        match ch {
            '{' => {
                brace_depth += 1;
                current_token.push(ch);
            }
            '}' => {
                brace_depth -= 1;
                current_token.push(ch);
            }
            ' ' | '\t' if brace_depth == 0 => {
                if !current_token.is_empty() {
                    tokens.push(current_token.clone());
                    current_token.clear();
                }
            }
            _ => {
                current_token.push(ch);
            }
        }
    }
    
    if !current_token.is_empty() {
        tokens.push(current_token);
    }
    
    // Parse each token
    for token in tokens {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        
        match token {
            "+block" => action.block = true,
            "-block" => action.block = false,
            s if s.starts_with("+block{") && s.ends_with('}') => {
                action.block = true;
                action.block_reason = Some(s[7..s.len() - 1].to_string());
            }
            "+redirect" => action.redirect = Some(String::new()),
            "-redirect" => action.redirect = None,
            s if s.starts_with("+redirect{") && s.ends_with('}') => {
                action.redirect = Some(s[10..s.len() - 1].to_string());
            }
            "+filter" => action.filter = true,
            "-filter" => action.filter = false,
            s if s.starts_with("+filter{") && s.ends_with('}') => {
                action.filter = true;
                action.filter_names.push(s[8..s.len() - 1].to_string());
            }
            s if s.starts_with("-filter{") && s.ends_with('}') => {
                let filter_name = s[8..s.len() - 1].to_string();
                action.filter_names.retain(|name| name != &filter_name);
            }
            "+handle-as-empty-document" => action.handle_as_empty_document = true,
            "-handle-as-empty-document" => action.handle_as_empty_document = false,
            "+handle-as-image" => action.handle_as_image = true,
            "-handle-as-image" => action.handle_as_image = false,
            "+inspect-jpegs" => action.inspect_jpegs = true,
            "-inspect-jpegs" => action.inspect_jpegs = false,
            "+deanimate-gifs" => action.deanimate_gifs = Some("last".to_string()),
            s if s.starts_with("+deanimate-gifs{") && s.ends_with('}') => {
                action.deanimate_gifs = Some(s[16..s.len() - 1].to_string());
            }
            "-deanimate-gifs" => action.deanimate_gifs = None,
            "+hide-from-header" => action.hide_from_header = Some("block".to_string()),
            s if s.starts_with("+hide-from-header{") && s.ends_with('}') => {
                action.hide_from_header = Some(s[18..s.len() - 1].to_string());
            }
            "-hide-from-header" => action.hide_from_header = None,
            "+hide-user-agent" => action.hide_user_agent = Some(String::new()),
            s if s.starts_with("+hide-user-agent{") && s.ends_with('}') => {
                action.hide_user_agent = Some(s["+hide-user-agent{".len()..s.len() - 1].to_string());
            }
            "-hide-user-agent" => action.hide_user_agent = None,
            "+hide-referrer" => action.hide_referrer = Some("block".to_string()),
            s if s.starts_with("+hide-referrer{") && s.ends_with('}') => {
                action.hide_referrer = Some(s[15..s.len() - 1].to_string());
            }
            "-hide-referrer" => action.hide_referrer = None,
            "+hide-accept-language" => action.hide_accept_language = Some("block".to_string()),
            s if s.starts_with("+hide-accept-language{") && s.ends_with('}') => {
                action.hide_accept_language = Some(s[22..s.len() - 1].to_string());
            }
            "-hide-accept-language" => action.hide_accept_language = None,
            "+set-image-blocker" => action.set_image_blocker = Some("pattern".to_string()),
            s if s.starts_with("+set-image-blocker{") && s.ends_with('}') => {
                action.set_image_blocker = Some(s[20..s.len() - 1].to_string());
            }
            "-set-image-blocker" => action.set_image_blocker = None,
            "+prevent-compression" => action.prevent_compression = true,
            "-prevent-compression" => action.prevent_compression = false,
            "+session-cookies-only" => action.session_cookies_only = true,
            "-session-cookies-only" => action.session_cookies_only = false,
            "+crunch-incoming-cookies" => action.crunch_incoming_cookies = true,
            "-crunch-incoming-cookies" => action.crunch_incoming_cookies = false,
            "+crunch-outgoing-cookies" => action.crunch_outgoing_cookies = true,
            "-crunch-outgoing-cookies" => action.crunch_outgoing_cookies = false,
            "+crunch-if-none-match" => action.crunch_if_none_match = true,
            "-crunch-if-none-match" => action.crunch_if_none_match = false,
            "+force-text-mode" => action.force_text_mode = true,
            "-force-text-mode" => action.force_text_mode = false,
            "+downgrade-http-version" => action.downgrade_http_version = true,
            "-downgrade-http-version" => action.downgrade_http_version = false,
            "+https-inspection" => action.https_inspection = true,
            "-https-inspection" => action.https_inspection = false,
            "+ignore-certificate-errors" => action.ignore_certificate_errors = true,
            "-ignore-certificate-errors" => action.ignore_certificate_errors = false,
            s if s.starts_with("+add-header{") && s.ends_with('}') => {
                action.add_headers.push(s[12..s.len() - 1].to_string());
            }
            s if s.starts_with("-add-header{") && s.ends_with('}') => {
                let header = s[12..s.len() - 1].to_string();
                action.add_headers.retain(|h| h != &header);
            }
            s if s.starts_with("+crunch-client-header{") && s.ends_with('}') => {
                action.crunch_client_headers.push(s[22..s.len() - 1].to_string());
            }
            s if s.starts_with("-crunch-client-header{") && s.ends_with('}') => {
                let header = s[22..s.len() - 1].to_string();
                action.crunch_client_headers.retain(|h| h != &header);
            }
            s if s.starts_with("+crunch-server-header{") && s.ends_with('}') => {
                action.crunch_server_headers.push(s[21..s.len() - 1].to_string());
            }
            s if s.starts_with("-crunch-server-header{") && s.ends_with('}') => {
                let header = s[21..s.len() - 1].to_string();
                action.crunch_server_headers.retain(|h| h != &header);
            }
            s if s.starts_with("+client-header-tagger{") && s.ends_with('}') => {
                action.client_header_tagger_names.push(s[22..s.len() - 1].to_string());
            }
            s if s.starts_with("-client-header-tagger{") && s.ends_with('}') => {
                let tagger = s[22..s.len() - 1].to_string();
                action.client_header_tagger_names.retain(|t| t != &tagger);
            }
            s if s.starts_with("+server-header-tagger{") && s.ends_with('}') => {
                action.server_header_tagger_names.push(s[22..s.len() - 1].to_string());
            }
            s if s.starts_with("-server-header-tagger{") && s.ends_with('}') => {
                let tagger = s[22..s.len() - 1].to_string();
                action.server_header_tagger_names.retain(|t| t != &tagger);
            }
            s if s.starts_with("+content-type-overwrite{") && s.ends_with('}') => {
                action.content_type_overwrite = Some(s[24..s.len() - 1].to_string());
            }
            "-content-type-overwrite" => action.content_type_overwrite = None,
            s if s.starts_with("+send-user-agent{") && s.ends_with('}') => {
                action.send_user_agent = Some(s[17..s.len() - 1].to_string());
            }
            "-send-user-agent" => action.send_user_agent = None,
            s if s.starts_with("+send-wafer{") && s.ends_with('}') => {
                action.send_wafer = Some(s[12..s.len() - 1].to_string());
            }
            "-send-wafer" => action.send_wafer = None,
            s if s.starts_with("+fast-redirects{") && s.ends_with('}') => {
                action.fast_redirects = Some(s[15..s.len() - 1].to_string());
            }
            "-fast-redirects" => action.fast_redirects = None,
            s if s.starts_with("+limit-connect{") && s.ends_with('}') => {
                action.limit_connect = Some(s[13..s.len() - 1].to_string());
            }
            "-limit-connect" => action.limit_connect = None,
            s if s.starts_with("+limit-connect-retries{") && s.ends_with('}') => {
                action.limit_connect_retries = Some(
                    s[22..s.len() - 1].parse()
                        .map_err(|_| PrivoxyError::Config(format!("Invalid limit-connect-retries value: {}", s)))?
                );
            }
            "-limit-connect-retries" => action.limit_connect_retries = None,
            s if s.starts_with("+limit-cookie-lifetime{") && s.ends_with('}') => {
                action.limit_cookie_lifetime = Some(
                    s[23..s.len() - 1].parse()
                        .map_err(|_| PrivoxyError::Config(format!("Invalid limit-cookie-lifetime value: {}", s)))?
                );
            }
            "-limit-cookie-lifetime" => action.limit_cookie_lifetime = None,
            s if s.starts_with("+overwrite-last-modified{") && s.ends_with('}') => {
                action.overwrite_last_modified = Some(s[25..s.len() - 1].to_string());
            }
            "-overwrite-last-modified" => action.overwrite_last_modified = None,
            s if s.starts_with("+change-x-forwarded-for{") && s.ends_with('}') => {
                action.change_x_forwarded_for = Some(s[23..s.len() - 1].to_string());
            }
            "-change-x-forwarded-for" => action.change_x_forwarded_for = None,
            s if s.starts_with("+forward-override{") && s.ends_with('}') => {
                action.forward_override = Some(s[18..s.len() - 1].to_string());
            }
            "-forward-override" => action.forward_override = None,
            s if s.starts_with("+delay-response{") && s.ends_with('}') => {
                action.delay_response = Some(
                    s[15..s.len() - 1].parse()
                        .map_err(|_| PrivoxyError::Config(format!("Invalid delay-response value: {}", s)))?
                );
            }
            "-delay-response" => action.delay_response = None,
            s if s.starts_with("+hide-content-disposition{") && s.ends_with('}') => {
                action.hide_content_disposition = Some(s[27..s.len() - 1].to_string());
            }
            "-hide-content-disposition" => action.hide_content_disposition = None,
            s if s.starts_with("+hide-if-modified-since{") && s.ends_with('}') => {
                action.hide_if_modified_since = Some(s[23..s.len() - 1].to_string());
            }
            "-hide-if-modified-since" => action.hide_if_modified_since = None,
            "+kill-popups" => action.kill_popups = true,
            "-kill-popups" => action.kill_popups = false,
            _ => {
                debug!("Unknown action token: {}", token);
            }
        }
    }

    Ok(())
}

pub fn parse_forward_directive(line: &str) -> PrivoxyResult<Option<ForwardSpec>> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    
    if parts.is_empty() {
        return Ok(None);
    }

    let (directive, forward_type) = match parts[0] {
        "forward" => (parts[0], ForwardType::Direct),
        "forward-socks4" => (parts[0], ForwardType::Socks4),
        "forward-socks4a" => (parts[0], ForwardType::Socks4a),
        "forward-socks5" => (parts[0], ForwardType::Socks5),
        "forward-socks5t" => (parts[0], ForwardType::Socks5t),
        "forward-http" => (parts[0], ForwardType::Http),
        "forward-webserver" => (parts[0], ForwardType::ForwardWebserver),
        _ => return Ok(None),
    };

    if parts.len() < 2 {
        return Err(PrivoxyError::Config(format!(
            "{} directive requires at least one argument",
            directive
        )));
    }

    // forward-override format does NOT have a source pattern.
    // forward host:port
    // forward-socks5 socks_host:port http_parent_host:port
    
    let mut spec = ForwardSpec {
        pattern: String::new(), // Pattern is handled by caller in connection.rs
        forward_type,
        gateway_host: None,
        gateway_port: 0,
        gateway_username: None,
        gateway_password: None,
        forward_host: None,
        forward_port: 0,
    };

    match spec.forward_type {
        ForwardType::Direct | ForwardType::ForwardWebserver | ForwardType::Http => {
            // forward (.|host:port)
            if parts.len() < 2 {
                return Ok(None);
            }
            let (host, port, _user, _pass) = parse_host_port(parts[1])?;
            if host != "0.0.0.0" {
                spec.forward_host = Some(host);
                spec.forward_port = port;
            }
        }
        ForwardType::Socks4 | ForwardType::Socks4a | ForwardType::Socks5 | ForwardType::Socks5t => {
            // forward-socks5 gateway [parent-proxy]
            if parts.len() < 2 {
                return Ok(None);
            }
            let (host, port, user, pass) = parse_host_port(parts[1])?;
            if host != "0.0.0.0" {
                spec.gateway_host = Some(host);
                spec.gateway_port = port;
                spec.gateway_username = user;
                spec.gateway_password = pass;
            }
            
            if parts.len() >= 3 {
                let (host, port, _user, _pass) = parse_host_port(parts[2])?;
                if host != "0.0.0.0" {
                    spec.forward_host = Some(host);
                    spec.forward_port = port;
                }
            }
        }
    }

    Ok(Some(spec))
}

fn parse_host_port(host_port: &str) -> PrivoxyResult<(String, u16, Option<String>, Option<String>)> {
    if host_port == "." {
        return Ok(("0.0.0.0".to_string(), 0, None, None));
    }

    let mut auth = (None, None);
    let host_part = if let Some((user_pass, host_port)) = host_port.split_once('@') {
        if let Some((user, pass)) = user_pass.split_once(':') {
            auth = (Some(user.to_string()), Some(pass.to_string()));
        } else {
            auth = (Some(user_pass.to_string()), None);
        }
        host_port
    } else {
        host_port
    };

    if let Some((host, port_str)) = host_part.rsplit_once(':') {
        let port: u16 = port_str.parse()
            .map_err(|_| PrivoxyError::Config(format!("Invalid port number: {}", port_str)))?;
        Ok((host.to_string(), port, auth.0, auth.1))
    } else {
        // Default port can be 0 or a logical default; caller handles specific defaults
        Ok((host_part.to_string(), 0, auth.0, auth.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn create_temp_actions_file(content: &str) -> std::path::PathBuf {
        let temp_dir = std::env::temp_dir();
        let thread_id = format!("{:?}", std::thread::current().id());
        let thread_id = thread_id.replace("ThreadId(", "").replace(")", "");
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let file_path = temp_dir.join(format!("actions_test_{}_{}_{}.txt", std::process::id(), thread_id, now));
        fs::write(&file_path, content).expect("Failed to write to temp file");
        file_path
    }

    #[test]
    fn test_parse_action_string_block() {
        let mut action = Action::default();
        parse_action_string("+block{Access denied}", &mut action).unwrap();
        
        assert!(action.block);
        assert_eq!(action.block_reason, Some("Access denied".to_string()));
    }

    #[test]
    fn test_parse_action_string_redirect() {
        let mut action = Action::default();
        parse_action_string("+redirect{http://example.com}", &mut action).unwrap();
        
        assert_eq!(action.redirect, Some("http://example.com".to_string()));
    }

    #[test]
    fn test_parse_action_string_filter() {
        let mut action = Action::default();
        parse_action_string("+filter{filter-name}", &mut action).unwrap();
        
        assert!(action.filter);
        assert_eq!(action.filter_names, vec!["filter-name".to_string()]);
    }

    #[test]
    fn test_parse_action_string_multiple() {
        let mut action = Action::default();
        parse_action_string("+block +filter{test} +handle-as-image", &mut action).unwrap();
        
        assert!(action.block);
        assert!(action.filter);
        assert!(action.handle_as_image);
        assert_eq!(action.filter_names, vec!["test".to_string()]);
    }

    #[test]
    fn test_parse_action_string_hide_headers() {
        let mut action = Action::default();
        parse_action_string("+hide-from-header{block} +hide-user-agent{Custom Agent}", &mut action).unwrap();
        
        assert_eq!(action.hide_from_header, Some("block".to_string()));
        assert_eq!(action.hide_user_agent, Some("Custom Agent".to_string()));
    }

    #[test]
    fn test_parse_action_string_cookies() {
        let mut action = Action::default();
        parse_action_string("+session-cookies-only +crunch-incoming-cookies", &mut action).unwrap();
        
        assert!(action.session_cookies_only);
        assert!(action.crunch_incoming_cookies);
    }

    #[test]
    fn test_parse_forward_directive_socks5() {
        let line = "forward-socks5 127.0.0.1:1080";
        let spec = parse_forward_directive(line).unwrap().unwrap();
        
        assert_eq!(spec.gateway_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(spec.gateway_port, 1080);
        assert_eq!(spec.forward_type, ForwardType::Socks5);
    }

    #[test]
    fn test_parse_forward_directive_direct() {
        let line = "forward .";
        let result = parse_forward_directive(line).unwrap().unwrap();
        
        assert!(result.gateway_host.is_none());
        assert!(result.forward_host.is_none());
    }

    #[test]
    fn test_file_list_modification() {
        let temp_file = create_temp_actions_file("{+block} test.com");
        let file_list = FileList::new(temp_file.clone(), FileType::Actions).unwrap();
        
        assert!(!file_list.has_been_modified());
        
        // Use a longer sleep BEFORE writing modification to ensure mtime diff
        thread::sleep(Duration::from_millis(3200));
        
        // Modify the file
        fs::write(&temp_file, "{+block} modified.com").unwrap();
        
        assert!(file_list.has_been_modified());
        
        // Cleanup
        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_actions_file_load() {
        let content = r#"
# Test actions file
{+block{Test block} +filter{test-filter}}
test.com
example.com

{+redirect{http://blocked.com}}
blocked.com
"#;
        
        let temp_file = create_temp_actions_file(content);
        let actions_file = ActionsFile::load(&temp_file).unwrap();
        
        // Use a longer sleep if needed for filesystem sync, but should be fine since load() is synchronous
        assert!(actions_file.url_actions.len() >= 1, "Should have loaded at least some actions");
        assert_eq!(actions_file.url_actions.len(), 3);
        assert!(actions_file.url_actions[0].action.block);
        assert_eq!(actions_file.url_actions[2].action.redirect, Some("http://blocked.com".to_string()));
        
        // Cleanup
        let _ = fs::remove_file(&temp_file);
    }

    #[test]
    fn test_check_file_changed() {
        let temp_file = create_temp_actions_file("{+block} test.com");
        
        let file_list = FileList::new(temp_file.clone(), FileType::Actions).unwrap();
        
        // File hasn't changed
        let result = FileList::check_file_changed(Some(&file_list), &temp_file).unwrap();
        assert!(result.is_none());
        
        // File has changed
        fs::write(&temp_file, "{+block} modified.com").unwrap();
        thread::sleep(Duration::from_millis(3100));
        
        let result = FileList::check_file_changed(Some(&file_list), &temp_file).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().filename, temp_file);
        
        // Cleanup
        let _ = fs::remove_file(&temp_file);
    }
}
