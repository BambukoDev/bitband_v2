use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use heapless::Vec;
use heapless::String;
use static_cell::StaticCell;
use crate::ui::menu_core::*;

// A shared scratch buffer to hold the label string for the UI frame.
// We use a BlockingMutex + RefCell because this is accessed in the UI loop.
static LABEL_BUFFER: BlockingMutex<CriticalSectionRawMutex, RefCell<String<MAX_NAME>>> = 
    BlockingMutex::new(RefCell::new(String::new()));

pub const MAX_FILES: usize = 32;
pub const MAX_NAME: usize = 32;

#[derive(Clone)]
pub struct FileEntry {
    pub name: String<MAX_NAME>,
}

pub struct FileMenu {
    pub title: &'static str,
    // Wrap the Vec in a Mutex for safe cross-task updates
    pub entries: Mutex<CriticalSectionRawMutex, Vec<FileEntry, MAX_FILES>>,
}

impl MenuSource for FileMenu {
    fn title(&self) -> &str { self.title }

    fn len(&self) -> usize {
        self.entries.try_lock().map(|e| e.len()).unwrap_or(0)
    }

    fn label(&self, index: usize) -> &str {
        let mut result = "";
        
        if let Ok(entries) = self.entries.try_lock() {
            if let Some(entry) = entries.get(index) {
                let name_str = entry.name.as_str();
                
                LABEL_BUFFER.lock(|buf| {
                    let mut b = buf.borrow_mut();
                    b.clear();
                    let _ = b.push_str(name_str);
                    
                    unsafe {
                        let ptr = b.as_str().as_ptr();
                        let len = b.len();
                        result = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
                    }
                });
            }
        }
        result
    }

    fn action(&self, index: usize) -> MenuAction {
        if let Ok(entries) = self.entries.try_lock() {
            if let Some(entry) = entries.get(index) {
                return MenuAction::Trigger(Action::RunDuck(entry.name.clone()));
            }
        }
        // Fallback if lock fails or index is gone (e.g. card pulled)
        MenuAction::Trigger(Action::Reboot)
    }
}

pub static FILE_BROWSER_CELL: StaticCell<FileMenu> = StaticCell::new();

pub fn get_file_browser() -> &'static FileMenu {
    FILE_BROWSER_CELL.init(FileMenu {
        title: "Payloads",
        entries: Mutex::new(Vec::new()),
    })
}
