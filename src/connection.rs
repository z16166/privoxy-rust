#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_socks::tcp::socks5::Socks5Stream;
use tracing::{debug, error, info, trace, warn};

use crate::action::{
    find_action_for_url, ActionContext,
    apply_client_header_taggers,
    apply_add_header, apply_crunch_client_header,
    apply_crunch_server_header, apply_crunch_outgoing_cookies, apply_crunch_incoming_cookies,
    apply_crunch_if_none_match, apply_change_x_forwarded_for, apply_hide_referrer,
    apply_hide_user_agent, apply_send_user_agent, apply_hide_from_header,
    apply_hide_accept_language, apply_hide_if_modified_since, apply_hide_content_disposition,
    apply_prevent_compression, apply_downgrade_http_version, apply_session_cookies_only,
    apply_content_type_overwrite, apply_overwrite_last_modified, apply_limit_cookie_lifetime,
    apply_force_text_mode, check_limit_connect, apply_fast_redirects,
    apply_suppress_tags, create_blocked_page_response, create_blocked_image_response,
    create_empty_document_response, create_redirect_response,
    apply_server_header_taggers, apply_client_body_taggers, apply_client_body_filter,
    apply_content_filters, apply_client_header_filters, apply_server_header_filters,
};

#[cfg(feature = "image-blocking")]
use crate::action::deanimate_gif;
use crate::compression::decompress;
use crate::config::{ConfigRef, ForwardType};
use crate::constants::*;
use crate::error::{PrivoxyError, PrivoxyResult};
use crate::filter::{FilterType, FilterVariables};
use crate::http::{HttpRequest, HttpResponse, create_connect_response, create_error_response};
use crate::state::ClientState;
use regex::Regex;

pub struct ConnectionHandler {
    client_stream: TcpStream,
    client_addr: SocketAddr,
    server_stream: Option<TcpStream>,
    buffer: BytesMut,
    config: ConfigRef,
}

impl ConnectionHandler {
    pub fn new(client_stream: TcpStream, client_addr: SocketAddr, config: ConfigRef) -> Self {
        Self {
            client_stream,
            client_addr,
            server_stream: None,
            buffer: BytesMut::with_capacity(BUFFER_SIZE),
            config,
        }
    }

    pub async fn handle(&mut self, client_state: &mut ClientState) -> PrivoxyResult<()> {
        debug!("Handling connection from {}", self.client_addr);

        let request = match self.read_request().await? {
            Some(req) => req,
            None => {
                debug!("No request received from client");
                return Ok(());
            }
        };

        trace!("Received request: {}", request.cmd);
        client_state.set_request(request.clone());

        if request.method == "CONNECT" {
            return self.handle_connect(request).await;
        }

        self.handle_http_request(request).await
    }

    async fn read_request(&mut self) -> PrivoxyResult<Option<HttpRequest>> {
        let mut temp_buffer = vec![0u8; BUFFER_SIZE];

        loop {
            match timeout(
                Duration::from_secs(CONNECTION_TIMEOUT_SECS),
                self.client_stream.read(&mut temp_buffer)
            ).await {
                Ok(Ok(0)) => {
                    return Ok(None);
                }
                Ok(Ok(n)) => {
                    self.buffer.extend_from_slice(&temp_buffer[..n]);

                    match HttpRequest::parse(&self.buffer) {
                        Ok(request) => {
                            self.buffer.clear();
                            return Ok(Some(request));
                        }
                        Err(PrivoxyError::Http(msg)) if msg.contains("Incomplete") => {
                            continue;
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
                Ok(Err(e)) => {
                    return Err(PrivoxyError::Io(e));
                }
                Err(_) => {
                    return Err(PrivoxyError::Timeout);
                }
            }
        }
    }

    fn find_forward_spec(&self, host: &str, _port: u16) -> Option<(String, u16, ForwardType)> {
        let config = self.config.read();
        
        // First check actionsfile rules for forward-override
        for url_action in &config.url_actions {
            for pattern in &url_action.patterns {
                if Self::matches_pattern(host, pattern) {
                    if let Some(forward_override) = &url_action.action.forward_override {
                        // Parse the forward directive from forward_override
                        match crate::loaders::parse_forward_directive(forward_override) {
                            Ok(Some(spec)) => {
                                return Some((spec.proxy_host.clone(), spec.proxy_port, spec.forward_type.clone()));
                            }
                            Ok(None) => {
                                // Handle forward . case - direct connection
                                return Some((String::new(), 0, crate::config::ForwardType::Direct));
                            }
                            Err(e) => {
                                // Log error but continue checking other rules
                                error!("Failed to parse forward-override: {}", e);
                            }
                        }
                    }
                }
            }
        }
        
        // Then check main config forward specs
        for spec in &config.forward_specs {
            if Self::matches_pattern(host, &spec.pattern) {
                return Some((spec.proxy_host.clone(), spec.proxy_port, spec.forward_type.clone()));
            }
        }
        None
    }

    fn matches_pattern(host: &str, pattern: &str) -> bool {
        if pattern == "." {
            return true;
        }

        let pattern = pattern.trim_end_matches('/');

        if pattern.starts_with('.') {
            return host.ends_with(pattern) || host == &pattern[1..];
        }

        if host == pattern {
            return true;
        }

        if pattern.contains('*') {
            let regex_pattern = pattern
                .replace(".", r"\.")
                .replace("*", ".*");
            if let Ok(re) = Regex::new(&format!("^{}$", regex_pattern)) {
                return re.is_match(host);
            }
        }

        false
    }

    async fn connect_to_target(&self, host: &str, port: u16) -> PrivoxyResult<TcpStream> {
        let forward_spec = self.find_forward_spec(host, port);
        
        match forward_spec {
            Some((proxy_host, proxy_port, forward_type)) => {
                debug!("Using {} proxy {}:{}", 
                    match &forward_type {
                        ForwardType::Socks5 => "SOCKS5",
                        ForwardType::Socks5t => "SOCKS5T (Tor optimistic)",
                        ForwardType::Socks4 => "SOCKS4",
                        ForwardType::Socks4a => "SOCKS4a",
                        ForwardType::Http => "HTTP",
                        ForwardType::Direct => "Direct",
                        ForwardType::ForwardWebserver => "ForwardWebserver",
                    },
                    proxy_host, proxy_port
                );
                
                match forward_type {
                    ForwardType::Socks5 | ForwardType::Socks5t => {
                        let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
                        let target_addr = format!("{}:{}", host, port);
                        
                        let stream = TcpStream::connect(&proxy_addr).await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("Failed to connect to SOCKS5 proxy {}: {}", proxy_addr, e)
                            ))?;
                        
                        // For SOCKS5T, we use the same connection as SOCKS5
                        // The optimistic data handling is done at the HTTP request level
                        let socks_stream = Socks5Stream::connect_with_socket(stream, target_addr.as_str())
                            .await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("SOCKS5 connect failed: {:?}", e)
                            ))?;
                        
                        Ok(socks_stream.into_inner())
                    }
                    ForwardType::Socks4 | ForwardType::Socks4a => {
                        let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
                        let target_addr = format!("{}:{}", host, port);
                        
                        let stream = TcpStream::connect(&proxy_addr).await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("Failed to connect to SOCKS4 proxy {}: {}", proxy_addr, e)
                            ))?;
                        
                        let socks_stream = tokio_socks::tcp::socks4::Socks4Stream::connect_with_socket(
                            stream, 
                            target_addr.as_str()
                        )
                            .await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("SOCKS4 connect failed: {:?}", e)
                            ))?;
                        
                        Ok(socks_stream.into_inner())
                    }
                    ForwardType::Http | ForwardType::ForwardWebserver => {
                        // For both HTTP proxy and ForwardWebserver, connect to the proxy/server
                        let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
                        TcpStream::connect(&proxy_addr).await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("Failed to connect to {}: {}", proxy_addr, e)
                            ))
                    }
                    ForwardType::Direct => {
                        let target_addr = format!("{}:{}", host, port);
                        TcpStream::connect(&target_addr).await
                            .map_err(|e| PrivoxyError::Connection(
                                format!("Failed to connect to {}: {}", target_addr, e)
                            ))
                    }
                }
            }
            None => {
                let target_addr = format!("{}:{}", host, port);
                TcpStream::connect(&target_addr).await
                    .map_err(|e| PrivoxyError::Connection(
                        format!("Failed to connect to {}: {}", target_addr, e)
                    ))
            }
        }
    }

    async fn handle_connect(&mut self, request: HttpRequest) -> PrivoxyResult<()> {
        debug!("Handling CONNECT request to {}:{}", request.host, request.port);

        let url = format!("{}:{}", request.host, request.port);
        let action = {
            let config = self.config.read();
            find_action_for_url(&url, &config)
        };
        
        if let Some(ref a) = action {
            if !check_limit_connect(a, request.port) {
                warn!("CONNECT to port {} blocked by limit-connect action", request.port);
                let response = create_error_response(403, &format!(
                    "CONNECT to port {} is not allowed by limit-connect policy", request.port
                ));
                self.client_stream.write_all(&response.to_bytes()).await?;
                return Ok(());
            }
        }

        match self.connect_to_target(&request.host, request.port).await {
            Ok(stream) => {
                self.server_stream = Some(stream);

                let response = create_connect_response();
                self.client_stream.write_all(&response.to_bytes()).await?;

                self.run_tunnel().await
            }
            Err(e) => {
                error!("Failed to connect to {}:{}: {}", request.host, request.port, e);
                let response = create_error_response(502, &format!("Cannot connect to {}:{}", request.host, request.port));
                self.client_stream.write_all(&response.to_bytes()).await?;
                Ok(())
            }
        }
    }

    async fn handle_http_request(&mut self, mut request: HttpRequest) -> PrivoxyResult<()> {
        let target_host = request.host.clone();
        let target_port = request.port;
        let url = format!("{}{}", target_host, request.path);

        debug!("Proxying request to {}:{}", target_host, target_port);

        let mut action_ctx = ActionContext::default();
        let action = {
            let config = self.config.read();
            find_action_for_url(&url, &config)
        };

        if let Some(ref a) = action {
            apply_suppress_tags(a, &mut action_ctx);
        }

        if let Some(ref a) = action {
            if a.block {
                let reason = a.block_reason.as_deref().unwrap_or("Blocked by Privoxy");
                info!("Blocking URL: {} - {}", url, reason);
                
                let response = if a.handle_as_image {
                    create_blocked_image_response()
                } else if a.handle_as_empty_document {
                    create_empty_document_response()
                } else {
                    create_blocked_page_response(&url, reason)
                };
                
                self.client_stream.write_all(&response.to_bytes()).await?;
                return Ok(());
            }

            if let Some(ref redirect_url) = a.redirect {
                info!("Redirecting {} to {}", url, redirect_url);
                let response = create_redirect_response(redirect_url);
                self.client_stream.write_all(&response.to_bytes()).await?;
                return Ok(());
            }
        }

        if let Some(ref a) = action {
            self.apply_action_to_request(a, &mut request, &url, &mut action_ctx);
        }
        
        self.apply_client_header_filters(&mut request, &url);

        // Get forward spec to determine request format
        let forward_spec = self.find_forward_spec(&target_host, target_port);
        let is_forward_webserver = forward_spec.as_ref().map_or(false, |(_, _, ft)| *ft == ForwardType::ForwardWebserver);
        let is_http_proxy = forward_spec.as_ref().map_or(false, |(_, _, ft)| *ft == ForwardType::Http);

        let mut server_stream = match self.connect_to_target(&target_host, target_port).await {
            Ok(stream) => stream,
            Err(e) => {
                error!("Failed to connect to {}:{}: {}", target_host, target_port, e);
                let response = create_error_response(502, &format!("Cannot connect to {}:{}", target_host, target_port));
                self.client_stream.write_all(&response.to_bytes()).await?;
                return Ok(());
            }
        };

        // Build request bytes with correct format based on forward type
        let request_bytes = self.build_request_bytes(&request, is_forward_webserver, is_http_proxy);
        server_stream.write_all(&request_bytes).await?;

        self.forward_response(&mut server_stream, &url, &mut action_ctx).await
    }

    /// Build HTTP request bytes with correct request line format
    fn build_request_bytes(&self, request: &HttpRequest, is_forward_webserver: bool, is_http_proxy: bool) -> Bytes {
        use std::fmt::Write;
        
        let mut result = String::new();
        
        // Build request line
        // For ForwardWebserver: send only path (e.g., "GET /path HTTP/1.1")
        // For HTTP proxy: send full URL (e.g., "GET http://host/path HTTP/1.1")
        // For direct connection: send only path
        if is_http_proxy && !request.is_ssl {
            // HTTP proxy needs full URL
            let scheme = if request.is_ssl { "https" } else { "http" };
            let _ = write!(result, "{} {}://{}:{}{} {}\r\n", 
                request.method, scheme, request.host, request.port, request.path, request.version);
        } else {
            // Direct connection, ForwardWebserver, or SOCKS: send only path
            // The client sends full URL (e.g., "http://host/path") in proxy mode,
            // but the target server expects just the path portion.
            let path = normalize_request_path(&request.path);
            let _ = write!(result, "{} {} {}\r\n", 
                request.method, path, request.version);
        }

        // Add headers
        for (name, value) in &request.headers {
            let _ = write!(result, "{}: {}\r\n", name, value);
        }

        result.push_str("\r\n");

        let mut bytes = BytesMut::from(result.as_bytes());

        // Add body if present
        if let Some(ref body) = request.body {
            bytes.extend_from_slice(body);
        }

        bytes.freeze()
    }

    async fn forward_response(&mut self, server_stream: &mut TcpStream, url: &str, action_ctx: &mut ActionContext) -> PrivoxyResult<()> {
        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut response_data = BytesMut::new();

        loop {
            match timeout(
                Duration::from_secs(CONNECTION_TIMEOUT_SECS),
                server_stream.read(&mut buffer)
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    response_data.extend_from_slice(&buffer[..n]);

                    if response_data.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Ok(Err(e)) => return Err(PrivoxyError::Io(e)),
                Err(_) => return Err(PrivoxyError::Timeout),
            }
        }

        let mut response = match HttpResponse::parse(&response_data) {
            Ok(resp) => resp,
            Err(_) => {
                self.client_stream.write_all(&response_data).await?;
                return self.forward_remaining(server_stream).await;
            }
        };

        if let Some(content_encoding) = response.headers.get("Content-Encoding") {
            let compression = if content_encoding == "gzip" {
                Some(crate::compression::CompressionAlgorithm::Gzip)
            } else if content_encoding == "deflate" {
                Some(crate::compression::CompressionAlgorithm::Deflate)
            } else {
                None
            };

            if let Some(alg) = compression {
                if !response.body.is_empty() {
                    match decompress(&response.body, alg) {
                        Ok(decompressed) => {
                            response.body = decompressed.to_vec();
                            response.headers.remove("Content-Encoding");
                            response.headers.insert("Content-Length".to_string(), response.body.len().to_string());
                            debug!("Decompressed response body");
                        }
                        Err(e) => {
                            warn!("Failed to decompress response: {}", e);
                        }
                    }
                }
            }
        }

        let action = {
            let config = self.config.read();
            find_action_for_url(url, &config)
        };

        if let Some(ref a) = action {
            self.apply_action_to_response(a, &mut response, url, action_ctx);
            
            #[cfg(feature = "image-blocking")]
            {
                let deanimate_mode = a.deanimate_gifs.clone();
                let delay_ms = a.delay_response;
                
                if let Some(ref mode) = deanimate_mode {
                    let content_type = response.headers.get("Content-Type")
                        .map(|s| s.to_lowercase())
                        .unwrap_or_default();
                    
                    if content_type.contains("image/gif") || response.body.starts_with(b"GIF") {
                        if let Some(deanimated) = deanimate_gif(&response.body, mode) {
                            response.body = deanimated;
                            response.headers.insert("Content-Length".to_string(), response.body.len().to_string());
                            debug!("GIF deanimated");
                        }
                    }
                }
                
                if let Some(delay) = delay_ms {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    debug!("Delayed response by {} ms", delay);
                }
            }
            
            #[cfg(not(feature = "image-blocking"))]
            {
                let delay_ms = a.delay_response;
                if let Some(delay) = delay_ms {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    debug!("Delayed response by {} ms", delay);
                }
            }
        }

        self.apply_server_header_filters(&mut response, url);
        self.apply_content_filters(&mut response, url, action_ctx);

        self.client_stream.write_all(&response.to_bytes()).await?;

        self.forward_remaining(server_stream).await
    }

    fn apply_content_filters(&self, response: &mut HttpResponse, url: &str, action_ctx: &mut ActionContext) {
        if response.body.is_empty() {
            return;
        }

        let content_type = match response.headers.get("Content-Type") {
            Some(ct) => ct.to_lowercase(),
            None => return,
        };

        if !content_type.starts_with("text/") 
            && !content_type.contains("javascript")
            && !content_type.contains("json")
            && !content_type.contains("xml") {
            return;
        }

        let config = self.config.read();
        
        if config.filters.is_empty() {
            return;
        }

        let action = find_action_for_url(url, &config);

        let mut variables = FilterVariables::default();
        if let Some(pos) = url.find('/') {
            variables.host = url[..pos].to_string();
            variables.path = format!("/{}", &url[pos..]);
        } else {
            variables.host = url.to_string();
        }
        variables.url = format!("http://{}", url);
        variables.origin = self.client_addr.to_string();

        // Apply content filters using the action module function
        if let Some(ref a) = action {
            if !a.filter_names.is_empty() || a.filter {
                let _ = apply_content_filters(&mut response.body, &config, a, &variables);
            }
            
            if !a.client_body_filter_names.is_empty() {
                let _ = apply_client_body_filter(&mut response.body, &config, a, &variables);
            }
            
            if !a.client_body_tagger_names.is_empty() {
                apply_client_body_taggers(&response.body, &config, a, action_ctx, &variables);
            }
        }
    }

    fn apply_server_header_filters(&self, response: &mut HttpResponse, url: &str) {
        let config = self.config.read();
        
        if config.filters.is_empty() {
            return;
        }

        let action = find_action_for_url(url, &config);

        let filter_names: Vec<&str> = if let Some(ref a) = action {
            a.filter_names.iter().map(|s| s.as_str()).collect()
        } else {
            return;
        };

        let mut variables = FilterVariables::default();
        if let Some(pos) = url.find('/') {
            variables.host = url[..pos].to_string();
            variables.path = format!("/{}", &url[pos..]);
        } else {
            variables.host = url.to_string();
        }
        variables.url = format!("http://{}", url);
        variables.origin = self.client_addr.to_string();

        let mut headers_to_remove = Vec::new();
        let mut headers_to_add: Vec<(String, String)> = Vec::new();

        for filter_name in &filter_names {
            if let Some(filter) = config.filters.iter().find(|f| &f.name == filter_name) {
                if !filter.enabled || filter.filter_type != FilterType::ServerHeader {
                    continue;
                }

                for (header_name, header_value) in &response.headers {
                    let header_line = format!("{}: {}", header_name, header_value);
                    
                    let filtered = if filter.dynamic {
                        filter.apply_with_variables(&header_line, Some(&variables))
                    } else {
                        filter.apply(&header_line)
                    };

                    if filtered != header_line {
                        if filtered.is_empty() {
                            headers_to_remove.push(header_name.clone());
                            debug!("Removing header '{}' via filter '{}'", header_name, filter.name);
                        } else if let Some((new_name, new_value)) = filtered.split_once(':') {
                            headers_to_remove.push(header_name.clone());
                            headers_to_add.push((new_name.trim().to_string(), new_value.trim().to_string()));
                            debug!("Modified header '{}' via filter '{}'", header_name, filter.name);
                        }
                    }
                }
            }
        }

        for header in headers_to_remove {
            response.headers.remove(&header);
        }
        for (name, value) in headers_to_add {
            response.headers.insert(name, value);
        }
    }

    fn apply_client_header_filters(&self, request: &mut HttpRequest, url: &str) {
        let config = self.config.read();
        
        if config.filters.is_empty() {
            return;
        }

        let action = find_action_for_url(url, &config);

        let filter_names: Vec<&str> = if let Some(ref a) = action {
            a.filter_names.iter().map(|s| s.as_str()).collect()
        } else {
            return;
        };

        let mut variables = FilterVariables::default();
        if let Some(pos) = url.find('/') {
            variables.host = url[..pos].to_string();
            variables.path = format!("/{}", &url[pos..]);
        } else {
            variables.host = url.to_string();
        }
        variables.url = format!("http://{}", url);
        variables.origin = self.client_addr.to_string();

        let mut headers_to_remove = Vec::new();
        let mut headers_to_add: Vec<(String, String)> = Vec::new();

        for filter_name in &filter_names {
            if let Some(filter) = config.filters.iter().find(|f| &f.name == filter_name) {
                if !filter.enabled || filter.filter_type != FilterType::ClientHeader {
                    continue;
                }

                for (header_name, header_value) in &request.headers {
                    let header_line = format!("{}: {}", header_name, header_value);
                    
                    let filtered = if filter.dynamic {
                        filter.apply_with_variables(&header_line, Some(&variables))
                    } else {
                        filter.apply(&header_line)
                    };

                    if filtered != header_line {
                        if filtered.is_empty() {
                            headers_to_remove.push(header_name.clone());
                            debug!("Removing request header '{}' via filter '{}'", header_name, filter.name);
                        } else if let Some((new_name, new_value)) = filtered.split_once(':') {
                            headers_to_remove.push(header_name.clone());
                            headers_to_add.push((new_name.trim().to_string(), new_value.trim().to_string()));
                            debug!("Modified request header '{}' via filter '{}'", header_name, filter.name);
                        }
                    }
                }
            }
        }

        for header in headers_to_remove {
            request.headers.remove(&header);
        }
        for (name, value) in headers_to_add {
            request.headers.insert(name, value);
        }
    }

    fn apply_action_to_request(&self, action: &crate::config::Action, request: &mut HttpRequest, url: &str, action_ctx: &mut ActionContext) {
        let config = self.config.read();
        
        let mut variables = FilterVariables::default();
        if let Some(pos) = url.find('/') {
            variables.host = url[..pos].to_string();
            variables.path = format!("/{}", &url[pos..]);
        } else {
            variables.host = url.to_string();
        }
        variables.url = format!("http://{}", url);
        variables.origin = self.client_addr.to_string();
        
        // Apply client header filters (ported from filter_header in parsers.c)
        apply_client_header_filters(&mut request.headers, &config, action, &variables);
        
        apply_client_header_taggers(&request.headers, &config, action, action_ctx, &variables);
        
        apply_add_header(&mut request.headers, action);
        apply_crunch_client_header(&mut request.headers, action);
        apply_crunch_outgoing_cookies(&mut request.headers, action);
        apply_crunch_if_none_match(&mut request.headers, action);
        apply_change_x_forwarded_for(&mut request.headers, action, &self.client_addr.to_string());
        apply_hide_referrer(&mut request.headers, action);
        apply_hide_user_agent(&mut request.headers, action);
        apply_send_user_agent(&mut request.headers, action);
        apply_hide_from_header(&mut request.headers, action);
        apply_hide_accept_language(&mut request.headers, action);
        apply_hide_if_modified_since(&mut request.headers, action);
        apply_hide_content_disposition(&mut request.headers, action);
        apply_prevent_compression(&mut request.headers, action);
        apply_downgrade_http_version(&mut request.version, action);
        apply_session_cookies_only(&mut request.headers, action);
    }

    fn apply_action_to_response(&self, action: &crate::config::Action, response: &mut HttpResponse, url: &str, action_ctx: &mut ActionContext) {
        let config = self.config.read();
        
        let mut variables = FilterVariables::default();
        if let Some(pos) = url.find('/') {
            variables.host = url[..pos].to_string();
            variables.path = format!("/{}", &url[pos..]);
        } else {
            variables.host = url.to_string();
        }
        variables.url = format!("http://{}", url);
        variables.origin = self.client_addr.to_string();
        
        // Apply server header filters (port from filter_header in parsers.c)
        apply_server_header_filters(&mut response.headers, &config, action, &variables);
        
        apply_server_header_taggers(&response.headers, &config, action, action_ctx, &variables);
        
        apply_crunch_incoming_cookies(&mut response.headers, action);
        apply_crunch_server_header(&mut response.headers, action);
        apply_content_type_overwrite(&mut response.headers, action);
        apply_overwrite_last_modified(&mut response.headers, action);
        apply_limit_cookie_lifetime(&mut response.headers, action);
        apply_force_text_mode(&mut response.headers, action);
        
        if let Some(ref mode) = action.fast_redirects {
            if response.status_code >= 300 && response.status_code < 400 {
                if let Some(location) = response.headers.get("Location").cloned() {
                    if let Some(decoded_location) = apply_fast_redirects(response.status_code, Some(&location), mode) {
                        debug!("Fast redirect: {} -> {}", location, decoded_location);
                    }
                }
            }
        }
        
        if action.handle_as_image && action.block {
            if let Some(ref blocker) = action.set_image_blocker {
                if blocker != "blank" && blocker != "pattern" {
                    response.status_code = 302;
                    response.status_text = "Found".to_string();
                    response.headers.insert("Location".to_string(), blocker.clone());
                    response.body.clear();
                    response.headers.insert("Content-Length".to_string(), "0".to_string());
                }
            }
        }
    }

    async fn forward_remaining(&mut self, server_stream: &mut TcpStream) -> PrivoxyResult<()> {
        let mut buffer = vec![0u8; BUFFER_SIZE];

        loop {
            match timeout(
                Duration::from_secs(CONNECTION_TIMEOUT_SECS),
                server_stream.read(&mut buffer)
            ).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    self.client_stream.write_all(&buffer[..n]).await?;
                }
                Ok(Err(e)) => return Err(PrivoxyError::Io(e)),
                Err(_) => break,
            }
        }

        Ok(())
    }

    async fn run_tunnel(&mut self) -> PrivoxyResult<()> {
        let mut server_stream = self.server_stream.take().unwrap();
        
        tokio::io::copy_bidirectional(&mut self.client_stream, &mut server_stream)
            .await
            .map_err(|e| PrivoxyError::Connection(format!("Tunnel error: {}", e)))?;
        
        Ok(())
    }
}

/// Normalize a request path by stripping the scheme and host prefix.
/// When a browser sends a request through an HTTP proxy, the request line
/// contains the full URL: "GET http://example.com/path HTTP/1.1".
/// When forwarding through SOCKS5 or direct connection, the target server
/// expects just the path: "GET /path HTTP/1.1".
/// This follows the original C code's parse_http_url() behavior.
fn normalize_request_path(path: &str) -> &str {
    // Strip http:// or https:// prefix and the host part
    let rest = if let Some(r) = path.strip_prefix("http://") {
        r
    } else if let Some(r) = path.strip_prefix("https://") {
        r
    } else {
        return path;
    };

    // Find the first '/' after the host[:port] to get the path
    match rest.find('/') {
        Some(i) => &rest[i..],
        None => "/",
    }
}
