#![allow(dead_code)]
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use tracing::{Level, Metadata, Event, subscriber::Interest};

use crate::errlog;
use crate::error::PrivoxyResult;

/// Global flag to track if logging has been initialized
static LOGGING_INITIALIZED: AtomicBool = AtomicBool::new(false);

use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Global registry for the GUI log cache to allow late-binding
#[cfg(feature = "tray-icon")]
static GUI_LOG_CACHE_REGISTRY: Lazy<RwLock<Option<LogCache>>> = Lazy::new(|| RwLock::new(None));

/// Global registry for the log file handle to allow late-binding
pub struct LogState {
    pub file: File,
    pub lines_since_flush: usize,
    pub last_flush: std::time::Instant,
}

static GUI_LOG_FILE_REGISTRY: Lazy<RwLock<Option<Arc<parking_lot::Mutex<LogState>>>>> = Lazy::new(|| RwLock::new(None));

/// UI handle and log textarea handle for real-time updates
#[cfg(feature = "tray-icon")]
#[derive(Clone)]
pub struct UiLogHandles {
    pub event_queue: Arc<libui::EventQueue>,
    pub window: Option<libui::controls::Window>,
    pub log_textarea: Option<libui::controls::MultilineEntry>,
}

#[cfg(feature = "tray-icon")]
unsafe impl Send for UiLogHandles {}
#[cfg(feature = "tray-icon")]
unsafe impl Sync for UiLogHandles {}

#[cfg(feature = "tray-icon")]
pub type LogCache = Arc<std::sync::Mutex<(Vec<String>, Option<UiLogHandles>)>>;

#[cfg(not(feature = "tray-icon"))]
pub type LogCache = Arc<std::sync::Mutex<Vec<String>>>;

/// Custom layer that writes log messages to both file and GUI cache (unified logging)
pub struct GuiLogLayer {
    log_cache: Option<LogCache>,
    log_file: Option<Arc<parking_lot::Mutex<LogState>>>,
    max_buffer_lines: usize,
}

impl GuiLogLayer {
    pub fn new(log_cache: Option<LogCache>, max_buffer_lines: usize) -> Self {
        Self {
            log_cache,
            log_file: None,
            max_buffer_lines,
        }
    }
}

/// Global flag to track if a UI update is already pending to prevent flooding
#[cfg(feature = "tray-icon")]
static UI_UPDATE_PENDING: AtomicBool = AtomicBool::new(false);

/// Global flag to track if the log window is currently visible
#[cfg(feature = "tray-icon")]
pub static LOG_WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);

impl<S> Layer<S> for GuiLogLayer
where
    S: tracing::Subscriber,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) -> bool {
        metadata.target().starts_with("privoxy") || metadata.target().starts_with("privoxy_rust")
    }

    fn register_callsite(&self, _metadata: &Metadata<'_>) -> Interest {
        Interest::always()
    }

    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Use either the provided cache or the global registry
        #[cfg(feature = "tray-icon")]
        let log_cache_opt = self.log_cache.clone().or_else(|| GUI_LOG_CACHE_REGISTRY.read().clone());
        #[cfg(not(feature = "tray-icon"))]
        let log_cache_opt = self.log_cache.clone();

        // Use global file registry if not provided (usually not provided in constructor as it's late-bound)
        let log_file_opt = self.log_file.clone().or_else(|| GUI_LOG_FILE_REGISTRY.read().clone());

        // Extract message
        let mut visitor = StringVisitor::new();
        event.record(&mut visitor);
        
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        
        let now = chrono::Local::now();
        let timestamp = now.format("%Y-%m-%dT%H:%M:%S%.3f%:z");
        let formatted_message = format!("{} {}: {}", timestamp, level, visitor.string);

        // 1. Write to memory cache (and UI if handles are available)
        if let Some(ref log_cache) = log_cache_opt {
            if let Ok(mut cache) = log_cache.lock() {
                #[cfg(feature = "tray-icon")]
                {
                    cache.0.push(formatted_message.clone());
                    
                    if cache.0.len() > self.max_buffer_lines {
                        let drain_count = cache.0.len() - self.max_buffer_lines;
                        cache.0.drain(0..drain_count);
                    }
                    
                    // Real-time UI update if handles are matched and no update is pending
                    if let Some(ui_handles) = cache.1.as_ref() {
                        // Only proceed if window is visible and no update is pending
                        // (Use manual flag since libui doesn't have a reliable visible() method)
                        if LOG_WINDOW_VISIBLE.load(Ordering::SeqCst) && !UI_UPDATE_PENDING.load(Ordering::SeqCst) {
                            UI_UPDATE_PENDING.store(true, Ordering::SeqCst);
                            
                            let event_queue = ui_handles.event_queue.clone();
                            let textarea_ptr = ui_handles.log_textarea.as_ref().map(|t| t.ptr() as usize);
                            let log_cache_ui = log_cache.clone();
                            
                            event_queue.queue_main(move || {
                                if let Some(ptr) = textarea_ptr {
                                    if let Ok(mut cache) = log_cache_ui.lock() {
                                        // Update the textarea with ALL messages currently in the cache
                                        // (In a more optimized version, we'd only append NEW messages, 
                                        // but for simplicity we'll just append what's there and then clear the pending flag)
                                        unsafe {
                                            let mut textarea = libui::controls::MultilineEntry::from_raw(ptr as *mut _);
                                            // To keep it simple and responsive, we just append the latest message
                                            // the logic here ensures we don't call this 1000 times/sec
                                            if let Some(last) = cache.0.last() {
                                                textarea.append(last);
                                                textarea.append("\n");
                                            }
                                        }
                                    }
                                }
                                UI_UPDATE_PENDING.store(false, Ordering::SeqCst);
                            });
                        }
                    }
                }
                #[cfg(not(feature = "tray-icon"))]
                {
                    cache.push(formatted_message.clone());
                    if cache.len() > self.max_buffer_lines {
                        let drain_count = cache.len() - self.max_buffer_lines;
                        cache.drain(0..drain_count);
                    }
                }
            }
        }

        // 2. Write to file if configured
        if let Some(ref state_arc) = log_file_opt {
            let mut state = state_arc.lock();
            let _ = writeln!(state.file, "{}", formatted_message);
            
            // Increment line count and check if we should flush
            state.lines_since_flush += 1;
            if state.lines_since_flush >= 20 || state.last_flush.elapsed().as_secs() >= 5 {
                let _ = state.file.flush();
                state.lines_since_flush = 0;
                state.last_flush = std::time::Instant::now();
            }
        }
    }
}

struct StringVisitor {
    string: String,
}

impl StringVisitor {
    fn new() -> Self {
        Self {
            string: String::new(),
        }
    }
}

impl<'a> tracing::field::Visit for StringVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.string = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.string = format!("{:?}", value);
        }
    }
}



pub fn init_logging(level: &str) -> PrivoxyResult<()> {
    // Check if logging has already been initialized
    if LOGGING_INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }
    
    errlog::init_log_module();
    
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let gui_layer = GuiLogLayer::new(None, 1000);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(gui_layer)
        .init();

    LOGGING_INITIALIZED.store(true, Ordering::Relaxed);

    Ok(())
}

/// Register a GUI log cache with the global registry for late-binding
#[cfg(feature = "tray-icon")]
pub fn register_gui_cache(log_cache: LogCache) {
    *GUI_LOG_CACHE_REGISTRY.write() = Some(log_cache);
}

pub fn init_logging_with_gui(level: &str, log_cache: Option<LogCache>, max_buffer_lines: usize) -> PrivoxyResult<()> {
    // Check if logging has already been initialized
    if LOGGING_INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }
    
    errlog::init_log_module();
    
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let gui_layer = GuiLogLayer::new(log_cache, max_buffer_lines);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(gui_layer)
        .init();

    LOGGING_INITIALIZED.store(true, Ordering::Relaxed);

    Ok(())
}

/// Update the log file handle globally for late-binding
pub fn update_log_file(path: &Path) -> PrivoxyResult<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| crate::error::PrivoxyError::Other(format!("Failed to open log file {:?}: {}", path, e)))?;
    
    let state = LogState {
        file,
        lines_since_flush: 0,
        last_flush: std::time::Instant::now(),
    };
    
    *GUI_LOG_FILE_REGISTRY.write() = Some(Arc::new(parking_lot::Mutex::new(state)));
    Ok(())
}

pub fn set_debug_level(level: u32) {
    errlog::set_debug_level(level);
}

pub fn get_debug_level() -> u32 {
    errlog::DEBUG_LEVEL.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn debug_level_is_enabled(level: u32) -> bool {
    errlog::is_log_level_enabled(level)
}

#[macro_export]
macro_rules! log_error {
    ($level:expr, $($arg:tt)*) => {
        if $crate::logger::debug_level_is_enabled($level) {
            match $level {
                $crate::errlog::LOG_LEVEL_FATAL => {
                    tracing::error!($($arg)*);
                    std::process::exit(1);
                }
                $crate::errlog::LOG_LEVEL_ERROR => tracing::error!($($arg)*),
                _ => tracing::warn!($($arg)*),
            }
        }
    };
}

pub fn show_version() {
    use crate::constants::{VERSION, CODE_STATUS, HOME_PAGE_URL};
    println!("Privoxy version {} ({})", VERSION, CODE_STATUS);
    println!("Home page: {}", HOME_PAGE_URL);
}

/// Generate Common Log Format timestamp
/// Format: [dd/Mon/yyyy:HH:MM:SS +/-HHMM]
pub fn get_clf_timestamp() -> String {
    use chrono::{DateTime, Local, Offset, Datelike, Timelike};
    
    let now: DateTime<Local> = Local::now();
    let offset = now.offset().fix();
    let offset_seconds = offset.local_minus_utc();
    let offset_hours = offset_seconds / 3600;
    let offset_mins = (offset_seconds % 3600).abs() / 60;
    
    format!("[{}/{:02}/{:04}:{:02}:{:02}:{:02} {:+03}{:02}]",
        now.day(),
        now.month(),
        now.year(),
        now.hour(),
        now.minute(),
        now.second(),
        offset_hours,
        offset_mins
    )
}

/// Get log timestamp with milliseconds
/// Format: YYYY-MM-DD HH:MM:SS.mmm
pub fn get_log_timestamp() -> String {
    use chrono::Local;
    
    let now = Local::now();
    format!("{}.{:03}", now.format("%Y-%m-%d %H:%M:%S"), now.timestamp_subsec_millis())
}
