use crate::config::{AppConfig, DiscordChannel};
use serde::{Deserialize, Serialize};

/// Messages sent from the injected Rustcord UI (JS) to Rust backend.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum FrontendCmd {
    Ready,
    SaveConfig {
        config: AppConfig,
    },
    FlushRam,
    HibernateNow,
    SwitchProfile {
        id: String,
    },
    CreateProfile {
        name: String,
        color: String,
        channel: DiscordChannel,
        note: String,
    },
    UpdateProfile {
        id: String,
        name: String,
        color: String,
        channel: DiscordChannel,
        note: String,
    },
    DeleteProfile {
        id: String,
    },
    CleanCache {
        id: String,
    },
    TogglePin,
    SetOpacity {
        opacity: u8,
    },
    ToggleTopBar {
        visible: bool,
    },
    OpenDataFolder,
    OpenDevTools,
    ReloadPage,
    ResetConfig,
    VoiceState {
        active: bool,
    },
}

/// Live telemetry & status pushed from Rust backend to the injected Rustcord UI.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStats {
    pub total_mb: f64,
    pub host_mb: f64,
    pub webview_mb: f64,
    pub private_mb: f64,
    pub peak_mb: f64,
    pub last_freed_mb: f64,
    pub total_freed_mb: f64,
    pub process_count: usize,
    pub blocked_requests: u64,
    pub startup_ms: u64,
    pub binary_size_mb: f64,
    pub in_voice: bool,
    pub is_suspended: bool,
    pub data_dir: String,
    pub profile_sizes: Vec<ProfileSizeInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileSizeInfo {
    pub id: String,
    pub disk_mb: f64,
}
