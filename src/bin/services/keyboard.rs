
use crate::services::hid::*;

pub fn ascii_to_key(c: u8) -> (u8, u8) {
    match c {
        b'a'..=b'z' => (0, 0x04 + (c - b'a')),
        b'A'..=b'Z' => (MOD_SHIFT, 0x04 + (c - b'A')),

        b'1' => (0, 0x1E),
        b'2' => (0, 0x1F),
        b'3' => (0, 0x20),
        b'4' => (0, 0x21),
        b'5' => (0, 0x22),
        b'6' => (0, 0x23),
        b'7' => (0, 0x24),
        b'8' => (0, 0x25),
        b'9' => (0, 0x26),
        b'0' => (0, 0x27),

        b'!' => (MOD_SHIFT, 0x1E),
        b'@' => (MOD_SHIFT, 0x1F),
        b'#' => (MOD_SHIFT, 0x20),
        b'$' => (MOD_SHIFT, 0x21),
        b'%' => (MOD_SHIFT, 0x22),
        b'^' => (MOD_SHIFT, 0x23),
        b'&' => (MOD_SHIFT, 0x24),
        b'*' => (MOD_SHIFT, 0x25),
        b'(' => (MOD_SHIFT, 0x26),
        b')' => (MOD_SHIFT, 0x27),

        b' ' => (0, KEY_SPACE),
        b'\n' => (0, KEY_ENTER),

        b'.' => (0, 0x37),
        b'>' => (MOD_SHIFT, 0x37),

        b',' => (0, 0x36),
        b'<' => (MOD_SHIFT, 0x36),

        b'/' => (0, 0x38),
        b'?' => (MOD_SHIFT, 0x38),

        b'-' => (0, 0x2D),
        b'_' => (MOD_SHIFT, 0x2D),

        b'=' => (0, 0x2E),
        b'+' => (MOD_SHIFT, 0x2E),

        b';' => (0, 0x33),
        b':' => (MOD_SHIFT, 0x33),

        b'\'' => (0, 0x34),
        b'"' => (MOD_SHIFT, 0x34),

        b'[' => (0, 0x2F),
        b'{' => (MOD_SHIFT, 0x2F),

        b']' => (0, 0x30),
        b'}' => (MOD_SHIFT, 0x30),

        b'\\' => (0, 0x31),
        b'|' => (MOD_SHIFT, 0x31),

        _ => (0, 0),
    }
}
