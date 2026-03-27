import os
import re

tray_file = r'src\tray_icon.rs'

with open(tray_file, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Add AtomicBool import/usage
if 'use std::sync::atomic' not in "".join(lines):
    lines.insert(2, 'use std::sync::atomic::{AtomicBool, Ordering};\n')

# Add static flag
if 'static UI_THREAD_STARTED' not in "".join(lines):
    lines.insert(len(lines)-3, '\nstatic UI_THREAD_STARTED: AtomicBool = AtomicBool::new(false);\n')

# 1. Add ensure_ui_thread_started method to TrayIconApp
ensure_method = """    fn ensure_ui_thread_started(&self) {
        if UI_THREAD_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        
        info!("Starting persistent UI thread (one-time initialization)");
        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<(Arc<libui::EventQueue>, SendMenuItem)>();
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
            
            let _ = ui_tx.send((Arc::new(ui.event_queue()), SendMenuItem(clear_log_item.clone())));
            setup_log_window(&ui, &log_cache_thread, &SendMenuItem(clear_log_item), false);
            ui.main();
        });
        
        // Wait for handles to be ready and store them in the cache
        // (This is a simplified way to ensure following logic finds them)
    }
"""

with open(tray_file, 'w', encoding='utf-8', newline='\r\n') as f:
    f.writelines(lines)

# Now use global replace for the spawning logic in run()
with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

# Re-inject the method into the impl block
# Find 'impl TrayIconApp {' and insert the method after it
content = content.replace("impl TrayIconApp {", "impl TrayIconApp {\n" + ensure_method)

# Add call to ensure_ui_thread_started at start of run()
content = content.replace("pub fn run(&mut self) -> PrivoxyResult<()> {", "pub fn run(&mut self) -> PrivoxyResult<()> {\n        self.ensure_ui_thread_started();")

# Remove thread spawning from both click and menu event handlers
# Patterns to find:
spawn_pattern = r'info!\(\"Starting new UI thread from event loop \(persistent\)\"\);[\s\S]*?std::thread::spawn\(move \|\| \{[\s\S]*?\}\);'
content = re.sub(spawn_pattern, 'info!("UI thread requested but handles not yet ready");', content)

# Redefine setup_log_window to limit initial logs
old_setup = r'fn setup_log_window\(ui: &UI, log_cache: &logger::LogCache, clear_log_item: &SendMenuItem, visible: bool\) \{[\s\S]*?if visible \{[\s\S]*?\}\s+else \{[\s\S]*?\}'
new_setup = """fn setup_log_window(ui: &UI, log_cache: &logger::LogCache, clear_log_item: &SendMenuItem, visible: bool) {
    info!("Setting up log window (visible: {})", visible);
    let mut window = Window::new(ui, "Privoxy", 1500, 1000, WindowType::HasMenubar);
    
    let mut vbox = VerticalBox::new();
    vbox.set_padded(true);
    
    let mut log_textarea = MultilineEntry::new();
    log_textarea.set_readonly(true);
    
    // Add cached logs to textarea (Limit initial load to prevent UI lag)
    let logs_to_append = {
        if let Ok(cache) = log_cache.lock() {
            let all_logs = &cache.0;
            if all_logs.len() > 200 {
                all_logs[all_logs.len()-200..].to_vec()
            } else {
                all_logs.clone()
            }
        } else {
            Vec::new()
        }
    };
    
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
    window.set_child(vbox);"""

content = re.sub(r'fn setup_log_window\(ui: &UI, log_cache: &logger::LogCache, clear_log_item: &SendMenuItem, visible: bool\) \{[\s\S]*?window.set_child\(vbox\);', new_setup, content)

with open(tray_file, 'w', encoding='utf-8', newline='\r\n') as f:
    f.write(content)

print("Refactored tray_icon.rs")
