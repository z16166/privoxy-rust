import sys
import re

tray_file = 'src/tray_icon.rs'
with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Add AtomicBool
if 'UI_THREAD_STARTED' not in content:
    content = content.replace('use std::sync::Arc;', 'use std::sync::Arc;\\nuse std::sync::atomic::{AtomicBool, Ordering};\\n\\nstatic UI_THREAD_STARTED: AtomicBool = AtomicBool::new(false);')

# 2. Fix show_window multiple threads bug
show_win_old = '''    fn show_window(&mut self) {
        if let Some(ref handles) = self.ui_handles {'''
show_win_new = '''    fn show_window(&mut self) {
        if let Some(ref handles) = self.ui_handles {
            let event_queue = handles.event_queue.clone();
            let log_cache = self.log_cache.clone();
            event_queue.queue_main(move || {
                if let Ok(mut cache) = log_cache.lock() {
                    if let Some(ref mut gui_handles) = cache.1 {
                        if let Some(ref mut window) = gui_handles.window {
                            window.show();
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
            return;
        }

        if UI_THREAD_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }'''
content = content.replace(show_win_old, show_win_new)

# 3. Fix Help Menu opener calls blocking the UI thread (in show_window's thread spawn)
help_old = '''            help_menu.append_item("Status").on_clicked(|_, _| {
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
            });'''
help_new = '''            help_menu.append_item("Status").on_clicked(|_, _| {
                std::thread::spawn(|| { let _ = opener::open("http://config.privoxy.org/show-status"); });
            });
            help_menu.append_item("FAQ").on_clicked(|_, _| {
                std::thread::spawn(|| { let _ = opener::open("https://www.privoxy.org/faq/"); });
            });
            help_menu.append_item("Manual").on_clicked(|_, _| {
                std::thread::spawn(|| { let _ = opener::open("https://www.privoxy.org/user-manual/"); });
            });
            help_menu.append_item("GPL").on_clicked(|_, _| {
                std::thread::spawn(|| { let _ = opener::open("https://www.gnu.org/copyleft/gpl.html"); });
            });'''
content = content.replace(help_old, help_new)

# 4. Fix setup_log_window replacing history and size
log_old = '''    if let Ok(cache) = log_cache.lock() {
        #[cfg(feature = "tray-icon")]
        let logs = cache.0.clone();
        #[cfg(not(feature = "tray-icon"))]
        let logs = cache.clone();
        
        let full_log = logs.join("\\n");
        log_textarea.set_value(&full_log);
    }'''
log_new = '''    let logs_to_append = if let Ok(cache) = log_cache.lock() {
        #[cfg(feature = "tray-icon")]
        let all_logs = &cache.0;
        #[cfg(not(feature = "tray-icon"))]
        let all_logs = &*cache;
        
        if all_logs.len() > 200 { all_logs[all_logs.len()-200..].to_vec() } else { all_logs.clone() }
    } else { Vec::new() };
    
    if !logs_to_append.is_empty() {
        let mut full_log = String::with_capacity(logs_to_append.len() * 100);
        for msg in logs_to_append { full_log.push_str(&msg); full_log.push_str("\\n"); }
        log_textarea.append(&full_log);
    }'''
content = content.replace(log_old, log_new)

# 5. Fix run() flat menu and right click latency!
menu_old = '''        // Create menu items for File submenu
        let menu_item_show_window = TrayMenuItem::new("Show Window", true, None);'''
to_replace = content[content.find(menu_old):content.find('self.menu = Some(menu.clone());')]

flat_menu_new = '''        let menu_item_show_window = TrayMenuItem::new("Privoxy Window", true, None);
        let menu_item_edit_config = TrayMenuItem::new("Main Configuration", true, None);
        let menu_item_edit_default_actions = TrayMenuItem::new("Default Actions", true, None);
        let menu_item_edit_user_actions = TrayMenuItem::new("User Actions", true, None);
        let menu_item_edit_default_filters = TrayMenuItem::new("Default Filters", true, None);
        let menu_item_edit_user_filters = TrayMenuItem::new("User Filters", true, None);
        #[cfg(feature = "trust")]
        let menu_item_edit_trust = TrayMenuItem::new("Trust list", true, None);
        
        let is_enabled = self.state.is_enabled();
        let menu_item_enable = tray_icon::menu::CheckMenuItem::new("Enable", true, is_enabled, None);
        
        let menu_item_status = TrayMenuItem::new("Privoxy Status", true, None);
        let menu_item_exit = TrayMenuItem::new("Exit Privoxy", true, None);

        let mut flat_items: Vec<&dyn tray_icon::menu::IsMenuItem> = vec![
            &menu_item_show_window,
            &PredefinedMenuItem::separator(),
            &menu_item_edit_config,
            &menu_item_edit_default_actions,
            &menu_item_edit_user_actions,
            &menu_item_edit_default_filters,
            &menu_item_edit_user_filters,
        ];
        #[cfg(feature = "trust")]
        flat_items.push(&menu_item_edit_trust);
        
        flat_items.extend_from_slice(&[
            &PredefinedMenuItem::separator(),
            &menu_item_enable,
            &PredefinedMenuItem::separator(),
            &menu_item_status,
            &PredefinedMenuItem::separator(),
            &menu_item_exit,
        ]);
        
        let menu = TrayMenu::with_items(&flat_items).map_err(|e| PrivoxyError::Other(e.to_string()))?;
        '''

content = content.replace(to_replace, flat_menu_new)

# 6. Make opener logic in run() non-blocking
run_opener_old = '''                } else if event_id == &edit_config_id {
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
                }'''

run_opener_new = '''                } else if event_id == &edit_config_id {
                    if let Some(p) = &config.config_file { let p = p.clone(); std::thread::spawn(move || { let _ = opener::open(p); }); }
                } else if event_id == &edit_default_actions_id {
                    if let Some(ref d) = config.confdir { let app = d.join("default.action"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if event_id == &edit_user_actions_id {
                    if let Some(ref d) = config.confdir { let app = d.join("user.action"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if event_id == &edit_default_filters_id {
                    if let Some(ref d) = config.confdir { let app = d.join("default.filter"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if event_id == &edit_user_filters_id {
                    if let Some(ref d) = config.confdir { let app = d.join("user.filter"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if {
                    #[cfg(feature = "trust")]
                    { event_id == &edit_trust_id }
                    #[cfg(not(feature = "trust"))]
                    { false }
                } {
                    if let Some(ref d) = config.confdir { let app = d.join("trust"); std::thread::spawn(move || { let _ = opener::open(app); }); }
                } else if event_id == &status_id {
                    std::thread::spawn(|| { let _ = opener::open("http://config.privoxy.org/show-status"); });
                }'''
content = content.replace(run_opener_old, run_opener_new)

# 7. Apply menu on StartCause::Init to fix delayed right-click!
# Find the end of `if let winit::event::Event::UserEvent(()) = event {` handling to inject init logic
init_hook = '''            // Check for shutdown event
            if let winit::event::Event::UserEvent(()) = event {'''

new_init_hook = '''            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                if let (Some(icon), Some(menu)) = (&self.tray_icon, &self.menu) {
                    let _ = icon.set_menu(Some(Box::new(menu.clone())));
                }
            }

            // Check for shutdown event
            if let winit::event::Event::UserEvent(()) = event {'''

content = content.replace(init_hook, new_init_hook)

# Remove old right click menu attach since it's already attached at Init!
right_click_old = '''                    TrayIconEvent::Click { button: MouseButton::Right, .. } => {
                        info!("Tray icon right clicked - attaching menu");
                        if let (Some(icon), Some(menu)) = (&self.tray_icon, &self.menu) {
                            icon.set_menu(Some(Box::new(menu.clone())));
                        }
                    }'''
content = content.replace(right_click_old, "")

with open('src/tray_icon.rs', 'w', encoding='utf-8', newline='\n') as f:
    f.write(content)
