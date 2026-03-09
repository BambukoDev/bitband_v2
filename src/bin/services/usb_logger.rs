use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;

// A global pipe for log data. 1024 bytes is usually enough for most log lines.
pub static LOGGER_PIPE: Pipe<CriticalSectionRawMutex, 1024> = Pipe::new();

#[defmt::global_logger]
struct Logger;

unsafe impl defmt::Logger for Logger {
    fn acquire() {}
    unsafe fn release() {}
    unsafe fn write(bytes: &[u8]) {
        // Push logs into the pipe. If full, it drops the oldest data (non-blocking).
        let _ = LOGGER_PIPE.try_write(bytes);
    }
    unsafe fn flush() {}
}
