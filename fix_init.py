import sys

with open('src/tray_icon.rs', 'r', encoding='utf-8') as f:
    lines = f.read()

old_builder = '''        #[cfg(not(target_os = "macos"))]
        {
            let t_icon = TrayIconBuilder::new()
                .with_tooltip("Privoxy")
                .with_icon(icon.clone())
                .build()
                .map_err(|e| PrivoxyError::Other(e.to_string()))?;
            
            t_icon.set_menu(Some(Box::new(menu.clone())));
            self.tray_icon = Some(t_icon);
        }'''

# Remove the outside block completely
lines = lines.replace(old_builder, '')

old_init = '''        event_loop.run(move |event, _| {
            #[cfg(target_os = "macos")]
            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 if self.tray_icon.is_none() {
                     if let Ok(i) = TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_icon(icon.clone()).build() {
                         self.tray_icon = Some(i);
                     }
                 }
            }'''

new_init = '''        event_loop.run(move |event, _| {
            if matches!(event, winit::event::Event::NewEvents(winit::event::StartCause::Init)) {
                 if self.tray_icon.is_none() {
                     if let Ok(i) = TrayIconBuilder::new().with_tooltip("Privoxy").with_menu(Box::new(menu.clone())).with_icon(icon.clone()).build() {
                         self.tray_icon = Some(i);
                     }
                 }
            }'''

lines = lines.replace(old_init, new_init)

with open('src/tray_icon.rs', 'w', encoding='utf-8') as f:
    f.write(lines)
