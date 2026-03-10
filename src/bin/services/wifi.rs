use bt_hci::cmd::info;
use embassy_time::{Duration, Timer};
use esp_hal_dhcp_server::{simple_leaser::SingleDhcpLeaser, structs::DhcpServerConfig};
use esp_radio::wifi::{AccessPointConfig, ClientConfig, Config as RadioConfig, ModeConfig, Protocol, WifiController, WifiDevice, WifiMode as RadioWifiMode};
use embassy_net::{Config, DhcpConfig, IpAddress, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use defmt::{info,error};
use alloc::string::{String, ToString};
use esp_println::println;
use core::{fmt::Write, str::FromStr};

use crate::ui;

// Signal to trigger AP on/off from the menu
pub static WIFI_MODE_SIGNAL: Signal<CriticalSectionRawMutex, WifiMode> = Signal::new();
static IS_STA: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub enum WifiMode {
    Ap,
    Sta(String, String),
    Disabled
}

#[embassy_executor::task]
pub async fn wifi_ap_task(
    controller: &'static mut WifiController<'static>,
    stack: &'static Stack<'static>,
) {
    loop {
        let mode = WIFI_MODE_SIGNAL.wait().await;

        match mode {
            WifiMode::Ap => {
                let _ = controller.stop();
                // let _ = controller.stop();
                IS_STA.store(false, core::sync::atomic::Ordering::SeqCst);

                let config = StaticConfigV4 {
                    address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(192, 168, 4, 1), 24),
                    gateway: Some(embassy_net::Ipv4Address::new(192, 168, 4, 1)),
                    dns_servers: Default::default(),
                };
                
                stack.set_config_v4(embassy_net::ConfigV4::Static(config));

                let ap_config = AccessPointConfig::default()
                    .with_ssid("BitBandV2".to_string())
                    .with_password("bitband123".to_string())
                    .with_auth_method(esp_radio::wifi::AuthMethod::Wpa2Personal);
                controller.set_config(&ModeConfig::AccessPoint(ap_config)).unwrap();
                controller.start().unwrap();

                for i in 1..=5 {
                    if stack.is_link_up() {
                        info!("AP Started");
                    }
                    Timer::after_millis(500).await;
                }
            }
            WifiMode::Sta(ssid, pass) => {
                // controller.set_mode(RadioWifiMode::Sta).unwrap();
                let _ = controller.stop();
                IS_STA.store(true, core::sync::atomic::Ordering::SeqCst);
                info!("Connecting to {}", ssid.as_str());
                // let config = StaticConfigV4 {
                //     address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(192, 168, 1, 200), 24),
                //     gateway: Some(embassy_net::Ipv4Address::new(192, 168, 4, 1)),
                //     dns_servers: Default::default(),
                // };

                let mut config = DhcpConfig::default();
                config.hostname = core::convert::TryInto::try_into("BitBandV2").ok();
                stack.set_config_v4(embassy_net::ConfigV4::Dhcp(config));

                let client_config = ClientConfig::default()
                    .with_ssid(ssid.to_string())
                    .with_password(pass.to_string());
                
                controller.set_config(&ModeConfig::Client(client_config)).unwrap();
                controller.start().unwrap();
                controller.connect().unwrap();

                Timer::after_secs(1).await;

                for i in 1..=40 {
                    if stack.is_link_up() {
                        if let Some(config) = stack.config_v4() {
                            let ip = config.address.address();
                            if !ip.is_unspecified() {
                                info!("Connected! IP: {}", ip);
                                
                                // Update the OLED top bar with IP
                                let mut msg = heapless::String::<64>::new();
                                let _ = core::write!(msg, "IP: {}", ip);
                                ui::top_bar::TOP_BAR_CH.send(ui::top_bar::TopBarMode::Message { text: msg }).await;
                                break;
                            }
                        }
                    }
                    Timer::after_millis(500).await;
                }
                if (!stack.is_link_up()) {
                    error!("Failed to connect :(");
                }
            }
            WifiMode::Disabled => {
                info!("Stopped WiFi");
                let _ = controller.stop();
                controller.set_config(&ModeConfig::None).unwrap();
                IS_STA.store(false, core::sync::atomic::Ordering::SeqCst);
            }
        }
        info!("AP state: {}", esp_radio::wifi::ap_state());
        info!("STA state: {}", esp_radio::wifi::sta_state());
    }
}

#[embassy_executor::task]
pub async fn dhcp_server_task(stack: &'static Stack<'static>) {
    loop {
        info!("Stack Link: {}", stack.is_link_up());
        info!("Condif: {}", stack.is_config_up());
        if stack.is_link_up() /* && !IS_STA.load(core::sync::atomic::Ordering::SeqCst) */ {
            if let Some(cfg) = stack.config_v4() {
                if cfg.address.address() == Ipv4Address::new(192, 168, 4, 1) {
                    let config = DhcpServerConfig {
                        ip: Ipv4Address::new(192, 168, 4, 1),
                        lease_time: Duration::from_secs(3600),
                        gateways: &[Ipv4Address::new(192, 168, 4, 1)],
                        subnet: None,
                        dns: &[Ipv4Address::new(192, 168, 4, 1)],
                        use_captive_portal: false,
                    };

                    let mut leaser = SingleDhcpLeaser::new(Ipv4Address::new(192, 168, 4, 69));

                    info!("[DHCP] Starting server...");
                    let res = esp_hal_dhcp_server::run_dhcp_server(*stack, config, &mut leaser).await;
                    
                    if let Err(e) = res {
                        error!("[DHCP] SERVER ERROR: {:?}", e);
                    }
                }
            }
        }
        
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) -> ! {
    runner.run().await
}
