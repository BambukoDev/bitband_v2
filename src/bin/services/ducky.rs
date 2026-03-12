use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::string::ToString;
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embedded_hal_bus::spi::RefCellDevice;
use embedded_sdmmc::filesystem::ToShortFileName;
use embedded_sdmmc::SdCard;
use esp_hal::gpio::Output;
use esp_hal::spi;
use esp_hal::Blocking;
use smart_leds::RGB;
use static_cell::StaticCell;
use crate::services::hid::*;
use crate::services::keyboard::*;
use crate::DummyTime;
use crate::ui::file_browser::MAX_NAME;
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use embedded_sdmmc::{VolumeManager, TimeSource};
use defmt::{error, info, warn};
use alloc::{boxed::Box, vec::Vec};

use core::cell::RefCell;
use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering;
use esp_hal::spi::master::Spi;

use crate::services::led::{LED_CMD_CH, LedState};
use crate::ui::top_bar::*;

#[derive(Clone)]
pub enum DuckCmd {
    Delay(u32),

    String(heapless::String<128>),
    StringLn(heapless::String<128>),

    Key { modifier: u8, keys: Vec<u8> },

    Hold { modifier: u8, keys: Vec<u8> },
    Release,

    Rem(heapless::String<64>),

    Pause,

    DefaultDelay(u32),
    RandomDelay(u32, u32),

    Repeat(u16),
}

pub static DUCKY_CH: Channel<
    CriticalSectionRawMutex,
    heapless::String<MAX_NAME>,
    4,
> = Channel::new();

pub static PAUSE_BUTTON_CH: Channel<
    CriticalSectionRawMutex,
    (),
    2
> = Channel::new();

pub static DIRECT_EXEC_CH: Channel<
    CriticalSectionRawMutex,
    String,
    2,
> = Channel::new();

// Global signal to track the state of the executor
// 1 -> Running
// 2 -> Paused
// other -> idle
pub static DUCKY_STATE: AtomicU8 = AtomicU8::new(0);

pub static READ_FILE_CH: Channel<CriticalSectionRawMutex, String, 1> = Channel::new();
pub static READ_FILE_CONTENTS: Channel<CriticalSectionRawMutex, String, 1> = Channel::new();

static mut LAST_CMD: Option<DuckCmd> = None;

const KEY_PRESS_MS: u64 = 15;
const KEY_RELEASE_MS: u64 = 8;

static CURRENT_REPORT: embassy_sync::blocking_mutex::Mutex<CriticalSectionRawMutex, core::cell::RefCell<KeyReport>> = 
    embassy_sync::blocking_mutex::Mutex::new(core::cell::RefCell::new(KeyReport {
        modifier: 0,
        keys: [0; 6],
    }));

async fn sync_hid() {
    let report = CURRENT_REPORT.lock(|cell| *cell.borrow());
    HID_CH.send(report).await;
}

fn key_from_name(name: &str) -> Option<u8> {
    match name {
        "ENTER" => Some(KEY_ENTER),
        "ESC" => Some(KEY_ESC),
        "BACKSPACE" => Some(KEY_BACKSPACE),
        "TAB" => Some(KEY_TAB),
        "SPACE" => Some(KEY_SPACE),
        "DELETE" => Some(KEY_DELETE),
        "UP" => Some(KEY_UP),
        "DOWN" => Some(KEY_DOWN),
        "LEFT" => Some(KEY_LEFT),
        "RIGHT" => Some(KEY_RIGHT),

        "F1" => Some(KEY_F1),
        "F2" => Some(KEY_F2),
        "F3" => Some(KEY_F3),
        "F4" => Some(KEY_F4),
        "F5" => Some(KEY_F5),
        "F6" => Some(KEY_F6),
        "F7" => Some(KEY_F7),
        "F8" => Some(KEY_F8),
        "F9" => Some(KEY_F9),
        "F10" => Some(KEY_F10),
        "F11" => Some(KEY_F11),
        "F12" => Some(KEY_F12),

        _ => None,
    }
}

fn parse_ducky_line(line: &str) -> Option<DuckCmd> {
    let line = line.trim();

    if line.is_empty() {
        return None;
    }

    if line.starts_with("REM ") {
        let mut msg: heapless::String<64> = heapless::String::new();
        let _ = msg.push_str(&line[4..]);
        return Some(DuckCmd::Rem(msg));
    }

    if line.starts_with("STRINGLN ") {
        let mut s: heapless::String<128> = heapless::String::new();
        let _ = s.push_str(&line[9..]);
        return Some(DuckCmd::StringLn(s));
    }

    if line.starts_with("STRING ") {
        let mut s: heapless::String<128> = heapless::String::new();
        let _ = s.push_str(&line[7..]);
        return Some(DuckCmd::String(s));
    }

    if line.starts_with("DELAY ") {
        let ms = line[6..].trim().parse().ok()?;
        return Some(DuckCmd::Delay(ms));
    }

    if line.starts_with("DEFAULT_DELAY ") {
        let ms = line[14..].trim().parse().ok()?;
        return Some(DuckCmd::DefaultDelay(ms));
    }

    if line == "PAUSE" {
        return Some(DuckCmd::Pause);
    }

    if line.starts_with("REPEAT ") {
        let n = line[7..].trim().parse().ok()?;
        return Some(DuckCmd::Repeat(n));
    }

    if line.starts_with("RANDOM_DELAY ") {
        let mut parts = line[13..].split_whitespace();
        let min = parts.next()?.parse().ok()?;
        let max = parts.next()?.parse().ok()?;

        return Some(DuckCmd::RandomDelay(min, max));
    }

    if line.starts_with("HOLD ") {
        let token = &line[5..];

        let (modifier, keys) = parse_key_combo(token)?;
        return Some(DuckCmd::Hold { modifier, keys });
    }

    if line == "RELEASE" {
        return Some(DuckCmd::Release);
    }

    if let Some((modifier, keys)) = parse_key_combo(line) {
        return Some(DuckCmd::Key { modifier, keys });
    }
    None
}

fn parse_key_combo(line: &str) -> Option<(u8, Vec<u8>)> {
    let mut modifier = 0;
    let mut keys = Vec::new();

    for token in line.split('+') {
        let t = token.trim();
        match t {
            "CTRL" | "CONTROL" => modifier |= MOD_CTRL,
            "SHIFT" => modifier |= MOD_SHIFT,
            "ALT" => modifier |= MOD_ALT,
            "GUI" | "WINDOWS" => modifier |= MOD_GUI,
            k => {
                if let Some(code) = key_from_name(k) {
                    keys.push(code);
                } else if k.len() == 1 {
                    let (m, kcode) = ascii_to_key(k.as_bytes()[0]);
                    modifier |= m;
                    keys.push(kcode);
                }
            },
        }
    }
    if keys.is_empty() && modifier == 0 { None } else { Some((modifier, keys)) }
}

async fn press_key(modifier: u8, key: u8) {
    HID_CH.send(KeyReport {
        modifier,
        keys: [key, 0, 0, 0, 0, 0],
    }).await;

    LED_CMD_CH.send(LedState::On(RGB {
        r: 0,
        g: 0,
        b: 10,
    })).await;

    Timer::after(Duration::from_millis(KEY_PRESS_MS)).await;

    HID_CH.send(KeyReport {
        modifier: 0,
        keys: [0; 6],
    }).await;

    LED_CMD_CH.send(LedState::Off).await;

    Timer::after(Duration::from_millis(KEY_RELEASE_MS)).await;
}


#[task]
pub async fn ducky_task(
    spi_bus: &'static RefCell<Spi<'static, Blocking>>, 
    cs_pin: &'static RefCell<Output<'static>>,
) {
    info!("[DUCKY] task started");
    let mut rng_state: u32 = 0x12345678;

    loop {
        // Wait for either a filename or a raw script string
        let script_to_run = embassy_futures::select::select3(
            DUCKY_CH.receive(),
            DIRECT_EXEC_CH.receive(),
            READ_FILE_CH.receive(),
        ).await;

        match script_to_run {
            // Received a filename from the Web File List
            embassy_futures::select::Either3::First(filename) => {
                info!("[DUCKY] Running file: {}", filename);
                if let Some(content) = read_file_to_string(filename.as_str(), spi_bus, cs_pin).await {
                    execute_full_script(&content, &mut rng_state).await;
                }
            }
            // Received raw text from the Live Editor
            embassy_futures::select::Either3::Second(raw_script) => {
                info!("[DUCKY] Running live script");
                execute_full_script(&raw_script, &mut rng_state).await;
            }
            embassy_futures::select::Either3::Third(filename) => {
                READ_FILE_CONTENTS.send(read_file_to_string(filename.as_str(), spi_bus, cs_pin).await.unwrap()).await;
            }
        }
        
        info!("[DUCKY] Execution finished");
        LED_CMD_CH.send(LedState::Off).await;
    }
}

pub async fn read_file_to_string(
    filename: &str,
    spi_bus: &'static RefCell<Spi<'static, Blocking>>, 
    cs_pin: &'static RefCell<Output<'static>>,
) -> Option<String> {
    // Borrow CS Pin with a small retry loop to avoid collisions with the SD monitor or Ducky task
    let mut cs_borrow = loop {
        if let Ok(b) = cs_pin.try_borrow_mut() {
            break b;
        }
        Timer::after(Duration::from_millis(10)).await;
    };

    let spi_device = RefCellDevice::new(spi_bus, &mut *cs_borrow, esp_hal::delay::Delay::new()).ok()?;
    let sdcard = SdCard::new(spi_device, esp_hal::delay::Delay::new());
    let mut volume_mgr = VolumeManager::new(sdcard, DummyTime);

    let mut volume = volume_mgr.open_volume(embedded_sdmmc::VolumeIdx(0)).ok()?;
    let root = volume.open_root_dir().ok()?;
    let ducky_dir = root.open_dir("DUCKY").ok()?;
    
    let mut file = ducky_dir.open_file_in_dir(
        filename,
        embedded_sdmmc::Mode::ReadOnly,
    ).ok()?;

    let mut content = String::new();
    let mut buf = [0u8; 256]; 
    while let Ok(n) = file.read(&mut buf) {
        if n == 0 { break; }
        if let Ok(s) = core::str::from_utf8(&buf[..n]) {
            content.push_str(s);
        }
    }
    Some(content)
}

async fn execute_full_script(content: &str, rng_state: &mut u32) {
    DUCKY_STATE.store(1, Ordering::Relaxed);
    let mut default_delay: u32 = 0;
    let mut last_cmd: Option<DuckCmd> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        if let Some(cmd) = parse_ducky_line(line) {
            match cmd.clone() {
                DuckCmd::Repeat(n) => {
                    if let Some(prev) = &last_cmd {
                        for _ in 0..n {
                            execute_ducky_cmd(prev.clone()).await;
                            if default_delay > 0 { 
                                Timer::after(Duration::from_millis(default_delay as u64)).await; 
                            }
                        }
                    }
                }

                DuckCmd::DefaultDelay(ms) => {
                    default_delay = ms;
                }

                DuckCmd::RandomDelay(min, max) => {
                    let delay = rand_range(min, max, rng_state);
                    Timer::after(Duration::from_millis(delay as u64)).await;
                }

                _ => {
                    execute_ducky_cmd(cmd.clone()).await;
                    if default_delay > 0 {
                        Timer::after(Duration::from_millis(default_delay as u64)).await;
                    }
                    last_cmd = Some(cmd);
                }
            }
        }
    }
    DUCKY_STATE.store(0, Ordering::Relaxed);
}

async fn execute_ducky_cmd(cmd: DuckCmd) {
    match cmd {
        DuckCmd::Delay(ms) => {
            Timer::after(Duration::from_millis(ms as u64)).await;
        }

        DuckCmd::String(s) => {
            for b in s.bytes() {
                let (modifier, key) = ascii_to_key(b);
                if key != 0 {
                    press_key(modifier, key).await;
                }
            }
        }

        DuckCmd::StringLn(s) => {
            for b in s.bytes() {
                let (modifier, key) = ascii_to_key(b);
                if key != 0 {
                    press_key(modifier, key).await;
                }
            }

            press_key(0, KEY_ENTER).await;
        }

        DuckCmd::Rem(text) => {
            TOP_BAR_CH.send(TopBarMode::Message { text }).await;
        }

        DuckCmd::Hold { modifier, keys } => {
            CURRENT_REPORT.lock(|cell| {
                let mut report = cell.borrow_mut();
                report.modifier |= modifier;
                for k in keys {
                    if !report.keys.contains(&k) {
                        if let Some(slot) = report.keys.iter_mut().find(|s| **s == 0) {
                            *slot = k;
                        }
                    }
                }
            });
            sync_hid().await;
        }

        DuckCmd::Release => {
            CURRENT_REPORT.lock(|cell| {
                *cell.borrow_mut() = KeyReport { modifier: 0, keys: [0; 6] };
            });
            sync_hid().await;
        }

        DuckCmd::Key { modifier, keys } => {
            // Merge with currently held keys
            let mut report = CURRENT_REPORT.lock(|cell| *cell.borrow());
            report.modifier |= modifier;
            for (i, k) in keys.iter().enumerate().take(6) {
                report.keys[i] = *k; 
            }
            
            HID_CH.send(report).await;
            Timer::after(Duration::from_millis(KEY_PRESS_MS)).await;
            
            // Return to the "Hold" state
            sync_hid().await;
            Timer::after(Duration::from_millis(KEY_RELEASE_MS)).await;
        }

        DuckCmd::Pause => {
            DUCKY_STATE.store(2, Ordering::Relaxed);
            let mut text: heapless::String<64> = heapless::String::new();
            let _ = text.push_str("PAUSED");
            TOP_BAR_CH.send(TopBarMode::Message { text }).await;
            PAUSE_BUTTON_CH.receive().await;
            DUCKY_STATE.store(1, Ordering::Relaxed);
        }

        DuckCmd::DefaultDelay(_) => {}

        DuckCmd::Repeat(_) => {}

        DuckCmd::Key { modifier, keys } => {
            // Merge with currently held keys
            let mut report = CURRENT_REPORT.lock(|cell| *cell.borrow());
            report.modifier |= modifier;
            for (i, k) in keys.iter().enumerate().take(6) {
                report.keys[i] = *k; 
            }
            
            HID_CH.send(report).await;
            Timer::after(Duration::from_millis(KEY_PRESS_MS)).await;
            
            // Return to the "Hold" state
            sync_hid().await;
            Timer::after(Duration::from_millis(KEY_RELEASE_MS)).await;
        }

        DuckCmd::RandomDelay(min, max) => {
            // handled by executor
        }
    }
}

fn rand_range(min: u32, max: u32, state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    min + (*state % (max - min + 1))
}
