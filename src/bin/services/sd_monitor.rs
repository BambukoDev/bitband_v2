use embedded_hal_bus::spi::RefCellDevice;
use embedded_sdmmc::SdCard;
use embedded_sdmmc::VolumeManager;
use esp_hal::gpio::Output;
use esp_hal::gpio::OutputConfig;
use esp_hal::Blocking;
use esp_hal::DriverMode;
use heapless::Vec;
use heapless::String;
use defmt::{info, error};
use esp_println as _;
use esp_hal::gpio::Input;
use embassy_time::{Duration, Timer};
use smart_leds::RGB;
use core::fmt::Write;

use core::cell::RefCell;
use esp_hal::spi::master::Spi;

use crate::ui::file_browser::{FileEntry, FileMenu, MAX_FILES, MAX_NAME};
use crate::services::led::*;
use crate::DummyTime;

#[embassy_executor::task]
pub async fn sd_monitor_task(
    mut cd_pin: Input<'static>,
    spi_bus: &'static RefCell<Spi<'static, Blocking>>, 
    cs_pin: &'static RefCell<Output<'static>>,
    file_menu: &'static FileMenu,
) {
     loop {

        // Wait for the card to be physically inserted (assuming Active Low)

        if cd_pin.is_low() {
            info!("[SD] Waiting for card insertion...");
            LED_CMD_CH.send(LedState::Blink(RGB {
                r: 10,
                g: 0,
                b: 0,
            }, 100)).await;
            cd_pin.wait_for_high().await;
            // Debounce to ensure stable contact
            Timer::after(Duration::from_millis(200)).await;
        }

        info!("[SD] Card detected on GPIO15. Mounting...");

        // Attempt to mount and read files
        if let Ok(v) = volume_mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) {
            if let Ok(root) = v.open_root_dir() {
                if let Ok(ducky_dir) = root.open_dir("DUCKY") {
                    let mut new_entries = Vec::<FileEntry, MAX_FILES>::new();

                    let _ = ducky_dir.iterate_dir(|file| {
                        if !file.attributes.is_directory() {
                            let mut name = heapless::String::<MAX_NAME>::new();
                            let base = core::str::from_utf8(file.name.base_name()).unwrap_or("");
                            let ext = core::str::from_utf8(file.name.extension()).unwrap_or("");
                            let _ = write!(name, "{}.{}", base, ext);
                            let _ = new_entries.push(FileEntry { name });
                        }
                    });

                    // Update the menu task's data
                    let mut entries = file_menu.entries.lock().await;
                    *entries = new_entries;
                    info!("[SD] Menu populated with {} files", entries.len());
                } else {
                    error!("Failed to open DUCKY dir")
                }
            } else {
                error!("Failed to open root dir")
            }
        } else {
            error!("Failed to open volume_mgr");
        }

        LED_CMD_CH.send(LedState::Blink(RGB {
            r: 0,
            g: 10,
            b: 0,
        }, 100)).await;

        // Wait for the card to be removed
        cd_pin.wait_for_low().await;
        info!("[SD] Card removed. Clearing menu.");

        let mut entries = file_menu.entries.lock().await;
        entries.clear();
    }
} 
