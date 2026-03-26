#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_socks::tcp::socks5::Socks5Stream;
use tracing::{debug, error, info, trace, warn};

use crate::action::{
    find_action_for_url, ActionContext,
    apply_client_header_actions, apply_server_header_actions,
    apply_client_header_taggers, apply_server_header_taggers,
    apply_client_body_taggers, apply_client_body_filter,
    apply_content_filters, check_limit_connect,
    apply_suppress_tags, create_blocked_page_response, create_blocked_image_response,
    create_empty_document_response, create_redirect_response,
    apply_fast_redirects, apply_downgrade_http_version,
    apply_limit_cookie_lifetime, apply_force_text_mode,
};

#[cfg(feature = "image-blocking")]
use crate::action::deanimate_gif;
use crate::compression::decompress;
use crate::config::ConfigRef;
use crate::constants::*;
use crate::error::{PrivoxyError, PrivoxyResult};
use crate::filter::FilterVariables;
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

    fn find_forward_spec(&self, host: &str, _port: u16) -> Option<crate::config::ForwardSpec> {
        let config = self.config.read();
        
        // First check actionsfile rules for forward-override
        for url_action in &config.url_actions {
            for pattern in &url_action.patterns {
                if crate::config::matches_host_pattern(host, pattern) {
                    if let Some(forward_override) = &url_action.action.forward_override {
                        // Parse the forward directive from forward_override
                        match crate::loaders::parse_forward_directive(forward_override) {
                            Ok(Some(mut spec)) => {
                                // Pattern in override is not used, it's just a proxy spec
                                spec.pattern = pattern.to_string();
                                return Some(spec);
                            }
                            Ok(None) => {
                                // Handle forward . case - direct connection
                                let spec = crate::config::ForwardSpec {
                                    pattern: pattern.to_string(),
                                    forward_type: crate::config::ForwardType::Direct,
                                    gateway_host: None,
                                    gateway_port: 0,
                                    forward_host: None,
                                    forward_port: 0,
                                };
                                return Some(spec);
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
            if crate::config::matches_host_pattern(host, &spec.pattern) {
                return Some(spec.clone());
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

    async fn connect_to_target(&self, host: &str, port: u16) -> PrivoxyResult<(TcpStream, crate::config::ForwardSpec)> {
        let forward_spec = self.find_forward_spec(host, port).unwrap_or_else(|| {
            crate::config::ForwardSpec {
                pattern: String::new(),
                forward_type: crate::config::ForwardType::Direct,
                gateway_host: None,
                gateway_port: 0,
                forward_host: None,
                forward_port: 0,
            }
        });
        
        debug!("Forwarding decision for {}:{}: {:?}", host, port, forward_spec.forward_type);
        
        let (timeout_secs, retries) = {
            let config = self.config.read();
            (config.connection_timeout_secs, config.forwarded_connect_retries)
        };

        let mut last_error = None;
        for i in 0..=retries {
            if i > 0 {
                debug!("Retrying connection to {}:{} (attempt {}/{})", host, port, i, retries);
            }

            let forward_spec_clone = forward_spec.clone();
            let host_clone = host.to_string();
            let result = timeout(Duration::from_secs(timeout_secs), async move {
                let host = host_clone.as_str();
                // Step 1: Connect to gateway (SOCKS) if specified
                if let Some(ref gateway_host) = forward_spec_clone.gateway_host {
                    let gateway_addr = format!("{}:{}", gateway_host, forward_spec_clone.gateway_port);
                    
                    // Determine next hop address for SOCKS tunnel
                    let next_hop = if let Some(ref forward_host) = forward_spec_clone.forward_host {
                        format!("{}:{}", forward_host, forward_spec_clone.forward_port)
                    } else {
                        format!("{}:{}", host, port)
                    };

                    debug!("Connecting to SOCKS gateway {} to reach {}", gateway_addr, next_hop);
                    
                    let tcp_stream = TcpStream::connect(&gateway_addr).await
                        .map_err(|e| PrivoxyError::Connection(
                            format!("Failed to connect to gateway {}: {}", gateway_addr, e)
                        ))?;
                    
                    match forward_spec_clone.forward_type {
                        crate::config::ForwardType::Socks5 | crate::config::ForwardType::Socks5t => {
                            let socks_stream = Socks5Stream::connect_with_socket(tcp_stream, next_hop.as_str())
                                .await
                                .map_err(|e| PrivoxyError::Connection(
                                    format!("SOCKS5 connect failed: {:?}", e)
                                ))?;
                            Ok(socks_stream.into_inner())
                        }
                        crate::config::ForwardType::Socks4 | crate::config::ForwardType::Socks4a => {
                            let socks_stream = tokio_socks::tcp::socks4::Socks4Stream::connect_with_socket(
                                tcp_stream, 
                                next_hop.as_str()
                            )
                                .await
                                .map_err(|e| PrivoxyError::Connection(
                                    format!("SOCKS4 connect failed: {:?}", e)
                                ))?;
                            Ok(socks_stream.into_inner())
                        }
                        _ => Ok(tcp_stream),
                    }
                } else if let Some(ref forward_host) = forward_spec_clone.forward_host {
                    // Direct HTTP proxy connection (no SOCKS)
                    let proxy_addr = format!("{}:{}", forward_host, forward_spec_clone.forward_port);
                    debug!("Connecting to HTTP proxy {} to reach {}:{}", proxy_addr, host, port);
                    TcpStream::connect(&proxy_addr).await
                        .map_err(|e| PrivoxyError::Connection(
                            format!("Failed to connect to HTTP proxy {}: {}", proxy_addr, e)
                        ))
                } else {
                    // Truly direct connection
                    let target_addr = format!("{}:{}", host, port);
                    debug!("Connecting directly to {}", target_addr);
                    TcpStream::connect(&target_addr).await
                        .map_err(|e| PrivoxyError::Connection(
                            format!("Failed to connect to {}: {}", target_addr, e)
                        ))
                }
            }).await;

            match result {
                Ok(Ok(stream)) => return Ok((stream, forward_spec)),
                Ok(Err(e)) => {
                    last_error = Some(e);
                }
                Err(_) => {
                    last_error = Some(PrivoxyError::Connection(format!("Connection to {}:{} timed out after {}s", host, port, timeout_secs)));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| PrivoxyError::Connection("Unknown connection error".to_string())))
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
            Ok((stream, _)) => {
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

        // Get forward spec and connect
        let (mut server_stream, forward_spec) = match self.connect_to_target(&target_host, target_port).await {
            Ok(res) => res,
            Err(e) => {
                error!("Failed to connect to {}:{}: {}", target_host, target_port, e);
                let response = create_error_response(502, &format!("Cannot connect to {}:{}", target_host, target_port));
                self.client_stream.write_all(&response.to_bytes()).await?;
                return Ok(());
            }
        };
        
        // Build request bytes with correct format based on forward type
        let request_bytes = self.build_request_bytes(&request, &forward_spec);
        server_stream.write_all(&request_bytes).await?;
        
        self.forward_response(&mut server_stream, &url, &mut action_ctx, action.as_ref()).await
    }

    /// Build HTTP request bytes with correct request line format
    fn build_request_bytes(&self, request: &HttpRequest, forward_spec: &crate::config::ForwardSpec) -> Bytes {
        use std::fmt::Write;
        
        let mut result = String::new();
        
        // Build request line
        // For ForwardWebserver: send only path (e.g., "GET /path HTTP/1.1")
        // For HTTP proxy: send full URL (e.g., "GET http://host/path HTTP/1.1")
        // For direct connection: send only path
        
        // If there's an HTTP parent proxy, we need the full URL
        let is_http_proxy = forward_spec.forward_host.is_some() && forward_spec.forward_type != crate::config::ForwardType::ForwardWebserver;

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

    async fn forward_response(&mut self, server_stream: &mut TcpStream, url: &str, action_ctx: &mut ActionContext, action: Option<&crate::config::Action>) -> PrivoxyResult<()> {
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

        if let Some(a) = action {
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
        
        // Apply centralized header pipeline
        apply_client_header_actions(&mut request.headers, &config, action, &variables, &self.client_addr.to_string());
        
        // Taggers (from parsers.c: execute_header_tagger)
        apply_client_header_taggers(&request.headers, &config, action, action_ctx, &variables);
        
        // Handle non-header modifications
        apply_downgrade_http_version(&mut request.version, action);
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
        
        // Apply centralized header pipeline
        apply_server_header_actions(&mut response.headers, &config, action, &variables);
        
        // Taggers
        apply_server_header_taggers(&response.headers, &config, action, action_ctx, &variables);
        
        // Handle non-header modifications
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
