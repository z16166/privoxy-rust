#![windows_subsystem = "windows"]

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
#[cfg(not(feature = "tray-icon"))]
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
    
    // If tray-icon feature is enabled, we initialize with GUI support (cache will be linked later)
    #[cfg(feature = "tray-icon")]
    crate::logger::init_logging_with_gui(log_level, None, 1000)?;
    
    #[cfg(not(feature = "tray-icon"))]
    init_logging(log_level)?;

    info!("Privoxy version {} ({}) starting...", VERSION, CODE_STATUS);
    info!("Home page: {}", HOME_PAGE_URL);

    // Load configuration
    let config_val = Config::load(&cli.config_file)
        .map_err(|e| anyhow::anyhow!("Failed to load config from {:?}: {}", cli.config_file, e))?;
    
    // Late-bind log file if specified in config
    if let Some(log_file) = &config_val.log_file {
        crate::logger::update_log_file(log_file)?;
    }

    let config = Arc::new(RwLock::new(config_val));

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

    // CGI processing is now integrated into the ProxyServer's request handling loop.
    // Requests for config.privoxy.org or p.p will be intercepted and handled by CgiHandler.

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
                    
                    // Force the entire process to exit after server shutdown
                    // This handles the case where the main thread is blocking on a GUI event loop (tray icon)
                    info!("Privoxy background server stopped, exiting process...");
                    std::process::exit(0);
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
