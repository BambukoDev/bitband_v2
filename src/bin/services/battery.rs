use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use esp_hal::gpio;
use crate::top_bar::StatusBar;
use crate::top_bar::STATUS_SIGNAL;

#[embassy_executor::task]
pub async fn battery_task() {
    let mut percent: u8 = 100;

    loop {
        percent = percent.saturating_sub(1); // fake ADC for now
        // STATUS_SIGNAL.wait().await;
        let mut status = StatusBar {
            battery_percent: percent,
            hours: 0,
            minutes: 0,
        };

        status.battery_percent = percent;
        STATUS_SIGNAL.signal(status);

        Timer::after(Duration::from_secs(30)).await;
    }
}
