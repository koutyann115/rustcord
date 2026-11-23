use crate::config::AppConfig;
use crate::icon;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

pub const ID_SHOW_HIDE: &str = "tray_show_hide";
pub const ID_FLUSH_RAM: &str = "tray_flush_ram";
pub const ID_LITE_MODE: &str = "tray_lite_mode";
pub const ID_ALWAYS_ON_TOP: &str = "tray_always_on_top";
pub const ID_HIBERNATE: &str = "tray_hibernate";
pub const ID_SETTINGS: &str = "tray_settings";
pub const ID_QUIT: &str = "tray_quit";
pub const PROFILE_PREFIX: &str = "tray_profile:";

pub struct AppTray {
    pub tray: TrayIcon,
}

impl AppTray {
    pub fn new(config: &AppConfig, ram_mb: f64, is_suspended: bool) -> Result<Self, String> {
        let menu = build_menu(config, ram_mb, is_suspended);
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(format_tooltip(config, ram_mb, is_suspended));

        if let Some(ic) = icon::tray_icon() {
            builder = builder.with_icon(ic);
        }

        let tray = builder.build().map_err(|e| format!("Tray init failed: {e}"))?;
        Ok(Self { tray })
    }

    /// Rebuilds the tray context menu AND updates tooltip. Call ONLY on actual state changes
    /// (profile/theme/mode/pin/suspend), never on periodic timers, so an open menu won't close.
    pub fn refresh_menu(&self, config: &AppConfig, ram_mb: f64, is_suspended: bool) {
        let menu = build_menu(config, ram_mb, is_suspended);
        self.tray.set_menu(Some(Box::new(menu)));
        self.update_tooltip(config, ram_mb, is_suspended);
    }

    /// Lightweight tooltip-only update safe to call on periodic timers.
    pub fn update_tooltip(&self, config: &AppConfig, ram_mb: f64, is_suspended: bool) {
        let _ = self
            .tray
            .set_tooltip(Some(format_tooltip(config, ram_mb, is_suspended)));
    }
}

fn format_tooltip(config: &AppConfig, ram_mb: f64, is_suspended: bool) -> String {
    let profile = config.active_profile();
    let state = if is_suspended {
        "休止中"
    } else if config.performance.lite_mode {
        "軽量ON"
    } else {
        "通常"
    };
    format!(
        "Rustcord [{}] | {:.0} MB | {}",
        profile.name, ram_mb, state
    )
}

fn build_menu(config: &AppConfig, ram_mb: f64, is_suspended: bool) -> Menu {
    let menu = Menu::new();
    let active = config.active_profile();

    let header = MenuItem::with_id(
        MenuId::new("tray_header"),
        format!("🦀 Rustcord — {} ({:.0} MB)", active.name, ram_mb),
        false,
        None,
    );
    let _ = menu.append(&header);
    let _ = menu.append(&PredefinedMenuItem::separator());

    let show_hide = MenuItem::with_id(
        MenuId::new(ID_SHOW_HIDE),
        "表示 / 隠す  (Ctrl+Alt+D)",
        true,
        None,
    );
    let flush = MenuItem::with_id(
        MenuId::new(ID_FLUSH_RAM),
        "🧹 今すぐメモリ解放  (Ctrl+Alt+F)",
        true,
        None,
    );
    let hibernate = MenuItem::with_id(
        MenuId::new(ID_HIBERNATE),
        if is_suspended {
            "⚡ 休止から復帰  (Ctrl+Alt+H)"
        } else {
            "💤 休止モードにする  (Ctrl+Alt+H)"
        },
        true,
        None,
    );

    let lite = CheckMenuItem::with_id(
        MenuId::new(ID_LITE_MODE),
        "⚡ 軽量モード",
        true,
        config.performance.lite_mode,
        None,
    );
    let pin = CheckMenuItem::with_id(
        MenuId::new(ID_ALWAYS_ON_TOP),
        "📌 最前面に固定  (Ctrl+Alt+T)",
        true,
        config.gamer.always_on_top,
        None,
    );

    let _ = menu.append(&show_hide);
    let _ = menu.append(&flush);
    let _ = menu.append(&hibernate);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&lite);
    let _ = menu.append(&pin);

    // Multi-account submenu
    let profile_sub = Submenu::new("👤 アカウント切替", true);
    for p in &config.profiles {
        let item = CheckMenuItem::with_id(
            MenuId::new(format!("{PROFILE_PREFIX}{}", p.id)),
            format!("{} ({})", p.name, p.channel.label()),
            true,
            p.id == config.active_profile_id,
            None,
        );
        let _ = profile_sub.append(&item);
    }
    let _ = menu.append(&profile_sub);

    let _ = menu.append(&PredefinedMenuItem::separator());
    let settings = MenuItem::with_id(
        MenuId::new(ID_SETTINGS),
        "⚙️ Rustcord 設定を開く",
        true,
        None,
    );
    let quit = MenuItem::with_id(MenuId::new(ID_QUIT), "終了", true, None);
    let _ = menu.append(&settings);
    let _ = menu.append(&quit);

    menu
}
