#![allow(dead_code)]
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::net::TcpStream;
use tokio::sync::RwLock;

use crate::config::{Action, Config, ForwardSpec};
use crate::http::HttpRequest;

/// Global statistics counters
pub struct Statistics {
    pub urls_read: AtomicU64,
    pub urls_rejected: AtomicU64,
    pub requests_received: AtomicU64,
    pub requests_blocked: AtomicU64,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            urls_read: AtomicU64::new(0),
            urls_rejected: AtomicU64::new(0),
            requests_received: AtomicU64::new(0),
            requests_blocked: AtomicU64::new(0),
        }
    }
}

impl Statistics {
    pub fn increment_urls_read(&self) {
        self.urls_read.fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_urls_rejected(&self) {
        self.urls_rejected.fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_requests_received(&self) {
        self.requests_received.fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_requests_blocked(&self) {
        self.requests_blocked.fetch_add(1, Ordering::SeqCst);
    }

    pub fn get_urls_read(&self) -> u64 {
        self.urls_read.load(Ordering::SeqCst)
    }

    pub fn get_urls_rejected(&self) -> u64 {
        self.urls_rejected.load(Ordering::SeqCst)
    }

    pub fn get_requests_received(&self) -> u64 {
        self.requests_received.load(Ordering::SeqCst)
    }

    pub fn get_requests_blocked(&self) -> u64 {
        self.requests_blocked.load(Ordering::SeqCst)
    }
}

/// Client connection state
pub struct ClientState {
    /// Client ID
    pub id: u64,
    /// Client socket
    pub stream: Option<TcpStream>,
    /// Client IP address
    pub ip_addr: String,
    /// Client port
    pub port: u16,
    /// Current HTTP request
    pub request: Option<HttpRequest>,
    /// Current action
    pub action: Option<Action>,
    /// Forward specification
    pub forward_spec: Option<ForwardSpec>,
    /// Connection flags
    pub flags: u32,
    /// Server socket (if connected)
    pub server_stream: Option<TcpStream>,
    /// Buffer for reading data
    pub buffer: Vec<u8>,
    /// Content buffer for filtering
    pub content_buffer: Option<Vec<u8>>,
    /// Client tags
    pub tags: Vec<String>,
}

impl ClientState {
    pub fn new(id: u64, addr: SocketAddr) -> Self {
        Self {
            id,
            stream: None,
            ip_addr: addr.ip().to_string(),
            port: addr.port(),
            request: None,
            action: None,
            forward_spec: None,
            flags: 0,
            server_stream: None,
            buffer: Vec::with_capacity(8192),
            content_buffer: None,
            tags: Vec::new(),
        }
    }

    pub fn set_stream(&mut self, stream: TcpStream) {
        self.stream = Some(stream);
    }

    pub fn set_request(&mut self, request: HttpRequest) {
        self.request = Some(request);
    }

    pub fn clear_request(&mut self) {
        self.request = None;
        self.action = None;
        self.content_buffer = None;
    }

    pub fn add_tag(&mut self, tag: String) {
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    pub fn set_flag(&mut self, flag: u32) {
        self.flags |= flag;
    }

    pub fn clear_flag(&mut self, flag: u32) {
        self.flags &= !flag;
    }

    pub fn has_flag(&self, flag: u32) -> bool {
        (self.flags & flag) != 0
    }
}

/// Global client state manager
pub struct ClientManager {
    clients: RwLock<HashMap<u64, Arc<RwLock<ClientState>>>>,
    next_id: AtomicU64,
}

impl Default for ClientManager {
    fn default() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl ClientManager {
    pub async fn create_client(&self, addr: SocketAddr) -> Arc<RwLock<ClientState>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let client = Arc::new(RwLock::new(ClientState::new(id, addr)));

        let mut clients = self.clients.write().await;
        clients.insert(id, client.clone());

        client
    }

    pub async fn remove_client(&self, id: u64) {
        let mut clients = self.clients.write().await;
        clients.remove(&id);
    }

    pub async fn get_client(&self, id: u64) -> Option<Arc<RwLock<ClientState>>> {
        let clients = self.clients.read().await;
        clients.get(&id).cloned()
    }

    pub async fn client_count(&self) -> usize {
        let clients = self.clients.read().await;
        clients.len()
    }

    pub async fn get_all_clients(&self) -> Vec<Arc<RwLock<ClientState>>> {
        let clients = self.clients.read().await;
        clients.values().cloned().collect()
    }
}

/// Global application state
pub struct AppState {
    pub config: Arc<Config>,
    pub statistics: Statistics,
    pub client_manager: ClientManager,
    pub global_toggle: AtomicU64, // Using AtomicU64 for bool to allow shared access
    #[cfg(feature = "client-tags")]
    pub client_tag_manager: Arc<crate::client_tags::ClientTagManager>,
}

impl AppState {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            statistics: Statistics::default(),
            client_manager: ClientManager::default(),
            global_toggle: AtomicU64::new(1), // Enabled by default
            #[cfg(feature = "client-tags")]
            client_tag_manager: Arc::new(crate::client_tags::ClientTagManager::new()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.global_toggle.load(Ordering::SeqCst) != 0
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.global_toggle.store(if enabled { 1 } else { 0 }, Ordering::SeqCst);
    }

    pub fn toggle(&self) -> bool {
        let current = self.is_enabled();
        self.set_enabled(!current);
        !current
    }

    pub fn get_statistics(&self) -> &Statistics {
        &self.statistics
    }
}

/// Connection pool for keep-alive connections
pub struct ConnectionPool {
    connections: RwLock<HashMap<String, Vec<TcpStream>>>,
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }
}

impl ConnectionPool {
    pub async fn get_connection(&self, key: &str) -> Option<TcpStream> {
        let mut connections = self.connections.write().await;
        if let Some(pool) = connections.get_mut(key) {
            // Find a valid connection
            while let Some(stream) = pool.pop() {
                // Check if connection is still alive (simplified)
                return Some(stream);
            }
        }
        None
    }

    pub async fn return_connection(&self, key: String, stream: TcpStream) {
        let mut connections = self.connections.write().await;
        connections.entry(key).or_default().push(stream);
    }

    pub async fn clear(&self) {
        let mut connections = self.connections.write().await;
        connections.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics() {
        let stats = Statistics::default();
        stats.increment_urls_read();
        stats.increment_urls_rejected();
        assert_eq!(stats.get_urls_read(), 1);
        assert_eq!(stats.get_urls_rejected(), 1);
    }

    #[test]
    fn test_client_state_flags() {
        use crate::constants::*;

        let addr = "127.0.0.1:12345".parse().unwrap();
        let mut client = ClientState::new(1, addr);

        client.set_flag(CSP_FLAG_CHUNKED);
        assert!(client.has_flag(CSP_FLAG_CHUNKED));

        client.clear_flag(CSP_FLAG_CHUNKED);
        assert!(!client.has_flag(CSP_FLAG_CHUNKED));
    }
}
