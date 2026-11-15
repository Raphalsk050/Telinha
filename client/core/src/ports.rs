//! The local UDP ports of calls and streams come from a window picked at random, not from a fixed
//! range. With a fixed range every computer and every attempt used the same port. On a router
//! that treats 50000-50100 differently, the other person's packet arriving before the first one
//! from here left a stuck entry, the way out moved to another port and the direct connection
//! never closed, attempt after attempt. Two computers in the same house also fought over the same
//! port on the router. A new window changes the pair of ports and escapes the stuck entry.

pub const WINDOW_SIZE: u16 = 20;
pub const FIRST_PORT: u16 = 51200;
pub const WINDOW_COUNT: u16 = 440;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortWindow {
    pub min: u16,
    pub max: u16,
}

fn window(index: u16) -> PortWindow {
    let min = FIRST_PORT + WINDOW_SIZE * index;
    PortWindow { min, max: min + WINDOW_SIZE - 1 }
}

/// A window different from `except`, the one in use now.
pub fn random_port_window(except: Option<PortWindow>) -> PortWindow {
    let bytes: [u8; 2] = crate::text::random_bytes();
    pick_port_window(u16::from_le_bytes(bytes) % WINDOW_COUNT, except)
}

fn pick_port_window(index: u16, except: Option<PortWindow>) -> PortWindow {
    let picked = window(index);
    if except.is_some_and(|except| except.min == picked.min) { window((index + 1) % WINDOW_COUNT) } else { picked }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_stay_in_range_and_never_repeat_the_one_in_use() {
        assert_eq!(pick_port_window(0, None), PortWindow { min: 51200, max: 51219 });
        assert_eq!(pick_port_window(439, None), PortWindow { min: 59980, max: 59999 });
        assert_eq!(pick_port_window(3, Some(window(3))), window(4));
        assert_eq!(pick_port_window(439, Some(window(439))), window(0));
        for _ in 0..200 {
            let picked = random_port_window(None);
            assert!(picked.min >= FIRST_PORT && picked.max <= 59999 && picked.max - picked.min == 19);
        }
    }
}
