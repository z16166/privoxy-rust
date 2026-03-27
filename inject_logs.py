import sys

with open('src/tray_icon.rs', 'r', encoding='utf-8') as f:
    lines = f.read()

import re

# Insert debug logic at event_loop.run
old_run = '''        let event_loop = EventLoop::<()>::new().map_err(|e| PrivoxyError::Other(e.to_string()))?;
        let state = self.state.clone();
        let shutdown_sender = self.shutdown_sender.clone();
        let config_captured = self.config.clone();
        let log_cache_captured = self.log_cache.clone();

        event_loop.run(move |event, _| {'''

new_run = '''        let event_loop = EventLoop::<()>::new().map_err(|e| PrivoxyError::Other(e.to_string()))?;
        let state = self.state.clone();
        let shutdown_sender = self.shutdown_sender.clone();
        let config_captured = self.config.clone();
        let log_cache_captured = self.log_cache.clone();

        // DEBUGLOG: Open debug file
        let mut dbg_log = std::fs::OpenOptions::new().create(true).append(true).open("debug_tray.log").unwrap_or_else(|_| std::fs::File::create("debug_tray.log").unwrap());
        use std::io::Write;
        writeln!(dbg_log, "[DEBUG] EventLoop created, setting up run closure").unwrap();
        dbg_log.flush().unwrap();

        event_loop.run(move |event, _| {
            let mut dbg_log = std::fs::OpenOptions::new().append(true).open("debug_tray.log").unwrap();
            writeln!(dbg_log, "[DEBUG] Event received: {:?}", event).unwrap();
            dbg_log.flush().unwrap();'''

lines = lines.replace(old_run, new_run)

old_init = '''            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 if self.tray_icon.is_none() {
                     if let Ok(i) = TrayIconBuilder::new().with_tooltip("Privoxy").with_menu(Box::new(menu.clone())).with_icon(icon.clone()).build() {
                         self.tray_icon = Some(i);
                     }
                 }
            }'''

new_init = '''            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 writeln!(dbg_log, "[DEBUG] NewEvents(Init) triggered.").unwrap();
                 dbg_log.flush().unwrap();
                 if self.tray_icon.is_none() {
                     writeln!(dbg_log, "[DEBUG] Building tray icon.").unwrap();
                     dbg_log.flush().unwrap();
                     match TrayIconBuilder::new().with_tooltip("Privoxy").with_menu(Box::new(menu.clone())).with_icon(icon.clone()).build() {
                         Ok(i) => {
                             writeln!(dbg_log, "[DEBUG] Tray icon built successfully.").unwrap();
                             dbg_log.flush().unwrap();
                             self.tray_icon = Some(i);
                         }
                         Err(e) => {
                             writeln!(dbg_log, "[DEBUG] Tray icon build FAILED: {:?}", e).unwrap();
                             dbg_log.flush().unwrap();
                         }
                     }
                 }
            }'''

lines = lines.replace(old_init, new_init)

with open('src/tray_icon.rs', 'w', encoding='utf-8') as f:
    f.write(lines)
