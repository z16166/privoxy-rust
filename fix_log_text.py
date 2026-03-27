import sys
content = open('src/tray_icon.rs', 'r', encoding='utf-8').read()

content = content.replace('MultilineEntry::new_non_wrapping()', 'MultilineEntry::new()')
content = content.replace('log_textarea.set_value(&full_log);', 'log_textarea.append(&full_log);')

# In the case where `clear_log_item.0.on_clicked` is used, the clear logic was reset to `set_value("")`.
# That might be OK because clearing the log shouldn't ruin it if the next line is append. 
# But let's just make sure.

open('src/tray_icon.rs', 'w', encoding='utf-8', newline='\r\n').write(content)
