#![allow(dead_code)]

use std::sync::Arc;
use std::io::Write;

use crate::config::Config;
use crate::error::{PrivoxyError, PrivoxyResult};
use crate::state::AppState;
use crate::logger::{self, LogCache};
use tracing::{info, error, warn, debug};

#[cfg(feature = "tray-icon")]
use tray_icon::{
    menu::{Menu as TrayMenu, MenuEvent, MenuItem as TrayMenuItem, PredefinedMenuItem, Submenu},
    TrayIconBuilder, TrayIconEvent, MouseButton,
};

/// Wrapper to allow sending libui MenuItem across threads.
/// SAFETY: This must only be used from the UI thread.
#[cfg(feature = "tray-icon")]
#[derive(Clone)]
pub(crate) struct SendMenuItem(pub(crate) libui::menus::MenuItem);
#[cfg(feature = "tray-icon")]
unsafe impl Send for SendMenuItem {}
#[cfg(feature = "tray-icon")]
unsafe impl Sync for SendMenuItem {}

#[cfg(feature = "tray-icon")]
use winit::event_loop::EventLoop;

#[cfg(feature = "tray-icon")]
use rfd::MessageDialog;

#[cfg(feature = "tray-icon")]
use libui::prelude::*;
#[cfg(feature = "tray-icon")]
use libui::controls::{VerticalBox, MultilineEntry, LayoutStrategy, TextEntry};
#[cfg(feature = "tray-icon")]
use libui::menus::Menu;

#[cfg(all(feature = "tray-icon", windows))]
use windows::core::{PCWSTR, w};
#[cfg(all(feature = "tray-icon", windows))]
use windows::Win32::Foundation::{HINSTANCE, HWND};
#[cfg(all(feature = "tray-icon", windows))]
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, IsIconic};

pub struct TrayIconApp {
    config: Arc<Config>,
    state: Arc<AppState>,
    shutdown_sender: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    log_messages: bool,
    message_highlighting: bool,
    limit_buffer_size: bool,
    activity_animation: bool,
    show_window: bool,
    window_thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(feature = "tray-icon")]
    log_cache: logger::LogCache,
    #[cfg(not(feature = "tray-icon"))]
    log_cache: logger::LogCache,
    max_buffer_lines: usize,
    log_file: Option<std::fs::File>,
    icon_manager: Option<IconManager>,
    current_frame: usize,
    tray_icon: Option<tray_icon::TrayIcon>,
    #[cfg(feature = "tray-icon")]
    menu: Option<TrayMenu>,
    pub ui_handles: Option<logger::UiLogHandles>,
    pub clear_log_item_libui: Option<SendMenuItem>,
}

#[cfg(feature = "tray-icon")]
impl TrayIconApp {
    pub fn new(config: Arc<Config>, state: Arc<AppState>, shutdown_sender: tokio::sync::oneshot::Sender<()>) -> Self {
        // Create log cache first
        let log_cache: LogCache = Arc::new(std::sync::Mutex::new((Vec::new(), None)));
        let max_buffer_lines = 200;
        
        // Initialize logging with GUI cache support
        let _log_level = if config.log_level & crate::constants::LOG_LEVEL_HEADER != 0 {
            "debug"
        } else if config.log_level & crate::constants::LOG_LEVEL_ERROR != 0 {
            "error"
        } else {
            "info"
        };
        
        // Register the log cache with the global registry (initialized in main.rs)
        crate::logger::register_gui_cache(log_cache.clone());
        
        // Open log file if configured
        let log_file = match config.log_file.as_ref() {
            Some(log_path) => {
                match std::fs::File::create(log_path) {
                    Ok(file) => Some(file),
                    Err(e) => {
                        eprintln!("Failed to open log file: {}", e);
                        None
                    }
                }
            }
            None => None,
        };
        
        // Load icon manager
        let icon_manager = match IconManager::new() {
            Ok(manager) => {
                info!("Loaded {} animated icons", manager.animated_icons.len());
                Some(manager)
            }
            Err(e) => {
                warn!("Failed to load icon manager: {}", e);
                None
            }
        };
        
        Self {
            config,
            state,
            shutdown_sender: Arc::new(std::sync::Mutex::new(Some(shutdown_sender))),
            log_messages: true,
            message_highlighting: true,
            limit_buffer_size: true,
            activity_animation: true,
            show_window: true,
            window_thread: None,
            log_cache,
            max_buffer_lines,
            log_file,
            icon_manager,
            current_frame: 0,
            tray_icon: None,
            #[cfg(feature = "tray-icon")]
            menu: None,
            ui_handles: None,
            clear_log_item_libui: None,
        }
    }

    fn show_window(&mut self) {
        if let Some(ref handles) = self.ui_handles {
            let event_queue = handles.event_queue.clone();
            let log_cache = self.log_cache.clone();
            
            event_queue.queue_main(move || {
                if let Ok(mut cache) = log_cache.lock() {
                    if let Some(ref mut gui_handles) = cache.1 {
                        if let Some(ref mut window) = gui_handles.window {
                            window.show();
                            
                            // Bring to foreground on Windows
                            #[cfg(windows)]
                            unsafe {
                                if let Ok(hwnd) = FindWindowW(None, w!("Privoxy")) {
                                    if !hwnd.is_invalid() {
                                        // Restore only if currently minimized (iconic)
                                        if IsIconic(hwnd).as_bool() {
                                            let _ = ShowWindow(hwnd, SW_RESTORE);
                                        }
                                        
                                        // Always bring to front
                                        let _ = SetForegroundWindow(hwnd);
                                    }
                                }
                            }
                        }
                    }
                }
            });
            return;
        }
        
        info!("Starting new UI thread (persistent)");
        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<(Arc<libui::EventQueue>, SendMenuItem)>();
        let log_cache_thread = self.log_cache.clone();
        let shutdown_sender_arc = self.shutdown_sender.clone();
        let visible_on_start = self.show_window;
        let config = self.config.clone();
        
        let thread = std::thread::spawn(move || {
            let ui = match UI::init() {
                Ok(ui) => ui,
                Err(e) => {
                    error!("Failed to initialize UI: {}", e);
                    return;
                }
            };
            
            // Hidden dummy window to keep thread alive
            let mut dummy = libui::controls::Window::new(&ui, "Privoxy Service", 1, 1, WindowType::NoMenubar);
            dummy.hide();
            
            // --- Global Menu Setup ---
            // File Menu
            let file_menu = Menu::new("File");
            file_menu.append_item("Show Window");
            file_menu.append_separator();
            let exit_item = file_menu.append_item("Exit");
            let shutdown_sender_ui = shutdown_sender_arc.clone();
            exit_item.on_clicked(move |_, _| {
                info!("File -> Exit clicked");
                if let Ok(mut guard) = shutdown_sender_ui.lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(());
                    }
                }
                std::process::exit(0);
            });
            
            // View Menu
            let view_menu = Menu::new("View");
            let clear_log_item = view_menu.append_item("Clear Log");
            
            // Tools Menu
            let tools_menu = Menu::new("Tools");
            let config_clone = config.clone();
            tools_menu.append_item("Edit Config").on_clicked(move |_, _| {
                if let Some(path) = &config_clone.config_file {
                    let _ = opener::open(path);
                }
            });
            
            let config_clone = config.clone();
            tools_menu.append_item("Edit Default Actions").on_clicked(move |_, _| {
                if let Some(ref confdir) = config_clone.confdir {
                    let path = confdir.join("default.action");
                    let _ = opener::open(path);
                }
            });
            
            let config_clone = config.clone();
            tools_menu.append_item("Edit User Actions").on_clicked(move |_, _| {
                if let Some(ref confdir) = config_clone.confdir {
                    let path = confdir.join("user.action");
                    let _ = opener::open(path);
                }
            });
            
            let config_clone = config.clone();
            tools_menu.append_item("Edit Default Filters").on_clicked(move |_, _| {
                if let Some(ref confdir) = config_clone.confdir {
                    let path = confdir.join("default.filter");
                    let _ = opener::open(path);
                }
            });
            
            let config_clone = config.clone();
            tools_menu.append_item("Edit User Filters").on_clicked(move |_, _| {
                if let Some(ref confdir) = config_clone.confdir {
                    let path = confdir.join("user.filter");
                    let _ = opener::open(path);
                }
            });
            
            #[cfg(feature = "trust")]
            {
                let config_clone = config.clone();
                tools_menu.append_item("Edit Trust").on_clicked(move |_, _| {
                    if let Some(ref confdir) = config_clone.confdir {
                        let path = confdir.join("trust");
                        let _ = opener::open(path);
                    }
                });
            }

            // Help Menu
            let help_menu = Menu::new("Help");
            help_menu.append_item("Status").on_clicked(|_, _| {
                let _ = opener::open("http://config.privoxy.org/show-status");
            });
            help_menu.append_item("FAQ").on_clicked(|_, _| {
                let _ = opener::open("https://www.privoxy.org/faq/");
            });
            help_menu.append_item("Manual").on_clicked(|_, _| {
                let _ = opener::open("https://www.privoxy.org/user-manual/");
            });
            help_menu.append_item("GPL").on_clicked(|_, _| {
                let _ = opener::open("https://www.gnu.org/copyleft/gpl.html");
            });
            help_menu.append_separator();
            help_menu.append_item("About").on_clicked(|_, _| {
                info!("Help -> About clicked");
                let dialog = rfd::MessageDialog::new()
                    .set_title("About Privoxy")
                    .set_description("Privoxy Rust implementation\nVersion 4.1.0\n\nCopyright (C) 2024 The Privoxy Team");
                dialog.show();
            });
            
            // Send handles back to main thread
            let _ = ui_tx.send((Arc::new(ui.event_queue()), SendMenuItem(clear_log_item.clone())));
            
            // Initial window setup
            setup_log_window(&ui, &log_cache_thread, &SendMenuItem(clear_log_item), visible_on_start);
            
            info!("UI thread entering persistent main loop");
            ui.main();
        });
        
        // Block main thread briefly to get the UI handle (only happens once at start)
        if let Ok((event_queue, clear_item)) = ui_rx.recv() {
            self.ui_handles = Some(logger::UiLogHandles {
                event_queue: event_queue.clone(),
                window: None,
                log_textarea: None,
            });
            self.clear_log_item_libui = Some(clear_item);
        }
        
        self.window_thread = Some(thread);
    }

    /// Update tray icon based on current state
    fn update_tray_icon(&mut self) {
        if let Some(ref mut icon) = self.tray_icon {
            if let Some(ref manager) = self.icon_manager {
                let new_icon = if !self.state.is_enabled() {
                    // Use off icon when disabled
                    manager.get_off().unwrap_or_else(|| manager.get_default())
                } else if self.activity_animation {
                    // Use animated icon when enabled and animation is on
                    self.current_frame = (self.current_frame + 1) % 8;
                    manager.get_animated(self.current_frame).unwrap_or_else(|| manager.get_default())
                } else {
                    // Use default icon
                    manager.get_default()
                };
                
                let _ = icon.set_icon(Some(new_icon.clone()));
            }
        }
    }

    /// Start or stop animation timer based on activity
    fn update_animation(&mut self) {
        if self.activity_animation && self.state.is_enabled() {
            // Animation will be updated in event loop
        }
    }
    
    fn add_log_message(&mut self, message: &str) {
        if self.log_messages {
            // Add to cache
            if let Ok(mut cache) = self.log_cache.lock() {
                #[cfg(feature = "tray-icon")]
                {
                    cache.0.push(message.to_string());
                    
                    // Limit cache size
                    if self.limit_buffer_size && cache.0.len() > self.max_buffer_lines {
                        let drain_count = cache.0.len() - self.max_buffer_lines;
                        cache.0.drain(0..drain_count);
                    }
                }
                
                #[cfg(not(feature = "tray-icon"))]
                {
                    cache.push(message.to_string());
                    
                    // Limit cache size
                    if self.limit_buffer_size && cache.len() > self.max_buffer_lines {
                        let drain_count = cache.len() - self.max_buffer_lines;
                        cache.drain(0..drain_count);
                    }
                }
            }
            
            // Write to log file if configured
            if let Some(log_file) = &mut self.log_file {
                if let Err(e) = writeln!(log_file, "{}", message) {
                    eprintln!("Failed to write to log file: {}", e);
                }
            }
        }
    }

    pub fn run(&mut self) -> PrivoxyResult<()> {
        // Create menu items for File submenu
        let menu_item_show_window = TrayMenuItem::new("Show Window", true, None);
        let menu_item_exit = TrayMenuItem::new("Exit", true, None);
        
        // Create File submenu
        let submenu_file = Submenu::with_items(
            "File",
            true,
            &[&menu_item_show_window, &PredefinedMenuItem::separator(), &menu_item_exit],
        ).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;
        
        // Create menu items for View submenu
        let menu_item_clear_log = TrayMenuItem::new("Clear Log", true, None);
        let menu_item_log_messages = TrayMenuItem::new("Log Messages", true, None);
        let menu_item_message_highlighting = TrayMenuItem::new("Message Highlighting", true, None);
        let menu_item_limit_buffer_size = TrayMenuItem::new("Limit Buffer Size", true, None);
        let menu_item_activity_animation = TrayMenuItem::new("Activity Animation", true, None);
        
        // Create View submenu
        let submenu_view = Submenu::with_items(
            "View",
            true,
            &[
                &menu_item_clear_log,
                &menu_item_log_messages,
                &menu_item_message_highlighting,
                &menu_item_limit_buffer_size,
                &menu_item_activity_animation,
            ],
        ).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;
        
        // Create menu items for Tools submenu
        let menu_item_edit_config = TrayMenuItem::new("Edit Config", true, None);
        let menu_item_edit_default_actions = TrayMenuItem::new("Edit Default Actions", true, None);
        let menu_item_edit_user_actions = TrayMenuItem::new("Edit User Actions", true, None);
        let menu_item_edit_default_filters = TrayMenuItem::new("Edit Default Filters", true, None);
        let menu_item_edit_user_filters = TrayMenuItem::new("Edit User Filters", true, None);
        
        // Create Tools submenu with optional Edit Trust item
        #[cfg(feature = "trust")]
        let menu_item_edit_trust = TrayMenuItem::new("Edit Trust", true, None);
        
        #[cfg(feature = "trust")]
        let submenu_tools = Submenu::with_items(
            "Tools",
            true,
            &[
                &menu_item_edit_config,
                &menu_item_edit_default_actions,
                &menu_item_edit_user_actions,
                &menu_item_edit_default_filters,
                &menu_item_edit_user_filters,
                &menu_item_edit_trust,
            ],
        ).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;
        
        #[cfg(not(feature = "trust"))]
        let submenu_tools = Submenu::with_items(
            "Tools",
            true,
            &[
                &menu_item_edit_config,
                &menu_item_edit_default_actions,
                &menu_item_edit_user_actions,
                &menu_item_edit_default_filters,
                &menu_item_edit_user_filters,
            ],
        ).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;
        
        // Create menu items for Help submenu
        let menu_item_status = TrayMenuItem::new("Status", true, None);
        let menu_item_faq = TrayMenuItem::new("FAQ", true, None);
        let menu_item_manual = TrayMenuItem::new("Manual", true, None);
        let menu_item_gpl = TrayMenuItem::new("GPL", true, None);
        let menu_item_about = TrayMenuItem::new("About", true, None);
        
        // Create Help submenu
        let submenu_help = Submenu::with_items(
            "Help",
            true,
            &[
                &menu_item_status,
                &menu_item_faq,
                &menu_item_manual,
                &menu_item_gpl,
                &PredefinedMenuItem::separator(),
                &menu_item_about,
            ],
        ).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;
        
        // Create top-level Toggle Enabled menu item
        let menu_item_toggle = TrayMenuItem::new("Toggle Enabled", true, None);
        
        // Create main menu with all submenus and items
        let menu = TrayMenu::with_items(&[
            &submenu_file,
            &submenu_view,
            &submenu_tools,
            &submenu_help,
            &PredefinedMenuItem::separator(),
            &menu_item_toggle,
        ]).map_err(|e| PrivoxyError::Other(format!("Menu error: {}", e)))?;

        // Store menu for dynamic attachment
        self.menu = Some(menu.clone());

        // Load icon from embedded resource or file
        let icon = load_icon().map_err(|e| PrivoxyError::Other(format!("Failed to load icon: {}", e)))?;

        // Store menu item IDs for comparison
        let show_window_id = menu_item_show_window.id().clone();
        let exit_id = menu_item_exit.id().clone();
        let clear_log_id = menu_item_clear_log.id().clone();
        let log_messages_id = menu_item_log_messages.id().clone();
        let message_highlighting_id = menu_item_message_highlighting.id().clone();
        let limit_buffer_size_id = menu_item_limit_buffer_size.id().clone();
        let activity_animation_id = menu_item_activity_animation.id().clone();
        let edit_config_id = menu_item_edit_config.id().clone();
        let edit_default_actions_id = menu_item_edit_default_actions.id().clone();
        let edit_user_actions_id = menu_item_edit_user_actions.id().clone();
        let edit_default_filters_id = menu_item_edit_default_filters.id().clone();
        let edit_user_filters_id = menu_item_edit_user_filters.id().clone();
        #[cfg(feature = "trust")]
        let edit_trust_id = menu_item_edit_trust.id().clone();
        let status_id = menu_item_status.id().clone();
        let faq_id = menu_item_faq.id().clone();
        let manual_id = menu_item_manual.id().clone();
        let gpl_id = menu_item_gpl.id().clone();
        let about_id = menu_item_about.id().clone();
        let toggle_id = menu_item_toggle.id().clone();

        // Clone config for use in the event loop
        let config = self.config.clone();

        // Create tray icon WITHOUT initial menu
        #[cfg(not(target_os = "macos"))]
        {
            let tray_icon = TrayIconBuilder::new()
                .with_tooltip("Privoxy - Web Proxy")
                .with_icon(icon.clone())
                .build();
            
            match tray_icon {
                Ok(icon) => {
                    info!("Tray icon created successfully");
                    self.tray_icon = Some(icon);
                }
                Err(e) => {
                    error!("Failed to create tray icon: {}", e);
                }
            }
        };
        
        #[cfg(target_os = "macos")]
        let mut tray_icon: Option<tray_icon::TrayIcon> = None;
        
        // Cross-platform event loop using winit
        let event_loop = EventLoop::<()>::new().map_err(|e| PrivoxyError::Other(format!("Failed to create event loop: {}", e)))?;
        
        // On macOS, the tray icon must be created after the event loop is running
        // We use a flag to track if the tray icon has been created
        #[cfg(target_os = "macos")]
        let mut tray_icon_created = false;
        
        // Create a channel to receive shutdown signal
        let (_shutdown_notify_tx, shutdown_notify_rx) = std::sync::mpsc::channel::<()>();
        
        // Monitor for shutdown signal in a separate thread
        std::thread::spawn(move || {
            // Wait for shutdown signal
            let _ = shutdown_notify_rx.recv();
            // Exit the process directly
            std::process::exit(0);
        });
        
        event_loop.run(move |event, _| {
            // On macOS, create tray icon after event loop is initialized
            #[cfg(target_os = "macos")]
            if tray_icon.is_none() {
                if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                    let tray_icon_result = TrayIconBuilder::new()
                        .with_menu(Box::new(menu.clone()))
                        .with_tooltip("Privoxy - Web Proxy")
                        .with_icon(icon.clone())
                        .build();
                    
                    match tray_icon_result {
                        Ok(t_icon) => {
                            info!("Tray icon created successfully on macOS");
                            tray_icon = Some(t_icon);
                            self.tray_icon = Some(t_icon);
                        }
                        Err(e) => {
                            error!("Failed to create tray icon on macOS: {}", e);
                        }
                    }
                }
            }
            
            // Check for shutdown event
            if let winit::event::Event::UserEvent(()) = event {
                // Exit the event loop
                info!("Shutdown signal received, exiting event loop");
                std::process::exit(0);
            }
            
            // Check for tray icon events
            if let Ok(tray_event) = TrayIconEvent::receiver().try_recv() {
                match tray_event {
                    TrayIconEvent::Click { button: MouseButton::Left, .. } => {
                        info!("Tray icon left clicked - showing window");
                        // Ensure menu is not attached during left click
                        if let Some(icon) = &self.tray_icon {
                            icon.set_menu(None);
                        }
                        self.show_window();
                    }
                    TrayIconEvent::Click { button: MouseButton::Right, .. } => {
                        info!("Tray icon right clicked - attaching menu");
                        if let (Some(icon), Some(menu)) = (&self.tray_icon, &self.menu) {
                            icon.set_menu(Some(Box::new(menu.clone())));
                        }
                    }
                    _ => {}
                }
            }
            
            // Check for menu events
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                let event_id = &event.id;
                if event_id == &show_window_id {
                    info!("Show window clicked");
                    self.show_window();
                } else if event_id == &clear_log_id {
                    info!("Tray -> Clear Log clicked");
                    let log_cache = self.log_cache.clone();
                    if let Some(ref handles) = self.ui_handles {
                        let event_queue = handles.event_queue.clone();
                        event_queue.queue_main(move || {
                            if let Ok(mut cache) = log_cache.lock() {
                                cache.0.clear();
                                if let Some(ref mut gui_handles) = cache.1 {
                                    if let Some(ref mut textarea_wrap) = gui_handles.log_textarea {
                                        let mut textarea_mut = textarea_wrap.clone();
                                        textarea_mut.set_value("");
                                    }
                                }
                            }
                        });
                    }
                } else if event_id == &exit_id {
                    info!("Exit clicked");
                    if let Some(sender) = self.shutdown_sender.lock().unwrap().take() {
                        let _ = sender.send(());
                    }
                    std::process::exit(0);
                } else if event_id == &log_messages_id {
                    // Show log window
                    info!("Log Messages clicked - showing window");
                    self.show_window();
                } else if event_id == &message_highlighting_id {
                    // Toggle message highlighting
                    self.message_highlighting = !self.message_highlighting;
                    info!("Message highlighting {}", if self.message_highlighting { "enabled" } else { "disabled" });
                } else if event_id == &limit_buffer_size_id {
                    // Toggle limit buffer size
                    self.limit_buffer_size = !self.limit_buffer_size;
                    info!("Limit buffer size {}", if self.limit_buffer_size { "enabled" } else { "disabled" });
                } else if event_id == &activity_animation_id {
                    // Toggle activity animation
                    self.activity_animation = !self.activity_animation;
                    info!("Activity animation {}", if self.activity_animation { "enabled" } else { "disabled" });
                    
                    // Update tray icon
                    self.update_tray_icon();
                } else if event_id == &edit_config_id {
                    // Open config file for editing
                    if let Some(config_path) = &config.config_file {
                        let config_path_str = config_path.to_string_lossy();
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("cmd.exe")
                            .args(["/c", "start", &config_path_str])
                            .spawn();
                        #[cfg(target_os = "macos")]
                        let _ = std::process::Command::new("open")
                            .arg(&config_path_str)
                            .spawn();
                        #[cfg(target_os = "linux")]
                        let _ = std::process::Command::new("xdg-open")
                            .arg(&config_path_str)
                            .spawn();
                    } else {
                        error!("Config file path not set");
                    }
                } else if event_id == &edit_default_actions_id {
                    // Open default actions file for editing
                    info!("Edit default actions clicked");
                    if let Some(ref confdir) = config.confdir {
                        let path = confdir.join("default.action");
                        let _ = opener::open(path);
                    }
                } else if event_id == &edit_user_actions_id {
                    // Open user actions file for editing
                    info!("Edit user actions clicked");
                    if let Some(ref confdir) = config.confdir {
                        let path = confdir.join("user.action");
                        let _ = opener::open(path);
                    }
                } else if event_id == &edit_default_filters_id {
                    // Open default filters file for editing
                    info!("Edit default filters clicked");
                    if let Some(ref confdir) = config.confdir {
                        let path = confdir.join("default.filter");
                        let _ = opener::open(path);
                    }
                } else if event_id == &edit_user_filters_id {
                    // Open user filters file for editing
                    info!("Edit user filters clicked");
                    if let Some(ref confdir) = config.confdir {
                        let path = confdir.join("user.filter");
                        let _ = opener::open(path);
                    }
                }
                // Handle Edit Trust menu item (only when trust feature is enabled)
                else if {
                    #[cfg(feature = "trust")]
                    { event_id == &edit_trust_id }
                    #[cfg(not(feature = "trust"))]
                    { false }
                } {
                    // Open trust file for editing
                    info!("Edit trust clicked");
                    if let Some(ref confdir) = config.confdir {
                        let path = confdir.join("trust");
                        let _ = opener::open(path);
                    }
                } else if event_id == &status_id {
                    // Show status - open the status page in browser
                    let status_url = "http://localhost:8119/show-status";
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd.exe")
                        .args(["/c", "start", status_url])
                        .spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open")
                        .arg(status_url)
                        .spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open")
                        .arg(status_url)
                        .spawn();
                } else if event_id == &faq_id {
                    // Open FAQ page
                    let faq_url = "https://www.privoxy.org/faq/";
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd.exe")
                        .args(["/c", "start", faq_url])
                        .spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open")
                        .arg(faq_url)
                        .spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open")
                        .arg(faq_url)
                        .spawn();
                } else if event_id == &manual_id {
                    // Open manual page
                    let manual_url = "https://www.privoxy.org/user-manual/";
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd.exe")
                        .args(["/c", "start", manual_url])
                        .spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open")
                        .arg(manual_url)
                        .spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open")
                        .arg(manual_url)
                        .spawn();
                } else if event_id == &gpl_id {
                    // Open GPL license page
                    let gpl_url = "https://www.gnu.org/copyleft/gpl.html";
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd.exe")
                        .args(["/c", "start", gpl_url])
                        .spawn();
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open")
                        .arg(gpl_url)
                        .spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open")
                        .arg(gpl_url)
                        .spawn();
                } else if event_id == &about_id {
                    // Show about dialog
                    info!("About clicked");
                    
                    MessageDialog::new()
                        .set_title("About Privoxy")
                        .set_description(&format!("Privoxy version {} for Windows
Copyright (C) 2000-2023 the Privoxy Team (https://www.privoxy.org/)
Based on the Internet Junkbuster by Junkbusters Corp.
This is free software; it may be used and copied under the
GNU General Public License, version 2: https://www.gnu.org/licenses/old-licenses/gpl-2.0.html
This program comes with ABSOLUTELY NO WARRANTY OF ANY KIND.", crate::constants::VERSION))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                } else if event_id == &toggle_id {
                    // Toggle Privoxy enabled state
                    let enabled = self.state.toggle();
                    info!("Privoxy toggled {}", if enabled { "ON" } else { "OFF" });
                    
                    // Update tray icon to reflect new state
                    self.update_tray_icon();
                }
            }
        }).map_err(|e| PrivoxyError::Other(format!("Event loop error: {}", e)))?;

        Ok(())
    }

}

/// Icon manager for handling different icon states
#[cfg(feature = "tray-icon")]
struct IconManager {
    default_icon: tray_icon::Icon,
    off_icon: Option<tray_icon::Icon>,
    animated_icons: Vec<tray_icon::Icon>,
}

#[cfg(feature = "tray-icon")]
impl IconManager {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        info!("Loading icons from Windows resources");
        
        // Load icons from Windows resources
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{IMAGE_ICON, LR_DEFAULTSIZE, LoadImageW, GetIconInfo, ICONINFO};
            use windows::Win32::Graphics::Gdi::{
                CreateCompatibleDC, GetDIBits, SelectObject, DeleteDC, DeleteObject,
                BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, BITMAP, GetObjectW, BI_RGB,
            };
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            
            // Helper function to load icon from resources and convert to tray_icon::Icon
            let load_icon_from_resource = |icon_id: u16| -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
                unsafe {
                    // Get the current module handle (the running EXE)
                    let hmodule = GetModuleHandleW(None)
                        .map_err(|e| format!("GetModuleHandleW failed: {}", e))?;
                    let hinstance = HINSTANCE(hmodule.0);
                    
                    // Load icon from resource using MAKEINTRESOURCE(icon_id)
                    let icon_handle = LoadImageW(
                        hinstance,
                        PCWSTR(icon_id as *const u16),
                        IMAGE_ICON,
                        32,   // desired width
                        32,   // desired height
                        LR_DEFAULTSIZE,
                    ).map_err(|e| format!("LoadImageW failed for ID {}: {}", icon_id, e))?;
                    
                    // Get icon info to access the bitmap data
                    let mut icon_info = ICONINFO::default();
                    GetIconInfo(
                        windows::Win32::UI::WindowsAndMessaging::HICON(icon_handle.0),
                        &mut icon_info,
                    ).map_err(|e| format!("GetIconInfo failed: {}", e))?;
                    
                    // Get bitmap dimensions
                    let mut bmp = BITMAP::default();
                    GetObjectW(
                        icon_info.hbmColor,
                        std::mem::size_of::<BITMAP>() as i32,
                        Some(&mut bmp as *mut _ as *mut std::ffi::c_void),
                    );
                    
                    let width = bmp.bmWidth as u32;
                    let height = bmp.bmHeight as u32;
                    
                    if width == 0 || height == 0 {
                        // Clean up
                        if !icon_info.hbmColor.is_invalid() { let _ = DeleteObject(icon_info.hbmColor); }
                        if !icon_info.hbmMask.is_invalid() { let _ = DeleteObject(icon_info.hbmMask); }
                        return Err("Icon has zero dimensions".into());
                    }
                    
                    // Set up BITMAPINFO for GetDIBits
                    let mut bmi = BITMAPINFO {
                        bmiHeader: BITMAPINFOHEADER {
                            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                            biWidth: width as i32,
                            biHeight: -(height as i32), // negative = top-down
                            biPlanes: 1,
                            biBitCount: 32,
                            biCompression: BI_RGB.0 as u32,
                            biSizeImage: 0,
                            biXPelsPerMeter: 0,
                            biYPelsPerMeter: 0,
                            biClrUsed: 0,
                            biClrImportant: 0,
                        },
                        bmiColors: [Default::default()],
                    };
                    
                    // Allocate buffer for BGRA pixel data
                    let mut bgra_data = vec![0u8; (width * height * 4) as usize];
                    
                    let hdc = CreateCompatibleDC(None);
                    let old_bmp = SelectObject(hdc, icon_info.hbmColor);
                    
                    GetDIBits(
                        hdc,
                        icon_info.hbmColor,
                        0,
                        height,
                        Some(bgra_data.as_mut_ptr() as *mut std::ffi::c_void),
                        &mut bmi,
                        DIB_RGB_COLORS,
                    );
                    
                    // Clean up GDI objects
                    SelectObject(hdc, old_bmp);
                    let _ = DeleteDC(hdc);
                    if !icon_info.hbmColor.is_invalid() { let _ = DeleteObject(icon_info.hbmColor); }
                    if !icon_info.hbmMask.is_invalid() { let _ = DeleteObject(icon_info.hbmMask); }
                    
                    // Convert BGRA to RGBA
                    let mut rgba_data = vec![0u8; (width * height * 4) as usize];
                    for i in (0..bgra_data.len()).step_by(4) {
                        rgba_data[i]     = bgra_data[i + 2]; // R <- B
                        rgba_data[i + 1] = bgra_data[i + 1]; // G <- G
                        rgba_data[i + 2] = bgra_data[i];     // B <- R
                        rgba_data[i + 3] = bgra_data[i + 3]; // A <- A
                    }
                    
                    let icon = tray_icon::Icon::from_rgba(rgba_data, width, height)?;
                    Ok(icon)
                }
            };
            
            // Try to load default icon (IDI_MAINICON = 200) from resources
            let default_icon = match load_icon_from_resource(200) {
                Ok(icon) => {
                    info!("Successfully loaded default icon from resource ID 200");
                    icon
                }
                Err(_) => {
                    // Resource loading failed, try file loading
                    info!("Resource loading failed, trying file-based icon loading");
                    
                    // Try multiple possible icon paths
                    let possible_paths = vec![
                        std::path::Path::new("..").join("icons").join("privoxy.ico"),
                        std::path::Path::new("icons").join("privoxy.ico"),
                        std::path::PathBuf::from("privoxy.ico"),
                        std::path::Path::new("..").join("privoxy.ico"),
                    ];
                    
                    let mut loaded_icon = None;
                    for icon_path in possible_paths {
                        if icon_path.exists() {
                            match tray_icon::Icon::from_path(&icon_path, None) {
                                Ok(icon) => {
                                    info!("Successfully loaded default icon from file: {:?}", icon_path);
                                    loaded_icon = Some(icon);
                                    break;
                                }
                                Err(e) => {
                                    debug!("Failed to load icon from {:?}: {}", icon_path, e);
                                }
                            }
                        }
                    }
                    
                    loaded_icon.unwrap_or_else(|| {
                        warn!("Could not find icon file, creating default icon");
                        // Create a simple 16x16 icon (blue square)
                        let mut rgba = vec![0u8; 16 * 16 * 4];
                        for i in (0..rgba.len()).step_by(4) {
                            rgba[i] = 0;     // R
                            rgba[i + 1] = 120; // G
                            rgba[i + 2] = 215; // B
                            rgba[i + 3] = 255; // A
                        }
                        tray_icon::Icon::from_rgba(rgba, 16, 16).expect("Failed to create default icon")
                    })
                }
            };
            
            // Try to load off icon (IDI_OFF = 210) from resources
            let off_icon = load_icon_from_resource(210).ok();
            if off_icon.is_some() {
                info!("Successfully loaded off icon from resource ID 210");
            }
            
            // Try to load animated icons (IDI_ANIMATED1-8 = 201-208) from resources
            let mut animated_icons = Vec::new();
            for i in 1..=8 {
                 // Icon loaded from resource
                if let Ok(icon) = load_icon_from_resource(200 + i) {
                    info!("Successfully loaded animated icon {} from resource ID {}", i, 200 + i);
                    animated_icons.push(icon);
                }
            }
            
            info!("Loaded {} animated icons from resources", animated_icons.len());
            
            return Ok(Self {
                default_icon,
                off_icon,
                animated_icons,
            });
        }
        
        // Fallback for non-Windows: try to load from files
        #[cfg(not(windows))]
        {
            let icons_dir = std::path::Path::new("..").join("icons");
            
            info!("Loading icons from: {:?}", icons_dir);
            
            // Load default icon (privoxy.ico)
            let default_icon_path = icons_dir.join("privoxy.ico");
            let default_icon = if default_icon_path.exists() {
                info!("Loading default icon from: {:?}", default_icon_path);
                match tray_icon::Icon::from_path(&default_icon_path, None) {
                    Ok(icon) => {
                        info!("Successfully loaded default icon");
                        icon
                    }
                    Err(e) => {
                        error!("Failed to load default icon: {}", e);
                        let rgba = vec![0u8; 4];
                        tray_icon::Icon::from_rgba(rgba, 1, 1)?
                    }
                }
            } else {
                error!("Default icon not found at: {:?}", default_icon_path);
                let rgba = vec![0u8; 4];
                tray_icon::Icon::from_rgba(rgba, 1, 1)?
            };
            
            // Load off icon (off.ico)
            let off_icon_path = icons_dir.join("off.ico");
            let off_icon = if off_icon_path.exists() {
                info!("Loading off icon from: {:?}", off_icon_path);
                match tray_icon::Icon::from_path(&off_icon_path, None) {
                    Ok(icon) => {
                        info!("Successfully loaded off icon");
                        Some(icon)
                    }
                    Err(e) => {
                        error!("Failed to load off icon: {}", e);
                        None
                    }
                }
            } else {
                warn!("Off icon not found at: {:?}", off_icon_path);
                None
            };
            
            // Load animated icons (radar-01.ico to radar-08.ico)
            let mut animated_icons = Vec::new();
            for i in 1..=8 {
                let anim_path = icons_dir.join(format!("radar-{:02}.ico", i));
                if anim_path.exists() {
                    info!("Loading animated icon {}: {:?}", i, anim_path);
                    match tray_icon::Icon::from_path(&anim_path, None) {
                        Ok(icon) => {
                            animated_icons.push(icon);
                        }
                        Err(e) => {
                            error!("Failed to load animated icon {}: {}", i, e);
                        }
                    }
                } else {
                    warn!("Animated icon {} not found at: {:?}", i, anim_path);
                }
            }
            
            info!("Loaded {} animated icons", animated_icons.len());
            
            return Ok(Self {
                default_icon,
                off_icon,
                animated_icons,
            });
        }
    }
    
    fn get_default(&self) -> &tray_icon::Icon {
        &self.default_icon
    }
    
    fn get_off(&self) -> Option<&tray_icon::Icon> {
        self.off_icon.as_ref()
    }
    
    fn get_animated(&self, frame: usize) -> Option<&tray_icon::Icon> {
        if self.animated_icons.is_empty() {
            None
        } else {
            Some(&self.animated_icons[frame % self.animated_icons.len()])
        }
    }
}

#[cfg(feature = "tray-icon")]
fn load_icon() -> Result<tray_icon::Icon, Box<dyn std::error::Error>> {
    // On Windows, try to load from embedded resources first (IDI_MAINICON = 200)
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{IMAGE_ICON, LR_DEFAULTSIZE, LoadImageW, GetIconInfo, ICONINFO};
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, GetDIBits, SelectObject, DeleteDC, DeleteObject,
            BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, BITMAP, GetObjectW, BI_RGB,
        };
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::Foundation::HINSTANCE;
        use windows::core::PCWSTR;
        
        let result: Result<tray_icon::Icon, Box<dyn std::error::Error>> = (|| {
            unsafe {
                let hmodule = GetModuleHandleW(None)
                    .map_err(|e| format!("GetModuleHandleW failed: {}", e))?;
                let hinstance = HINSTANCE(hmodule.0);
                
                let icon_handle = LoadImageW(
                    hinstance,
                    PCWSTR(200u16 as *const u16),  // IDI_MAINICON = 200
                    IMAGE_ICON,
                    32,
                    32,
                    LR_DEFAULTSIZE,
                ).map_err(|e| format!("LoadImageW failed: {}", e))?;
                
                let mut ii = ICONINFO::default();
                GetIconInfo(
                    windows::Win32::UI::WindowsAndMessaging::HICON(icon_handle.0),
                    &mut ii,
                ).map_err(|e| format!("GetIconInfo failed: {}", e))?;
                
                let mut bmp = BITMAP::default();
                GetObjectW(
                    ii.hbmColor,
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut bmp as *mut _ as *mut std::ffi::c_void),
                );
                
                let width = bmp.bmWidth as u32;
                let height = bmp.bmHeight as u32;
                
                if width == 0 || height == 0 {
                    if !ii.hbmColor.is_invalid() { let _ = DeleteObject(ii.hbmColor); }
                    if !ii.hbmMask.is_invalid() { let _ = DeleteObject(ii.hbmMask); }
                    return Err("Icon has zero dimensions".into());
                }
                
                let mut bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width as i32,
                        biHeight: -(height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0 as u32,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [Default::default()],
                };
                
                let mut bgra_data = vec![0u8; (width * height * 4) as usize];
                let hdc = CreateCompatibleDC(None);
                let old_bmp = SelectObject(hdc, ii.hbmColor);
                
                GetDIBits(
                    hdc,
                    ii.hbmColor,
                    0,
                    height,
                    Some(bgra_data.as_mut_ptr() as *mut std::ffi::c_void),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                
                SelectObject(hdc, old_bmp);
                let _ = DeleteDC(hdc);
                if !ii.hbmColor.is_invalid() { let _ = DeleteObject(ii.hbmColor); }
                if !ii.hbmMask.is_invalid() { let _ = DeleteObject(ii.hbmMask); }
                
                let mut rgba_data = vec![0u8; (width * height * 4) as usize];
                for i in (0..bgra_data.len()).step_by(4) {
                    rgba_data[i]     = bgra_data[i + 2];
                    rgba_data[i + 1] = bgra_data[i + 1];
                    rgba_data[i + 2] = bgra_data[i];
                    rgba_data[i + 3] = bgra_data[i + 3];
                }
                
                let icon = tray_icon::Icon::from_rgba(rgba_data, width, height)?;
                info!("Successfully loaded tray icon from embedded resource");
                Ok(icon)
            }
        })();
        
        if let Ok(icon) = result {
            return Ok(icon);
        }
        
        info!("Resource loading failed for tray icon, trying file fallback");
    }
    
    // Fallback: try to load from file
    let possible_paths = vec![
        std::path::PathBuf::from("assets").join("privoxy.ico"),
        std::path::Path::new("..").join("icons").join("privoxy.ico"),
        std::path::Path::new("icons").join("privoxy.ico"),
        std::path::PathBuf::from("privoxy.ico"),
    ];
    
    for icon_path in &possible_paths {
        if icon_path.exists() {
            info!("Loading tray icon from: {:?}", icon_path);
            match tray_icon::Icon::from_path(icon_path, None) {
                Ok(icon) => {
                    info!("Successfully loaded tray icon from file");
                    return Ok(icon);
                }
                Err(e) => {
                    debug!("Failed to load tray icon from {:?}: {}", icon_path, e);
                }
            }
        }
    }
    
    // Final fallback: create a simple colored icon
    warn!("Could not load tray icon from resources or files, creating default icon");
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for i in (0..rgba.len()).step_by(4) {
        rgba[i] = 0;       // R
        rgba[i + 1] = 120; // G
        rgba[i + 2] = 215; // B
        rgba[i + 3] = 255; // A
    }
    let icon = tray_icon::Icon::from_rgba(rgba, 16, 16)?;
    Ok(icon)
}

/// Helper function to setup or recreate the log window on the UI thread
#[cfg(feature = "tray-icon")]
fn setup_log_window(ui: &UI, log_cache: &logger::LogCache, clear_log_item: &SendMenuItem, visible: bool) {
    info!("Setting up log window (visible: {})", visible);
    let mut window = Window::new(ui, "Privoxy", 1500, 1000, WindowType::HasMenubar);
    
    let mut vbox = VerticalBox::new();
    vbox.set_padded(true);
    
    let mut log_textarea = MultilineEntry::new();
    log_textarea.set_readonly(true);
    
    // Add cached logs to textarea
    let logs_to_append = {
        if let Ok(cache) = log_cache.lock() {
            cache.0.clone()
        } else {
            Vec::new()
        }
    };
    for msg in logs_to_append {
        log_textarea.append(&msg);
        log_textarea.append("\n");
    }
    
    let log_textarea_handle = log_textarea.clone();
    vbox.append(log_textarea, LayoutStrategy::Stretchy);
    window.set_child(vbox);
    
    if visible {
        window.show();
    } else {
        window.hide();
    }
    
    // Handle window closing: hide it, but keep handles to show it later
    window.on_closing(ui, move |w| {
        info!("Log window closing, hiding window");
        w.hide();
    });
    
    // Set the clear log handler
    let log_cache_clear = log_cache.clone();
    let mut log_textarea_clear = log_textarea_handle.clone();
    let item_clone = clear_log_item.clone();
    item_clone.0.on_clicked(move |_, _| {
        info!("View -> Clear Log clicked");
        if let Ok(mut cache) = log_cache_clear.lock() {
            cache.0.clear();
        }
        log_textarea_clear.set_value("");
    });

    // Store UI handles in log cache
    if let Ok(mut cache) = log_cache.lock() {
        #[cfg(feature = "tray-icon")]
        {
            cache.1 = Some(logger::UiLogHandles {
                event_queue: Arc::new(ui.event_queue()),
                window: Some(window.clone()),
                log_textarea: Some(log_textarea_handle.clone()),
            });
        }
    }
}

#[cfg(not(feature = "tray-icon"))]
pub struct TrayIconApp {
    _config: Arc<Config>,
    _state: Arc<AppState>,
}

#[cfg(not(feature = "tray-icon"))]
impl TrayIconApp {
    pub fn new(config: Arc<Config>, state: Arc<AppState>) -> Self {
        Self { _config: config, _state: state }
    }

    pub fn run(&mut self) -> PrivoxyResult<()> {
        Ok(())
    }
}