use bt_hci::event::le::LeBigSyncEstablished;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::Timer;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::text::renderer::CharacterStyle;
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

use alloc::format;

use crate::menu::{WifiApInfo, get_selected_ap};

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

pub enum TopInfo {
    WifiAp(&'static WifiApInfo),
    Default,
}

pub enum TopBarMode {
    Normal {
        battery_percent: u8,
        time_hhmm: (u8, u8),
    },
    WifiAp {
        ssid: &'static str,
        rssi: i8,
        channel: u8,
        // auth: AuthMethod,
    },
}

pub static TOP_BAR_CH: Channel<
    CriticalSectionRawMutex,
    TopBarMode,
    4,
> = Channel::new();

type Display = ssd1306::Ssd1306<ssd1306::prelude::I2CInterface<esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

#[embassy_executor::task]
pub async fn status_task(mut display: Display) {
    let mut state = TopBarMode::Normal {
        battery_percent: 100,
        time_hhmm: (0, 0),
    };
    let mut tick: u32 = 0;

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    loop {
        tick = tick.wrapping_add(1);

        if let Ok(msg) = TOP_BAR_CH.try_receive() {
            state = match msg {
                TopBarMode::Normal { battery_percent, time_hhmm } => TopBarMode::Normal { battery_percent, time_hhmm },
                TopBarMode::WifiAp { ssid, rssi, channel } =>
                    TopBarMode::WifiAp { ssid, rssi, channel },
            };
        }

        display.clear_buffer();

        match state {
            TopBarMode::Normal {battery_percent, time_hhmm} => {
                BatteryWidget.draw(&mut display, tick, style);
                ClockWidget.draw(&mut display, tick, style);
            }
            TopBarMode::WifiAp { ssid, rssi, channel } => {
                WifiApWidget.draw(&mut display, tick, style);
            }
        }

        display.flush().unwrap();
        Timer::after_millis(100).await;
    }
}

trait Widget {
    fn draw(&mut self, display: &mut Display, tick: u32, style: MonoTextStyle<'_, BinaryColor>);
}

pub struct BatteryWidget;

impl Widget for BatteryWidget {
    fn draw(&mut self, display: &mut Display, _tick: u32, style: MonoTextStyle<'_, BinaryColor>) {
        // let pct = get_battery_percent(); // your existing function
        let pct = 100;
        draw_text_at(display, &format!("BAT:{}%", pct), 0, 0, style);
    }
}

pub struct ClockWidget;

impl Widget for ClockWidget {
    fn draw(&mut self, display: &mut Display, _tick: u32, style: MonoTextStyle<'_, BinaryColor>) {
        // let (hh, mm) = get_time_hm(); // RTC helper
        let (hh, mm) = (0, 0);
        draw_text_at(display, &format!("{:02}:{:02}", hh, mm), 90, 0, style);
    }
}

pub struct WifiApWidget;

impl Widget for WifiApWidget {
    fn draw(&mut self, display: &mut Display, tick: u32, style: MonoTextStyle<'_, BinaryColor>) {
        if let Some(ap) = get_selected_ap() {
            // SSID (scrolling)
            draw_scrolling_text(
                display,
                ap.ssid,
                0,
                0,
                128,
                tick,
                style,
            );

            // Metadata
            draw_text_at(
                display,
                &format!("{}dBm  CH{}",
                    0,
                    // ap.signal_strenght,
                    ap.channel),
                0,
                10,
                style,
            );
        }
    }
}

pub fn draw_text_at(display: &mut Display, data: &str, pos_x: i32, pos_y: i32, style: MonoTextStyle<'_, BinaryColor>) {
    Text::with_baseline(
        data,
        Point::new(pos_x, pos_y),
        style,
        Baseline::Top,
    )
    .draw(display)
    .ok();
}

fn draw_scrolling_text(
    display: &mut Display,
    text: &str,
    x: i32,
    y: i32,
    max_width: i32,
    tick: u32,
    style: MonoTextStyle<'_, BinaryColor>
) {
    let w = text_width(text);

    if w <= max_width {
        draw_text_at(display, text, x, y, style);
    } else {
        let scroll = (tick / 2) as i32 % (w + 10);
        draw_text_at(display, text, x - scroll, y, style);
    }
}

fn text_width(s: &str) -> i32 {
    (s.len() as i32) * 6
}
