#![allow(dead_code)]

//! Client Tags module - Ported from client-tags.c
//! 
//! This module provides client-specific tag management functionality.
//! Tags can be associated with client IP addresses and used for conditional filtering.

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

/// A client-specific tag with optional expiration
#[derive(Debug, Clone)]
pub struct ClientTag {
    /// Tag name
    pub name: String,
    /// Optional expiration time (Unix timestamp)
    pub end_of_life: Option<u64>,
}

/// Tags requested by a specific client
#[derive(Debug, Clone)]
pub struct ClientTags {
    /// Client IP address
    pub client: String,
    /// List of tags requested by this client
    pub tags: Vec<ClientTag>,
}

/// Client tag manager
#[derive(Debug, Default)]
pub struct ClientTagManager {
    /// Map of client IP to their tags
    clients: RwLock<HashMap<String, ClientTags>>,
}

impl ClientTagManager {
    /// Create a new client tag manager
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// Get all tags for a client
    pub fn get_tags_for_client(&self, client_address: &str) -> Vec<ClientTag> {
        let clients = self.clients.read();
        clients
            .get(client_address)
            .map(|ct| ct.tags.clone())
            .unwrap_or_default()
    }

    /// Check if a client has requested a specific tag
    pub fn client_has_requested_tag(&self, client_address: &str, tag: &str) -> bool {
        let clients = self.clients.read();
        if let Some(client_tags) = clients.get(client_address) {
            client_tags.tags.iter().any(|t| t.name == tag)
        } else {
            false
        }
    }

    /// Enable a client-specific tag
    pub fn enable_client_tag(&self, client_address: &str, tag_name: &str, end_of_life: Option<u64>) {
        let mut clients = self.clients.write();
        
        let client_tags = clients
            .entry(client_address.to_string())
            .or_insert_with(|| ClientTags {
                client: client_address.to_string(),
                tags: Vec::new(),
            });

        // Check if tag already exists
        if !client_tags.tags.iter().any(|t| t.name == tag_name) {
            client_tags.tags.push(ClientTag {
                name: tag_name.to_string(),
                end_of_life,
            });
        }
    }

    /// Disable a client-specific tag
    pub fn disable_client_tag(&self, client_address: &str, tag_name: &str) {
        let mut clients = self.clients.write();
        
        if let Some(client_tags) = clients.get_mut(client_address) {
            client_tags.tags.retain(|t| t.name != tag_name);
            
            // Remove client entry if no tags left
            if client_tags.tags.is_empty() {
                clients.remove(client_address);
            }
        }
    }

    /// Check if a client tag matches a pattern
    pub fn client_tag_match(&self, client_address: &str, pattern: &str) -> bool {
        let clients = self.clients.read();
        if let Some(client_tags) = clients.get(client_address) {
            client_tags.tags.iter().any(|t| {
                // Simple pattern matching - can be extended with regex
                if pattern.ends_with('*') {
                    let prefix = &pattern[..pattern.len() - 1];
                    t.name.starts_with(prefix)
                } else {
                    t.name == pattern
                }
            })
        } else {
            false
        }
    }

    /// Set client address and associate tags from headers
    pub fn set_client_address(&self, client_address: &str, headers: &[String]) {
        // Parse headers to extract tag information
        // In the C version, this looks for special headers like "X-Client-Tag"
        for header in headers {
            if header.starts_with("X-Client-Tag:") {
                let tag_name = header["X-Client-Tag:".len()..].trim();
                self.enable_client_tag(client_address, tag_name, None);
            }
        }
    }

    /// Remove expired tags for all clients
    pub fn cleanup_expired_tags(&self) {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut clients = self.clients.write();
        let mut to_remove = Vec::new();

        for (client_addr, client_tags) in clients.iter_mut() {
            let initial_len = client_tags.tags.len();
            client_tags.tags.retain(|tag| {
                tag.end_of_life.map_or(true, |exp| exp > current_time)
            });

            if client_tags.tags.is_empty() {
                to_remove.push(client_addr.clone());
            } else if client_tags.tags.len() != initial_len {
                // Some tags were removed
            }
        }

        for client_addr in to_remove {
            clients.remove(&client_addr);
        }
    }

    /// Get the number of clients with tags
    pub fn get_client_count(&self) -> usize {
        self.clients.read().len()
    }

    /// Get all client addresses
    pub fn get_all_clients(&self) -> Vec<String> {
        self.clients.read().keys().cloned().collect()
    }

    /// Clear all tags for testing
    pub fn clear_all(&self) {
        self.clients.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enable_disable_tag() {
        let manager = ClientTagManager::new();
        let client = "192.168.1.1";
        
        // Initially no tags
        assert!(!manager.client_has_requested_tag(client, "test-tag"));
        
        // Enable tag
        manager.enable_client_tag(client, "test-tag", None);
        assert!(manager.client_has_requested_tag(client, "test-tag"));
        
        // Disable tag
        manager.disable_client_tag(client, "test-tag");
        assert!(!manager.client_has_requested_tag(client, "test-tag"));
    }

    #[test]
    fn test_tag_expiration() {
        let manager = ClientTagManager::new();
        let client = "192.168.1.2";
        
        // Enable tag with future expiration
        let future_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() + 3600; // 1 hour from now
        
        manager.enable_client_tag(client, "expiring-tag", Some(future_time));
        assert!(manager.client_has_requested_tag(client, "expiring-tag"));
        
        // Enable tag with past expiration
        let past_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() - 3600; // 1 hour ago
        
        manager.enable_client_tag(client, "expired-tag", Some(past_time));
        manager.cleanup_expired_tags();
        
        // Expired tag should be removed
        assert!(!manager.client_has_requested_tag(client, "expired-tag"));
        // Non-expired tag should remain
        assert!(manager.client_has_requested_tag(client, "expiring-tag"));
    }

    #[test]
    fn test_pattern_matching() {
        let manager = ClientTagManager::new();
        let client = "192.168.1.3";
        
        manager.enable_client_tag(client, "premium-user", None);
        manager.enable_client_tag(client, "beta-tester", None);
        
        // Exact match
        assert!(manager.client_tag_match(client, "premium-user"));
        
        // Wildcard match
        assert!(manager.client_tag_match(client, "premium-*"));
        assert!(manager.client_tag_match(client, "*-user"));
        
        // No match
        assert!(!manager.client_tag_match(client, "admin"));
    }
}
