use embassy_sync::mutex::Mutex;
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
use static_cell::StaticCell;

use core::sync::atomic::{AtomicPtr, Ordering};

use crate::button::*;
use crate::top_bar::{TopBarMode, TOP_BAR_CH, draw_text_at};
use crate::services::ducky::DUCKY_CH;
use crate::ui::menu_core::*;
use crate::ui::file_browser::*;

// NEW CODE

const TITLE_HEIGHT: i32 = 8;
const LINE_HEIGHT: i32 = 8;
const DISPLAY_HEIGHT: i32 = 32;
const VISIBLE_LINES: usize =
    ((DISPLAY_HEIGHT - TITLE_HEIGHT) / LINE_HEIGHT) as usize;

const MENU_DEPTH_MAX: usize = 4;

pub struct MenuState {
    pub stack: [Option<&'static dyn MenuSource>; MENU_DEPTH_MAX],
    pub depth: usize,
    pub selected: usize,
    pub scroll: usize,
}

impl MenuState {
    pub fn new(root: &'static dyn MenuSource) -> Self {
        let mut stack = [None; MENU_DEPTH_MAX];
        stack[0] = Some(root);

        Self {
            stack,
            depth: 1,
            selected: 0,
            scroll: 0,
        }
    }

    pub fn current(&self) -> &dyn MenuSource {
        self.stack[self.depth - 1].unwrap()
    }

    pub fn enter(&mut self, menu: &'static dyn MenuSource) {
        if self.depth < MENU_DEPTH_MAX {
            self.stack[self.depth] = Some(menu);
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

    Text::with_baseline(menu.title(), Point::new(0, 0), normal, Baseline::Top)
        .draw(display)
        .unwrap();

    for i in 0..visible_lines {
        let idx = state.scroll + i;
        if idx >= menu.len() {
            break;
        }

        let y = TITLE_HEIGHT + i as i32 * LINE_HEIGHT;
        let label = menu.label(idx);

        if idx == state.selected {
            Rectangle::new(Point::new(0, y), Size::new(128, LINE_HEIGHT as u32))
                .into_styled(
                    PrimitiveStyleBuilder::new()
                        .fill_color(BinaryColor::On)
                        .build(),
                )
                .draw(display)
                .unwrap();

            Text::with_baseline(label, Point::new(0, y), inverted, Baseline::Top)
                .draw(display)
                .unwrap();
        } else {
            Text::with_baseline(label, Point::new(0, y), normal, Baseline::Top)
                .draw(display)
                .unwrap();
        }
    }

    display.flush().unwrap();
}

pub static SETTINGS_MENU: StaticMenu = StaticMenu {
    title: "Settings",
    items: &[
        MenuItem {
            label: "Bluetooth",
            action: MenuAction::Trigger(Action::ToggleBluetooth),
        },
    ],
};

// pub enum MenuMsg {
//     PushMenu(&'static Menu),
//     UpdateTopBar(TopBarMode),
// }

// pub static MENU_MSG_CH: Channel<
//     CriticalSectionRawMutex,
//     MenuMsg,
//     4,
// > = Channel::new();

pub static MENU_CMD_CH: Channel<
    CriticalSectionRawMutex,
    Action,
    4,
> = Channel::new();

type Display = ssd1306::Ssd1306<ssd1306::prelude::I2CInterface<esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>>, DisplaySize128x32, BufferedGraphicsMode<DisplaySize128x32>>;

#[embassy_executor::task]
pub async fn menu_task(mut display: Display, file_menu: &'static FileMenu) {
    let normal = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();

    let inverted = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::Off)
        .build();

    // 1. Get the runtime-initialized 'static reference to the file browser

    // 2. Create the items array on the stack (temporarily) 
    // and promote it to 'static using StaticCell
    static ITEMS_CELL: StaticCell<[MenuItem; 3]> = StaticCell::new();
    let items = ITEMS_CELL.uninit().write([
        MenuItem {
            label: "Payloads",
            action: MenuAction::Enter(file_menu),
        },
        MenuItem {
            label: "Settings",
            action: MenuAction::Enter(&SETTINGS_MENU),
        },
        MenuItem {
            label: "Reboot",
            action: MenuAction::Trigger(Action::Reboot),
        },
    ]);

    // 3. Create the Root Menu using the 'static items
    static ROOT_CELL: StaticCell<StaticMenu> = StaticCell::new();
    let root = ROOT_CELL.uninit().write(StaticMenu {
        title: "Main Menu",
        items,
    });
    let mut state = MenuState::new(root);

    render_menu(&mut display, &state, normal, inverted, VISIBLE_LINES);

    loop {
        let evt = BUTTON_CH.receive().await;
        let len = state.current().len();

        match evt {
            ButtonEvent::Up => {
                if len > 0 {
                    state.selected = (state.selected + len - 1) % len;
                }
            }

            ButtonEvent::Down => {
                if len > 0 {
                    state.selected = (state.selected + 1) % len;
                }
            }

            ButtonEvent::Select => {
                let action = state.current().action(state.selected);

                match action {
                    MenuAction::Enter(menu) => state.enter(menu),
                    MenuAction::Trigger(cmd) => {
                        MENU_CMD_CH.send(cmd).await;
                    }
                }
            }

            ButtonEvent::Back => state.back(),
        }

        // scrolling logic
        normalize_menu_state(&mut state);

        // if let Ok(msg) = MENU_MSG_CH.try_receive() {
        //     match msg {
        //         MenuMsg::PushMenu(menu) => state.enter(menu),
        //         MenuMsg::UpdateTopBar(info) => {
        //             TOP_BAR_CH.send(info).await;
        //         }
        //     }
        // }

        render_menu(&mut display, &state, normal, inverted, VISIBLE_LINES);
    }
}

#[embassy_executor::task]
pub async fn action_handler() {
    loop {
        match MENU_CMD_CH.receive().await {
            Action::RunDuck(file) => {
                DUCKY_CH.send(file).await;
            }

            Action::ToggleBluetooth => {
                info!("Toggling Bluetooth AP");
            }

            Action::Reboot => {
                esp_hal::system::software_reset();
            }
            _ => {}
        }
    }
}

fn normalize_menu_state(state: &mut MenuState) {
    let menu = state.current();
    let len = menu.len();

    if len == 0 {
        state.selected = 0;
        state.scroll = 0;
        return;
    }

    if state.selected >= len {
        state.selected = len - 1;
    }

    if state.selected < state.scroll {
        state.scroll = state.selected;
    }

    if state.selected >= state.scroll + VISIBLE_LINES {
        state.scroll = state.selected + 1 - VISIBLE_LINES;
    }
}
