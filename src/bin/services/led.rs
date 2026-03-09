use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal_smartled::SmartLedsAdapter;
use smart_leds::{brightness, colors, SmartLedsWrite as _, RGB};

pub enum LedState {
    Off,
    On(RGB<u8>),
    Blink(RGB<u8>, u64),
}

pub static LED_CMD_CH: Channel<
    CriticalSectionRawMutex,
    LedState,
    4,
> = Channel::new();

#[embassy_executor::task]
pub async fn led_task(mut led: SmartLedsAdapter<'static, 25>) {
    let mut current_state = LedState::Off;

    loop {
        match &current_state {
            LedState::Off => {
                let _ = led.write([RGB { r: 0, g: 0, b: 0 }]).ok();
                // Wait for a new command
                current_state = LED_CMD_CH.receive().await;
            }
            LedState::On(color) => {
                let _ = led.write([*color]).ok();
                current_state = LED_CMD_CH.receive().await;
            }
            LedState::Blink(color, interval) => {
                // Blink once
                let _ = led.write([*color]).ok();
                Timer::after(Duration::from_millis(*interval / 2)).await;
                let _ = led.write([RGB { r: 0, g: 0, b: 0 }]).ok();
                Timer::after(Duration::from_millis(*interval / 2)).await;

                if let Ok(new_state) = LED_CMD_CH.try_receive() {
                    current_state = new_state;
                }
            }
        }
    }
}
