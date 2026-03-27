import sys
import re

tray_file = 'src/tray_icon.rs'
with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Imports and global flag
if 'AtomicBool' not in content:
    content = content.replace('use std::sync::Arc;', 'use std::sync::Arc;\nuse std::sync::atomic::{AtomicBool, Ordering};\n\nstatic UI_THREAD_STARTED: AtomicBool = AtomicBool::new(false);')

# 2. Extract load_icon and IconManager
icon_manager_start = content.find('struct IconManager')
if icon_manager_start == -1:
    icon_manager_start = content.find('/// Icon manager')
suffix = content[icon_manager_start:]

# 3. Build the new TrayIconApp impl block
new_impl = """
impl TrayIconApp {
    fn ensure_ui_thread_started(&self) {
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
            options_menu.append_item("Main Configuration").on_clicked(move |_, _| { 
                if let Some(path) = &config_o.config_file { let p = path.clone(); std::thread::spawn(move || { let _ = opener::open(p); }); } 
            });
            let config_a = config_ui.clone();
            options_menu.append_item("Default Actions").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_a.confdir { let p = confdir.join("default.action"); std::thread::spawn(move || { let _ = opener::open(p); }); } 
            });
            let config_u = config_ui.clone();
            options_menu.append_item("User Actions").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_u.confdir { let p = confdir.join("user.action"); std::thread::spawn(move || { let _ = opener::open(p); }); } 
            });
            let config_df = config_ui.clone();
            options_menu.append_item("Default Filters").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_df.confdir { let p = confdir.join("default.filter"); std::thread::spawn(move || { let _ = opener::open(p); }); } 
            });
            let config_uf = config_ui.clone();
            options_menu.append_item("User Filters").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_uf.confdir { let p = confdir.join("user.filter"); std::thread::spawn(move || { let _ = opener::open(p); }); } 
            });
            
            #[cfg(feature = "trust")]
            {
                let config_t = config_ui.clone();
                options_menu.append_item("Trust list").on_clicked(move |_, _| { 
                    if let Some(ref confdir) = config_t.confdir { let p = confdir.join("trust"); std::thread::spawn(move || { let _ = opener::open(p); }); } 
                });
            }
            
            options_menu.append_separator();
            options_menu.append_check_item("Enable");
            
            let help_menu = Menu::new("Help");
            help_menu.append_item("Status").on_clicked(move |_, _| {
                std::thread::spawn(move || { let _ = opener::open("http://config.privoxy.org/show-status"); });
            });
            help_menu.append_item("FAQ").on_clicked(|_, _| { 
                std::thread::spawn(move || { let _ = opener::open("https://www.privoxy.org/faq/"); }); 
            });
            help_menu.append_item("Manual").on_clicked(|_, _| { 
                std::thread::spawn(move || { let _ = opener::open("https://www.privoxy.org/user-manual/"); }); 
            });
            help_menu.append_item("GPL").on_clicked(|_, _| { 
                std::thread::spawn(move || { let _ = opener::open("https://www.gnu.org/copyleft/gpl.html"); }); 
            });
            help_menu.append_separator();
            help_menu.append_item("About").on_clicked(|_, _| {
                let dialog = rfd::MessageDialog::new().set_title("About Privoxy").set_description("Privoxy Rust implementation\\nVersion 4.1.0\\n\\nCopyright (C) 2024 The Privoxy Team");
                dialog.show();
            });
            
            setup_log_window(&ui, &log_cache_thread, &SendMenuItem(clear_log_item.clone()), false);
            ui.main();
        });
    }

    pub fn new(config: Arc<Config>, state: Arc<AppState>, shutdown_sender: tokio::sync::oneshot::Sender<()>) -> Self {
        let log_cache: LogCache = Arc::new(std::sync::Mutex::new((Vec::new(), None)));
        crate::logger::register_gui_cache(log_cache.clone());
        let log_file = config.log_file.as_ref().and_then(|p| std::fs::File::create(p).ok());
        let icon_manager = IconManager::new().ok();
        Self {
            config, state, shutdown_sender: Arc::new(std::sync::Mutex::new(Some(shutdown_sender))),
            log_messages: true, message_highlighting: true, limit_buffer_size: true, activity_animation: true,
            show_window: true, window_thread: None, log_cache, max_buffer_lines: 200, log_file, icon_manager,
            current_frame: 0, tray_icon: None, menu: None, ui_handles: None, clear_log_item_libui: None,
        }
    }

    fn show_window(&mut self) {
        self.ensure_ui_thread_started();
        restore_window(&self.log_cache);
    }

    pub fn run(&mut self) -> PrivoxyResult<()> {
        self.ensure_ui_thread_started();

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
        let is_enabled = self.state.is_enabled();
        let menu_item_enable = tray_icon::menu::CheckMenuItem::new("&Enable", true, is_enabled, None);
        let menu_item_show_window = tray_icon::menu::CheckMenuItem::new("Show Privoxy &Window", true, false, None); 

        let menu = TrayMenu::with_items(&[
            &menu_item_exit, 
            &PredefinedMenuItem::separator(), 
            &submenu_edit, 
            &PredefinedMenuItem::separator(),
            &menu_item_enable, 
            &menu_item_show_window,
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

        let icon = match self.icon_manager.as_ref() {
            Some(mgr) => mgr.default_icon.clone(),
            None => load_icon().map_err(|e| PrivoxyError::Other(e.to_string()))?
        };
        
        let tray_icon_builder = TrayIconBuilder::new()
            .with_tooltip("Privoxy")
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
        let config_captured = self.config.clone();
        let log_cache_captured = self.log_cache.clone();

        event_loop.run(move |event, _| {
            #[cfg(target_os = "macos")]
            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 if self.tray_icon.is_none() {
                     if let Ok(i) = TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_icon(icon.clone()).build() {
                         self.tray_icon = Some(i);
                     }
                 }
            }
            
            if let winit::event::Event::UserEvent(()) = event {
                info!("Shutdown signal received, exiting event loop");
                std::process::exit(0);
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
                } else if id == show_window_id {
                    restore_window(&log_cache_captured);
                } else if id == enable_id {
                    let new_state = !state.is_enabled();
                    state.set_enabled(new_state);
                    menu_item_enable.set_checked(new_state);
                } else if id == edit_config_id {
                    if let Some(p) = &config_captured.config_file { let pc = p.clone(); std::thread::spawn(move || { let _ = opener::open(pc); }); }
                } else if id == edit_default_actions_id {
                    if let Some(ref d) = config_captured.confdir { let app = d.join("default.action"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if id == edit_user_actions_id {
                    if let Some(ref d) = config_captured.confdir { let app = d.join("user.action"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if id == edit_default_filters_id {
                    if let Some(ref d) = config_captured.confdir { let app = d.join("default.filter"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if id == edit_user_filters_id {
                    if let Some(ref d) = config_captured.confdir { let app = d.join("user.filter"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                }
                #[cfg(feature = "trust")]
                if id == edit_trust_id {
                    if let Some(ref d) = config_captured.confdir { let app = d.join("trust"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                }
            }
        }).map_err(|e| PrivoxyError::Other(e.to_string()))
    }
}

fn restore_window(log_cache: &LogCache) {
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
                                use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, IsIconic};
                                use windows::core::w;
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
    let mut window = Window::new(ui, "Privoxy", 1000, 800, WindowType::HasMenubar);
    let mut vbox = VerticalBox::new();
    vbox.set_padded(true);
    let mut log_textarea = MultilineEntry::new();
    log_textarea.set_readonly(true);
    
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
        log_textarea.append(&full_log);
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
}
"""

prefix = content[:content.find('impl TrayIconApp {')]

new_content = prefix + new_impl + "\\n" + suffix

with open(tray_file, 'w', encoding='utf-8') as f:
    f.write(new_content)
