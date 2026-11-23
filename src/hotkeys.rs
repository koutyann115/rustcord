use crate::config::GamerConfig;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ShowHide,
    AlwaysOnTop,
    FlushRam,
    MicMute,
    Hibernate,
}

pub struct HotkeyController {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, HotkeyAction)>,
}

impl HotkeyController {
    pub fn new(cfg: &GamerConfig) -> Self {
        let manager = GlobalHotKeyManager::new().ok();
        let mut ctrl = Self {
            manager,
            registered: Vec::new(),
        };
        ctrl.sync(cfg);
        ctrl
    }

    pub fn sync(&mut self, cfg: &GamerConfig) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        // Unregister existing hotkeys first
        for (hk, _) in self.registered.drain(..) {
            let _ = manager.unregister(hk);
        }

        if !cfg.hotkeys_enabled {
            return;
        }

        let mods = Modifiers::CONTROL | Modifiers::ALT;
        let candidates = [
            (cfg.hotkey_show_hide, Code::KeyD, HotkeyAction::ShowHide),
            (cfg.hotkey_always_on_top, Code::KeyT, HotkeyAction::AlwaysOnTop),
            (cfg.hotkey_flush_ram, Code::KeyF, HotkeyAction::FlushRam),
            (cfg.hotkey_mic_mute, Code::KeyM, HotkeyAction::MicMute),
            (cfg.hotkey_hibernate, Code::KeyH, HotkeyAction::Hibernate),
        ];

        for (enabled, code, action) in candidates {
            if !enabled {
                continue;
            }
            let hk = HotKey::new(Some(mods), code);
            if manager.register(hk).is_ok() {
                self.registered.push((hk, action));
            }
        }
    }

    pub fn resolve(&self, hotkey_id: u32) -> Option<HotkeyAction> {
        self.registered
            .iter()
            .find(|(hk, _)| hk.id() == hotkey_id)
            .map(|(_, act)| *act)
    }
}
