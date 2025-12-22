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

use crate::button::*;

const DISPLAY_HEIGHT: i32 = 32;
const LINE_HEIGHT: i32 = 10;
const TITLE_HEIGHT: i32 = 10;
const VISIBLE_LINES: usize = ((DISPLAY_HEIGHT - TITLE_HEIGHT) / LINE_HEIGHT) as usize;

pub enum MenuAction {
    Enter(&'static Menu),
    Trigger(MenuCommand),
}


#[derive(Copy, Clone)]
pub enum MenuCommand {
    BleScan,
    WifiScan,
    ToggleBluetooth,
    Reboot,
    EnterDynamic(&'static Menu), // for dynamically created menus
}

pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
}

pub struct Menu {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

pub static SETTINGS_MENU_ITEMS: [MenuItem; 1] = [
    MenuItem {
        label: "Bluetooth",
        action: MenuAction::Trigger(MenuCommand::ToggleBluetooth),
    },
];

pub static SETTINGS_MENU: Menu = Menu {
    title: "Settings",
    items: &SETTINGS_MENU_ITEMS,
};

pub static RADIO_MENU_ITEMS: [MenuItem; 2] = [
    MenuItem {
        label: "BLE Scan",
        action: MenuAction::Trigger(MenuCommand::BleScan),
    },
    MenuItem {
        label: "WiFi Scan",
        action: MenuAction::Trigger(MenuCommand::WifiScan),
    },
];

pub static RADIO_MENU: Menu = Menu {
    title: "Radio Test",
    items: &RADIO_MENU_ITEMS,
};

pub static ROOT_MENU_ITEMS: [MenuItem; 3] = [
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
];

pub static ROOT_MENU: Menu = Menu {
    title: "Main Menu",
    items: &ROOT_MENU_ITEMS,
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

pub static MENU_CMD_CH: Channel<
    CriticalSectionRawMutex,
    MenuCommand,
    4,
> = Channel::new();

type Display = ssd1306::Ssd1306<ssd1306::prelude::I2CInterface<esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

#[embassy_executor::task]
pub async fn menu_task(mut display: Display) {
    const DISPLAY_HEIGHT: i32 = 32;
    const TITLE_HEIGHT: i32 = 8;
    const LINE_HEIGHT: i32 = 8;
    const VISIBLE_LINES: usize = ((DISPLAY_HEIGHT - TITLE_HEIGHT) / LINE_HEIGHT) as usize;

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
                if state.selected > 0 {
                    state.selected -= 1;
                } else {
                    state.selected = state.current().items.len() - 1; // wrap around
                }
            }
            ButtonEvent::Down => {
                if state.selected + 1 < state.current().items.len() {
                    state.selected += 1;
                } else {
                    state.selected = 0; // wrap around
                }
            }
            ButtonEvent::Select => {
                match state.current().items[state.selected].action {
                    MenuAction::Enter(sub) => state.enter(sub),
                    MenuAction::Trigger(cmd) => match cmd {
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
        if state.selected < state.scroll {
            state.scroll = state.selected;
        } else if state.selected >= state.scroll + VISIBLE_LINES {
            state.scroll = state.selected - (VISIBLE_LINES - 1);
        }

        if state.current().items.len() <= VISIBLE_LINES {
            state.scroll = 0;
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

    const TITLE_HEIGHT: i32 = 8;
    const LINE_HEIGHT: i32 = 8;

    for i in 0..visible_lines {
        let idx = state.scroll + i;
        if idx >= menu.items.len() {
            break;
        }
        let y = TITLE_HEIGHT + i as i32 * LINE_HEIGHT;

        if idx == state.selected {
            Rectangle::new(Point::new(0, y), Size::new(128, LINE_HEIGHT as u32))
                .into_styled(PrimitiveStyleBuilder::new().fill_color(BinaryColor::On).build())
                .draw(display)
                .unwrap();

            Text::with_baseline(menu.items[idx].label, Point::new(0, y), inverted, Baseline::Top)
                .draw(display)
                .unwrap();
        } else {
            Text::with_baseline(menu.items[idx].label, Point::new(0, y), normal, Baseline::Top)
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
            // MenuCommand::BleScan => {
            //     info!("Starting BLE scan...");
            //     // you can spawn the scan task here or handle BLE scan logic
            // }
            // MenuCommand::WifiScan => {
            //     info!("Starting WiFi scan...");
            //     // you can spawn the WiFi scan task here or handle WiFi scan logic
            // }
            MenuCommand::ToggleBluetooth => {
                info!("Toggling Bluetooth");
                // toggle BT hardware
            }
            MenuCommand::Reboot => {
                esp_hal::system::software_reset();
            }
            MenuCommand::EnterDynamic(menu) => {
                // push the dynamic menu onto the stack
                // if you want, you can also notify the menu_task via a separate channel
                // but since menu_task now handles EnterDynamic in ButtonEvent::Select, you may not need extra handling here
                info!("Dynamic menu ready: {}", menu.title);
            }
            _ => {}
        }
    }
}

fn create_dynamic_menu(title: &'static str, labels: &[&'static str]) -> &'static Menu {
    let items: Vec<MenuItem> = labels
        .iter()
        .map(|label| MenuItem {
            label,
            action: MenuAction::Trigger(MenuCommand::WifiScan), // can change to connect or info
        })
        .collect();

    Box::leak(Box::new(Menu {
        title,
        items: Box::leak(items.into_boxed_slice()),
    }))
}

#[embassy_executor::task]
pub async fn ble_scan_task() {
    loop {
        match MENU_CMD_CH.receive().await {
            MenuCommand::BleScan => {
                info!("Starting BLE scan...");

                // pseudo-code: scan for BLE devices
                // let devices = esp_radio::ble::scan().await; // returns Vec<Device>
                // let labels: Vec<&'static str> = devices.iter()
                //     .map(|d| Box::leak(d.name_or_address().to_string().into_boxed_str()))
                //     .collect();
                //
                // if labels.is_empty() {
                //     continue;
                // }
                //
                // let dyn_menu = create_dynamic_menu("BLE Devices", &labels);
                //
                // // send dynamic menu command to menu task
                // MENU_CMD_CH.send(MenuCommand::EnterDynamic(dyn_menu)).await;
            }
            _ => {}
        }
    }
}

#[embassy_executor::task]
pub async fn wifi_scan_task(wifi_controller: &'static mut WifiController<'static>) {
    loop {
        match MENU_CMD_CH.receive().await {
            MenuCommand::WifiScan => {
                info!("Starting WiFi scan...");

                // perform scan
                let scan_result = match wifi_controller.scan_with_config(ScanConfig::default()) {
                    Ok(list) => list,
                    Err(_) => {
                        info!("WiFi scan failed");
                        continue;
                    }
                };

                info!("Found {} access points", scan_result.len());

                if scan_result.is_empty() {
                    continue;
                }

                // build dynamic menu items
                let mut labels: Vec<&'static str> = Vec::new();
                for ap in scan_result {
                    info!(
                        "SSID: {} | RSSI: {} dBm | CH: {} | Auth: {:?}",
                        ap.ssid.as_str(),
                        ap.signal_strength,
                        ap.channel,
                        ap.auth_method
                    );

                    // convert SSID to a 'static str for menu
                    let ssid_static: &'static str =
                        Box::leak(ap.ssid.clone().into_boxed_str());
                    labels.push(ssid_static);
                }

                // create dynamic menu
                let dyn_menu = create_dynamic_menu("WiFi Networks", &labels);

                // send to menu task
                MENU_CMD_CH.send(MenuCommand::EnterDynamic(dyn_menu)).await;
            }
            _ => {}
        }
    }
}
