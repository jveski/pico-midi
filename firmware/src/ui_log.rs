//! UI log queue.
//!
//! Buffers human-readable log messages produced anywhere in the firmware so
//! the serial task can stream them to the host configurator over the same
//! CDC-ACM transport used for config + monitor traffic.
//!
//! The queue is fixed-capacity. When full, new pushes drop the oldest entry
//! so producers never block and recent context wins. Access is serialised
//! with a critical-section-backed `Mutex` on-target.
//!
//! Use the [`log_info!`], [`log_warn!`], and [`log_error!`] macros, which
//! also forward the message to `defmt-rtt` for probe-based debugging.

use heapless::{Deque, String};
use serde::Serialize;

/// Maximum bytes of formatted log message text per entry. Anything longer is
/// truncated.
pub const MAX_LOG_LEN: usize = 128;

/// Maximum number of buffered log entries before the oldest is dropped.
pub const QUEUE_CAPACITY: usize = 32;

/// Severity level for a log entry. Wire format is a single `u8` (variant
/// discriminant), so the numeric values are part of the public protocol.
#[derive(Copy, Clone, Serialize)]
#[repr(u8)]
pub enum Level {
    Info = 0,
    Warn = 1,
    Error = 2,
}

/// A single log entry as it travels over the wire.
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub level: u8,
    pub msg: String<MAX_LOG_LEN>,
}

/// Inner queue logic, decoupled from the on-target Mutex so it can be unit
/// tested on the host.
pub(crate) struct LogQueue {
    inner: Deque<LogEntry, QUEUE_CAPACITY>,
}

impl LogQueue {
    pub const fn new() -> Self {
        Self {
            inner: Deque::new(),
        }
    }

    pub fn push(&mut self, level: Level, msg: &str) {
        let mut entry = LogEntry {
            level: level as u8,
            msg: String::new(),
        };
        let _ = entry
            .msg
            .push_str(truncate_to_char_boundary(msg, MAX_LOG_LEN));
        if self.inner.is_full() {
            let _ = self.inner.pop_front();
        }
        // Capacity is guaranteed above; ignore the (impossible) error.
        let _ = self.inner.push_back(entry);
    }

    pub fn pop(&mut self) -> Option<LogEntry> {
        self.inner.pop_front()
    }
}

fn truncate_to_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------------------
// On-target static queue (embedded only). Tests run on the host and exercise
// `LogQueue` directly without needing critical-section support.
// ---------------------------------------------------------------------------

#[cfg(target_os = "none")]
mod runtime {
    use super::{Level, LogEntry, LogQueue};
    use core::cell::RefCell;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embassy_sync::blocking_mutex::Mutex;

    static QUEUE: Mutex<CriticalSectionRawMutex, RefCell<LogQueue>> =
        Mutex::new(RefCell::new(LogQueue::new()));

    pub fn push(level: Level, msg: &str) {
        QUEUE.lock(|cell| cell.borrow_mut().push(level, msg));
    }

    pub fn pop() -> Option<LogEntry> {
        QUEUE.lock(|cell| cell.borrow_mut().pop())
    }
}

#[cfg(target_os = "none")]
pub use runtime::{pop, push};

/// Push a `core::fmt::Arguments`-formatted entry. Used by the macros.
#[cfg(target_os = "none")]
pub fn push_fmt(level: Level, args: core::fmt::Arguments<'_>) {
    use core::fmt::Write as _;
    let mut buf: String<MAX_LOG_LEN> = String::new();
    // write! into a heapless::String returns Err once capacity is reached
    // but partial output is preserved, which is exactly what we want.
    let _ = buf.write_fmt(args);
    push(level, &buf);
}

/// Log an info-level message to the UI log queue and to defmt-rtt.
#[cfg(target_os = "none")]
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        ::defmt::info!($($arg)*);
        $crate::ui_log::push_fmt(
            $crate::ui_log::Level::Info,
            ::core::format_args!($($arg)*),
        );
    }};
}

/// Log a warn-level message to the UI log queue and to defmt-rtt.
#[cfg(target_os = "none")]
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        ::defmt::warn!($($arg)*);
        $crate::ui_log::push_fmt(
            $crate::ui_log::Level::Warn,
            ::core::format_args!($($arg)*),
        );
    }};
}

/// Log an error-level message to the UI log queue and to defmt-rtt.
#[cfg(target_os = "none")]
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        ::defmt::error!($($arg)*);
        $crate::ui_log::push_fmt(
            $crate::ui_log::Level::Error,
            ::core::format_args!($($arg)*),
        );
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_roundtrip() {
        let mut q = LogQueue::new();
        q.push(Level::Info, "hello");
        let e = q.pop().unwrap();
        assert_eq!(e.level, 0);
        assert_eq!(e.msg.as_str(), "hello");
        assert!(q.pop().is_none());
    }

    #[test]
    fn drops_oldest_on_overflow() {
        let mut q = LogQueue::new();
        for i in 0..(QUEUE_CAPACITY + 5) {
            let mut s: String<8> = String::new();
            let _ = core::fmt::write(&mut s, format_args!("{}", i));
            q.push(Level::Info, &s);
        }
        // First 5 entries (0..4) should have been dropped.
        let first = q.pop().unwrap();
        assert_eq!(first.msg.as_str(), "5");
    }

    #[test]
    fn truncates_long_message() {
        let mut q = LogQueue::new();
        let big = "x".repeat(MAX_LOG_LEN + 50);
        q.push(Level::Warn, &big);
        let e = q.pop().unwrap();
        assert_eq!(e.msg.len(), MAX_LOG_LEN);
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // 'é' is 2 bytes in UTF-8; place it straddling the limit.
        let s = "a".repeat(MAX_LOG_LEN - 1) + "é";
        let mut q = LogQueue::new();
        q.push(Level::Info, &s);
        let e = q.pop().unwrap();
        // Multibyte char dropped rather than sliced through.
        assert_eq!(e.msg.len(), MAX_LOG_LEN - 1);
    }
}
