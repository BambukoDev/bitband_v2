use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;
use esp_hal::peripherals::FLASH;
use defmt::{info, error};
use alloc::string::String;

pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
}

// Default NVS partition coordinates for ESP32-S3
const NVS_ADDR: usize = 0x9000;
const NVS_SIZE: usize = 0x6000;

pub fn save_wifi_credentials(ssid: &str, pass: &str) {
    let flash_periph = unsafe { FLASH::steal() };
    let flash = FlashStorage::new(flash_periph);
    
    let mut nvs = Nvs::new(NVS_ADDR, NVS_SIZE, flash).expect("Failed to open NVS");

    let namespace = Key::from_str("wifi");

    nvs.set(&namespace, &Key::from_str("ssid"), ssid).expect("Failed to write ssid");
    nvs.set(&namespace, &Key::from_str("pass"), pass).expect("Failed to write pass");
    
    info!("Credentials saved to NVS!");
}

pub fn load_wifi_credentials() -> Option<WifiConfig> {
    let flash_periph = unsafe { FLASH::steal() };
    let flash = FlashStorage::new(flash_periph);
    
    let mut nvs = Nvs::new(NVS_ADDR, NVS_SIZE, flash).ok()?;

    let namespace = Key::from_str("wifi");
    let ssid  = match nvs.get::<String>(&namespace, &Key::from_str("ssid")) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to get data from NVS: {}", e);
            return None;
        }
    };
    let password = nvs.get::<String>(&namespace, &Key::from_str("pass")).expect("Failed to get pass");

    let config = WifiConfig {
        ssid,
        password, 
    };

    Some(config)
}
