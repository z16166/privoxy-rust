import sys

tray_file = 'src/tray_icon.rs'
with open(tray_file, 'r', encoding='utf-8') as f:
    content = f.read()

start_idx = content.find('    pub fn run(&mut self) -> PrivoxyResult<()> {')
end_idx = content.find('\n}\n\n/// Icon manager')

if start_idx == -1 or end_idx == -1:
    print('Failed to find run indices', start_idx, end_idx)
    sys.exit(1)

new_run = '''    pub fn run(&mut self) -> PrivoxyResult<()> {
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
                    self.show_window();
                }
            }
            
            if let Ok(menu_event) = MenuEvent::receiver().try_recv() {
                let id = menu_event.id;
                if id == exit_id {
                    let mut guard = shutdown_sender.lock().unwrap();
                    if let Some(s) = guard.take() { let _ = s.send(()); }
                    std::process::exit(0);
                } else if id == show_window_id {
                    self.show_window();
                } else if id == enable_id {
                    let new_state = !state.is_enabled();
                    state.set_enabled(new_state);
                    menu_item_enable.set_checked(new_state);
                } else if id == edit_config_id {
                    if let Some(p) = &config_captured.config_file { std::thread::spawn({ let p = p.clone(); move || { let _ = opener::open(p); } }); }
                } else if id == edit_default_actions_id {
                    if let Some(ref d) = config_captured.confdir { std::thread::spawn({ let app = d.join("default.action"); move || { let _ = opener::open(app); } }); }
                } else if id == edit_user_actions_id {
                    if let Some(ref d) = config_captured.confdir { std::thread::spawn({ let app = d.join("user.action"); move || { let _ = opener::open(app); } }); }
                } else if id == edit_default_filters_id {
                    if let Some(ref d) = config_captured.confdir { std::thread::spawn({ let app = d.join("default.filter"); move || { let _ = opener::open(app); } }); }
                } else if id == edit_user_filters_id {
                    if let Some(ref d) = config_captured.confdir { std::thread::spawn({ let app = d.join("user.filter"); move || { let _ = opener::open(app); } }); }
                }
                #[cfg(feature = "trust")]
                if id == edit_trust_id {
                    if let Some(ref d) = config_captured.confdir { std::thread::spawn({ let app = d.join("trust"); move || { let _ = opener::open(app); } }); }
                }
            }
        }).map_err(|e| PrivoxyError::Other(e.to_string()))
    }'''

content = content[:start_idx] + new_run + content[end_idx:]
with open('src/tray_icon.rs', 'w', encoding='utf-8', newline='\r\n') as f:
    f.write(content)
