#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
// #![deny(clippy::large_stack_frames)]

use embassy_net::Config;
use esp_hal::{gpio::{self, InputConfig}, i2c, ledc::channel, peripherals};

use bt_hci::controller::ExternalController;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::{SmartLedsAdapter, smart_led_buffer};
use esp_println as _;
use esp_radio::{ble::controller::BleConnector, wifi::{ClientConfig, ModeConfig, ScanConfig, WifiController}};
use trouble_host::prelude::*;

use smart_leds::{brightness, colors, SmartLedsWrite as _};

use ssd1306::{mode::TerminalMode, prelude::*, I2CDisplayInterface, Ssd1306, command};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text}
};

use alloc::{boxed::Box, string::String};

mod input;
mod services;
mod ui;

use input::button;

use services::battery;
use services::clock;

use ui::menu;
use ui::top_bar;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]

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
    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    let transport = BleConnector::new(&radio_init, peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _stack = trouble_host::new(ble_controller, &mut resources);

    let client_conf = ClientConfig::default()
        .with_ssid(String::new())
        .with_password(String::new());
    let modeconf = ModeConfig::Client(client_conf);

    _wifi_controller.set_config(&modeconf).unwrap();
    _wifi_controller.start().unwrap();

    let mut pulse_code = smart_led_buffer!(1);
    let frequency = Rate::from_mhz(80);
    let rmt = Rmt::new(peripherals.RMT, frequency).expect("Failed to initialize RMT0");
    let mut led = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO38, &mut pulse_code);
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


    // let text_style = MonoTextStyleBuilder::new()
    //     .font(&FONT_6X10)
    //     .text_color(BinaryColor::On)
    //     .build();

    // loop {
    //     Text::with_baseline("Hello world!", Point::zero(), text_style, Baseline::Top)
    //         .draw(&mut display_top)
    //         .unwrap();
    //     display_top.flush().unwrap();
    //     for x in 0..128 {
    //         for y in 0..32 {
    //             display_bot.set_pixel(x, y, x % 2 == 0 && y % 2 == 0);
    //         }
    //     }
    //     display_bot.flush().unwrap();
    //     // rgb_led_r.set_high();
    //     led.write(brightness([colors::RED].into_iter(), 10)).unwrap();
    //     info!("Hello world!");
    //     Timer::after(Duration::from_secs(1)).await;
    //     // rgb_led_r.set_low();
    //     led.write(brightness([colors::BLUE].into_iter(), 10)).unwrap();
    //     Timer::after(Duration::from_secs(1)).await;
    // }
    
    let btn_up = gpio::Input::new(peripherals.GPIO1, InputConfig::default().with_pull(gpio::Pull::Up));
    let btn_down = gpio::Input::new(peripherals.GPIO43, InputConfig::default().with_pull(gpio::Pull::Up));
    let btn_sel = gpio::Input::new(peripherals.GPIO44, InputConfig::default().with_pull(gpio::Pull::Up));

    let wifi_ctrl: &'static mut WifiController<'static> = Box::leak(Box::new(_wifi_controller));

    spawner.spawn(button::button_task(btn_up, btn_down, btn_sel)).unwrap();
    spawner.spawn(ui::menu::menu_task(display_bot)).unwrap();
    spawner.spawn(ui::top_bar::status_task(display_top)).unwrap();
    spawner.spawn(services::battery::battery_task()).unwrap();
    spawner.spawn(ui::menu::radio_task()).unwrap();
    spawner.spawn(ui::menu::ble_scan_task()).unwrap();
    spawner.spawn(ui::menu::wifi_scan_task(wifi_ctrl)).unwrap();
    // spawner.spawn(services::clock::clock_task()).unwrap();

    loop {
        // info!("KEEPALIVE");
        led.write(brightness([colors::RED].into_iter(), 10)).unwrap();
        Timer::after(Duration::from_millis(300)).await;
        led.write(brightness([colors::GREEN].into_iter(), 10)).unwrap();
        Timer::after(Duration::from_millis(300)).await;
        led.write(brightness([colors::BLUE].into_iter(), 10)).unwrap();
        Timer::after(Duration::from_millis(300)).await;
    }

    // core::future::pending::<()>().await;

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v~1.0/examples
}
