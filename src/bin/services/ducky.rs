use alloc::string::String;
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_hal_bus::spi::RefCellDevice;
use embedded_sdmmc::filesystem::ToShortFileName;
use embedded_sdmmc::SdCard;
use esp_hal::gpio::Output;
use esp_hal::spi;
use esp_hal::Blocking;
use crate::services::hid::*;
use crate::services::keyboard::*;
use crate::DummyTime;
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use embedded_sdmmc::{VolumeManager, TimeSource};
use defmt::{error, info};

#[derive(Clone)]
pub enum DuckCmd {
    Delay(u32),
    Key { modifier: u8, key: u8 },
    String(&'static str),
}

pub static DUCKY_CH: Channel<
    CriticalSectionRawMutex,
    &'static str,
    2,
> = Channel::new();


#[task]
pub async fn ducky_task(
    volume_mgr: &'static VolumeManager<
        SdCard<
            RefCellDevice <
                'static, 
                spi::master::Spi<'static, Blocking>,
                Output<'static>,
                esp_hal::delay::Delay
            >,
            esp_hal::delay::Delay
        >,
        DummyTime
    >
) {
    // loop {
    //     let filename = DUCKY_CH.receive().await;
    //
    //     let volume = volume_mgr.open_volume(embedded_sdmmc::VolumeIdx(0)).unwrap();
    //     let root = volume.open_root_dir().unwrap();
    //     let mut file = root.open_file_in_dir(filename, embedded_sdmmc::Mode::ReadOnly).unwrap();
    //
    //     let mut buf = [0u8; 64];
    //
    //     while let Ok(n) = file.read(&mut buf) {
    //         if n == 0 { break; }
    //
    //         for &b in &buf[..n] {
    //             let (modi, key) = ascii_to_key(b);
    //             if key == 0 { continue; }
    //
    //             HID_CH.send(KeyReport {
    //                 modifier: modi,
    //                 keys: [key, 0, 0, 0, 0, 0],
    //             }).await;
    //
    //             Timer::after(Duration::from_millis(5)).await;
    //
    //             HID_CH.send(KeyReport {
    //                 modifier: 0,
    //                 keys: [0; 6],
    //             }).await;
    //         }
    //     }
    // }

    info!("[DUCKY] task started");

    loop {
        let filename = DUCKY_CH.receive().await;
        info!("[DUCKY] received request: {}", filename);

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
                root.make_dir_in_dir("DUCKY").expect("Failed to create DUCKY dir");
                root.open_dir("DUCKY").expect("Failed to open DUCKY dir")
            }
        };

        info!("Iterating directory...");
        ducky_dir.iterate_dir(|file| {
            let filename = str::from_utf8(file.name.base_name()).expect("Failed to parse file name");
            let extension = str::from_utf8(file.name.extension()).expect("Failed to parse file extension");
            info!("{}.{}", filename, extension);
        }).expect("Failed to iterate directory");

        let mut file = match ducky_dir.open_file_in_dir(
            filename,
            embedded_sdmmc::Mode::ReadOnly,
        ) {
            Ok(f) => f,
            Err(e) => {
                error!("[DUCKY] open_file failed");
                continue;
            }
        };

        info!("[DUCKY] file opened successfully");

        let mut buf = [0u8; 32];

        loop {
            match file.read(&mut buf) {
                Ok(0) => {
                    info!("[DUCKY] EOF");
                    break;
                }
                Ok(n) => {
                    for &b in &buf[..n] {
                        info!("[DUCKY] byte: 0x{:02X} '{}'", b, b as char);
                    }
                }
                Err(e) => {
                    // error!("[DUCKY] read error: {:?}", e);
                    break;
                }
            }
        }

        info!("[DUCKY] script finished");
    }
}
