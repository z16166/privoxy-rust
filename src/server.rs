#![allow(dead_code)]
use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, trace, warn};

use crate::config::{Config, ConfigRef};
use crate::connection::ConnectionHandler;
use crate::error::PrivoxyResult;
use crate::state::AppState;
use crate::constants::*;

pub struct ProxyServer {
    config: ConfigRef,
    state: Arc<AppState>,
    listeners: Vec<TcpListener>,
    cgi_handler: Arc<crate::cgi::CgiHandler>,
}

impl ProxyServer {
    pub fn new(config: Arc<Config>) -> PrivoxyResult<Self> {
        let config_ref = Arc::new(RwLock::new(config.as_ref().clone()));
        let state = Arc::new(AppState::new(config.clone()));
        let cgi_handler = Arc::new(crate::cgi::CgiHandler::new(config.clone(), state.clone()));

        Ok(Self {
            config: config_ref,
            state,
            listeners: Vec::new(),
            cgi_handler,
        })
    }

    pub fn new_with_state(config: Arc<Config>, state: Arc<AppState>) -> PrivoxyResult<Self> {
        let config_ref = Arc::new(RwLock::new(config.as_ref().clone()));
        let cgi_handler = Arc::new(crate::cgi::CgiHandler::new(config.clone(), state.clone()));
        Ok(Self {
            config: config_ref,
            state,
            listeners: Vec::new(),
            cgi_handler,
        })
    }

    pub fn new_with_config_ref(config: ConfigRef) -> PrivoxyResult<Self> {
        let state = {
            let config_guard = config.read();
            Arc::new(AppState::new(Arc::new(config_guard.clone())))
        };
        let cgi_handler = {
            let config_guard = config.read();
            Arc::new(crate::cgi::CgiHandler::new(Arc::new(config_guard.clone()), state.clone()))
        };

        Ok(Self {
            config,
            state,
            listeners: Vec::new(),
            cgi_handler,
        })
    }

    pub async fn run(&self) -> PrivoxyResult<()> {
        info!("Server starting...");
        
        // Bind to all configured addresses
        let mut listeners = Vec::new();

        {
            let config = self.config.read();
            info!("Configuring {} listen addresses", config.listen_addresses.len());
            for listen_addr in &config.listen_addresses {
                let socket_addr = listen_addr.to_socket_addr()?;
                let listener = TcpListener::bind(socket_addr).await?;
                info!("Listening on {}", socket_addr);
                listeners.push(listener);
            }
        }

        if listeners.is_empty() {
            // Bind to default address if none configured
            let addr: SocketAddr = DEFAULT_LISTEN_ADDR.parse().unwrap();
            let listener = TcpListener::bind(addr).await?;
            info!("Listening on {} (default)", addr);
            listeners.push(listener);
        }

        // Accept connections from all listeners concurrently
        let mut handles = Vec::new();

        for listener in listeners {
            let state = self.state.clone();
            let config = self.config.clone();

            let cgi_handler = self.cgi_handler.clone();

            let handle = tokio::spawn(async move {
                Self::accept_loop(listener, state, config, cgi_handler).await;
            });

            handles.push(handle);
        }

        // Wait for all listeners to complete (they shouldn't unless there's an error)
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Listener task error: {}", e);
            }
        }

        Ok(())
    }

    async fn accept_loop(listener: TcpListener, state: Arc<AppState>, config: ConfigRef, cgi_handler: Arc<crate::cgi::CgiHandler>) {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    trace!("New connection from {}", addr);

                    // Check for config file changes and reload if necessary
                    {
                        let config_guard = config.read();
                        if let Ok(Some(new_config)) = config_guard.reload_if_changed() {
                            drop(config_guard);
                            let mut config_guard = config.write();
                            *config_guard = new_config;
                            info!("Configuration reloaded");
                        }
                    }

                    // Check if we have too many connections
                    let client_count = state.client_manager.client_count().await;
                    let max_connections = {
                        let config_guard = config.read();
                        config_guard.max_client_connections
                    };
                    if client_count >= max_connections {
                        warn!("Too many connections ({}), rejecting {}", client_count, addr);
                        Self::send_too_many_connections_error(stream).await;
                        continue;
                    }

                    // Check access control
                    let access_allowed = {
                        let config_guard = config.read();
                        config_guard.is_access_allowed(&addr.ip().to_string())
                    };
                    if !access_allowed {
                        warn!("Access denied for {}", addr);
                        Self::send_access_denied_error(stream).await;
                        continue;
                    }

                    // Note: Proxy toggle state (is_enabled) is now checked in connection.rs 
                    // to bypass filters rather than rejecting connections, matching original Privoxy.

                    // Spawn a task to handle the connection
                    let state_clone = state.clone();
                    let config_clone = config.clone();
                    let cgi_handler_clone = cgi_handler.clone();

                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, addr, state_clone, config_clone, cgi_handler_clone).await {
                            debug!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        state: Arc<AppState>,
        config: ConfigRef,
        cgi_handler: Arc<crate::cgi::CgiHandler>,
    ) -> PrivoxyResult<()> {
        let client = state.client_manager.create_client(addr).await;
        let client_id = {
            let client_guard = client.read().await;
            client_guard.id
        };

        state.statistics.increment_requests_received();

        debug!("Client {} connected from {}", client_id, addr);
        let mut handler = ConnectionHandler::new(stream, addr, config, cgi_handler);

        let mut client_guard = client.write().await;
        let result = handler.handle(&mut client_guard).await;
        drop(client_guard);

        state.client_manager.remove_client(client_id).await;
        debug!("Client {} disconnected", client_id);

        result
    }

    async fn send_too_many_connections_error(mut stream: TcpStream) {
        let response = format!(
            "HTTP/1.1 503 Service Unavailable\r\n\
             Content-Type: text/plain\r\n\
             Connection: close\r\n\
             \r\n\
             Too many connections. Please try again later.\n"
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn send_access_denied_error(mut stream: TcpStream) {
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\n\
             Content-Type: text/plain\r\n\
             Connection: close\r\n\
             \r\n\
             Access denied.\n"
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn send_proxy_disabled_error(mut stream: TcpStream) {
        let response = format!(
            "HTTP/1.1 503 Service Unavailable\r\n\
             Content-Type: text/plain\r\n\
             Connection: close\r\n\
             \r\n\
             Proxy is currently disabled.\n"
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    pub async fn shutdown(&self) {
        info!("Shutting down proxy server");
        // Close all connections
        let clients = self.state.client_manager.get_all_clients().await;
        for client in clients {
            let client_guard = client.read().await;
            debug!("Closing connection for client {}", client_guard.id);
        }
    }

    pub fn get_state(&self) -> Arc<AppState> {
        self.state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_proxy_server_creation() {
        let config = Arc::new(Config::default());
        let server = ProxyServer::new(config);
        assert!(server.is_ok());
    }
}
