use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{PrivoxyError, PrivoxyResult};
use crate::constants::*;

use ipnet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenAddress {
    pub addr: String,
    pub port: u16,
}

impl Default for ListenAddress {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".to_string(),
            port: 8118,
        }
    }
}

impl ListenAddress {
    pub fn to_socket_addr(&self) -> PrivoxyResult<SocketAddr> {
        let addr_str = format!("{}:{}", self.addr, self.port);
        addr_str.parse()
            .map_err(|e| PrivoxyError::Config(format!("Invalid listen address: {}", e)))
    }
}

impl std::fmt::Display for ListenAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.addr, self.port)
    }
}

#[derive(Debug, Clone)]
pub struct FileModification {
    pub path: PathBuf,
    pub modified_time: SystemTime,
}

impl FileModification {
    pub fn new(path: PathBuf) -> PrivoxyResult<Self> {
        let metadata = fs::metadata(&path)
            .map_err(|e| PrivoxyError::Config(format!("Failed to stat {}: {}", path.display(), e)))?;
        let modified_time = metadata.modified()
            .map_err(|e| PrivoxyError::Config(format!("Failed to get mtime for {}: {}", path.display(), e)))?;
        Ok(Self { path, modified_time })
    }
    
    pub fn has_been_modified(&self) -> bool {
        fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .map(|t| t != self.modified_time)
            .unwrap_or(true)
    }
}

pub type ConfigRef = Arc<RwLock<Config>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwardSpec {
    pub pattern: String,
    pub forward_type: ForwardType,
    pub gateway_host: Option<String>,
    pub gateway_port: u16,
    pub forward_host: Option<String>,
    pub forward_port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ForwardType {
    Direct,
    Socks4,
    Socks4a,
    Socks5,
    Socks5t,  // SOCKS5 with Tor optimistic data extension
    Http,
    ForwardWebserver,  // Forward directly to web server (no full URL in request line)
}

impl Default for ForwardType {
    fn default() -> Self {
        ForwardType::Direct
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub enabled: bool,
    pub block: bool,
    pub block_reason: Option<String>,
    pub redirect: Option<String>,
    pub filter: bool,
    pub filter_names: Vec<String>,
    pub client_body_filter_names: Vec<String>,
    pub client_header_filter_names: Vec<String>,
    pub server_header_filter_names: Vec<String>,
    pub client_header_tagger_names: Vec<String>,
    pub server_header_tagger_names: Vec<String>,
    pub client_body_tagger_names: Vec<String>,
    #[cfg(feature = "external-filter")]
    pub external_filter_names: Vec<String>,
    pub tags: Vec<String>,
    pub suppress_tags: Vec<String>,
    pub handle_as_empty_document: bool,
    pub handle_as_image: bool,
    pub limit_connect: Option<String>,
    pub limit_connect_retries: Option<u32>,
    pub fast_redirects: Option<String>,
    pub force_text_mode: bool,
    pub hide_accept_language: Option<String>,
    pub hide_content_disposition: Option<String>,
    pub hide_from_header: Option<String>,
    pub hide_if_modified_since: Option<String>,
    pub hide_referrer: Option<String>,
    pub hide_user_agent: Option<String>,
    pub inspect_jpegs: bool,
    pub kill_popups: bool,
    pub limit_cookie_lifetime: Option<u64>,
    pub overwrite_last_modified: Option<String>,
    pub prevent_compression: bool,
    pub send_wafer: Option<String>,
    pub send_user_agent: Option<String>,
    pub session_cookies_only: bool,
    pub set_image_blocker: Option<String>,
    pub add_headers: Vec<String>,
    pub crunch_client_headers: Vec<String>,
    pub crunch_server_headers: Vec<String>,
    pub crunch_incoming_cookies: bool,
    pub crunch_outgoing_cookies: bool,
    pub crunch_if_none_match: bool,
    pub deanimate_gifs: Option<String>,
    pub downgrade_http_version: bool,
    pub content_type_overwrite: Option<String>,
    pub change_x_forwarded_for: Option<String>,
    pub delay_response: Option<u64>,
    pub forward_override: Option<String>,
    pub https_inspection: bool,
    pub ignore_certificate_errors: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ClientSpecificTag {
    pub name: String,
    pub description: String,
}

impl Default for Action {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            block: false,
            block_reason: None,
            redirect: None,
            filter: false,
            filter_names: Vec::new(),
            client_body_filter_names: Vec::new(),
            client_header_filter_names: Vec::new(),
            server_header_filter_names: Vec::new(),
            client_header_tagger_names: Vec::new(),
            server_header_tagger_names: Vec::new(),
            client_body_tagger_names: Vec::new(),
            #[cfg(feature = "external-filter")]
            external_filter_names: Vec::new(),
            tags: Vec::new(),
            suppress_tags: Vec::new(),
            handle_as_empty_document: false,
            handle_as_image: false,
            limit_connect: None,
            limit_connect_retries: None,
            fast_redirects: None,
            force_text_mode: false,
            hide_accept_language: None,
            hide_content_disposition: None,
            hide_from_header: None,
            hide_if_modified_since: None,
            hide_referrer: None,
            hide_user_agent: None,
            inspect_jpegs: false,
            kill_popups: false,
            limit_cookie_lifetime: None,
            overwrite_last_modified: None,
            prevent_compression: false,
            send_wafer: None,
            send_user_agent: None,
            session_cookies_only: false,
            set_image_blocker: None,
            add_headers: Vec::new(),
            crunch_client_headers: Vec::new(),
            crunch_server_headers: Vec::new(),
            crunch_incoming_cookies: false,
            crunch_outgoing_cookies: false,
            crunch_if_none_match: false,
            deanimate_gifs: None,
            downgrade_http_version: false,
            content_type_overwrite: None,
            change_x_forwarded_for: None,
            delay_response: None,
            forward_override: None,
            https_inspection: false,
            ignore_certificate_errors: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlAction {
    pub patterns: Vec<String>,
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub pattern: String,
    #[serde(skip)]
    pub regex: Option<Regex>,
    pub replacement: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub listen_addresses: Vec<ListenAddress>,
    pub forward_specs: Vec<ForwardSpec>,
    pub actions: Vec<Action>,
    pub url_actions: Vec<UrlAction>,
    pub filter_rules: Vec<FilterRule>,
    #[serde(skip)]
    pub filters: Vec<crate::filter::Filter>,
    pub log_file: Option<PathBuf>,
    pub log_level: u32,
    pub toggle: bool,
    pub enable_remote_toggle: bool,
    pub enable_edit_actions: bool,
    pub buffer_limit: usize,
    pub connection_timeout_secs: u64,
    pub keep_alive_timeout_secs: u64,
    pub max_client_connections: usize,
    pub trust_files: Vec<PathBuf>,
    pub actions_files: Vec<PathBuf>,
    pub filter_files: Vec<PathBuf>,
    pub permit_access: Vec<String>,
    pub deny_access: Vec<String>,
    pub forwarded_connect_retries: u32,
    pub admin_address: Option<String>,
    pub proxy_info_url: Option<String>,
    pub user_manual: Option<String>,
    pub trust_info_url: Vec<String>,
    pub confdir: Option<PathBuf>,
    pub templdir: Option<PathBuf>,
    pub temporary_directory: Option<PathBuf>,
    pub logdir: Option<PathBuf>,
    pub enable_remote_http_toggle: bool,
    pub enforce_blocks: bool,
    pub enable_proxy_authentication_forwarding: bool,
    pub trusted_cgi_referers: Vec<String>,
    pub cors_allowed_origins: Vec<String>,
    pub hostname: Option<String>,
    pub single_threaded: bool,
    pub compression_level: u8,
    pub feature_flags: FeatureFlags,
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
    #[serde(skip)]
    pub file_modifications: Vec<FileModification>,
    #[serde(skip)]
    pub reload_requested: bool,
    pub client_header_order: Vec<String>,
    pub client_specific_tags: Vec<ClientSpecificTag>,
    pub accept_intercepted_requests: bool,
    pub client_tag_lifetime: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    pub enable_compression: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            enable_compression: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addresses: vec![ListenAddress::default()],
            forward_specs: Vec::new(),
            actions: Vec::new(),
            url_actions: Vec::new(),
            filter_rules: Vec::new(),
            filters: Vec::new(),
            log_file: None,
            log_level: LOG_LEVEL_INFO | LOG_LEVEL_ERROR | LOG_LEVEL_FATAL,
            toggle: true,
            enable_remote_toggle: true,
            enable_edit_actions: true,
            buffer_limit: 4096 * 1024, // 4MB
            connection_timeout_secs: CONNECTION_TIMEOUT_SECS,
            keep_alive_timeout_secs: KEEP_ALIVE_TIMEOUT_SECS,
            max_client_connections: MAX_CONNECTIONS,
            trust_files: Vec::new(),
            actions_files: Vec::new(),
            filter_files: Vec::new(),
            permit_access: Vec::new(), // Deny all by default, matching C implementation
            deny_access: Vec::new(),
            forwarded_connect_retries: 0,
            admin_address: None,
            proxy_info_url: Some(HOME_PAGE_URL.to_string()),
            user_manual: None,
            trust_info_url: Vec::new(),
            confdir: None,
            templdir: None,
            temporary_directory: None,
            logdir: None,
            enable_remote_http_toggle: false,
            enforce_blocks: false,
            enable_proxy_authentication_forwarding: false,
            trusted_cgi_referers: Vec::new(),
            cors_allowed_origins: Vec::new(),
            hostname: None,
            single_threaded: false,
            compression_level: 6, // Default compression level
            feature_flags: FeatureFlags::default(),
            config_file: None,
            file_modifications: Vec::new(),
            reload_requested: false,
            client_header_order: Vec::new(),
            client_specific_tags: Vec::new(),
            accept_intercepted_requests: false,
            client_tag_lifetime: 60, // Default to 60 seconds
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> PrivoxyResult<Self> {
        info!("Loading configuration from {:?}", path);

        if !path.exists() {
            warn!("Config file not found at {:?}, using defaults", path);
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| PrivoxyError::Config(format!("Failed to read config file: {}", e)))?;

        let mut config = Self::parse(&content)?;
        config.config_file = Some(path.to_path_buf());

        // Set confdir to the directory of the config file if not specified
        if config.confdir.is_none() {
            if let Some(parent) = path.parent() {
                config.confdir = Some(parent.to_path_buf());
            }
        }

        // Resolve all file paths relative to confdir
        if let Some(confdir) = &config.confdir {
            // Resolve actions files
            config.actions_files = config.actions_files.into_iter()
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        confdir.join(path)
                    }
                })
                .collect();

            // Resolve filter files
            config.filter_files = config.filter_files.into_iter()
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        confdir.join(path)
                    }
                })
                .collect();

            // Resolve trust files
            config.trust_files = config.trust_files.into_iter()
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        confdir.join(path)
                    }
                })
                .collect();

            // Resolve log file relative to logdir or confdir
            if let Some(log_file) = &config.log_file {
                if !log_file.is_absolute() {
                    if let Some(logdir) = &config.logdir {
                        config.log_file = Some(logdir.join(log_file));
                    } else {
                        config.log_file = Some(confdir.join(log_file));
                    }
                }
            }
        }
        
        // Record modification time for config file
        if let Ok(file_mod) = FileModification::new(path.to_path_buf()) {
            config.file_modifications.push(file_mod);
        }
        
        // Record modification times for action files
        for actions_file in &config.actions_files {
            if actions_file.exists() {
                if let Ok(file_mod) = FileModification::new(actions_file.clone()) {
                    config.file_modifications.push(file_mod);
                }
            }
        }
        
        // Record modification times for filter files
        for filter_file in &config.filter_files {
            if filter_file.exists() {
                if let Ok(file_mod) = FileModification::new(filter_file.clone()) {
                    config.file_modifications.push(file_mod);
                }
                
                // Parse filter file
                match crate::filter::parse_filter_file(&std::fs::read_to_string(filter_file).unwrap_or_default()) {
                    Ok(filters) => {
                        info!("Loaded {} filters from {:?}", filters.len(), filter_file);
                        config.filters.extend(filters);
                    }
                    Err(e) => {
                        warn!("Failed to parse filter file {:?}: {}", filter_file, e);
                    }
                }
            }
        }
        
        // Record modification times for trust files
        for trust_file in &config.trust_files {
            if trust_file.exists() {
                if let Ok(file_mod) = FileModification::new(trust_file.clone()) {
                    config.file_modifications.push(file_mod);
                }
            }
        }

        Ok(config)
    }

    pub fn parse(content: &str) -> PrivoxyResult<Self> {
        let mut config = Config::default();

        let reader = std::io::BufReader::new(content.as_bytes());
        let lines = crate::util::read_lines(reader);
        for line in lines {
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            // Parse configuration directives
            if let Some((key, value)) = line.split_once(' ') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "listen-address" => {
                        config.listen_addresses.extend(parse_listen_addresses(value)?);
                    }
                    "logfile" => {
                        config.log_file = Some(PathBuf::from(value));
                    }
                    "toggle" => {
                        config.toggle = parse_bool(value)?;
                    }
                    "enable-remote-toggle" => {
                        config.enable_remote_toggle = parse_bool(value)?;
                    }
                    "enable-edit-actions" => {
                        config.enable_edit_actions = parse_bool(value)?;
                    }
                    "buffer-limit" => {
                        let val: usize = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid buffer limit: {}", value)))?;
                        config.buffer_limit = val * 1024; // C version uses KB
                    }
                    "max-client-connections" => {
                        config.max_client_connections = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid max connections: {}", value)))?;
                    }
                    "actionsfile" => {
                        config.actions_files.push(PathBuf::from(value));
                    }
                    "filterfile" => {
                        config.filter_files.push(PathBuf::from(value));
                    }
                    "trustfile" => {
                        config.trust_files.push(PathBuf::from(value));
                    }
                    "permit-access" => {
                        config.permit_access.push(value.to_string());
                    }
                    "deny-access" => {
                        config.deny_access.push(value.to_string());
                    }
                    "admin-address" => {
                        config.admin_address = Some(value.to_string());
                    }
                    "forward" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Direct)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-socks4" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Socks4)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-socks4a" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Socks4a)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-socks5" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Socks5)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-socks5t" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Socks5t)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-http" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::Http)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "forward-webserver" => {
                        if let Some(spec) = parse_forward_spec(value, ForwardType::ForwardWebserver)? {
                            config.forward_specs.push(spec);
                        }
                    }
                    "debug" | "log-level" => {
                        config.log_level |= parse_debug_level(value)?;
                    }
                    "enable-compression" => {
                        config.feature_flags.enable_compression = parse_bool(value)?;
                    }
                    "compression-level" => {
                        let level: u8 = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid compression level: {}", value)))?;
                        if level > 9 {
                            return Err(PrivoxyError::Config("Compression level must be 0-9".to_string()));
                        }
                        config.compression_level = level;
                    }
                    "user-manual" => {
                        config.user_manual = Some(value.to_string());
                    }
                    "trust-info-url" => {
                        config.trust_info_url.push(value.to_string());
                    }
                    "confdir" => {
                        config.confdir = Some(PathBuf::from(value));
                    }
                    "templdir" => {
                        config.templdir = Some(PathBuf::from(value));
                    }
                    "temporary-directory" => {
                        config.temporary_directory = Some(PathBuf::from(value));
                    }
                    "logdir" => {
                        config.logdir = Some(PathBuf::from(value));
                    }
                    "enable-remote-http-toggle" => {
                        config.enable_remote_http_toggle = parse_bool(value)?;
                    }
                    "enforce-blocks" => {
                        config.enforce_blocks = parse_bool(value)?;
                    }
                    "enable-proxy-authentication-forwarding" => {
                        config.enable_proxy_authentication_forwarding = parse_bool(value)?;
                    }
                    "trusted-cgi-referer" => {
                        config.trusted_cgi_referers.push(value.to_string());
                    }
                    "cors-allowed-origin" => {
                        config.cors_allowed_origins.push(value.to_string());
                    }
                    "hostname" => {
                        config.hostname = Some(value.to_string());
                    }
                    "single-threaded" => {
                        config.single_threaded = parse_bool(value)?;
                    }
                    "proxy-info-url" => {
                        config.proxy_info_url = Some(value.to_string());
                    }
                    "client-header-order" => {
                        config.client_header_order = value.split_whitespace()
                            .map(|s| s.to_string())
                            .collect();
                    }
                    "client-specific-tag" => {
                        if let Some((name, description)) = value.split_once(' ') {
                            config.client_specific_tags.push(ClientSpecificTag {
                                name: name.trim().to_string(),
                                description: description.trim().to_string(),
                            });
                        }
                    }
                    "client-tag-lifetime" => {
                        config.client_tag_lifetime = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid client-tag-lifetime: {}", value)))?;
                    }
                    "accept-intercepted-requests" => {
                        config.accept_intercepted_requests = parse_bool(value)?;
                    }
                    "log-file" => {
                        config.log_file = Some(PathBuf::from(value));
                    }
                    "socket-timeout" | "connection-timeout" => {
                        config.connection_timeout_secs = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid timeout: {}", value)))?;
                    }
                    "keep-alive-timeout" => {
                        config.keep_alive_timeout_secs = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid keep-alive timeout: {}", value)))?;
                    }
                    "forwarded-connect-retries" => {
                        config.forwarded_connect_retries = value.parse()
                            .map_err(|_| PrivoxyError::Config(format!("Invalid retries: {}", value)))?;
                    }
                    "header-order" => {
                        // Header ordering is handled during HTTP parsing
                        debug!("Header-order directive: {}", value);
                    }
                    "accept-intercepted-connections" => {
                        // This is a toggle to accept intercepted connections
                        debug!("Accept-intercepted-connections directive: {}", value);
                    }
                    "allow-cgi-request-crunching" => {
                        // Allow CGI request crunching
                        debug!("Allow-cgi-request-crunching directive: {}", value);
                    }
                    "split-large-forms" => {
                        // Split large forms in CGI interface
                        debug!("Split-large-forms directive: {}", value);
                    }
                    _ => {
                        warn!("Unknown configuration directive: {} {}", key, value);
                    }
                }
            }
        }

        // Parse action files using loaders module
        let actions_files: Vec<_> = config.actions_files.clone();
        for actions_file_path in &actions_files {
            if actions_file_path.exists() {
                match crate::loaders::ActionsFile::load(actions_file_path) {
                    Ok(actions_file) => {
                        info!("Loaded {} action rules from {:?}", actions_file.url_actions.len(), actions_file_path);
                        config.url_actions.extend(actions_file.url_actions);
                        // Record file modification time
                        if let Ok(file_mod) = crate::loaders::FileList::new(actions_file_path.clone(), crate::loaders::FileType::Actions) {
                            config.file_modifications.push(crate::config::FileModification {
                                path: file_mod.filename,
                                modified_time: file_mod.last_modified,
                            });
                        }
                    }
                    Err(e) => warn!("Failed to load actions file {:?}: {}", actions_file_path, e),
                }
            }
        }

        info!("Configuration loaded successfully");
        Ok(config)
    }

    pub fn get_forward_spec(&self, host: &str, _port: u16) -> Option<&ForwardSpec> {
        self.forward_specs.iter().find(|spec| {
            matches_host_pattern(host, &spec.pattern)
        })
    }

    pub fn is_access_allowed(&self, addr: &str) -> bool {
        // Check deny list first
        for pattern in &self.deny_access {
            if matches_pattern(addr, pattern) {
                return false;
            }
        }

        // If permit list is empty, allow all (unless specifically denied above)
        if self.permit_access.is_empty() {
            return true;
        }

        // Then check permit list
        for pattern in &self.permit_access {
            if matches_pattern(addr, pattern) {
                return true;
            }
        }

        // Default deny if permit list is not empty and no match was found
        false
    }
    
    pub fn any_loaded_file_changed(&self) -> bool {
        for file_mod in &self.file_modifications {
            if file_mod.has_been_modified() {
                info!("Configuration file {:?} has been modified", file_mod.path);
                return true;
            }
        }
        false
    }
    
    pub fn needs_reload(&self) -> bool {
        self.reload_requested || self.any_loaded_file_changed()
    }
    
    pub fn request_reload(&mut self) {
        self.reload_requested = true;
    }
    
    pub fn reload_if_changed(&self) -> PrivoxyResult<Option<Config>> {
        if !self.needs_reload() {
            return Ok(None);
        }
        
        info!("Reloading configuration due to file changes or reload request");
        
        let config_file = match &self.config_file {
            Some(path) => path,
            None => return Ok(None),
        };
        
        match Config::load(config_file) {
            Ok(mut new_config) => {
                new_config.reload_requested = false;
                info!("Configuration reloaded successfully");
                Ok(Some(new_config))
            }
            Err(e) => {
                warn!("Failed to reload configuration: {}", e);
                Err(e)
            }
        }
    }
}

fn parse_listen_addresses(value: &str) -> PrivoxyResult<Vec<ListenAddress>> {
    let mut addresses = Vec::new();

    for part in value.split_whitespace() {
        if let Some((addr, port_str)) = part.rsplit_once(':') {
            let port = port_str.parse()
                .map_err(|_| PrivoxyError::Config(format!("Invalid port: {}", port_str)))?;
            addresses.push(ListenAddress {
                addr: addr.to_string(),
                port,
            });
        } else {
            // Just a port number, use default address
            let port = part.parse()
                .map_err(|_| PrivoxyError::Config(format!("Invalid port: {}", part)))?;
            addresses.push(ListenAddress {
                addr: "127.0.0.1".to_string(),
                port,
            });
        }
    }

    if addresses.is_empty() {
        addresses.push(ListenAddress::default());
    }

    Ok(addresses)
}

fn parse_bool(value: &str) -> PrivoxyResult<bool> {
    match value.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Ok(true),
        "0" | "no" | "false" | "off" => Ok(false),
        _ => Err(PrivoxyError::Config(format!("Invalid boolean value: {}", value))),
    }
}

fn parse_proxy_spec(proxy: &str, default_port: u16) -> PrivoxyResult<(String, u16)> {
    if proxy == "." {
        return Ok(("0.0.0.0".to_string(), 0));
    }
    if let Some((host, port_str)) = proxy.rsplit_once(':') {
        let port = port_str.parse()
            .map_err(|_| PrivoxyError::Config(format!("Invalid proxy port: {}", port_str)))?;
        Ok((host.to_string(), port))
    } else {
        Ok((proxy.to_string(), default_port))
    }
}

fn parse_forward_spec(value: &str, forward_type: ForwardType) -> PrivoxyResult<Option<ForwardSpec>> {
    // C format in config file:
    // forward  url-pattern  http-proxy-host[:port]
    // forward-socks5  url-pattern  socks-proxy[:port]  http-proxy-host[:port]
    
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(None);
    }

    let pattern = parts[0].to_string();
    let mut spec = ForwardSpec {
        pattern,
        forward_type,
        gateway_host: None,
        gateway_port: 0,
        forward_host: None,
        forward_port: 0,
    };

    match spec.forward_type {
        ForwardType::Direct | ForwardType::ForwardWebserver | ForwardType::Http => {
            // forward pattern (.|http-proxy)
            if parts.len() >= 2 {
                let (host, port) = parse_proxy_spec(parts[1], 8000)?;
                if host != "0.0.0.0" {
                    spec.forward_host = Some(host);
                    spec.forward_port = port;
                }
            }
        }
        ForwardType::Socks4 | ForwardType::Socks4a | ForwardType::Socks5 | ForwardType::Socks5t => {
            // forward-socks5 pattern socks-proxy [http-proxy]
            if parts.len() >= 2 {
                let (host, port) = parse_proxy_spec(parts[1], 1080)?;
                if host != "0.0.0.0" {
                    spec.gateway_host = Some(host);
                    spec.gateway_port = port;
                }
            }
            if parts.len() >= 3 {
                let (host, port) = parse_proxy_spec(parts[2], 8000)?;
                if host != "0.0.0.0" {
                    spec.forward_host = Some(host);
                    spec.forward_port = port;
                }
            }
        }
    }

    Ok(Some(spec))
}

pub fn matches_host_pattern(host: &str, pattern: &str) -> bool {
    // Pattern "." matches all hosts
    if pattern == "." {
        return true;
    }

    // Remove trailing slash if present
    let pattern = pattern.trim_end_matches('/');

    // Domain suffix matching: .example.com matches www.example.com, api.example.com, etc.
    if pattern.starts_with('.') {
        return host.ends_with(pattern) || host == &pattern[1..];
    }

    // Exact match
    if host == pattern {
        return true;
    }

    // Wildcard matching
    if pattern.contains('*') {
        let regex_pattern = pattern
            .replace(".", r"\.")
            .replace("*", ".*");
        if let Ok(re) = regex::Regex::new(&format!("^{}$", regex_pattern)) {
            return re.is_match(host);
        }
    }

    false
}

fn parse_debug_level(value: &str) -> PrivoxyResult<u32> {
    let mut level = 0u32;

    for part in value.split_whitespace() {
        match part {
            "1" | "request" => level |= LOG_LEVEL_REQUEST,
            "2" | "connect" => level |= LOG_LEVEL_CONNECT,
            "4" | "tagging" => level |= LOG_LEVEL_TAGGING,
            "8" | "header" => level |= LOG_LEVEL_HEADER,
            "16" | "writing" => level |= LOG_LEVEL_WRITING,
            "32" | "force" => level |= LOG_LEVEL_FORCE,
            "64" | "re-filter" => level |= LOG_LEVEL_RE_FILTER,
            "128" | "redirects" => level |= LOG_LEVEL_REDIRECTS,
            "256" | "deanimate" => level |= LOG_LEVEL_DEANIMATE,
            "512" | "clf" => level |= LOG_LEVEL_CLF,
            "1024" | "crunch" => level |= LOG_LEVEL_CRUNCH,
            "2048" | "cgi" => level |= LOG_LEVEL_CGI,
            "4096" | "info" => level |= LOG_LEVEL_INFO,
            "8192" | "error" => level |= LOG_LEVEL_ERROR,
            "16384" | "fatal" => level |= LOG_LEVEL_FATAL,
            "32768" | "received" => level |= LOG_LEVEL_RECEIVED,
            _ => {}
        }
    }

    if level == 0 {
        level = LOG_LEVEL_INFO | LOG_LEVEL_ERROR | LOG_LEVEL_FATAL;
    }

    Ok(level)
}

fn matches_pattern(addr_str: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern == "0.0.0.0/0" || pattern == "::/0" {
        return true;
    }

    let addr = match addr_str.parse::<std::net::IpAddr>() {
        Ok(a) => a,
        Err(_) => return addr_str == pattern, // Fallback to exact string match for hostnames
    };

    // 1. Try standard CIDR (e.g., 192.168.1.0/24)
    if let Ok(net) = pattern.parse::<ipnet::IpNet>() {
        return net.contains(&addr);
    }

    // 2. Handle IP/Mask format (e.g., 192.168.1.0/255.255.255.0)
    if pattern.contains('/') {
        let parts: Vec<&str> = pattern.split('/').collect();
        if parts.len() == 2 {
            if let (Ok(net_addr), Ok(mask)) = (parts[0].parse::<std::net::IpAddr>(), parts[1].parse::<std::net::IpAddr>()) {
                match (addr, net_addr, mask) {
                    (std::net::IpAddr::V4(a), std::net::IpAddr::V4(n), std::net::IpAddr::V4(m)) => {
                        return (u32::from(a) & u32::from(m)) == (u32::from(n) & u32::from(m));
                    }
                    _ => {} // IPv6 masks are usually CIDR only
                }
            }
        }
    }

    // 3. Fallback to exact match or hostname match
    addr_str == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern() {
        // Exact IP
        assert!(matches_pattern("127.0.0.1", "127.0.0.1"));
        assert!(!matches_pattern("127.0.0.1", "127.0.0.2"));
        
        // Wildcard
        assert!(matches_pattern("192.168.1.1", "*"));
        assert!(matches_pattern("192.168.1.1", "0.0.0.0/0"));
        
        // CIDR
        assert!(matches_pattern("192.168.1.5", "192.168.1.0/24"));
        assert!(!matches_pattern("192.168.2.5", "192.168.1.0/24"));
        
        // IP/Mask
        assert!(matches_pattern("192.168.1.5", "192.168.1.0/255.255.255.0"));
        assert!(!matches_pattern("192.168.2.5", "192.168.1.0/255.255.255.0"));
        
        // Hostname fallback
        assert!(matches_pattern("localhost", "localhost"));
        assert!(!matches_pattern("example.com", "localhost"));
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        // C implementation defaults to NO ACLs (all permitted)
        assert!(config.permit_access.is_empty());
        assert!(config.is_access_allowed("127.0.0.1"));
        assert_eq!(config.listen_addresses.len(), 1);
        assert_eq!(config.listen_addresses[0].port, 8118);
    }

    #[test]
    fn test_is_access_allowed() {
        let mut config = Config::default();
        
        // Default: empty permit/deny lists should allow all
        assert!(config.is_access_allowed("127.0.0.1"));
        assert!(config.is_access_allowed("192.168.1.1"));

        // Only permit 127.0.0.1
        config.permit_access.push("127.0.0.1".to_string());
        assert!(config.is_access_allowed("127.0.0.1"));
        assert!(!config.is_access_allowed("192.168.1.1"));

        // Deny 127.0.0.1 even if permitted
        config.deny_access.push("127.0.0.1".to_string());
        assert!(!config.is_access_allowed("127.0.0.1"));
    }

    #[test]
    fn test_config_audit_fixes() {
        let config_text = r#"
listen-address 127.0.0.1:8118
listen-address 0.0.0.0:8119
buffer-limit 4096
debug 1
debug 2
forward .example.com 127.0.0.1:8080
forward-socks5 .onion 127.0.0.1:9050 .
"#;
        let config = Config::parse(config_text).unwrap();
        
        // Match C behavior for buffer-limit (KB -> bytes)
        assert_eq!(config.buffer_limit, 4096 * 1024);
        
        // Cumulative debug levels
        assert_ne!(config.log_level & LOG_LEVEL_REQUEST, 0);
        assert_ne!(config.log_level & LOG_LEVEL_CONNECT, 0);
        
        // Multiple listeners (1 default + 2 from config)
        assert_eq!(config.listen_addresses.len(), 3);
        assert_eq!(config.listen_addresses[1].port, 8118);
        assert_eq!(config.listen_addresses[2].port, 8119);
        
        // Forward pattern order (pattern first, then proxy)
        let fwd = config.get_forward_spec("test.example.com", 80).expect("Should have forward spec");
        assert_eq!(fwd.forward_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(fwd.forward_port, 8080);
        assert!(fwd.gateway_host.is_none());
        
        let socks_fwd = config.get_forward_spec("something.onion", 80).expect("Should have socks forward spec");
        assert_eq!(socks_fwd.gateway_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(socks_fwd.gateway_port, 9050);
        assert_eq!(socks_fwd.forward_type, ForwardType::Socks5);
        assert!(socks_fwd.forward_host.is_none());
    }

    #[test]
    fn test_parse_listen_addresses() {
        let result = parse_listen_addresses("127.0.0.1:8118 192.168.1.1:8080").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].port, 8118);
        assert_eq!(result[1].port, 8080);
    }

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("yes").unwrap());
        assert!(parse_bool("1").unwrap());
        assert!(!parse_bool("no").unwrap());
        assert!(!parse_bool("0").unwrap());
    }
}
