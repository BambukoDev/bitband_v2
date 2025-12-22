use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use esp_hal::gpio;

const DEBOUNCE_MS: u64 = 30;
const LONG_PRESS_MS: u64 = 600;
const POLL_MS: u64 = 10;

#[derive(Copy, Clone, Debug)]
pub enum ButtonEvent {
    Up,
    Down,
    Select,
    Back,
}


pub static BUTTON_CH: Channel<CriticalSectionRawMutex, ButtonEvent, 4> =
    Channel::new();

#[embassy_executor::task]
pub async fn button_task(
    mut up: gpio::Input<'static>,
    mut down: gpio::Input<'static>,
    mut select: gpio::Input<'static>
) {
    loop {
        if up.is_low() {
            BUTTON_CH.send(ButtonEvent::Up).await;
            Timer::after(Duration::from_millis(200)).await;
        }
        if down.is_low() {
            BUTTON_CH.send(ButtonEvent::Down).await;
            Timer::after(Duration::from_millis(200)).await;
        }
        if select.is_low() {
            Timer::after(Duration::from_millis(DEBOUNCE_MS)).await;
            if select.is_low() {
                handle_select_press(&mut select).await;
            }
        }

        Timer::after(Duration::from_millis(POLL_MS)).await;
        Timer::after(Duration::from_millis(10)).await;
    }
}

async fn handle_select_press(btn: &mut gpio::Input<'static>) {
    let mut elapsed = 0;
    let mut long_sent = false;

    while btn.is_low() {
        Timer::after(Duration::from_millis(POLL_MS)).await;
        elapsed += POLL_MS;

        if elapsed >= LONG_PRESS_MS && !long_sent {
            BUTTON_CH.send(ButtonEvent::Back).await;
            long_sent = true;
        }
    }

    // released
    if !long_sent {
        BUTTON_CH.send(ButtonEvent::Select).await;
    }
}
