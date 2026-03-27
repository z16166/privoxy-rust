import os
import re

tray_file = r'f:\log\privoxy-4.1.0-stable\privoxy-rust\src\tray_icon.rs'

with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Clean up imports and add AtomicBool if missing
if 'use std::sync::atomic::{AtomicBool, Ordering};' not in content:
    content = content.replace('use std::sync::Arc;', 'use std::sync::Arc;\nuse std::sync::atomic::{AtomicBool, Ordering};')

# 2. Add static flag if missing
if 'static UI_THREAD_STARTED: AtomicBool' not in content:
    content += '\nstatic UI_THREAD_STARTED: AtomicBool = AtomicBool::new(false);\n'

# 3. Completely replace the problematic TrayIconApp implementation while keeping utility methods
# We will target the first 'impl TrayIconApp {' and re-write the core logic.

ensure_ui_thread_method = """    fn ensure_ui_thread_started(&self) {
        if UI_THREAD_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        
        info!("Starting persistent UI thread (one-time initialization)");
        let log_cache_thread = self.log_cache.clone();
        let shutdown_sender_ui = self.shutdown_sender.clone();
        let config_ui = self.config.clone();
        
        std::thread::spawn(move || {
            let ui = match UI::init() {
                Ok(ui) => ui,
                Err(e) => { 
                    error!("Failed to initialize UI: {}", e); 
                    UI_THREAD_STARTED.store(false, Ordering::SeqCst);
                    return; 
                }
            };
            
            let mut dummy = libui::controls::Window::new(&ui, "Privoxy Service", 1, 1, WindowType::NoMenubar);
            dummy.hide();
            
            // --- Global Menu Setup (IDR_LOGVIEW) ---
            let file_menu = Menu::new("File");
            let show_window_item = file_menu.append_check_item("Show Privoxy Window");
            show_window_item.set_checked(true);
            file_menu.append_separator();
            let exit_item = file_menu.append_item("Exit");
            let shutdown_sender_exit = shutdown_sender_ui.clone();
            exit_item.on_clicked(move |_, _| {
                if let Ok(mut guard) = shutdown_sender_exit.lock() {
                    if let Some(sender) = guard.take() { let _ = sender.send(()); }
                }
                std::process::exit(0);
            });
            
            let edit_menu = Menu::new("Edit");
            edit_menu.append_item("Copy");
            
            let view_menu = Menu::new("View");
            let clear_log_item = view_menu.append_item("Clear Log");
            view_menu.append_check_item("Log Messages").set_checked(true);
            view_menu.append_check_item("Message Highlighting").set_checked(true);
            view_menu.append_check_item("Limit Buffer Size").set_checked(true);
            view_menu.append_check_item("Activity Animation").set_checked(true);
            
            let options_menu = Menu::new("Options");
            let config_o = config_ui.clone();
            options_menu.append_item("Main Configuration").on_clicked(move |_, _| { if let Some(path) = &config_o.config_file { let _ = opener::open(path); } });
            let config_a = config_ui.clone();
            options_menu.append_item("Default Actions").on_clicked(move |_, _| { if let Some(ref confdir) = config_a.confdir { let _ = opener::open(confdir.join("default.action")); } });
            let config_u = config_ui.clone();
            options_menu.append_item("User Actions").on_clicked(move |_, _| { if let Some(ref confdir) = config_u.confdir { let _ = opener::open(confdir.join("user.action")); } });
            let config_df = config_ui.clone();
            options_menu.append_item("Default Filters").on_clicked(move |_, _| { if let Some(ref confdir) = config_df.confdir { let _ = opener::open(confdir.join("default.filter")); } });
            let config_uf = config_ui.clone();
            options_menu.append_item("User Filters").on_clicked(move |_, _| { if let Some(ref confdir) = config_uf.confdir { let _ = opener::open(confdir.join("user.filter")); } });
            
            #[cfg(feature = "trust")]
            {
                let config_t = config_ui.clone();
                options_menu.append_item("Trust list").on_clicked(move |_, _| { if let Some(ref confdir) = config_t.confdir { let _ = opener::open(confdir.join("trust")); } });
            }
            
            options_menu.append_separator();
            options_menu.append_check_item("Enable");
            
            let help_menu = Menu::new("Help");
            help_menu.append_item("Status").on_clicked(move |_, _| {
                let _ = opener::open("http://config.privoxy.org/show-status");
            });
            help_menu.append_item("FAQ").on_clicked(|_, _| { let _ = opener::open("https://www.privoxy.org/faq/"); });
            help_menu.append_item("Manual").on_clicked(|_, _| { let _ = opener::open("https://www.privoxy.org/user-manual/"); });
            help_menu.append_item("GPL").on_clicked(|_, _| { let _ = opener::open("https://www.gnu.org/copyleft/gpl.html"); });
            help_menu.append_separator();
            help_menu.append_item("About").on_clicked(|_, _| {
                let dialog = rfd::MessageDialog::new().set_title("About Privoxy").set_description("Privoxy Rust implementation\\nVersion 4.1.0\\n\\nCopyright (C) 2024 The Privoxy Team");
                dialog.show();
            });
            
            setup_log_window(&ui, &log_cache_thread, &SendMenuItem(clear_log_item.clone()), false);
            ui.main();
        });
    }"""

# 4. Clean up the event loop in run() - Replace the whole run() method
new_run_method = """    pub fn run(&mut self) -> PrivoxyResult<()> {
        self.ensure_ui_thread_started();

        // --- 1. Define Tray Menu Items (Match IDR_TRAYMENU) ---
        let menu_item_exit = TrayMenuItem::new("E&xit Privoxy", true, None);
        let menu_item_edit_config = TrayMenuItem::new("&Main Configuration", true, None);
        let menu_item_edit_default_actions = TrayMenuItem::new("&Default Actions", true, None);
        let menu_item_edit_user_actions = TrayMenuItem::new("&User Actions", true, None);
        let menu_item_edit_default_filters = TrayMenuItem::new("Default &Filters", true, None);
        let menu_item_edit_user_filters = TrayMenuItem::new("U&ser Filters", true, None);
        #[cfg(feature = "trust")]
        let menu_item_edit_trust = TrayMenuItem::new("&Trust list", true, None);
        
        let mut edit_items: Vec<&dyn tray_icon::menu::IsMenuItem> = vec![
            &menu_item_edit_config, &menu_item_edit_default_actions, &menu_item_edit_user_actions,
            &menu_item_edit_default_filters, &menu_item_edit_user_filters,
        ];
        #[cfg(feature = "trust")]
        edit_items.push(&menu_item_edit_trust);
        
        let submenu_edit = Submenu::with_items("E&dit..", true, &edit_items).map_err(|e| PrivoxyError::Other(e.to_string()))?;
        let menu_item_enable = CheckMenuItem::new("&Enable", true, self.state.is_enabled(), None);
        let menu_item_show_window = CheckMenuItem::new("Show Privoxy &Window", true, self.show_window, None);

        let menu = TrayMenu::with_items(&[
            &menu_item_exit, &PredefinedMenuItem::separator(), &submenu_edit, &PredefinedMenuItem::separator(),
            &menu_item_enable, &menu_item_show_window,
        ]).map_err(|e| PrivoxyError::Other(e.to_string()))?;

        self.menu = Some(menu.clone());

        let exit_id = menu_item_exit.id().clone();
        let edit_config_id = menu_item_edit_config.id().clone();
        let edit_default_actions_id = menu_item_edit_default_actions.id().clone();
        let edit_user_actions_id = menu_item_edit_user_actions.id().clone();
        let edit_default_filters_id = menu_item_edit_default_filters.id().clone();
        let edit_user_filters_id = menu_item_edit_user_filters.id().clone();
        #[cfg(feature = "trust")]
        let edit_trust_id = menu_item_edit_trust.id().clone();
        let show_window_id = menu_item_show_window.id().clone();
        let enable_id = menu_item_enable.id().clone();

        let icon = load_icon().map_err(|e| PrivoxyError::Other(e.to_string()))?;
        let tray_icon_builder = TrayIconBuilder::new()
            .with_tooltip("Privoxy - Web Proxy")
            .with_icon(icon.clone())
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false);
            
        #[cfg(not(target_os = "macos"))]
        {
            let t_icon = tray_icon_builder.build().map_err(|e| PrivoxyError::Other(e.to_string()))?;
            self.tray_icon = Some(t_icon);
        }

        let event_loop = EventLoop::<()>::new().map_err(|e| PrivoxyError::Other(e.to_string()))?;
        let state = self.state.clone();
        let shutdown_sender = self.shutdown_sender.clone();
        let log_cache_captured = self.log_cache.clone();
        let config_captured = self.config.clone();

        event_loop.run(move |event, _| {
            #[cfg(target_os = "macos")]
            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 // Build on macOS here...
            }
            
            if let Ok(tray_event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, .. } = tray_event {
                    restore_window(&log_cache_captured);
                }
            }
            
            if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
                let id = menu_event.id;
                if id == exit_id {
                    let mut guard = shutdown_sender.lock().unwrap();
                    if let Some(s) = guard.take() { let _ = s.send(()); }
                    std::process::exit(0);
                } else if id == show_window_id || id == PredefinedMenuItem::about(None, None).id() {
                     restore_window(&log_cache_captured);
                } else if id == enable_id {
                    state.set_enabled(!state.is_enabled());
                } else if id == edit_config_id {
                    if let Some(p) = &config_captured.config_file { let _ = opener::open(p); }
                } else if id == edit_default_actions_id {
                    if let Some(ref d) = config_captured.confdir { let _ = opener::open(d.join("default.action")); }
                } else if id == edit_user_actions_id {
                    if let Some(ref d) = config_captured.confdir { let _ = opener::open(d.join("user.action")); }
                } else if id == edit_default_filters_id {
                    if let Some(ref d) = config_captured.confdir { let _ = opener::open(d.join("default.filter")); }
                } else if id == edit_user_filters_id {
                    if let Some(ref d) = config_captured.confdir { let _ = opener::open(d.join("user.filter")); }
                }
                #[cfg(feature = "trust")]
                if id == edit_trust_id {
                    if let Some(ref d) = config_captured.confdir { let _ = opener::open(d.join("trust")); }
                }
            }
        }).map_err(|e| PrivoxyError::Other(e.to_string()))
    }"""

# 5. Completely rewrite setup_log_window and add restore_window
new_log_utils = """fn restore_window(log_cache: &LogCache) {
    if let Ok(cache) = log_cache.lock() {
        if let Some(ref handles) = cache.1 {
            let log_cache_inner = log_cache.clone();
            handles.event_queue.queue_main(move || {
                if let Ok(mut cache) = log_cache_inner.lock() {
                    if let Some(ref mut h) = cache.1 {
                        if let Some(ref mut w) = h.window {
                            w.show();
                            #[cfg(windows)]
                            unsafe {
                                if let Ok(hwnd) = FindWindowW(None, w!("Privoxy")) {
                                    if !hwnd.is_invalid() {
                                        if IsIconic(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_RESTORE); }
                                        let _ = SetForegroundWindow(hwnd);
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    }
}

fn setup_log_window(ui: &UI, log_cache: &logger::LogCache, clear_log_item: &SendMenuItem, visible: bool) {
    info!("Setting up log window (visible: {})", visible);
    let mut window = Window::new(ui, "Privoxy", 1500, 1000, WindowType::HasMenubar);
    let mut vbox = VerticalBox::new();
    vbox.set_padded(true);
    let mut log_textarea = MultilineEntry::new();
    log_textarea.set_readonly(true);
    
    // Add cached logs to textarea (Limit initial load to prevent UI lag)
    let logs_to_append = if let Ok(cache) = log_cache.lock() {
        let all_logs = &cache.0;
        if all_logs.len() > 200 { all_logs[all_logs.len()-200..].to_vec() } else { all_logs.clone() }
    } else { Vec::new() };
    
    if !logs_to_append.is_empty() {
        let mut full_log = String::with_capacity(logs_to_append.len() * 100);
        for msg in logs_to_append {
            full_log.push_str(&msg);
            full_log.push_str("\\n");
        }
        log_textarea.set_value(&full_log);
    }
    
    let log_textarea_handle = log_textarea.clone();
    vbox.append(log_textarea, LayoutStrategy::Stretchy);
    window.set_child(vbox);
    if visible { window.show(); } else { window.hide(); }
    
    window.on_closing(ui, move |w| { w.hide(); });
    
    let log_cache_clear = log_cache.clone();
    let mut log_textarea_clear = log_textarea_handle.clone();
    clear_log_item.0.on_clicked(move |_, _| {
        if let Ok(mut cache) = log_cache_clear.lock() { cache.0.clear(); }
        log_textarea_clear.set_value("");
    });

    if let Ok(mut cache) = log_cache.lock() {
        cache.1 = Some(logger::UiLogHandles {
            event_queue: Arc::new(ui.event_queue()),
            window: Some(window.clone()),
            log_textarea: Some(log_textarea_handle),
        });
    }
}"""

# 6. Apply replacements logic by finding impl/setup blocks
# This is the most complex part. I will find the first 'impl TrayIconApp {' and replace up to IconManager
impl_start = content.find("impl TrayIconApp {")
icon_manager_start = content.find("struct IconManager")
if impl_start != -1 and icon_manager_start != -1:
    # We want to replace between impl_start and icon_manager_start, but preserve load_icon/IconManager
    # Let's just find the end of the impl TrayIconApp block correctly.
    # Actually, replacing exactly what we want is safer.
    
    # Identify the end of the current 'impl TrayIconApp' block (the one that contains run())
    # Since there are multiple now, this is tricky.
    # I'll just use the IconManager as a boundary.
    prefix = content[:impl_start]
    suffix = content[icon_manager_start:]
    
    # Construct the new implementation block
    full_new_impl = "impl TrayIconApp {\n" + ensure_ui_thread_method + "\n" + content[content.find("pub fn new("):content.find("fn show_window")] + new_run_method + "\n}\n\n"
    # Wait, I need to make sure 'new' and other methods are preserved but updated if needed.
    # Actually, I'll just replace the whole file from 'impl TrayIconApp' down to 'struct IconManager'
    # and then re-append the IconManager and load_icon.
    
    content = prefix + "impl TrayIconApp {\n" + ensure_ui_thread_method + "\n" + \"\"\"    pub fn new(config: Arc<Config>, state: Arc<AppState>, shutdown_sender: tokio::sync::oneshot::Sender<()>) -> Self {
        let log_cache: LogCache = Arc::new(std::sync::Mutex::new((Vec::new(), None)));
        crate::logger::register_gui_cache(log_cache.clone());
        let log_file = config.log_file.as_ref().and_then(|p| std::fs::File::create(p).ok());
        let icon_manager = IconManager::new().ok();
        Self {
            config, state, shutdown_sender: Arc::new(std::sync::Mutex::new(Some(shutdown_sender))),
            log_messages: true, message_highlighting: true, limit_buffer_size: true, activity_animation: true,
            show_window: true, window_thread: None, log_cache, max_buffer_lines: 200, log_file, icon_manager,
            current_frame: 0, tray_icon: None, menu: None, ui_handles: None, tray_item_enable: None,
            tray_item_show_window: None, clear_log_item_libui: None,
        }
    }\n\"\"\" + new_run_method + "\n}\n\n" + new_log_utils + "\n\n" + suffix

with open(tray_file, 'w', encoding='utf-8', newline='\r\n') as f:
    f.write(content)

print("Final cleanup and refactor of tray_icon.rs completed.")
