#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
use tracing::{Level, Metadata};

use crate::errlog;
use crate::error::PrivoxyResult;

/// Global flag to track if logging has been initialized
static LOGGING_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "tray-icon")]
use once_cell::sync::Lazy;
#[cfg(feature = "tray-icon")]
use parking_lot::RwLock;

/// Global registry for the GUI log cache to allow late-binding
#[cfg(feature = "tray-icon")]
static GUI_LOG_CACHE_REGISTRY: Lazy<RwLock<Option<LogCache>>> = Lazy::new(|| RwLock::new(None));

/// UI handle and log textarea handle for real-time updates
#[cfg(feature = "tray-icon")]
#[derive(Clone)]
pub struct UiLogHandles {
    pub ui: libui::UI,
    pub window: libui::controls::Window,
    pub log_textarea: libui::controls::MultilineEntry,
}

#[cfg(feature = "tray-icon")]
unsafe impl Send for UiLogHandles {}
#[cfg(feature = "tray-icon")]
unsafe impl Sync for UiLogHandles {}

#[cfg(feature = "tray-icon")]
pub type LogCache = Arc<std::sync::Mutex<(Vec<String>, Option<UiLogHandles>)>>;

#[cfg(not(feature = "tray-icon"))]
pub type LogCache = Arc<std::sync::Mutex<Vec<String>>>;

/// Custom layer that writes log messages to both stderr and GUI cache
pub struct GuiLogLayer {
    log_cache: Option<LogCache>,
    max_buffer_lines: usize,
}

impl GuiLogLayer {
    pub fn new(log_cache: Option<LogCache>, max_buffer_lines: usize) -> Self {
        Self {
            log_cache,
            max_buffer_lines,
        }
    }
}

impl<S> Layer<S> for GuiLogLayer
where
    S: tracing::Subscriber,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) -> bool {
        metadata.target().starts_with("privoxy") || metadata.target().starts_with("privoxy_rust")
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Use either the provided cache or the global registry
        #[cfg(feature = "tray-icon")]
        let log_cache_opt = self.log_cache.clone().or_else(|| GUI_LOG_CACHE_REGISTRY.read().clone());
        #[cfg(not(feature = "tray-icon"))]
        let log_cache_opt = self.log_cache.clone();

        if let Some(ref log_cache) = log_cache_opt {
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
            let message = format!("{} {}: {}", timestamp, level, visitor.string);
            
            if let Ok(mut cache) = log_cache.lock() {
                #[cfg(feature = "tray-icon")]
                {
                    // Get UI handles before releasing the lock
                    let ui_handles = cache.1.clone();
                    cache.0.push(message.clone());
                    
                    if cache.0.len() > self.max_buffer_lines {
                        let drain_count = cache.0.len() - self.max_buffer_lines;
                        cache.0.drain(0..drain_count);
                    }
                    
                    // Update UI if available
                    if let Some(mut ui_handles) = ui_handles {
                        let message_clone = message.clone();
                        ui_handles.ui.queue_main(move || {
                            ui_handles.log_textarea.append(&message_clone);
                            ui_handles.log_textarea.append("\n");
                        });
                    }
                }
                
                #[cfg(not(feature = "tray-icon"))]
                {
                    cache.push(message);
                    
                    if cache.len() > self.max_buffer_lines {
                        let drain_count = cache.len() - self.max_buffer_lines;
                        cache.drain(0..drain_count);
                    }
                }
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

use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.format("%Y-%m-%dT%H:%M:%S%.6f%:z"))
    }
}

pub fn init_logging(level: &str) -> PrivoxyResult<()> {
    // Check if logging has already been initialized
    if LOGGING_INITIALIZED.load(Ordering::Relaxed) {
        return Ok(());
    }
    
    errlog::init_log_module();
    
    // Enable ANSI support on Windows if needed
    crate::util::enable_ansi_support();
    
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_ansi(true)
        .with_level(true)
        .with_timer(LocalTimer)
        .with_filter(env_filter);

    tracing_subscriber::registry()
        .with(fmt_layer)
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
    
    // Enable ANSI support on Windows if needed
    crate::util::enable_ansi_support();
    
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_ansi(true)
        .with_level(true)
        .with_timer(LocalTimer)
        .with_filter(env_filter);

    let gui_layer = GuiLogLayer::new(log_cache, max_buffer_lines);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(gui_layer)
        .init();

    LOGGING_INITIALIZED.store(true, Ordering::Relaxed);

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
