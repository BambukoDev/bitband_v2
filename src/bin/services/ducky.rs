use alloc::string::String;
use alloc::string::ToString;
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
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
use defmt::{error, info};
use alloc::{boxed::Box, vec::Vec};

use core::cell::RefCell;
use esp_hal::spi::master::Spi;

use crate::services::led::{LED_CMD_CH, LedState};
use crate::ui::top_bar::*;

#[derive(Clone)]
pub enum DuckCmd {
    Delay(u32),
    Key { modifier: u8, key: u8 },
    String(&'static str),
    Rem(heapless::String<64>),
    Pause,
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

static mut LAST_CMD: Option<DuckCmd> = None;

const KEY_PRESS_MS: u64 = 30;
const KEY_RELEASE_MS: u64 = 15;

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

fn parse_combo(line: &str) -> Option<(u8, u8)> {
    let mut modifier = 0u8;
    let mut key_code = 0u8;

    for part in line.split('+') {
        match part {
            "CTRL" => modifier |= MOD_CTRL,
            "SHIFT" => modifier |= MOD_SHIFT,
            "ALT" => modifier |= MOD_ALT,
            "GUI" | "WINDOWS" => modifier |= MOD_GUI,
            k => {
                if let Some(code) = key_from_name(k) {
                    key_code = code;
                } else if k.len() == 1 {
                    let (m, kcode) = ascii_to_key(k.as_bytes()[0]);
                    modifier |= m;
                    key_code = kcode;
                }
            }
        }
    }

    if key_code == 0 { None } else { Some((modifier, key_code)) }
}

fn parse_ducky_line(line: &str) -> Option<DuckCmd> {
    let line = line.trim();

    if line.is_empty() {
        return None;
    }

    if line.starts_with("REM ") {
        let text = &line[4..];

        info!("[DUCKY] REM {}", text);

        let mut msg: heapless::String<64> = heapless::String::new();
        let _ = msg.push_str(text); // truncate if too long
        
        return Some(DuckCmd::Rem(msg));
    }

    if line.starts_with("DELAY ") {
        let ms = line[6..].trim().parse().ok()?;
        return Some(DuckCmd::Delay(ms));
    }

    if line.starts_with("STRING ") {
        let text = &line[7..];
        return Some(DuckCmd::String(Box::leak(text.to_string().into_boxed_str())));
    }

    if line == "PAUSE" {
        return Some(DuckCmd::Pause);
    }

    if line.starts_with("REPEAT ") {
        let n = line[7..].trim().parse().ok()?;
        return Some(DuckCmd::Repeat(n));
    }

    if let Some((modifier, key)) = parse_combo(line) {
        return Some(DuckCmd::Key { modifier, key });
    }

    None
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
    let mut last_cmd: Option<DuckCmd> = None;

    loop {
        let filename = DUCKY_CH.receive().await;
        info!("[DUCKY] received request: {}", filename);

        info!("Attempting to borrow CS");

        // Try to borrow. If the monitor is currently scanning, we wait briefly and try again.
        let mut cs_borrow = loop {
            if let Ok(b) = cs_pin.try_borrow_mut() {
                break b;
            }
            info!("CS busy, retrying...");
            Timer::after(Duration::from_millis(100)).await;
        };

        info!("Borrowed CS successfully");

        // 2. Re-initialize the SPI device and SD card locally
        let spi_device = RefCellDevice::new(spi_bus, &mut *cs_borrow, esp_hal::delay::Delay::new()).inspect_err(|f| {
            error!("Failed to create SPI device on Ducky!");
        }).unwrap();
        let mut sdcard = SdCard::new(spi_device, esp_hal::delay::Delay::new());

        // 3. Initialize VolumeManager only when needed
        let mut volume_mgr = VolumeManager::new(sdcard, DummyTime);

        let volume = match volume_mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) {
            Ok(v) => v,
            Err(e) => {
                error!("[DUCKY] open_volume failed");
                continue;
            }
        };

        let root = match volume.open_root_dir() {
            Ok(r) => r,
            Err(e) => {
                error!("[DUCKY] open_root_dir failed");
                continue;
            }
        };
        
        let ducky_dir = match root.open_dir("DUCKY") {
            Ok(d) => d,
            Err(_) => {
                root.make_dir_in_dir("DUCKY").inspect_err(|_| {
                    error!("Failed to make DUCKY dir");
                }).unwrap();
                root.open_dir("DUCKY").inspect_err(|_| {
                    error!("Failed to open DUCKY dir");
                }).unwrap()
            }
        };

        info!("Iterating directory...");
        ducky_dir.iterate_dir(|file| {
            let filename = str::from_utf8(file.name.base_name()).expect("Failed to parse file name");
            let extension = str::from_utf8(file.name.extension()).expect("Failed to parse file extension");
            info!("{}.{}", filename, extension);
        }).expect("Failed to iterate directory");

        let mut file = match ducky_dir.open_file_in_dir(
            filename.as_str(),
            embedded_sdmmc::Mode::ReadOnly,
        ) {
            Ok(f) => f,
            Err(e) => {
                error!("[DUCKY] open_file failed");
                continue;
            }
        };

        info!("[DUCKY] file opened successfully");

        let mut buf = [0u8; 64];
        let mut partial_line = Vec::new();

        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        if b == b'\n' || b == b'\r' {
                            if !partial_line.is_empty() {
                                let line = str::from_utf8(&partial_line).unwrap_or("");
                                if let Some(cmd) = parse_ducky_line(line) {
                                    match &cmd {
                                        DuckCmd::Repeat(n) => {
                                            if let Some(prev) = &last_cmd {
                                                for _ in 0..*n {
                                                    execute_ducky_cmd(prev.clone()).await;
                                                }
                                            }
                                        }

                                        _ => {
                                            execute_ducky_cmd(cmd.clone()).await;

                                            match cmd {
                                                DuckCmd::Delay(_) |
                                                DuckCmd::String(_) |
                                                DuckCmd::Key { .. } => {
                                                    last_cmd = Some(cmd);
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                partial_line.clear();
                            }
                        } else {
                            partial_line.push(b);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        info!("[DUCKY] script finished");
    }
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

        DuckCmd::Rem(text) => {
            TOP_BAR_CH.send(TopBarMode::Message { text }).await;
        }

        DuckCmd::Key { modifier, key } => {
            press_key(modifier, key).await;
        }

        DuckCmd::Pause => {
            info!("[DUCKY] Waiting for button");
            LED_CMD_CH.send(LedState::Blink(RGB { r:10,g:0,b:0 }, 100)).await;

            let mut msg: heapless::String<64> = heapless::String::new();
            let _ = msg.push_str("PAUSED");

            TOP_BAR_CH.send(TopBarMode::Message { text: msg }).await;

            PAUSE_BUTTON_CH.receive().await;

            info!("[DUCKY] Resuming");
        }
        
        DuckCmd::Repeat(_) => { /* handled in ducky_task */}
    }
}
