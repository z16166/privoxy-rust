#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::signal;
use tracing::{error, info};

mod action;
mod compression;
mod config;
mod connection;
mod constants;
mod encode;
mod error;
mod errlog;
mod feature;
mod filter;
mod http;
mod loaders;
mod logger;
mod server;
mod ssl;
mod state;
mod util;

#[cfg(feature = "cgi")]
mod cgi;

#[cfg(feature = "cgi-edit-actions")]
mod cgiedit;

#[cfg(feature = "client-tags")]
mod client_tags;

#[cfg(feature = "image-blocking")]
mod deanimate;

#[cfg(feature = "windows-service")]
mod windows_service;

#[cfg(feature = "tray-icon")]
mod tray_icon;

use crate::config::{Config, ConfigRef};
use crate::logger::init_logging;
use crate::server::ProxyServer;

const VERSION: &str = "4.1.0";
const CODE_STATUS: &str = "stable";
const HOME_PAGE_URL: &str = "https://www.privoxy.org/";

#[derive(Parser, Debug)]
#[command(
    name = "privoxy",
    about = "Privoxy - A web proxy with advanced filtering capabilities",
    version = "4.1.0 (stable)",
    long_about = None
)]
struct Cli {
    /// Configuration file path
    #[arg(value_name = "CONFIG_FILE", default_value = "config.txt")]
    config_file: PathBuf,

    /// Test configuration and exit
    #[arg(long = "config-test")]
    config_test: bool,

    /// Don't run as daemon (Unix only)
    #[cfg(unix)]
    #[arg(long = "no-daemon")]
    no_daemon: bool,

    /// PID file path (Unix only)
    #[cfg(unix)]
    #[arg(long = "pidfile", value_name = "FILE")]
    pidfile: Option<PathBuf>,

    /// Run as specified user and group (Unix only)
    #[cfg(unix)]
    #[arg(long = "user", value_name = "USER[.GROUP]")]
    user: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Disable logging
    #[arg(long)]
    quiet: bool,

    /// Run as Windows service
    #[cfg(windows)]
    #[arg(long = "install-service")]
    install_service: bool,

    /// Uninstall Windows service
    #[cfg(windows)]
    #[arg(long = "uninstall-service")]
    uninstall_service: bool,

    /// Enable tray icon
    #[cfg(windows)]
    #[arg(long = "tray-icon", default_value_t = true)]
    tray_icon: bool,

    /// Enable CGI web interface
    #[arg(long = "enable-cgi")]
    enable_cgi: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.quiet {
        "error"
    } else if cli.verbose {
        "debug"
    } else {
        "info"
    };
    init_logging(log_level)?;

    info!("Privoxy version {} ({}) starting...", VERSION, CODE_STATUS);
    info!("Home page: {}", HOME_PAGE_URL);

    // Load configuration
    let config = Arc::new(RwLock::new(
        Config::load(&cli.config_file)
            .map_err(|e| anyhow::anyhow!("Failed to load config from {:?}: {}", cli.config_file, e))?
    ));

    // Test configuration if requested
    if cli.config_test {
        info!("Configuration test successful");
        return Ok(());
    }

    // Handle Windows service installation/uninstallation and service mode
    #[cfg(all(windows, feature = "windows-service"))]
    {
        if cli.install_service {
            use crate::windows_service::install_service;
            let binary_path = std::env::current_exe()
                .map_err(|e| anyhow::anyhow!("Failed to get binary path: {}", e))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid binary path"))?
                .to_string();
            
            install_service("Privoxy", "Privoxy Web Proxy", &binary_path)
                .map_err(|e| anyhow::anyhow!("Failed to install service: {}", e))?;
            info!("Service installed successfully");
            return Ok(());
        }

        if cli.uninstall_service {
            use crate::windows_service::uninstall_service;
            uninstall_service("Privoxy")
                .map_err(|e| anyhow::anyhow!("Failed to uninstall service: {}", e))?;
            info!("Service uninstalled successfully");
            return Ok(());
        }

        // Windows service entry point is handled by the service manager
        // The ffi_privoxy_service function is called directly by the service manager
        // when the service is started
    }

    // Create and start the proxy server
    let server = ProxyServer::new_with_config_ref(config.clone())
        .map_err(|e| anyhow::anyhow!("Failed to create server: {}", e))?;

    // Get application state from server
    let state = server.get_state();

    // Start CGI web interface if enabled
    #[cfg(feature = "cgi")]
    let _cgi_handle = if cli.enable_cgi {
        use crate::cgi::CgiHandler;
        
        let config_for_cgi = {
            let config_guard = config.read();
            Arc::new(config_guard.clone())
        };
        let cgi_handler = Arc::new(CgiHandler::new(config_for_cgi, state.clone()));
        
        let cgi_handle = tokio::spawn(async move {
            if let Err(e) = cgi_handler.start_web_interface("127.0.0.1:8119").await {
                error!("CGI server error: {}", e);
            }
        });
        
        Some(cgi_handle)
    } else {
        None
    };

    // Setup graceful shutdown with config for SIGHUP handling
    let shutdown_signal = setup_shutdown_handler(config.clone());

    // Start server in a separate thread if tray icon is enabled
    #[cfg(feature = "tray-icon")]
    {
        if cli.tray_icon {
            use crate::tray_icon::TrayIconApp;
            
            // Create shutdown channel for tray icon
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            
            // Start server in a background thread
            let server_clone = server;
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime");
                
                rt.block_on(async {
                    info!("Privoxy is ready to accept connections");
                    
                    // Run server with graceful shutdown
                    tokio::select! {
                        result = server_clone.run() => {
                            if let Err(e) = result {
                                error!("Server error: {}", e);
                            }
                        }
                        _ = shutdown_rx => {
                            info!("Shutdown signal received, stopping server...");
                        }
                        _ = shutdown_signal => {
                            info!("Shutdown signal received, stopping server...");
                        }
                    }
                });
            });
            
            // Run tray icon on the main thread (required for macOS)
            // On Windows and Linux, tray icon must be created on the same thread as the event loop
            // On macOS, tray icon must be created on the main thread
            let config_for_tray = {
                let config_guard = config.read();
                Arc::new(config_guard.clone())
            };
            let mut tray_app = TrayIconApp::new(config_for_tray, state.clone(), shutdown_tx);
            if let Err(e) = tray_app.run() {
                error!("Failed to start tray icon: {}", e);
                return Err(anyhow::anyhow!("Failed to start tray icon: {}", e));
            }
        } else {
            // Run server normally if tray icon is disabled
            info!("Privoxy is ready to accept connections");
            
            // Run server with graceful shutdown
            tokio::select! {
                result = server.run() => {
                    if let Err(e) = result {
                        error!("Server error: {}", e);
                        return Err(anyhow::anyhow!("Server error: {}", e));
                    }
                }
                _ = shutdown_signal => {
                    info!("Shutdown signal received, stopping server...");
                }
            }
        }
    }
    
    #[cfg(not(feature = "tray-icon"))]
    {
        // Run server normally if tray icon feature is not enabled
        info!("Tray icon feature not enabled, running server normally");
        info!("Privoxy is ready to accept connections");
        
        // Run server with graceful shutdown
        tokio::select! {
            result = server.run() => {
                if let Err(e) = result {
                    error!("Server error: {}", e);
                    return Err(anyhow::anyhow!("Server error: {}", e));
                }
            }
            _ = shutdown_signal => {
                info!("Shutdown signal received, stopping server...");
            }
        }
    }

    info!("Privoxy has been shut down gracefully");
    Ok(())
}

fn setup_shutdown_handler(_config: ConfigRef) -> tokio::sync::oneshot::Receiver<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to create SIGTERM handler");
            let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
                .expect("Failed to create SIGINT handler");
            let mut sighup = signal::unix::signal(signal::unix::SignalKind::hangup())
                .expect("Failed to create SIGHUP handler");

            loop {
                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM signal");
                        let _ = tx.send(());
                        break;
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT signal");
                        let _ = tx.send(());
                        break;
                    }
                    _ = sighup.recv() => {
                        info!("Received SIGHUP signal, requesting configuration reload");
                        config.write().request_reload();
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            let _ = signal::ctrl_c().await;
            info!("Received Ctrl+C signal");
            let _ = tx.send(());
        }
    });

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "4.1.0");
    }
}
