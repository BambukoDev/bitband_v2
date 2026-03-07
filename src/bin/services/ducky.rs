use alloc::string::String;
use alloc::string::ToString;
use bt_hci::event::NumberOfCompletedDataBlocks;
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

    String(heapless::String<128>),
    StringLn(heapless::String<128>),

    Key { modifier: u8, key: u8 },

    Hold { modifier: u8, key: u8 },
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

        let (modifier, key) = parse_key_combo(token)?;
        return Some(DuckCmd::Hold { modifier, key });
    }

    if line == "RELEASE" {
        return Some(DuckCmd::Release);
    }

    if let Some((modifier, key)) = parse_key_combo(line) {
        return Some(DuckCmd::Key { modifier, key });
    }
    None
}

fn parse_key_combo(line: &str) -> Option<(u8, u8)> {
    let mut modifier = 0;
    let mut key = 0;

    for token in line.split('+') {
        match token {
            "CTRL" | "CONTROL" => modifier |= MOD_CTRL,
            "SHIFT" => modifier |= MOD_SHIFT,
            "ALT" => modifier |= MOD_ALT,
            "GUI" | "WINDOWS" => modifier |= MOD_GUI,

            k => {
                if let Some(code) = key_from_name(k) {
                    key = code;
                } else if k.len() == 1 {
                    let (m, kcode) = ascii_to_key(k.as_bytes()[0]);
                    modifier |= m;
                    key = kcode;
                }
            },
        }
    }

    if key == 0 { None } else { Some((modifier, key)) }
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
    let mut default_delay: u32 = 0;
    let mut last_cmd: Option<DuckCmd> = None;
    let mut rng_state: u32 = 0x12345678;

    loop {
        let filename = DUCKY_CH.receive().await;
        PAUSE_BUTTON_CH.clear();
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
                                let mut default_delay: u32 = 0;
                                let mut last_cmd: Option<DuckCmd> = None;

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
                                            let delay = rand_range(min, max, &mut rng_state);
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

        DuckCmd::Key { modifier, key } => {
            press_key(modifier, key).await;
        }

        DuckCmd::Pause => {
            info!("[DUCKY] Waiting for button");

            LED_CMD_CH
                .send(LedState::Blink(RGB { r:10,g:0,b:0 }, 100))
                .await;

            let mut msg: heapless::String<64> = heapless::String::new();
            let _ = msg.push_str("PAUSED");

            TOP_BAR_CH.send(TopBarMode::Message { text: msg }).await;

            PAUSE_BUTTON_CH.receive().await;

            info!("[DUCKY] Resuming");
        }

        DuckCmd::DefaultDelay(_) => {}

        DuckCmd::Repeat(_) => {}

        DuckCmd::Hold { modifier, key } => {
            HID_CH
                .send(KeyReport {
                    modifier,
                    keys: [key, 0, 0, 0, 0, 0],
                })
                .await;
        }

        DuckCmd::Release => {
            HID_CH
                .send(KeyReport {
                    modifier: 0,
                    keys: [0; 6],
                })
                .await;
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
