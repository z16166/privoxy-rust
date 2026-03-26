#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::error::{PrivoxyError, PrivoxyResult};
use crate::state::AppState;

#[cfg(all(feature = "windows-service", windows))]
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[cfg(all(feature = "windows-service", windows))]
define_windows_service!(ffi_privoxy_service, service_main);

#[cfg(all(feature = "windows-service", windows))]
pub struct WindowsService {
    config: Arc<Config>,
    state: Arc<AppState>,
}

#[cfg(all(feature = "windows-service", windows))]
impl WindowsService {
    pub fn new(config: Arc<Config>, state: Arc<AppState>) -> Self {
        Self { config, state }
    }

    pub fn run(&self) -> PrivoxyResult<()> {
        service_dispatcher::start("PrivoxyService", ffi_privoxy_service)
            .map_err(|e| PrivoxyError::Other(format!("Service dispatcher error: {:?}", e)))?;
        Ok(())
    }
}

#[cfg(all(feature = "windows-service", windows))]
fn service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(e) = run_service() {
        tracing::error!("Service error: {}", e);
    }
}

#[cfg(all(feature = "windows-service", windows))]
fn run_service() -> Result<(), windows_service::Error> {
    use crate::server::ProxyServer;
    use parking_lot::RwLock;
    use tokio::runtime::Runtime;
    use tokio::sync::oneshot;

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_tx = Arc::new(std::sync::Mutex::new(Some(shutdown_tx)));
    
    // Create atomic flag for shutdown
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    let shutdown_tx_clone = shutdown_tx.clone();

    // Event handler for service control
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop => {
                tracing::info!("Service received STOP control event");
                shutdown_flag_clone.store(true, Ordering::SeqCst);
                // Send shutdown signal to server
                if let Some(sender) = shutdown_tx_clone.lock().unwrap().take() {
                    let _ = sender.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Shutdown => {
                tracing::info!("Service received SHUTDOWN control event");
                shutdown_flag_clone.store(true, Ordering::SeqCst);
                // Send shutdown signal to server
                if let Some(sender) = shutdown_tx_clone.lock().unwrap().take() {
                    let _ = sender.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => {
                tracing::debug!("Service received INTERROGATE control event");
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Pause => {
                tracing::info!("Service received PAUSE control event");
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Continue => {
                tracing::info!("Service received CONTINUE control event");
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register service control handler
    let status_handle = service_control_handler::register("PrivoxyService", event_handler)?;

    // Report SERVICE_START_PENDING
    tracing::info!("Service status: START_PENDING");
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: std::time::Duration::from_secs(3),
        process_id: None,
    })?;

    // Load configuration
    tracing::info!("Loading configuration for service");
    let config_path = std::path::Path::new("config");
    let config = match Config::load(config_path) {
        Ok(config) => Arc::new(RwLock::new(config)),
        Err(e) => {
            tracing::error!("Failed to load config from {:?}: {}", config_path, e);
            status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(1), // ERROR_SERVICE_SPECIFIC_ERROR
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            })?;
            return Err(windows_service::Error::Winapi(e.into()));
        }
    };

    // Create proxy server
    tracing::info!("Creating proxy server");
    let server = match ProxyServer::new_with_config_ref(config.clone()) {
        Ok(server) => server,
        Err(e) => {
            tracing::error!("Failed to create server: {}", e);
            status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            })?;
            return Err(windows_service::Error::Winapi(
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string()).into()
            ));
        }
    };

    // Report SERVICE_RUNNING
    tracing::info!("Service status: RUNNING");
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::PAUSE_CONTINUE,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: std::time::Duration::default(),
        process_id: None,
    })?;

    // Create tokio runtime
    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("Failed to create tokio runtime: {}", e);
            status_handle.set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: std::time::Duration::default(),
                process_id: None,
            })?;
            return Err(windows_service::Error::Winapi(e.into()));
        }
    };

    // Run server with graceful shutdown
    rt.block_on(async {
        tracing::info!("Starting proxy server");
        
        // Select between server running and shutdown signal
        tokio::select! {
            result = server.run() => {
                match result {
                    Ok(_) => {
                        tracing::info!("Server stopped normally");
                    }
                    Err(e) => {
                        tracing::error!("Server error: {}", e);
                    }
                }
            }
            _ = shutdown_rx => {
                tracing::info!("Shutdown signal received");
            }
        }
    });

    // Check if shutdown was requested
    if shutdown_flag.load(Ordering::SeqCst) {
        // Report SERVICE_STOP_PENDING
        tracing::info!("Service status: STOP_PENDING");
        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::StopPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::from_secs(2),
            process_id: None,
        })?;
    }

    // Report SERVICE_STOPPED
    tracing::info!("Service status: STOPPED");
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: std::time::Duration::default(),
        process_id: None,
    })?;

    Ok(())
}

#[cfg(not(all(feature = "windows-service", windows)))]
pub struct WindowsService {
    _config: Arc<Config>,
    _state: Arc<AppState>,
}

#[cfg(not(all(feature = "windows-service", windows)))]
impl WindowsService {
    pub fn new(config: Arc<Config>, state: Arc<AppState>) -> Self {
        Self {
            _config: config,
            _state: state,
        }
    }

    pub fn run(&self) -> PrivoxyResult<()> {
        Err(PrivoxyError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows service is not enabled. Build with --features windows-service",
        )))
    }
}

pub fn install_service(service_name: &str, display_name: &str, binary_path: &str) -> PrivoxyResult<()> {
    #[cfg(all(feature = "windows-service", windows))]
    {
        use windows_service::service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType};

        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
            .map_err(|e| PrivoxyError::Other(format!("Service manager error: {:?}", e)))?;

        let service_info = ServiceInfo {
            name: service_name.into(),
            display_name: display_name.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: std::path::PathBuf::from(binary_path),
            launch_arguments: vec![],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };

        let _service = manager.create_service(&service_info, ServiceAccess::empty())
            .map_err(|e| PrivoxyError::Other(format!("Create service error: {:?}", e)))?;

        tracing::info!("Service '{}' installed successfully", service_name);
        Ok(())
    }

    #[cfg(not(all(feature = "windows-service", windows)))]
    {
        let _ = service_name;
        let _ = display_name;
        let _ = binary_path;
        Err(PrivoxyError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows service is not enabled",
        )))
    }
}

pub fn uninstall_service(service_name: &str) -> PrivoxyResult<()> {
    #[cfg(all(feature = "windows-service", windows))]
    {
        use windows_service::service::ServiceAccess;

        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(|e| PrivoxyError::Other(format!("Service manager error: {:?}", e)))?;

        let service = manager.open_service(service_name, ServiceAccess::DELETE)
            .map_err(|e| PrivoxyError::Other(format!("Open service error: {:?}", e)))?;

        service.delete()
            .map_err(|e| PrivoxyError::Other(format!("Delete service error: {:?}", e)))?;

        tracing::info!("Service '{}' uninstalled successfully", service_name);
        Ok(())
    }

    #[cfg(not(all(feature = "windows-service", windows)))]
    {
        let _ = service_name;
        Err(PrivoxyError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows service is not enabled",
        )))
    }
}
