use embassy_time::{Duration, Timer};
use esp_hal_dhcp_server::{simple_leaser::SingleDhcpLeaser, structs::DhcpServerConfig};
use esp_radio::wifi::{AccessPointConfig, Config as RadioConfig, ModeConfig, Protocol, WifiController, WifiDevice};
use embassy_net::{Config, IpAddress, Ipv4Address, Ipv4Cidr, Runner, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use defmt::{info,error};
use alloc::string::{String, ToString};

// Signal to trigger AP on/off from the menu
pub static WIFI_SIGNAL: Signal<CriticalSectionRawMutex, bool> = Signal::new();

#[embassy_executor::task]
pub async fn wifi_ap_task(
    controller: &'static mut WifiController<'static>,
    stack: &'static Stack<'static>,
) {
    let mut is_ap_running = false;

    loop {
        let enable = WIFI_SIGNAL.wait().await;

        if enable && !is_ap_running {
            info!("Configuring WiFi Access Point...");
            
            // 1. Ensure any previous instance is stopped before reconfiguring
            let _ = controller.stop(); 

            let ap_config = AccessPointConfig::default()
                .with_ssid("BitBandV2".to_string())
                // .with_password("babojajo123".to_string())
                .with_auth_method(esp_radio::wifi::AuthMethod::None);
            
            // 2. Apply config. If it still crashes here, check the password length (min 8)
            match controller.set_config(&ModeConfig::AccessPoint(ap_config)) {
                Ok(_) => {
                    info!("Set config for AP");
                    if let Err(e) = controller.start() {
                        defmt::error!("Failed to start controller: {:?}", e);
                    } else {
                        info!("WiFi AP Started! IP: 192.168.4.1");
                        Timer::after(Duration::from_millis(500)).await;
                        is_ap_running = true;
                    }
                }
                Err(e) => {
                    defmt::error!("Failed to set AP config: {:?}", e);
                }
            }
        } else if !enable && is_ap_running {
            let _ = controller.stop();
            info!("WiFi AP Stopped.");
            is_ap_running = false;
        }
    }
}

#[embassy_executor::task]
pub async fn dhcp_server_task(stack: &'static Stack<'static>) {
    loop {
        if stack.is_link_up() {
            // Define config inside the block so it's fresh for every run
            let config = DhcpServerConfig {
                ip: Ipv4Address::new(192, 168, 4, 1),
                lease_time: Duration::from_secs(3600),
                gateways: &[Ipv4Address::new(192, 168, 4, 1)],
                subnet: None,
                dns: &[Ipv4Address::new(192, 168, 4, 1)],
                use_captive_portal: true,
            };

            let mut leaser = SingleDhcpLeaser::new(Ipv4Address::new(192, 168, 4, 69));

            info!("[DHCP] Starting server...");
            // Now 'config' is moved here, but it's okay because 
            // the next loop iteration will recreate it.
            let res = esp_hal_dhcp_server::run_dhcp_server(*stack, config, &mut leaser).await;
            
            if let Err(e) = res {
                error!("[DHCP] SERVER ERROR: {:?}", e);
            }
        }
        
        // Wait a bit before checking link status again to save power/cycles
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) -> ! {
    runner.run().await
}
