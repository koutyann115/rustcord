pub const ICON_RGBA_32: &[u8] = include_bytes!("../assets/rustcord_32.rgba");

pub fn window_icon() -> Option<tao::window::Icon> {
    tao::window::Icon::from_rgba(ICON_RGBA_32.to_vec(), 32, 32).ok()
}

pub fn tray_icon() -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(ICON_RGBA_32.to_vec(), 32, 32).ok()
}
