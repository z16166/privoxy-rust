import sys

tray_file = 'src/tray_icon.rs'
with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

start_marker = 'let options_menu = Menu::new("Options");'
end_marker = 'setup_log_window(&ui, &log_cache_thread, &SendMenuItem(clear_log_item.clone()), false);'

start_idx = content.find(start_marker)
end_idx = content.find(end_marker)

if start_idx == -1 or end_idx == -1:
    print('Failed to find markers')
    sys.exit(1)

replacement = """let options_menu = Menu::new("Options");
            let config_o = config_ui.clone();
            options_menu.append_item("Main Configuration").on_clicked(move |_, _| { 
                if let Some(path) = &config_o.config_file { 
                    let p = path.clone();
                    std::thread::spawn(move || { let _ = opener::open(p); }); 
                } 
            });
            let config_a = config_ui.clone();
            options_menu.append_item("Default Actions").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_a.confdir { 
                    let p = confdir.join("default.action");
                    std::thread::spawn(move || { let _ = opener::open(p); }); 
                } 
            });
            let config_u = config_ui.clone();
            options_menu.append_item("User Actions").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_u.confdir { 
                    let p = confdir.join("user.action");
                    std::thread::spawn(move || { let _ = opener::open(p); }); 
                } 
            });
            let config_df = config_ui.clone();
            options_menu.append_item("Default Filters").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_df.confdir { 
                    let p = confdir.join("default.filter");
                    std::thread::spawn(move || { let _ = opener::open(p); }); 
                } 
            });
            let config_uf = config_ui.clone();
            options_menu.append_item("User Filters").on_clicked(move |_, _| { 
                if let Some(ref confdir) = config_uf.confdir { 
                    let p = confdir.join("user.filter");
                    std::thread::spawn(move || { let _ = opener::open(p); }); 
                } 
            });
            
            #[cfg(feature = "trust")]
            {
                let config_t = config_ui.clone();
                options_menu.append_item("Trust list").on_clicked(move |_, _| { 
                    if let Some(ref confdir) = config_t.confdir { 
                        let p = confdir.join("trust");
                        std::thread::spawn(move || { let _ = opener::open(p); }); 
                    } 
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
            
            """

content = content[:start_idx] + replacement + content[end_idx:]

with open(tray_file, 'w', encoding='utf-8') as f:
    f.write(content)
