use alloc::vec::Vec;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};

use crate::services::hid::*;
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

pub fn ascii_to_key(c: u8) -> (u8, u8) {
    match c {
        b'a'..=b'z' => (0, 0x04 + (c - b'a')),
        b'A'..=b'Z' => (MOD_SHIFT, 0x04 + (c - b'A')),
        b' ' => (0, KEY_SPACE),
        b'\n' => (0, KEY_ENTER),
        _ => (0, 0),
    }
}

// #[task]
// pub async fn ble_keyboard_task(
//     mut hid_input: Characteristic<AttributeServer<>>,
// ) {
//     loop {
//         let report = HID_CH.receive().await;
//         // Send input report notification
//         let data = [
//             report.modifier,
//             0x00,
//             report.keys[0],
//             report.keys[1],
//             report.keys[2],
//             report.keys[3],
//             report.keys[4],
//             report.keys[5],
//         ];
//
//         let _ = hid_input.notify(&data).await;
//         Timer::after(Duration::from_millis(8)).await;
//     }
// }
