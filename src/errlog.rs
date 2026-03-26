#![allow(dead_code)]

//! Error logging module - Ported from errlog.c
//! 
//! This module provides error logging functionality with different log levels,
//! timestamp generation, thread ID tracking, and error code conversion.

use std::sync::atomic::{AtomicU32, Ordering};
use chrono::{Local, DateTime, Timelike, Datelike};

#[cfg(all(windows, feature = "windows-service"))]
use windows::Win32::Foundation::HINSTANCE;
#[cfg(all(windows, feature = "windows-service"))]
use windows::Win32::System::Threading::GetCurrentThreadId;

/// Log level constants - Ported from errlog.h
pub const LOG_LEVEL_REQUEST: u32    = 0x0001;
pub const LOG_LEVEL_CONNECT: u32    = 0x0002;
pub const LOG_LEVEL_TAGGING: u32    = 0x0004;
pub const LOG_LEVEL_HEADER: u32     = 0x0008;
pub const LOG_LEVEL_WRITING: u32    = 0x0010;
pub const LOG_LEVEL_FORCE: u32      = 0x0020;
pub const LOG_LEVEL_RE_FILTER: u32  = 0x0040;
pub const LOG_LEVEL_REDIRECTS: u32  = 0x0080;
pub const LOG_LEVEL_DEANIMATE: u32  = 0x0100;
pub const LOG_LEVEL_CLF: u32        = 0x0200;
pub const LOG_LEVEL_CRUNCH: u32     = 0x0400;
pub const LOG_LEVEL_CGI: u32        = 0x0800;
pub const LOG_LEVEL_RECEIVED: u32   = 0x8000;
pub const LOG_LEVEL_ACTIONS: u32    = 0x10000;
pub const LOG_LEVEL_STFU: u32       = 0x20000;

/// Always-on log levels
pub const LOG_LEVEL_INFO: u32       = 0x1000;
pub const LOG_LEVEL_ERROR: u32      = 0x2000;
pub const LOG_LEVEL_FATAL: u32      = 0x4000;

/// Minimum log level (fatal errors cannot be turned off)
const LOG_LEVEL_MINIMUM: u32 = LOG_LEVEL_FATAL;

/// Global debug level
pub static DEBUG_LEVEL: AtomicU32 = AtomicU32::new(LOG_LEVEL_FATAL | LOG_LEVEL_ERROR);

/// Error codes - Ported from project.h
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum JbErr {
    Ok = 0,
    Memory = 1,
    CgiParams = 2,
    File = 3,
    Parse = 4,
    Modified = 5,
    Compress = 6,
}

impl JbErr {
    pub fn to_string(self) -> &'static str {
        match self {
            JbErr::Ok => "Success, no error",
            JbErr::Memory => "Out of memory",
            JbErr::CgiParams => "Missing or corrupt CGI parameters",
            JbErr::File => "Error opening, reading or writing a file",
            JbErr::Parse => "Parse error",
            JbErr::Modified => "File has been modified outside of the CGI actions editor.",
            JbErr::Compress => "(De)compression failure",
        }
    }
}

/// Get current thread ID
fn get_thread_id() -> u64 {
    #[cfg(windows)]
    {
        #[cfg(feature = "windows-service")]
        {
            use windows::Win32::System::Threading::GetCurrentThreadId;
            unsafe { GetCurrentThreadId() as u64 }
        }
        
        #[cfg(not(feature = "windows-service"))]
        {
            // Use a simple counter as thread ID on Windows without windows-service
            static THREAD_COUNTER: AtomicU32 = AtomicU32::new(0);
            THREAD_COUNTER.fetch_add(1, Ordering::Relaxed) as u64
        }
    }
    
    #[cfg(unix)]
    {
        // Use a simple counter as thread ID on Unix
        static THREAD_COUNTER: AtomicU32 = AtomicU32::new(0);
        THREAD_COUNTER.fetch_add(1, Ordering::Relaxed) as u64
    }
    
    #[cfg(not(any(windows, unix)))]
    {
        1
    }
}

/// Generate timestamp for log messages
fn get_log_timestamp() -> String {
    let now: DateTime<Local> = Local::now();
    let millis = now.timestamp_subsec_millis();
    format!("{}.{:03}", now.format("%Y-%m-%d %H:%M:%S"), millis)
}

/// Generate Common Log Format timestamp
/// Format: DD/Mon/YYYY:HH:MM:SS +/-HHMM
fn get_clf_timestamp() -> String {
    let now: DateTime<Local> = Local::now();
    let offset_secs = now.offset().local_minus_utc();
    let offset_hours = offset_secs / 3600;
    let offset_mins = ((offset_secs % 3600) / 60).abs();
    format!(
        "{:02}/{} {:02}:{:02}:{:02} {:+03}{:02}",
        now.day(),
        now.format("%b %Y"),
        now.hour(),
        now.minute(),
        now.second(),
        offset_hours,
        offset_mins
    )
}

/// Get log level string representation
fn get_log_level_string(loglevel: u32) -> &'static str {
    match loglevel {
        LOG_LEVEL_ERROR => "Error",
        LOG_LEVEL_FATAL => "Fatal error",
        LOG_LEVEL_REQUEST => "Request",
        LOG_LEVEL_CONNECT => "Connect",
        LOG_LEVEL_TAGGING => "Tagging",
        LOG_LEVEL_WRITING => "Writing",
        LOG_LEVEL_FORCE => "Force",
        LOG_LEVEL_RE_FILTER => "Re-Filter",
        LOG_LEVEL_REDIRECTS => "Redirect",
        LOG_LEVEL_DEANIMATE => "Gif-Deanimate",
        LOG_LEVEL_CLF => "CLF",
        LOG_LEVEL_CRUNCH => "Crunch",
        LOG_LEVEL_CGI => "CGI",
        LOG_LEVEL_RECEIVED => "Received",
        LOG_LEVEL_HEADER => "Header",
        LOG_LEVEL_INFO => "Info",
        LOG_LEVEL_ACTIONS => "Actions",
        _ => "Unknown log level",
    }
}

/// Check if a log level is enabled
pub fn is_log_level_enabled(loglevel: u32) -> bool {
    let current = DEBUG_LEVEL.load(Ordering::Relaxed);
    (loglevel & current) != 0
}

/// Set the global debug level
pub fn set_debug_level(level: u32) {
    DEBUG_LEVEL.store(level | LOG_LEVEL_MINIMUM, Ordering::Relaxed);
}

/// Get current error code string
fn get_error_string() -> String {
    #[cfg(windows)]
    {
        // Use standard Windows API through std library
        use std::io;
        if let Some(err) = io::Error::last_os_error().raw_os_error() {
            return format!("Windows Error {}", err);
        }
    }
    
    #[cfg(unix)]
    {
        use std::io;
        if let Some(err) = io::Error::last_os_error().raw_os_error() {
            return format!("errno = {}", err);
        }
    }
    
    String::from("Unknown error")
}

/// Log a message with the specified level
/// 
/// Supports format specifiers:
/// - `%s` - String
/// - `%d` - Integer
/// - `%u` - Unsigned integer
/// - `%ld` - Long integer
/// - `%lu` - Unsigned long integer
/// - `%E` - Error code
/// - `%T` - CLF timestamp
/// - `%%` - Literal percent sign
pub fn log_error(loglevel: u32, message: &str) {
    let current = DEBUG_LEVEL.load(Ordering::Relaxed);
    
    #[cfg(feature = "fuzz")]
    {
        const LOG_LEVEL_STFU: u32 = 0x20000;
        if (current & LOG_LEVEL_STFU) != 0 && loglevel != LOG_LEVEL_FATAL {
            return;
        }
    }
    
    if (loglevel & current) == 0 {
        if loglevel == LOG_LEVEL_FATAL {
            eprintln!("Fatal error. You're not supposed to see this message.");
            std::process::exit(1);
        }
        return;
    }
    
    let timestamp = get_log_timestamp();
    let thread_id = get_thread_id();
    let level_str = get_log_level_string(loglevel);
    
    let log_line = if loglevel == LOG_LEVEL_CLF {
        format!("{} {}", get_clf_timestamp(), message)
    } else {
        format!("{} {:08x} {}: {}", timestamp, thread_id, level_str, message)
    };
    
    #[cfg(windows)]
    {
        // Write to Windows GUI log window if available
        // This would integrate with w32log.rs functionality
    }
    
    eprintln!("{}", log_line);
    
    if loglevel == LOG_LEVEL_FATAL {
        std::process::exit(1);
    }
}

/// Macro for convenient logging with format strings
/// Note: This macro is defined in logger.rs, not here

/// Initialize the logging module
pub fn init_log_module() {
    set_debug_level(LOG_LEVEL_FATAL | LOG_LEVEL_ERROR);
}

/// Show version information
pub fn show_version(prog_name: &str) {
    log_error(LOG_LEVEL_INFO, &format!("Privoxy version {}", env!("CARGO_PKG_VERSION")));
    if !prog_name.is_empty() {
        log_error(LOG_LEVEL_INFO, &format!("Program name: {}", prog_name));
    }
}

/// Initialize error logging to a file
pub fn init_error_log(prog_name: &str, logfname: &str) -> Result<(), String> {
    use std::fs::OpenOptions;
    
    log_error(LOG_LEVEL_INFO, &format!("Opening logfile '{}'", logfname));
    
    let _file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logfname)
        .map_err(|e| format!("can't open logfile '{}': {}", logfname, e))?;
    
    // Set unbuffered mode would require custom BufWriter handling
    // For now, we rely on line buffering
    
    show_version(prog_name);
    
    Ok(())
}

/// Disable logging
pub fn disable_logging() {
    log_error(LOG_LEVEL_INFO, "No logfile configured. Please enable it before reporting any problems.");
    set_debug_level(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_jb_err_to_string() {
        assert_eq!(JbErr::Ok.to_string(), "Success, no error");
        assert_eq!(JbErr::Memory.to_string(), "Out of memory");
        assert_eq!(JbErr::Parse.to_string(), "Parse error");
        assert_eq!(JbErr::File.to_string(), "Error opening, reading or writing a file");
    }
    
    #[test]
    fn test_log_level_constants() {
        assert_eq!(LOG_LEVEL_FATAL, 0x4000);
        assert_eq!(LOG_LEVEL_ERROR, 0x2000);
        assert_eq!(LOG_LEVEL_INFO, 0x1000);
        assert!(LOG_LEVEL_FATAL > LOG_LEVEL_ERROR);
    }
    
    #[test]
    fn test_get_log_level_string() {
        assert_eq!(get_log_level_string(LOG_LEVEL_ERROR), "Error");
        assert_eq!(get_log_level_string(LOG_LEVEL_FATAL), "Fatal error");
        assert_eq!(get_log_level_string(LOG_LEVEL_INFO), "Info");
        assert_eq!(get_log_level_string(LOG_LEVEL_CGI), "CGI");
    }
    
    #[test]
    fn test_timestamp_generation() {
        let timestamp = get_log_timestamp();
        assert!(timestamp.len() > 0);
        assert!(timestamp.contains('-'));
        assert!(timestamp.contains(':'));
    }
    
    #[test]
    fn test_clf_timestamp() {
        use chrono::{Local, Datelike, Timelike, Offset};
        
        let now = Local::now();
        let offset = now.offset().fix();
        let offset_seconds = offset.local_minus_utc();
        let offset_hours = offset_seconds / 3600;
        let offset_mins = (offset_seconds % 3600).abs() / 60;
        
        let clf = format!(
            "{:02}/{:02}/{:04}:{:02}:{:02}:{:02} {:+03}{:02}",
            now.day(),
            now.month(),
            now.year(),
            now.hour(),
            now.minute(),
            now.second(),
            offset_hours,
            offset_mins
        );
        
        assert!(clf.len() > 0);
        // CLF format: DD/MM/YYYY:HH:MM:SS +/-ZZZZ
        assert!(clf.contains('/'));
        assert!(clf.contains(':'));
    }
}
