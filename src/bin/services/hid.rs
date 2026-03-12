use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub const MOD_CTRL: u8  = 0x01;
pub const MOD_SHIFT: u8 = 0x02;
pub const MOD_ALT: u8   = 0x04;
pub const MOD_GUI: u8   = 0x08;

pub const KEY_ENTER: u8      = 0x28;
pub const KEY_ESC: u8        = 0x29;
pub const KEY_BACKSPACE: u8  = 0x2A;
pub const KEY_TAB: u8        = 0x2B;
pub const KEY_SPACE: u8      = 0x2C;
pub const KEY_DELETE: u8     = 0x4C;

pub const KEY_RIGHT: u8 = 0x4F;
pub const KEY_LEFT: u8  = 0x50;
pub const KEY_DOWN: u8  = 0x51;
pub const KEY_UP: u8    = 0x52;

pub const KEY_F1: u8  = 0x3A;
pub const KEY_F2: u8  = 0x3B;
pub const KEY_F3: u8  = 0x3C;
pub const KEY_F4: u8  = 0x3D;
pub const KEY_F5: u8  = 0x3E;
pub const KEY_F6: u8  = 0x3F;
pub const KEY_F7: u8  = 0x40;
pub const KEY_F8: u8  = 0x41;
pub const KEY_F9: u8  = 0x42;
pub const KEY_F10: u8 = 0x43;
pub const KEY_F11: u8 = 0x44;
pub const KEY_F12: u8 = 0x45;
// Note: These vary by OS/Descriptor
pub const KEY_MUTE: u8          = 0x7F; 
pub const KEY_VOLUME_UP: u8     = 0x80;
pub const KEY_VOLUME_DOWN: u8   = 0x81;
pub const KEY_MEDIA_PLAYPAUSE: u8 = 0xE8;
pub const KEY_MEDIA_STOP: u8      = 0xE9;
pub const KEY_MEDIA_NEXTSONG: u8  = 0xEA;
pub const KEY_MEDIA_PREVSONG: u8  = 0xEB;

pub struct KeyState {
    pub modifier: u8,
    pub keys: [u8; 6],
}

impl KeyState {
    pub const fn new() -> Self {
        Self { modifier: 0, keys: [0; 6] }
    }
}


#[derive(Copy, Clone)]
pub struct KeyReport {
    pub modifier: u8,
    pub keys: [u8; 6],
}

pub static HID_CH: Channel<
    CriticalSectionRawMutex,
    KeyReport,
    8,
> = Channel::new();

pub const HID_REPORT_MAP: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01,
    0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7,
    0x15, 0x00, 0x25, 0x01, 0x75, 0x01,
    0x95, 0x08, 0x81, 0x02,
    0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
    0x95, 0x06, 0x75, 0x08, 0x15, 0x00,
    0x25, 0x65, 0x05, 0x07, 0x19, 0x00,
    0x29, 0x65, 0x81, 0x00,
    0xC0,
];
