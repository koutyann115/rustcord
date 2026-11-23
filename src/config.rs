use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscordChannel {
    #[default]
    Stable,
    Ptb,
    Canary,
}

impl DiscordChannel {
    pub fn url(self) -> &'static str {
        match self {
            DiscordChannel::Stable => "https://discord.com/app",
            DiscordChannel::Ptb => "https://ptb.discord.com/app",
            DiscordChannel::Canary => "https://canary.discord.com/app",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiscordChannel::Stable => "Stable",
            DiscordChannel::Ptb => "PTB",
            DiscordChannel::Canary => "Canary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrayMode {
    #[default]
    SmartTrim,
    DeepSuspend,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    #[default]
    Default,
    OledBlack,
    RustMidnight,
    CompactDark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub channel: DiscordChannel,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceConfig {
    pub lite_mode: bool,
    pub block_telemetry: bool,
    pub block_typing: bool,
    pub disable_gif_autoplay: bool,
    pub tray_mode: TrayMode,
    pub auto_trim_minutes: u32,
    pub auto_guard_mb: u32,
    pub js_heap_limit_mb: u32,
    pub hardware_acceleration: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            lite_mode: true,
            block_telemetry: true,
            block_typing: false,
            disable_gif_autoplay: true,
            tray_mode: TrayMode::SmartTrim,
            auto_trim_minutes: 5,
            auto_guard_mb: 350,
            js_heap_limit_mb: 384,
            hardware_acceleration: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: ThemePreset,
    pub hide_nitro_shop: bool,
    pub hide_gift_button: bool,
    pub hide_now_playing: bool,
    pub hide_help_button: bool,
    pub hide_boost_banner: bool,
    pub hide_sticker_gif_buttons: bool,
    pub auto_collapse_members: bool,
    pub show_top_bar: bool,
    pub custom_css: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreset::Default,
            hide_nitro_shop: true,
            hide_gift_button: true,
            hide_now_playing: false,
            hide_help_button: true,
            hide_boost_banner: true,
            hide_sticker_gif_buttons: false,
            auto_collapse_members: false,
            show_top_bar: true,
            custom_css: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GamerConfig {
    pub always_on_top: bool,
    pub window_opacity: u8,
    pub show_ram_in_title: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub hotkeys_enabled: bool,
    pub hotkey_show_hide: bool,
    pub hotkey_always_on_top: bool,
    pub hotkey_flush_ram: bool,
    pub hotkey_mic_mute: bool,
    pub hotkey_hibernate: bool,
    pub mute_beep: bool,
}

impl Default for GamerConfig {
    fn default() -> Self {
        Self {
            always_on_top: false,
            window_opacity: 100,
            show_ram_in_title: true,
            close_to_tray: true,
            start_minimized: false,
            hotkeys_enabled: true,
            hotkey_show_hide: true,
            hotkey_always_on_top: true,
            hotkey_flush_ram: true,
            hotkey_mic_mute: true,
            hotkey_hibernate: true,
            mute_beep: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometry {
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 820,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub active_profile_id: String,
    pub profiles: Vec<Profile>,
    pub performance: PerformanceConfig,
    pub appearance: AppearanceConfig,
    pub gamer: GamerConfig,
    pub window: WindowGeometry,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_profile_id: "main".to_string(),
            profiles: vec![Profile {
                id: "main".to_string(),
                name: "メイン".to_string(),
                color: "#5865F2".to_string(),
                channel: DiscordChannel::Stable,
                note: "メインアカウント".to_string(),
            }],
            performance: PerformanceConfig::default(),
            appearance: AppearanceConfig::default(),
            gamer: GamerConfig::default(),
            window: WindowGeometry::default(),
        }
    }
}

impl AppConfig {
    pub fn data_dir() -> PathBuf {
        let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("Rustcord")
    }

    pub fn config_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    pub fn profiles_dir() -> PathBuf {
        Self::data_dir().join("profiles")
    }

    pub fn profile_dir(profile_id: &str) -> PathBuf {
        Self::profiles_dir().join(sanitize_id(profile_id))
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(mut cfg) = serde_json::from_str::<AppConfig>(&raw) {
                cfg.ensure_valid();
                return cfg;
            }
        }
        let cfg = Self::default();
        let _ = cfg.save();
        cfg
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::data_dir();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(Self::config_path(), json).map_err(|e| e.to_string())
    }

    pub fn ensure_valid(&mut self) {
        if self.profiles.is_empty() {
            self.profiles = Self::default().profiles;
        }
        if !self.profiles.iter().any(|p| p.id == self.active_profile_id) {
            self.active_profile_id = self.profiles[0].id.clone();
        }
        self.gamer.window_opacity = self.gamer.window_opacity.clamp(45, 100);
    }

    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
            .unwrap_or(&self.profiles[0])
    }

    pub fn add_profile(
        &mut self,
        name: String,
        color: String,
        channel: DiscordChannel,
        note: String,
    ) -> Profile {
        let base_id = format!("profile-{}", chrono_like_stamp());
        let mut id = base_id.clone();
        let mut n = 1;
        while self.profiles.iter().any(|p| p.id == id) {
            id = format!("{base_id}-{n}");
            n += 1;
        }
        let profile = Profile {
            id,
            name: if name.trim().is_empty() {
                "サブアカウント".to_string()
            } else {
                name.trim().to_string()
            },
            color: if color.starts_with('#') && (color.len() == 7 || color.len() == 4) {
                color
            } else {
                "#FF5E1A".to_string()
            },
            channel,
            note,
        };
        self.profiles.push(profile.clone());
        profile
    }

    pub fn delete_profile(&mut self, id: &str) -> Result<bool, String> {
        if self.profiles.len() <= 1 {
            return Err("最後の1つのプロファイルは削除できません".to_string());
        }
        let was_active = self.active_profile_id == id;
        self.profiles.retain(|p| p.id != id);
        self.ensure_valid();
        let dir = Self::profile_dir(id);
        if !was_active && dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        Ok(was_active)
    }

    pub fn profile_disk_usage_mb(profile_id: &str) -> f64 {
        let dir = Self::profile_dir(profile_id);
        let bytes = dir_size(&dir);
        (bytes as f64) / (1024.0 * 1024.0)
    }

    /// Deletes only WebView2 cache directories inside a profile while preserving
    /// Cookies, Local Storage, IndexedDB, and Session/Network auth state.
    pub fn clean_profile_cache(profile_id: &str) -> u64 {
        let root = Self::profile_dir(profile_id);
        let default_dir = root.join("EBWebView").join("Default");
        let cache_targets = [
            default_dir.join("Cache"),
            default_dir.join("Code Cache"),
            default_dir.join("GPUCache"),
            default_dir.join("DawnGraphiteCache"),
            default_dir.join("DawnWebGPUCache"),
            default_dir.join("GrShaderCache"),
            default_dir.join("GraphiteDawnCache"),
            default_dir.join("Service Worker").join("CacheStorage"),
            default_dir.join("Service Worker").join("ScriptCache"),
            root.join("EBWebView").join("ShaderCache"),
            root.join("EBWebView").join("GrShaderCache"),
        ];

        let mut freed: u64 = 0;
        for target in &cache_targets {
            if target.exists() {
                let before = dir_size(target);
                let _ = remove_dir_best_effort(target);
                let after = dir_size(target);
                freed = freed.saturating_add(before.saturating_sub(after));
            }
        }
        freed
    }
}

fn sanitize_id(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        "main".to_string()
    } else {
        cleaned
    }
}

fn chrono_like_stamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if let Ok(meta) = entry.metadata() {
            if meta.is_dir() {
                total = total.saturating_add(dir_size(&p));
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

fn remove_dir_best_effort(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let _ = remove_dir_best_effort(&p);
            let _ = fs::remove_dir(&p);
        } else {
            let _ = fs::remove_file(&p);
        }
    }
    let _ = fs::remove_dir(path);
    Ok(())
}
