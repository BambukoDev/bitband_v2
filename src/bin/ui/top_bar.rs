use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use ssd1306::{prelude::*, mode::BufferedGraphicsMode};
use esp_hal::i2c;
use esp_hal::peripherals;
use ssd1306::{mode::TerminalMode, prelude::*, I2CDisplayInterface, Ssd1306, command};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text}
};
use defmt::info;

#[derive(Copy, Clone)]
pub struct StatusBar {
    pub battery_percent: u8,
    pub minutes: u8,
    pub hours: u8,
}

pub static STATUS_SIGNAL: Signal<CriticalSectionRawMutex, StatusBar> =
    Signal::new();

// type Display = Ssd1306<
//     I2CDisplayInterface,
//     DisplaySize128x32,
//     BufferedGraphicsMode<DisplaySize128x32>,
// >;


type Display = ssd1306::Ssd1306<ssd1306::prelude::I2CInterface<esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

#[embassy_executor::task]
pub async fn status_task(
    mut display: Display
) {
    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    loop {
        let status = STATUS_SIGNAL.wait().await;
        info!("TEST");
        display.clear_buffer();

        Text::with_baseline(
            &alloc::format!(
                "BAT {}%  {:02}:{:02}",
                status.battery_percent,
                status.hours,
                status.minutes
            ),
            Point::zero(),
            style,
            Baseline::Top,
        )
        .draw(&mut display)
        .unwrap();

        display.flush().unwrap();
    }
}
