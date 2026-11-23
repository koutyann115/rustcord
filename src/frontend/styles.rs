use crate::config::{AppConfig, ThemePreset};

/// Generates the full dynamic CSS injected into Discord's document based on
/// the current theme preset, lite mode, de-bloat toggles, top bar layout, and custom CSS.
pub fn build_discord_css(cfg: &AppConfig) -> String {
    let mut css = String::with_capacity(4096);

    // 1. Top bar spacing so Rustcord's sleek bar never covers or clips Discord's UI
    if cfg.appearance.show_top_bar {
        css.push_str(
            r#"
html, body {
  overflow: hidden !important;
  height: 100vh !important;
  margin: 0 !important;
  padding: 0 !important;
}
#app-mount {
  position: fixed !important;
  top: 30px !important;
  left: 0 !important;
  right: 0 !important;
  bottom: 0 !important;
  height: calc(100vh - 30px) !important;
  max-height: calc(100vh - 30px) !important;
  width: 100vw !important;
  overflow: hidden !important;
}
#app-mount > div,
#app-mount [class*="appAsidePanelWrapper_"],
#app-mount [class*="notAppAsidePanel_"],
#app-mount [class*="app_"],
#app-mount [class*="layers_"] {
  height: 100% !important;
  max-height: 100% !important;
}
#app-mount [class*="guilds_"] [class*="scroller_"] {
  padding-top: 10px !important;
}
"#,
        );
    } else {
        css.push_str(
            r#"
#app-mount {
  top: 0 !important;
  height: 100vh !important;
  max-height: 100vh !important;
}
"#,
        );
    }

    // 2. Theme preset (supports both classic & visual-refresh Discord CSS tokens)
    match cfg.appearance.theme {
        ThemePreset::Default => {}
        ThemePreset::OledBlack => {
            css.push_str(
                r#"
:root, .theme-dark, .theme-darker, .theme-midnight {
  --background-primary: #000000 !important;
  --background-secondary: #06070a !important;
  --background-secondary-alt: #040507 !important;
  --background-tertiary: #000000 !important;
  --background-floating: #0c0e14 !important;
  --background-surface-high: #0a0c11 !important;
  --background-surface-higher: #10131b !important;
  --background-surface-highest: #161a24 !important;
  --background-base-low: #050608 !important;
  --background-base-lower: #000000 !important;
  --background-base-lowest: #000000 !important;
  --bg-base-primary: #000000 !important;
  --bg-base-secondary: #06070a !important;
  --bg-base-tertiary: #000000 !important;
  --bg-surface-overlay: #0c0e14 !important;
  --bg-overlay-2: #08090d !important;
  --bg-overlay-3: #0c0e14 !important;
  --chat-background-default: #000000 !important;
  --home-background: #000000 !important;
  --channeltextarea-background: #0d0f15 !important;
  --input-background: #0d0f15 !important;
  --card-primary-bg: #0a0c10 !important;
  --border-subtle: rgba(255, 255, 255, 0.07) !important;
  --scrollbar-thin-thumb: #1e222e !important;
  --scrollbar-auto-thumb: #1e222e !important;
  --scrollbar-auto-track: #050608 !important;
}
[class*="chat_"], [class*="chatContent_"], [class*="sidebar_"],
[class*="panels_"], [class*="guilds_"], [class*="members_"],
[class*="container_"][class*="themed_"], [class*="privateChannels_"],
[class*="scroller_"][class*="scrollerBase_"] {
  background-color: var(--background-primary) !important;
}
[class*="sidebar_"], [class*="privateChannels_"] {
  background-color: #050609 !important;
  border-right: 1px solid rgba(255,255,255,0.06) !important;
}
[class*="guilds_"] {
  background-color: #000000 !important;
  border-right: 1px solid rgba(255,255,255,0.05) !important;
}
"#,
            );
        }
        ThemePreset::RustMidnight => {
            css.push_str(
                r#"
:root, .theme-dark, .theme-darker, .theme-midnight {
  --background-primary: #0f121b !important;
  --background-secondary: #0b0d14 !important;
  --background-secondary-alt: #080a10 !important;
  --background-tertiary: #07090f !important;
  --background-floating: #141826 !important;
  --background-base-low: #0e111a !important;
  --background-base-lower: #0a0c13 !important;
  --background-base-lowest: #07090f !important;
  --bg-base-primary: #0f121b !important;
  --bg-base-secondary: #0b0d14 !important;
  --bg-base-tertiary: #07090f !important;
  --chat-background-default: #0f121b !important;
  --home-background: #0f121b !important;
  --channeltextarea-background: #171b29 !important;
  --input-background: #171b29 !important;
  --brand-500: #ff6b2b !important;
  --brand-560: #e85a1c !important;
  --brand-600: #d14c12 !important;
  --brand-experiment: #ff6b2b !important;
  --scrollbar-thin-thumb: #252b40 !important;
  --scrollbar-auto-thumb: #252b40 !important;
  --scrollbar-auto-track: #0b0d14 !important;
}
[class*="modeSelected_"] [class*="link_"],
[class*="selected_"][class*="channel_"] {
  background: linear-gradient(90deg, rgba(255, 107, 43, 0.22), rgba(88, 101, 242, 0.14)) !important;
  border-left: 2px solid #ff6b2b !important;
}
"#,
            );
        }
        ThemePreset::CompactDark => {
            css.push_str(
                r#"
:root, .theme-dark, .theme-darker, .theme-midnight {
  --custom-channel-textarea-text-area-height: 40px !important;
}
[class*="messageListItem_"] [class*="groupStart_"] {
  margin-top: 0.45rem !important;
}
[class*="cozy_"][class*="wrapper_"] {
  padding-top: 1px !important;
  padding-bottom: 1px !important;
}
[class*="sidebar_"] {
  width: 214px !important;
}
[class*="form_"] {
  padding-left: 10px !important;
  padding-right: 10px !important;
  margin-top: -4px !important;
}
::-webkit-scrollbar {
  width: 6px !important;
  height: 6px !important;
}
"#,
            );
        }
    }

    // 3. Lite Mode (GPU & CPU saver: cuts transitions, animations, backdrop blur, off-screen paint)
    if cfg.performance.lite_mode {
        css.push_str(
            r#"
*, *::before, *::after {
  animation-duration: 0.001ms !important;
  animation-iteration-count: 1 !important;
  transition-duration: 0.001ms !important;
  scroll-behavior: auto !important;
  backdrop-filter: none !important;
  -webkit-backdrop-filter: none !important;
}
[class*="messageListItem_"] {
  content-visibility: auto;
  contain-intrinsic-size: auto 64px;
}
[class*="profileEffects_"], [class*="avatarDecoration_"], [class*="nameplate_"] {
  display: none !important;
}
"#,
        );
    }

    // 4. De-bloat toggles (works in Japanese & English Discord)
    if cfg.appearance.hide_nitro_shop {
        css.push_str(
            r#"
a[href="/store"],
a[href="/shop"],
a[href="/quest-home"],
a[href="/discovery/quests"],
[data-list-item-id*="___nitro"],
[data-list-item-id*="___shop"],
[data-list-item-id*="___quests"],
[class*="channel_"]:has(> [class*="interactive_"] a[href="/store"]),
[class*="channel_"]:has(> [class*="interactive_"] a[href="/shop"]),
[class*="channel_"]:has(> [class*="interactive_"] a[href="/quest-home"]),
[class*="shopCard_"],
[class*="questBar_"] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.hide_gift_button {
        css.push_str(
            r#"
[class*="channelTextArea_"] button[aria-label*="ギフト"],
[class*="channelTextArea_"] button[aria-label*="プレゼント"],
[class*="channelTextArea_"] button[aria-label*="gift" i] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.hide_now_playing {
        css.push_str(
            r#"
[class*="nowPlayingColumn_"] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.hide_help_button {
        css.push_str(
            r#"
a[href*="support.discord.com"],
[class*="toolbar_"] [aria-label="ヘルプ"],
[class*="toolbar_"] [aria-label="Help"] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.hide_boost_banner {
        css.push_str(
            r#"
[class*="upsellBanner_"],
[class*="nitroUpsell_"],
[class*="premiumUpsell_"],
[class*="boostGoal_"],
[class*="progressBarContainer_"][class*="clickable_"],
[class*="trialOffer_"],
[class*="tryItOutSection_"] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.hide_sticker_gif_buttons {
        css.push_str(
            r#"
[class*="channelTextArea_"] button[aria-label*="GIF"],
[class*="channelTextArea_"] button[aria-label*="スタンプ"],
[class*="channelTextArea_"] button[aria-label*="Sticker" i],
[class*="channelTextArea_"] [class*="app-launcher-entrypoint"] {
  display: none !important;
}
"#,
        );
    }

    if cfg.appearance.auto_collapse_members {
        css.push_str(
            r#"
[class*="membersWrap_"] {
  min-width: 56px !important;
  width: 56px !important;
  overflow: hidden !important;
  transition: width 140ms ease !important;
}
[class*="membersWrap_"]:hover {
  width: 240px !important;
}
"#,
        );
    }

    // 5. User custom CSS
    if !cfg.appearance.custom_css.trim().is_empty() {
        css.push_str("\n/* === Rustcord User Custom CSS === */\n");
        css.push_str(&cfg.appearance.custom_css);
        css.push('\n');
    }

    css
}
