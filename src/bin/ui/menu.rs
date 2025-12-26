use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
    signal::Signal,
};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::primitives::PrimitiveStyleBuilder;
use embedded_graphics::primitives::Rectangle;
use esp_radio::wifi::ScanConfig;
use esp_radio::wifi::WifiController;
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

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::vec;

use core::sync::atomic::{AtomicPtr, Ordering};

use crate::button::*;
use crate::top_bar::{TopBarMode, TOP_BAR_CH, draw_text_at};

const TITLE_HEIGHT: i32 = 8;
const LINE_HEIGHT: i32 = 8;
const DISPLAY_HEIGHT: i32 = 32;
const VISIBLE_LINES: usize =
    ((DISPLAY_HEIGHT - TITLE_HEIGHT) / LINE_HEIGHT) as usize;

pub enum MenuAction {
    Enter(&'static Menu),
    Trigger(MenuCommand),
    WifiAp(&'static WifiApInfo),
}

#[derive(Copy, Clone)]
pub enum MenuCommand {
    BleScan,
    WifiScan,
    WifiDeauthSelected,
    WifiClearSelected,
    ToggleBluetooth,
    Reboot,
    EnterDynamic(&'static Menu),
}

pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
}

pub struct Menu {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

pub static WIFI_ACTIONS_MENU: Menu = Menu {
    title: "WiFi Actions",
    items: &[
        MenuItem {
            label: "Connect",
            // action: MenuAction::Trigger(MenuCommand::WifiConnectSelected),
            action: MenuAction::Trigger(MenuCommand::Reboot),
        },
        MenuItem {
            label: "Deauth Test",
            // action: MenuAction::Trigger(MenuCommand::WifiDeauthSelected),
            action: MenuAction::Trigger(MenuCommand::WifiDeauthSelected),
        },
        MenuItem {
            label: "Clear Selection",
            // action: MenuAction::Trigger(MenuCommand::WifiClearSelected),
            action: MenuAction::Trigger(MenuCommand::WifiClearSelected),
        },
    ],
};

pub static SETTINGS_MENU: Menu = Menu {
    title: "Settings",
    items: &[
        MenuItem {
            label: "Bluetooth",
            action: MenuAction::Trigger(MenuCommand::ToggleBluetooth),
        },
    ],
};

pub static RADIO_MENU: Menu = Menu {
    title: "Radio Test",
    items: &[
        MenuItem {
            label: "BLE Scan",
            action: MenuAction::Trigger(MenuCommand::BleScan),
        },
        MenuItem {
            label: "WiFi Scan",
            action: MenuAction::Trigger(MenuCommand::WifiScan),
        },
    ],
};

pub static ROOT_MENU: Menu = Menu {
    title: "Main Menu",
    items: &[
        MenuItem {
            label: "WiFi Scan",
            action: MenuAction::Trigger(MenuCommand::WifiScan),
        },
        MenuItem {
            label: "WiFi Actions",
            action: MenuAction::Enter(&WIFI_ACTIONS_MENU),
        },
        MenuItem {
            label: "Settings",
            action: MenuAction::Enter(&SETTINGS_MENU),
        },
        MenuItem {
            label: "Radio Test",
            action: MenuAction::Enter(&RADIO_MENU),
        },
        MenuItem {
            label: "Reboot",
            action: MenuAction::Trigger(MenuCommand::Reboot),
        },
    ],
};

const MENU_DEPTH_MAX: usize = 4;

pub struct MenuState {
    pub stack: [&'static Menu; MENU_DEPTH_MAX],
    pub depth: usize,
    pub selected: usize,
    pub scroll: usize,
}

impl MenuState {
    pub fn new(root: &'static Menu) -> Self {
        let mut stack = [root; MENU_DEPTH_MAX];
        stack[0] = root;
        Self {
            stack,
            depth: 1,
            selected: 0,
            scroll: 0,
        }
    }

    pub fn current(&self) -> &'static Menu {
        self.stack[self.depth - 1]
    }

    pub fn enter(&mut self, menu: &'static Menu) {
        if self.depth < MENU_DEPTH_MAX {
            self.stack[self.depth] = menu;
            self.depth += 1;
            self.selected = 0;
            self.scroll = 0;
        }
    }

    pub fn back(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            self.selected = 0;
            self.scroll = 0;
        }
    }
}

pub enum MenuMsg {
    PushMenu(&'static Menu),
    UpdateTopBar(TopBarMode),
}

pub static MENU_MSG_CH: Channel<
    CriticalSectionRawMutex,
    MenuMsg,
    4,
> = Channel::new();

pub static MENU_CMD_CH: Channel<
    CriticalSectionRawMutex,
    MenuCommand,
    4,
> = Channel::new();

pub static WIFI_SCAN_CH: Channel<CriticalSectionRawMutex, (), 1> = Channel::new();

pub struct WifiApInfo {
    pub ssid: &'static str,
    pub rssi: i8,
    pub channel: u8,
    // pub auth: AuthMethod,
}

static SELECTED_WIFI_AP: AtomicPtr<WifiApInfo> =
    AtomicPtr::new(core::ptr::null_mut());

type Display = ssd1306::Ssd1306<ssd1306::prelude::I2CInterface<esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

#[embassy_executor::task]
pub async fn menu_task(mut display: Display) {
    let normal = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    let inverted = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::Off)
        .build();

    let mut state = MenuState::new(&ROOT_MENU);

    render_menu(&mut display, &state, normal, inverted, VISIBLE_LINES);

    loop {
        let evt = BUTTON_CH.receive().await;

        match evt {
            ButtonEvent::Up => {
                let len = state.current().items.len();
                if len == 0 {
                    state.selected = 0;
                } else if state.selected > 0 {
                    state.selected -= 1;
                } else {
                    state.selected = len - 1;
                }
            }
            ButtonEvent::Down => {
                let len = state.current().items.len();
                if len == 0 {
                    state.selected = 0;
                } else if state.selected + 1 < len {
                    state.selected += 1;
                } else {
                    state.selected = 0;
                }
            }
            ButtonEvent::Select => {
                match state.current().items[state.selected].action {
                    MenuAction::WifiAp(ap) => {
                        set_selected_ap(ap);
                    }
                    MenuAction::Enter(sub) => state.enter(sub),
                    MenuAction::Trigger(cmd) => match cmd {
                        MenuCommand::WifiScan => {
                            WIFI_SCAN_CH.send(()).await;
                        }
                        MenuCommand::EnterDynamic(menu) => state.enter(menu),
                        _ => {
                            MENU_CMD_CH.send(cmd).await;
                        }
                    },
                }
            }
            ButtonEvent::Back => state.back(),
            _ => {}
        }

        // scrolling logic
        normalize_menu_state(&mut state);

        if let Ok(msg) = MENU_MSG_CH.try_receive() {
            match msg {
                MenuMsg::PushMenu(menu) => state.enter(menu),
                MenuMsg::UpdateTopBar(info) => {
                    TOP_BAR_CH.send(info).await;
                }
            }
        }

        if let Some(MenuItem { action: MenuAction::WifiAp(ap), .. }) =
            state.current().items.get(state.selected)
        {
            let _ = MENU_MSG_CH.try_send(
                MenuMsg::UpdateTopBar(
                    TopBarMode::WifiAp {
                        ssid: ap.ssid,
                        rssi: ap.rssi,
                        channel: ap.channel,
                    }
                )
            );
        }

        render_menu(&mut display, &state, normal, inverted, VISIBLE_LINES);
    }
}

fn render_menu(
    display: &mut Display,
    state: &MenuState,
    normal: MonoTextStyle<'static, BinaryColor>,
    inverted: MonoTextStyle<'static, BinaryColor>,
    visible_lines: usize,
) {
    use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};

    display.clear_buffer();
    let menu = state.current();

    // title
    Text::with_baseline(menu.title, Point::new(0, 0), normal, Baseline::Top)
        .draw(display)
        .unwrap();

    for i in 0..visible_lines {
        let idx = state.scroll + i;
        if idx >= menu.items.len() {
            break;
        }
        let y = TITLE_HEIGHT + i as i32 * LINE_HEIGHT;

        let is_selected_ap = match (
            &menu.items[idx].action,
            get_selected_ap(),
        ) {
            (MenuAction::WifiAp(ap), Some(sel)) => core::ptr::eq(*ap, sel),
            _ => false,
        };

        let label = if is_selected_ap {
            alloc::format!("* {}", menu.items[idx].label)
        } else {
            alloc::format!("{}", menu.items[idx].label)
        };

        if idx == state.selected {

            Rectangle::new(Point::new(0, y), Size::new(128, LINE_HEIGHT as u32))
                .into_styled(PrimitiveStyleBuilder::new().fill_color(BinaryColor::On).build())
                .draw(display)
                .unwrap();

            Text::with_baseline(label.as_str(), Point::new(0, y), inverted, Baseline::Top)
                .draw(display)
                .unwrap();
        } else {
            Text::with_baseline(label.as_str(), Point::new(0, y), normal, Baseline::Top)
                .draw(display)
                .unwrap();
        }
    }

    display.flush().unwrap();
}

#[embassy_executor::task]
pub async fn radio_task() {
    loop {
        match MENU_CMD_CH.receive().await {
            MenuCommand::WifiDeauthSelected => {
                if let Some(ap) = get_selected_ap() {
                    wifi_deauth_test(ap).await;
                } else {
                    info!("No WiFi AP selected");
                }
            }

            MenuCommand::WifiClearSelected => {
                info!("WiFi selection cleared");
            }

            MenuCommand::ToggleBluetooth => {
                info!("Toggling Bluetooth");
            }

            MenuCommand::Reboot => {
                esp_hal::system::software_reset();
            }

            _ => {}
        }
    }
}

async fn wifi_deauth_test(ap: &'static WifiApInfo) {
    info!(
        "Deauth test requested for SSID='{}' CH={} RSSI={}",
        ap.ssid,
        ap.channel,
        ap.rssi
    );

    // 1️⃣ Switch radio to AP channel (allowed)
    // esp_wifi_set_channel(ap.channel, WIFI_SECOND_CHAN_NONE);

    // 2️⃣ Placeholder: we CANNOT enumerate stations on ESP32
    info!("ESP32 cannot enumerate stations of foreign APs");

    // 3️⃣ Placeholder loop for future injector
    for i in 0..5 {
        info!("(stub) Would send deauth burst {}", i);
        embassy_time::Timer::after_millis(100).await;
    }

    info!("Deauth test completed (stub)");
}

#[embassy_executor::task]
pub async fn ble_scan_task() {
    // loop {
    //     match MENU_CMD_CH.receive().await {
    //         MenuCommand::BleScan => {
    //             info!("Starting BLE scan...");
    //
    //             // pseudo-code: scan for BLE devices
    //             // let devices = esp_radio::ble::scan().await; // returns Vec<Device>
    //             // let labels: Vec<&'static str> = devices.iter()
    //             //     .map(|d| Box::leak(d.name_or_address().to_string().into_boxed_str()))
    //             //     .collect();
    //             //
    //             // if labels.is_empty() {
    //             //     continue;
    //             // }
    //             //
    //             // let dyn_menu = create_dynamic_menu("BLE Devices", &labels);
    //             //
    //             // // send dynamic menu command to menu task
    //             // MENU_CMD_CH.send(MenuCommand::EnterDynamic(dyn_menu)).await;
    //         }
    //         _ => {}
    //     }
    // }
}

#[embassy_executor::task]
pub async fn wifi_scan_task(
    wifi: &'static mut WifiController<'static>,
) {
    loop {
        WIFI_SCAN_CH.receive().await;

        let result = match wifi.scan_with_config(ScanConfig::default()) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut aps = Vec::new();

        for ap in result {
            let ssid: &'static str =
                Box::leak(ap.ssid.clone().into_boxed_str());

            aps.push(WifiApInfo {
                ssid,
                rssi: ap.signal_strength,
                channel: ap.channel,
                // auth: ap.auth_method,
            });
        }

        let menu = build_wifi_menu(aps);

        MENU_MSG_CH
            .send(MenuMsg::PushMenu(menu))
            .await;
    }
}

fn build_wifi_menu(aps: Vec<WifiApInfo>) -> &'static Menu {
    let ap_infos: &'static [WifiApInfo] =
        Box::leak(aps.into_boxed_slice());

    let items: Vec<MenuItem> = ap_infos
        .iter()
        .map(|ap| MenuItem {
            label: ap.ssid,
            action: MenuAction::WifiAp(ap),
        })
        .collect();

    Box::leak(Box::new(Menu {
        title: "WiFi Networks",
        items: Box::leak(items.into_boxed_slice()),
    }))
}

fn build_wifi_ap_action_menu(ap: &'static WifiApInfo) -> &'static Menu {
    let items = vec![
        MenuItem {
            label: "Connect",
            // action: MenuAction::Trigger(MenuCommand::WifiConnect(ap)),
            action: MenuAction::Trigger(MenuCommand::Reboot)
        },
        MenuItem {
            label: "Deauth Test",
            // action: MenuAction::Trigger(MenuCommand::WifiDeauth(ap)),
            action: MenuAction::Trigger(MenuCommand::Reboot)
        },
    ];

    Box::leak(Box::new(Menu {
        title: ap.ssid,
        items: Box::leak(items.into_boxed_slice()),
    }))
}

pub fn set_selected_ap(ap: &'static WifiApInfo) {
    SELECTED_WIFI_AP.store(ap as *const _ as *mut _, Ordering::Relaxed);
}

pub fn get_selected_ap() -> Option<&'static WifiApInfo> {
    let ptr = SELECTED_WIFI_AP.load(Ordering::Relaxed);
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr })
    }
}

fn normalize_menu_state(state: &mut MenuState) {
    let len = state.current().items.len();

    if len == 0 {
        state.selected = 0;
        state.scroll = 0;
        return;
    }

    if state.selected >= len {
        state.selected = len - 1;
    }

    if state.scroll > state.selected {
        state.scroll = state.selected;
    }

    if state.selected >= state.scroll + VISIBLE_LINES {
        state.scroll = state.selected + 1 - VISIBLE_LINES;
    }
}
