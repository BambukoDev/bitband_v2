#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
// #![deny(clippy::large_stack_frames)]
pub const L2CAP_MTU: usize = 255;

use core::{cell::RefCell, default, net::Ipv4Addr};

use embedded_hal::{digital::{InputPin, OutputPin}, spi::{ErrorType, SpiBus}};
use esp_hal::{gpio::{self, Input, InputConfig, OutputConfig, Pull}, i2c, ledc::channel, otg_fs::{asynch::Driver, Usb, UsbBus}, peripherals, spi, DriverMode};

use bt_hci::{cmd::info, controller::ExternalController};
// use log::{error, info};
use defmt::{info, error};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::rmt::Rmt;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::{SmartLedsAdapter, smart_led_buffer};
use esp_println as _;
use esp_println::println;
use esp_radio::{ble::controller::BleConnector, wifi::{ClientConfig, ModeConfig, ScanConfig, WifiController, WifiDevice}};
use static_cell::StaticCell;

use embassy_net::{Stack, StackResources, Config, IpAddress, Ipv4Address, Ipv4Cidr};
use smart_leds::{brightness, colors, SmartLedsWrite as _};

use ssd1306::{mode::TerminalMode, prelude::*, I2CDisplayInterface, Ssd1306, command};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text}
};

use embedded_sdmmc::{SdCard, TimeSource, VolumeManager};
use esp_hal::spi::master::Spi;
use esp_hal::gpio::Output;
use esp_hal::time::Rate;
use embedded_hal_bus::spi::RefCellDevice;

use alloc::{boxed::Box, string::String, vec::Vec};

use embassy_usb::class::hid::{HidWriter, HidReader, ReportId};
use embassy_usb::Builder;

mod input;
mod services;
mod ui;

use input::button;

use services::battery;
use services::clock;

use ui::menu;
use ui::top_bar;

use embedded_hal_bus::spi::ExclusiveDevice;

use esp_backtrace as _;

use crate::services::{bluetooth::run, sd_monitor::sd_monitor_task};
use crate::services::*;

// Replaced by esp_backtrace
// #[panic_handler]
// fn panic(p: &core::panic::PanicInfo) -> ! {
//     error!("Panicked: {}", p.message().as_str());
//     loop {}
// }

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]

static CS_CELL: StaticCell<RefCell<Output<'static>>> = StaticCell::new();

static AP_STACK: StaticCell<Stack> = StaticCell::new();
static AP_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
static STA_STACK: StaticCell<Stack> = StaticCell::new();
static STA_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    // generator version: 1.1.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

    let radio_init: &'static _ = Box::leak(Box::new(
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller")
    ));

    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    let transport = BleConnector::new(&radio_init, peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 20>::new(transport);

    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);

    let mut pulse_code = Box::leak(Box::new(smart_led_buffer!(1)));
    let frequency = Rate::from_mhz(80);
    let rmt = Rmt::new(peripherals.RMT, frequency).expect("Failed to initialize RMT0");
    let mut led = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO38, pulse_code);
    info!("RGB light initialized!");

    let disp_top_i2c = i2c::master::I2c::new(peripherals.I2C0, i2c::master::Config::default())
        .unwrap()
        .with_sda(peripherals.GPIO5)
        .with_scl(peripherals.GPIO4);
    let disp_top_interface = I2CDisplayInterface::new(disp_top_i2c);
    let mut display_top = Ssd1306::new(
        disp_top_interface,
        DisplaySize128x32,
        DisplayRotation::Rotate0
    ).into_buffered_graphics_mode();
    display_top.init().unwrap();
    display_top.clear_buffer();
    display_top.flush().unwrap();
    info!("Top display initialized");

    let disp_bot_i2c = i2c::master::I2c::new(peripherals.I2C1, i2c::master::Config::default())
        .unwrap()
        .with_sda(peripherals.GPIO7)
        .with_scl(peripherals.GPIO6);
    let disp_bot_interface = I2CDisplayInterface::new(disp_bot_i2c);
    let mut display_bot = Ssd1306::new(
        disp_bot_interface,
        DisplaySize128x32,
        DisplayRotation::Rotate0
    ).into_buffered_graphics_mode();
    display_bot.init().unwrap();
    display_bot.clear_buffer();
    display_bot.flush().unwrap();
    info!("Bottom display initialized");
    
    let btn_up = gpio::Input::new(peripherals.GPIO2, InputConfig::default().with_pull(gpio::Pull::Up));
    let btn_down = gpio::Input::new(peripherals.GPIO9, InputConfig::default().with_pull(gpio::Pull::Up));
    let btn_sel = gpio::Input::new(peripherals.GPIO1, InputConfig::default().with_pull(gpio::Pull::Up));

    spawner.spawn(services::usb_keyboard::usb_keyboard_task(usb)).unwrap();

    let (wifi_controller, interfaces) = esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default()).unwrap();

    // Configure static IP for the AP
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: embassy_net::Ipv4Cidr::new(embassy_net::Ipv4Address::new(192, 168, 4, 1), 24),
        gateway: Some(embassy_net::Ipv4Address::new(192, 168, 4, 1)),
        dns_servers: Default::default(),
    });

    let (ap_stack, ap_runner) = embassy_net::new(
        interfaces.ap,
        config,
        AP_RESOURCES.init(StackResources::<3>::new()),
        12345, // Seed
    );

    let config = embassy_net::Config::dhcpv4(Default::default());

    let (sta_stack, sta_runner) = embassy_net::new(
        interfaces.sta,
        config,
        STA_RESOURCES.init(StackResources::<3>::new()),
        12345, // Seed
    );

    let ap_stack = &*AP_STACK.init(ap_stack);
    let sta_stack = &*STA_STACK.init(sta_stack);
    let wifi_ctrl_static = Box::leak(Box::new(wifi_controller));

    spawner.spawn(wifi::ap_net_task(ap_runner)).unwrap();
    spawner.spawn(wifi::sta_net_task(sta_runner)).unwrap();
    spawner.spawn(wifi::wifi_task(wifi_ctrl_static, ap_stack, sta_stack)).unwrap();
    spawner.spawn(wifi::dhcp_server_task(ap_stack)).unwrap();
    spawner.spawn(web::web_server_task(ap_stack, sta_stack)).unwrap();

    spawner.spawn(services::led::led_task(led)).unwrap();
    spawner.spawn(button::button_task(btn_up, btn_down, btn_sel)).unwrap();
    spawner.spawn(ui::top_bar::status_task(display_top)).unwrap();
    spawner.spawn(services::battery::battery_task()).unwrap();
    spawner.spawn(ui::menu::action_handler()).unwrap();
    // spawner.spawn(services::bluetooth::bluetooth_task(ble_controller)).unwrap();
    // spawner.spawn(services::clock::clock_task()).unwrap();

    let cd = Input::new(peripherals.GPIO8, InputConfig::default().with_pull(Pull::Up));
    let cs = Output::new(peripherals.GPIO10, gpio::Level::High, OutputConfig::default());
    let cs_refcell = CS_CELL.init(RefCell::new(cs));
    let sck = peripherals.GPIO12;
    let mosi = peripherals.GPIO11;
    let miso = peripherals.GPIO13;

    let spi_bus_config = spi::master::Config::default()
        .with_frequency(Rate::from_khz(400))
        .with_mode(spi::Mode::_0);
    let spi_bus = spi::master::Spi::new(peripherals.SPI2, spi_bus_config)
        .expect("Failed to initialize SPI bus")
        .with_mosi(mosi)
        .with_miso(miso)
        .with_sck(sck);
    info!("SPI device created!");
    let shared_spi_bus = Box::leak(Box::new(RefCell::new(spi_bus)));

    let file_browser = ui::file_browser::get_file_browser();
    spawner.spawn(sd_monitor_task(cd, shared_spi_bus, cs_refcell, file_browser)).unwrap();
    spawner.spawn(services::ducky::ducky_task(shared_spi_bus, cs_refcell)).unwrap();
    spawner.spawn(ui::menu::menu_task(display_bot, file_browser)).unwrap();

    core::future::pending::<()>().await;
}

// TEMP FOR TESTING THE SD CARD
pub struct DummyTime;

impl TimeSource for DummyTime {
    fn get_timestamp(&self) -> embedded_sdmmc::Timestamp {
        embedded_sdmmc::Timestamp {
            year_since_1970: 54,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

pub static SD_MGR: StaticCell<SdMgrType> = StaticCell::new();

type SdMgrType =
    VolumeManager<
        SdCard<
            RefCellDevice<'static,
                spi::master::Spi<'static, esp_hal::Blocking>,
                Output<'static>,
                esp_hal::delay::Delay
            >,
            esp_hal::delay::Delay
        >,
        DummyTime
    >;
