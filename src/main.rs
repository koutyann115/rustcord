#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod frontend;
mod hotkeys;
mod icon;
mod ipc;
mod memory;
mod tray;
mod webview_host;

use config::{AppConfig, ThemePreset, TrayMode};
use frontend::styles::build_discord_css;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use hotkeys::{HotkeyAction, HotkeyController};
use ipc::{FrontendCmd, ProfileSizeInfo, RuntimeStats};
use memory::{ProcessTracker, TrimReport};
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Window, WindowBuilder};
use tray::AppTray;
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
use webview_host::{BlockerState, DiscordWebView, UserEvent};

fn main() {
    let boot_start = Instant::now();
    memory::cleanup_old_binary();

    // Prevent duplicate conflicting instances; if already running, wake & focus existing window
    #[cfg(windows)]
    let instance_guard = match memory::acquire_single_instance() {
        Some(g) => g,
        None => return,
    };

    let mut config = AppConfig::load();
    let args: Vec<String> = std::env::args().collect();
    let open_settings_on_start = args.iter().any(|a| a == "--settings");
    if let Some(pos) = args.iter().position(|a| a == "--profile") {
        if let Some(target) = args.get(pos + 1) {
            if let Some(found) = config
                .profiles
                .iter()
                .find(|p| p.id.eq_ignore_ascii_case(target) || p.name == *target)
            {
                config.active_profile_id = found.id.clone();
            }
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Wake this instance whenever a second Rustcord.exe launch is attempted
    #[cfg(windows)]
    {
        let p = proxy.clone();
        memory::start_wakeup_listener(&instance_guard, move || {
            let _ = p.send_event(UserEvent::WakeFromSecondInstance);
        });
    }

    // Wire zero-polling OS event handlers straight into the tao EventLoop
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = p.send_event(UserEvent::TrayMenu(event.id.0));
        }));
    }
    {
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = p.send_event(UserEvent::TrayClick);
            }
        }));
    }
    {
        let p = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let _ = p.send_event(UserEvent::GlobalHotkey(event.id));
            }
        }));
    }

    // Build main native window
    let mut wb = WindowBuilder::new()
        .with_title("Rustcord")
        .with_inner_size(LogicalSize::new(
            config.window.width.max(720),
            config.window.height.max(500),
        ))
        .with_min_inner_size(LogicalSize::new(640.0, 440.0))
        .with_always_on_top(config.gamer.always_on_top)
        .with_maximized(config.window.maximized)
        .with_visible(!config.gamer.start_minimized);

    if let (Some(x), Some(y)) = (config.window.x, config.window.y) {
        if (-4000..=8000).contains(&x) && (-2000..=4000).contains(&y) {
            wb = wb.with_position(LogicalPosition::new(x, y));
        }
    }
    if let Some(ic) = icon::window_icon() {
        wb = wb.with_window_icon(Some(ic));
    }

    let window = wb.build(&event_loop).expect("Failed to create Rustcord window");
    let hwnd = window.hwnd();
    apply_window_chrome(&window, hwnd, &config);

    let blocker = BlockerState::from_config(&config.performance);
    let mut discord_wv = Some(
        DiscordWebView::create(&window, &config, proxy.clone(), blocker.clone())
            .expect("Failed to initialize WebView2"),
    );

    let startup_ms = boot_start.elapsed().as_millis() as u64;
    let binary_size_mb = memory::current_binary_size_mb();

    let mut tracker = ProcessTracker::new();
    let mut hotkeys = HotkeyController::new(&config.gamer);
    let initial_snap = tracker.measure(discord_wv.as_ref().and_then(|w| w.browser_pid));
    let app_tray = AppTray::new(&config, initial_snap.total_mb, false).ok();

    // Background lightweight ticker thread (2.5s cadence for stats & auto-trim)
    {
        let p = proxy.clone();
        std::thread::spawn(move || {
            let mut ticks: u64 = 0;
            loop {
                std::thread::sleep(Duration::from_millis(2500));
                ticks = ticks.wrapping_add(1);
                if ticks == 3 {
                    // One-shot post-boot purge once initial Discord JS bundle parse settles
                    let _ = p.send_event(UserEvent::PostBootOptimize);
                }
                if p.send_event(UserEvent::TickStats).is_err() {
                    break;
                }
                if ticks % 24 == 0 {
                    // Every 60s check auto-trim interval
                    if p.send_event(UserEvent::AutoTrimTick).is_err() {
                        break;
                    }
                }
            }
        });
    }

    let mut window_visible = !config.gamer.start_minimized;
    let mut in_voice = false;
    let mut peak_mb = initial_snap.total_mb;
    let mut last_freed_mb = 0.0_f64;
    let mut total_freed_mb = 0.0_f64;
    let mut minutes_since_trim: u32 = 0;
    let mut last_guard_trim = Instant::now() - Duration::from_secs(120);
    let mut cached_profile_sizes = compute_profile_sizes(&config);
    let mut tick_counter: u64 = 0;
    let mut geometry_dirty = false;
    let mut pending_open_settings = open_settings_on_start;

    update_window_title(
        &window,
        &config,
        initial_snap.total_mb,
        blocker.is_suspended(),
    );

    event_loop.run(move |event, _, control_flow| {
        #[cfg(windows)]
        let _keep_guard_alive = &instance_guard;
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    save_window_geometry(&window, &mut config);
                    let _ = config.save();
                    geometry_dirty = false;
                    if config.gamer.close_to_tray && app_tray.is_some() {
                        hide_to_tray(
                            &window,
                            discord_wv.as_ref(),
                            &config,
                            in_voice,
                            &proxy,
                            &mut tracker,
                            &mut window_visible,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                    if window_visible && !window.is_minimized() {
                        save_window_geometry(&window, &mut config);
                        geometry_dirty = true;
                    }
                }
                _ => {}
            },

            Event::UserEvent(user_event) => match user_event {
                UserEvent::WakeFromSecondInstance => {
                    restore_from_tray(
                        &window,
                        hwnd,
                        discord_wv.as_ref(),
                        &config,
                        app_tray.as_ref(),
                        &mut tracker,
                        &mut window_visible,
                    );
                }

                UserEvent::IpcMessage(raw) => {
                    let Ok(cmd) = serde_json::from_str::<FrontendCmd>(&raw) else {
                        return;
                    };
                    match cmd {
                        FrontendCmd::Ready => {
                            cached_profile_sizes = compute_profile_sizes(&config);
                            if let Some(wv) = discord_wv.as_ref() {
                                wv.set_low_memory(config.performance.lite_mode);
                                push_config_to_js(wv, &config);
                                let snap = tracker.measure(wv.browser_pid);
                                peak_mb = peak_mb.max(snap.total_mb);
                                let stats = make_stats(
                                    snap,
                                    peak_mb,
                                    last_freed_mb,
                                    total_freed_mb,
                                    &blocker,
                                    startup_ms,
                                    binary_size_mb,
                                    in_voice,
                                    &cached_profile_sizes,
                                );
                                push_stats_to_js(wv, &stats);
                                if pending_open_settings {
                                    pending_open_settings = false;
                                    wv.eval("window.__rustcordBridge && window.__rustcordBridge.openSettings('perf');");
                                }
                            }
                        }

                        FrontendCmd::SaveConfig { config: mut new_cfg } => {
                            new_cfg.ensure_valid();
                            new_cfg.active_profile_id = config.active_profile_id.clone();
                            new_cfg.profiles = config.profiles.clone();
                            new_cfg.window = config.window.clone();

                            config = new_cfg;
                            let _ = config.save();
                            blocker.update(&config.performance);
                            hotkeys.sync(&config.gamer);
                            apply_window_chrome(&window, hwnd, &config);

                            if let Some(wv) = discord_wv.as_ref() {
                                wv.set_low_memory(config.performance.lite_mode);
                                push_config_to_js(wv, &config);
                                let snap = tracker.measure(wv.browser_pid);
                                update_window_title(
                                    &window,
                                    &config,
                                    snap.total_mb,
                                    blocker.is_suspended(),
                                );
                                if let Some(t) = app_tray.as_ref() {
                                    t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }

                        FrontendCmd::FlushRam => {
                            let report = perform_ram_flush(
                                discord_wv.as_ref(),
                                &config,
                                &mut tracker,
                                &mut last_freed_mb,
                                &mut total_freed_mb,
                            );
                            minutes_since_trim = 0;
                            last_guard_trim = Instant::now();

                            if let Some(wv) = discord_wv.as_ref() {
                                let msg = if report.freed_mb >= 1.0 {
                                    format!(
                                        "🧹 メモリ解放完了: {:.0} MB → {:.0} MB (-{:.0} MB / {}プロセス)",
                                        report.before_mb,
                                        report.after_mb,
                                        report.freed_mb,
                                        report.trimmed_processes
                                    )
                                } else {
                                    format!(
                                        "🧹 すでに最適化済みです（現在 {:.0} MB）",
                                        report.after_mb
                                    )
                                };
                                push_toast(wv, &msg, "ok");
                                let snap = tracker.measure(wv.browser_pid);
                                let stats = make_stats(
                                    snap,
                                    peak_mb,
                                    last_freed_mb,
                                    total_freed_mb,
                                    &blocker,
                                    startup_ms,
                                    binary_size_mb,
                                    in_voice,
                                    &cached_profile_sizes,
                                );
                                push_stats_to_js(wv, &stats);
                                update_window_title(
                                    &window,
                                    &config,
                                    snap.total_mb,
                                    blocker.is_suspended(),
                                );
                                if let Some(t) = app_tray.as_ref() {
                                    t.update_tooltip(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }

                        FrontendCmd::HibernateNow => {
                            if in_voice {
                                let rep = perform_ram_flush(
                                    discord_wv.as_ref(),
                                    &config,
                                    &mut tracker,
                                    &mut last_freed_mb,
                                    &mut total_freed_mb,
                                );
                                if let Some(wv) = discord_wv.as_ref() {
                                    push_toast(
                                        wv,
                                        &format!(
                                            "🎙 通話中のため休止は保護スキップ（メモリ解放 -{:.0} MB のみ実行）",
                                            rep.freed_mb
                                        ),
                                        "warn",
                                    );
                                }
                            } else {
                                hibernate_immediately(
                                    &window,
                                    discord_wv.as_ref(),
                                    &config,
                                    app_tray.as_ref(),
                                    &mut tracker,
                                    &mut window_visible,
                                    &mut last_freed_mb,
                                    &mut total_freed_mb,
                                );
                            }
                        }

                        FrontendCmd::SwitchProfile { id } => {
                            if id != config.active_profile_id
                                && config.profiles.iter().any(|p| p.id == id)
                            {
                                switch_profile(
                                    &id,
                                    &window,
                                    &mut config,
                                    &mut discord_wv,
                                    &proxy,
                                    &blocker,
                                    app_tray.as_ref(),
                                    &mut tracker,
                                    &mut in_voice,
                                    &mut cached_profile_sizes,
                                );
                            }
                        }

                        FrontendCmd::CreateProfile {
                            name,
                            color,
                            channel,
                            note,
                        } => {
                            let prof = config.add_profile(name, color, channel, note);
                            let _ = config.save();
                            cached_profile_sizes = compute_profile_sizes(&config);
                            if let Some(wv) = discord_wv.as_ref() {
                                push_config_to_js(wv, &config);
                                push_toast(
                                    wv,
                                    &format!("👤 プロファイル「{}」を追加しました", prof.name),
                                    "ok",
                                );
                                let snap = tracker.measure(wv.browser_pid);
                                if let Some(t) = app_tray.as_ref() {
                                    t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }

                        FrontendCmd::UpdateProfile {
                            id,
                            name,
                            color,
                            channel,
                            note,
                        } => {
                            if let Some(p) = config.profiles.iter_mut().find(|p| p.id == id) {
                                if !name.trim().is_empty() {
                                    p.name = name.trim().to_string();
                                }
                                p.color = color;
                                p.channel = channel;
                                p.note = note;
                                let _ = config.save();
                                if let Some(wv) = discord_wv.as_ref() {
                                    push_config_to_js(wv, &config);
                                    push_toast(wv, "プロファイルを更新しました", "ok");
                                    let snap = tracker.measure(wv.browser_pid);
                                    if let Some(t) = app_tray.as_ref() {
                                        t.refresh_menu(
                                            &config,
                                            snap.total_mb,
                                            blocker.is_suspended(),
                                        );
                                    }
                                }
                            }
                        }

                        FrontendCmd::DeleteProfile { id } => {
                            match config.delete_profile(&id) {
                                Ok(was_active) => {
                                    let _ = config.save();
                                    if was_active {
                                        let next_id = config.active_profile_id.clone();
                                        switch_profile(
                                            &next_id,
                                            &window,
                                            &mut config,
                                            &mut discord_wv,
                                            &proxy,
                                            &blocker,
                                            app_tray.as_ref(),
                                            &mut tracker,
                                            &mut in_voice,
                                            &mut cached_profile_sizes,
                                        );
                                        let old_dir = AppConfig::profile_dir(&id);
                                        let _ = std::fs::remove_dir_all(old_dir);
                                    } else {
                                        cached_profile_sizes = compute_profile_sizes(&config);
                                        if let Some(wv) = discord_wv.as_ref() {
                                            push_config_to_js(wv, &config);
                                            push_toast(wv, "🗑 プロファイルを削除しました", "info");
                                            let snap = tracker.measure(wv.browser_pid);
                                            if let Some(t) = app_tray.as_ref() {
                                                t.refresh_menu(
                                                    &config,
                                                    snap.total_mb,
                                                    blocker.is_suspended(),
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    if let Some(wv) = discord_wv.as_ref() {
                                        push_toast(wv, &err, "warn");
                                    }
                                }
                            }
                        }

                        FrontendCmd::CleanCache { id } => {
                            let is_active = id == config.active_profile_id;
                            let freed_bytes = if is_active {
                                discord_wv = None;
                                tracker.invalidate();
                                std::thread::sleep(Duration::from_millis(250));
                                let f = AppConfig::clean_profile_cache(&id);
                                discord_wv = DiscordWebView::create(
                                    &window,
                                    &config,
                                    proxy.clone(),
                                    blocker.clone(),
                                )
                                .ok();
                                f
                            } else {
                                AppConfig::clean_profile_cache(&id)
                            };
                            cached_profile_sizes = compute_profile_sizes(&config);
                            let freed_mb = (freed_bytes as f64) / (1024.0 * 1024.0);
                            if let Some(wv) = discord_wv.as_ref() {
                                push_toast(
                                    wv,
                                    &format!(
                                        "🧹 キャッシュを {:.1} MB 削除しました（ログイン維持）",
                                        freed_mb
                                    ),
                                    "ok",
                                );
                                let snap = tracker.measure(wv.browser_pid);
                                let stats = make_stats(
                                    snap,
                                    peak_mb,
                                    last_freed_mb,
                                    total_freed_mb,
                                    &blocker,
                                    startup_ms,
                                    binary_size_mb,
                                    in_voice,
                                    &cached_profile_sizes,
                                );
                                push_stats_to_js(wv, &stats);
                            }
                        }

                        FrontendCmd::TogglePin => {
                            config.gamer.always_on_top = !config.gamer.always_on_top;
                            let _ = config.save();
                            window.set_always_on_top(config.gamer.always_on_top);
                            if let Some(wv) = discord_wv.as_ref() {
                                push_config_to_js(wv, &config);
                                push_toast(
                                    wv,
                                    if config.gamer.always_on_top {
                                        "📌 常に最前面に表示 ON"
                                    } else {
                                        "📌 最前面固定 OFF"
                                    },
                                    "info",
                                );
                                let snap = tracker.measure(wv.browser_pid);
                                if let Some(t) = app_tray.as_ref() {
                                    t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }

                        FrontendCmd::SetOpacity { opacity } => {
                            config.gamer.window_opacity = opacity.clamp(45, 100);
                            memory::apply_window_opacity(hwnd, config.gamer.window_opacity);
                        }

                        FrontendCmd::ToggleTopBar { visible } => {
                            config.appearance.show_top_bar = visible;
                            let _ = config.save();
                            if let Some(wv) = discord_wv.as_ref() {
                                push_config_to_js(wv, &config);
                            }
                        }

                        FrontendCmd::OpenDataFolder => {
                            let _ = open::that(AppConfig::data_dir());
                        }

                        FrontendCmd::OpenDevTools => {
                            if let Some(wv) = discord_wv.as_ref() {
                                wv.open_devtools();
                            }
                        }

                        FrontendCmd::ReloadPage => {
                            if let Some(wv) = discord_wv.as_ref() {
                                wv.reload();
                            }
                        }

                        FrontendCmd::ResetConfig => {
                            let profiles = config.profiles.clone();
                            let active = config.active_profile_id.clone();
                            config = AppConfig::default();
                            config.profiles = profiles;
                            config.active_profile_id = active;
                            let _ = config.save();
                            blocker.update(&config.performance);
                            hotkeys.sync(&config.gamer);
                            apply_window_chrome(&window, hwnd, &config);
                            if let Some(wv) = discord_wv.as_ref() {
                                wv.set_low_memory(config.performance.lite_mode);
                                push_config_to_js(wv, &config);
                                push_toast(wv, "♻️ 設定を初期値に戻しました", "info");
                                let snap = tracker.measure(wv.browser_pid);
                                if let Some(t) = app_tray.as_ref() {
                                    t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }

                        FrontendCmd::VoiceState { active } => {
                            in_voice = active;
                        }
                    }
                }

                UserEvent::TrayClick => {
                    if window_visible && !window.is_minimized() {
                        hide_to_tray(
                            &window,
                            discord_wv.as_ref(),
                            &config,
                            in_voice,
                            &proxy,
                            &mut tracker,
                            &mut window_visible,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                    } else {
                        restore_from_tray(
                            &window,
                            hwnd,
                            discord_wv.as_ref(),
                            &config,
                            app_tray.as_ref(),
                            &mut tracker,
                            &mut window_visible,
                        );
                    }
                }

                UserEvent::TrayMenu(id) => {
                    if id == tray::ID_SHOW_HIDE {
                        if window_visible && !window.is_minimized() {
                            hide_to_tray(
                                &window,
                                discord_wv.as_ref(),
                                &config,
                                in_voice,
                                &proxy,
                                &mut tracker,
                                &mut window_visible,
                                &mut last_freed_mb,
                                &mut total_freed_mb,
                            );
                        } else {
                            restore_from_tray(
                                &window,
                                hwnd,
                                discord_wv.as_ref(),
                                &config,
                                app_tray.as_ref(),
                                &mut tracker,
                                &mut window_visible,
                            );
                        }
                    } else if id == tray::ID_FLUSH_RAM {
                        let rep = perform_ram_flush(
                            discord_wv.as_ref(),
                            &config,
                            &mut tracker,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                        minutes_since_trim = 0;
                        last_guard_trim = Instant::now();
                        if let Some(wv) = discord_wv.as_ref() {
                            if window_visible {
                                push_toast(
                                    wv,
                                    &format!(
                                        "🧹 メモリ解放: {:.0} MB → {:.0} MB (-{:.0} MB)",
                                        rep.before_mb, rep.after_mb, rep.freed_mb
                                    ),
                                    "ok",
                                );
                            }
                        }
                        if let Some(t) = app_tray.as_ref() {
                            t.update_tooltip(&config, rep.after_mb, blocker.is_suspended());
                        }
                    } else if id == tray::ID_LITE_MODE {
                        config.performance.lite_mode = !config.performance.lite_mode;
                        let _ = config.save();
                        if let Some(wv) = discord_wv.as_ref() {
                            wv.set_low_memory(config.performance.lite_mode);
                            push_config_to_js(wv, &config);
                            let snap = tracker.measure(wv.browser_pid);
                            update_window_title(
                                &window,
                                &config,
                                snap.total_mb,
                                blocker.is_suspended(),
                            );
                            if let Some(t) = app_tray.as_ref() {
                                t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                            }
                        }
                    } else if id == tray::ID_ALWAYS_ON_TOP {
                        config.gamer.always_on_top = !config.gamer.always_on_top;
                        let _ = config.save();
                        window.set_always_on_top(config.gamer.always_on_top);
                        if let Some(wv) = discord_wv.as_ref() {
                            push_config_to_js(wv, &config);
                            let snap = tracker.measure(wv.browser_pid);
                            if let Some(t) = app_tray.as_ref() {
                                t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                            }
                        }
                    } else if id == tray::ID_HIBERNATE {
                        if blocker.is_suspended() {
                            restore_from_tray(
                                &window,
                                hwnd,
                                discord_wv.as_ref(),
                                &config,
                                app_tray.as_ref(),
                                &mut tracker,
                                &mut window_visible,
                            );
                        } else if !in_voice {
                            hibernate_immediately(
                                &window,
                                discord_wv.as_ref(),
                                &config,
                                app_tray.as_ref(),
                                &mut tracker,
                                &mut window_visible,
                                &mut last_freed_mb,
                                &mut total_freed_mb,
                            );
                        }
                    } else if id == tray::ID_SETTINGS {
                        restore_from_tray(
                            &window,
                            hwnd,
                            discord_wv.as_ref(),
                            &config,
                            app_tray.as_ref(),
                            &mut tracker,
                            &mut window_visible,
                        );
                        if let Some(wv) = discord_wv.as_ref() {
                            wv.eval("window.__rustcordBridge && window.__rustcordBridge.openSettings('perf');");
                        }
                    } else if id == tray::ID_QUIT {
                        save_window_geometry(&window, &mut config);
                        let _ = config.save();
                        *control_flow = ControlFlow::Exit;
                    } else if let Some(prof_id) = id.strip_prefix(tray::PROFILE_PREFIX) {
                        restore_from_tray(
                            &window,
                            hwnd,
                            discord_wv.as_ref(),
                            &config,
                            app_tray.as_ref(),
                            &mut tracker,
                            &mut window_visible,
                        );
                        if prof_id != config.active_profile_id {
                            switch_profile(
                                prof_id,
                                &window,
                                &mut config,
                                &mut discord_wv,
                                &proxy,
                                &blocker,
                                app_tray.as_ref(),
                                &mut tracker,
                                &mut in_voice,
                                &mut cached_profile_sizes,
                            );
                        }
                    }
                }

                UserEvent::GlobalHotkey(hotkey_id) => {
                    let Some(action) = hotkeys.resolve(hotkey_id) else {
                        return;
                    };
                    match action {
                        HotkeyAction::ShowHide => {
                            if window_visible && !window.is_minimized() {
                                hide_to_tray(
                                    &window,
                                    discord_wv.as_ref(),
                                    &config,
                                    in_voice,
                                    &proxy,
                                    &mut tracker,
                                    &mut window_visible,
                                    &mut last_freed_mb,
                                    &mut total_freed_mb,
                                );
                            } else {
                                restore_from_tray(
                                    &window,
                                    hwnd,
                                    discord_wv.as_ref(),
                                    &config,
                                    app_tray.as_ref(),
                                    &mut tracker,
                                    &mut window_visible,
                                );
                            }
                        }
                        HotkeyAction::AlwaysOnTop => {
                            config.gamer.always_on_top = !config.gamer.always_on_top;
                            let _ = config.save();
                            window.set_always_on_top(config.gamer.always_on_top);
                            if let Some(wv) = discord_wv.as_ref() {
                                push_config_to_js(wv, &config);
                                push_toast(
                                    wv,
                                    if config.gamer.always_on_top {
                                        "📌 最前面固定 ON"
                                    } else {
                                        "📌 最前面固定 OFF"
                                    },
                                    "info",
                                );
                                let snap = tracker.measure(wv.browser_pid);
                                if let Some(t) = app_tray.as_ref() {
                                    t.refresh_menu(&config, snap.total_mb, blocker.is_suspended());
                                }
                            }
                        }
                        HotkeyAction::FlushRam => {
                            let rep = perform_ram_flush(
                                discord_wv.as_ref(),
                                &config,
                                &mut tracker,
                                &mut last_freed_mb,
                                &mut total_freed_mb,
                            );
                            minutes_since_trim = 0;
                            last_guard_trim = Instant::now();
                            if let Some(wv) = discord_wv.as_ref() {
                                if window_visible {
                                    push_toast(
                                        wv,
                                        &format!(
                                            "🧹 メモリ解放: {:.0} MB → {:.0} MB (-{:.0} MB)",
                                            rep.before_mb, rep.after_mb, rep.freed_mb
                                        ),
                                        "ok",
                                    );
                                }
                            }
                        }
                        HotkeyAction::MicMute => {
                            if let Some(wv) = discord_wv.as_ref() {
                                wv.eval("window.__rustcordBridge && window.__rustcordBridge.toggleMicMute();");
                            }
                        }
                        HotkeyAction::Hibernate => {
                            if blocker.is_suspended() {
                                restore_from_tray(
                                    &window,
                                    hwnd,
                                    discord_wv.as_ref(),
                                    &config,
                                    app_tray.as_ref(),
                                    &mut tracker,
                                    &mut window_visible,
                                );
                            } else if !in_voice {
                                hibernate_immediately(
                                    &window,
                                    discord_wv.as_ref(),
                                    &config,
                                    app_tray.as_ref(),
                                    &mut tracker,
                                    &mut window_visible,
                                    &mut last_freed_mb,
                                    &mut total_freed_mb,
                                );
                            }
                        }
                    }
                }

                UserEvent::TickStats => {
                    tick_counter = tick_counter.wrapping_add(1);

                    // Debounced persistence of window position/size
                    if geometry_dirty && tick_counter % 4 == 0 {
                        geometry_dirty = false;
                        let _ = config.save();
                    }

                    // While hidden in tray, sample only every ~10s to keep CPU ~0%
                    if !window_visible && tick_counter % 4 != 0 {
                        return;
                    }

                    let pid = discord_wv.as_ref().and_then(|w| w.browser_pid);
                    let mut snap = tracker.measure(pid);
                    peak_mb = peak_mb.max(snap.total_mb);

                    // Auto RAM Guard: gently runs WebView2 GC + trim if RAM crosses user ceiling
                    let guard_limit = config.performance.auto_guard_mb;
                    if guard_limit > 0
                        && !blocker.is_suspended()
                        && snap.total_mb > guard_limit as f64
                        && last_guard_trim.elapsed() >= Duration::from_secs(60)
                    {
                        last_guard_trim = Instant::now();
                        let _ = perform_ram_flush(
                            discord_wv.as_ref(),
                            &config,
                            &mut tracker,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                        snap = tracker.measure(pid);
                    }

                    if window_visible {
                        update_window_title(
                            &window,
                            &config,
                            snap.total_mb,
                            blocker.is_suspended(),
                        );
                        if let Some(wv) = discord_wv.as_ref() {
                            if !blocker.is_suspended() {
                                let stats = make_stats(
                                    snap,
                                    peak_mb,
                                    last_freed_mb,
                                    total_freed_mb,
                                    &blocker,
                                    startup_ms,
                                    binary_size_mb,
                                    in_voice,
                                    &cached_profile_sizes,
                                );
                                push_stats_to_js(wv, &stats);
                            }
                        }
                    }

                    // Tooltip-only update (never rebuilds menu so open context menus never close)
                    if tick_counter % 4 == 0 {
                        if let Some(t) = app_tray.as_ref() {
                            t.update_tooltip(&config, snap.total_mb, blocker.is_suspended());
                        }
                    }
                }

                UserEvent::AutoTrimTick => {
                    let interval = config.performance.auto_trim_minutes;
                    if interval == 0 || blocker.is_suspended() {
                        return;
                    }
                    minutes_since_trim = minutes_since_trim.saturating_add(1);
                    if minutes_since_trim >= interval {
                        minutes_since_trim = 0;
                        last_guard_trim = Instant::now();
                        let _ = perform_ram_flush(
                            discord_wv.as_ref(),
                            &config,
                            &mut tracker,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                    }
                }

                UserEvent::DelayedSuspendCheck => {
                    if !window_visible
                        && !in_voice
                        && config.performance.tray_mode == TrayMode::DeepSuspend
                    {
                        if let Some(wv) = discord_wv.as_ref() {
                            wv.try_suspend();
                            let rep = tracker.trim(wv.browser_pid);
                            last_freed_mb = rep.freed_mb;
                            total_freed_mb += rep.freed_mb;
                            if let Some(t) = app_tray.as_ref() {
                                t.refresh_menu(&config, rep.after_mb, true);
                            }
                        }
                    }
                }

                UserEvent::PostBootOptimize => {
                    if config.performance.lite_mode {
                        let rep = perform_ram_flush(
                            discord_wv.as_ref(),
                            &config,
                            &mut tracker,
                            &mut last_freed_mb,
                            &mut total_freed_mb,
                        );
                        last_guard_trim = Instant::now();
                        update_window_title(
                            &window,
                            &config,
                            rep.after_mb,
                            blocker.is_suspended(),
                        );
                        if let Some(t) = app_tray.as_ref() {
                            t.update_tooltip(&config, rep.after_mb, blocker.is_suspended());
                        }
                    }
                }
            },

            _ => {}
        }
    });
}

/// Unified memory optimization: triggers WebView2's native Low memory usage target
/// (V8 GC + Blink/Skia cache purge) followed by Win32 working-set trimming.
fn perform_ram_flush(
    wv: Option<&DiscordWebView>,
    config: &AppConfig,
    tracker: &mut ProcessTracker,
    last_freed_mb: &mut f64,
    total_freed_mb: &mut f64,
) -> TrimReport {
    if let Some(wv) = wv {
        wv.pulse_memory_purge(config.performance.lite_mode);
    }
    let pid = wv.and_then(|w| w.browser_pid);
    let rep = tracker.trim(pid);
    *last_freed_mb = rep.freed_mb;
    *total_freed_mb += rep.freed_mb;
    rep
}

fn apply_window_chrome(window: &Window, hwnd: isize, config: &AppConfig) {
    window.set_always_on_top(config.gamer.always_on_top);
    let caption_rgb = match config.appearance.theme {
        ThemePreset::OledBlack => (0, 0, 0),
        _ => (11, 13, 20),
    };
    memory::apply_dark_titlebar(hwnd, caption_rgb);
    memory::apply_window_opacity(hwnd, config.gamer.window_opacity);
}

fn update_window_title(window: &Window, config: &AppConfig, ram_mb: f64, suspended: bool) {
    let profile = config.active_profile();
    let title = if config.gamer.show_ram_in_title {
        let mode = if suspended {
            "休止"
        } else if config.performance.lite_mode {
            "軽量ON"
        } else {
            "通常"
        };
        format!(
            "Rustcord — [{}]  |  RAM: {:.0} MB  |  {}",
            profile.name, ram_mb, mode
        )
    } else {
        format!("Rustcord — {}", profile.name)
    };
    window.set_title(&title);
}

fn save_window_geometry(window: &Window, config: &mut AppConfig) {
    let scale = window.scale_factor();
    let size = window.inner_size().to_logical::<u32>(scale);
    if size.width >= 400 && size.height >= 300 {
        config.window.width = size.width;
        config.window.height = size.height;
    }
    if let Ok(pos) = window.outer_position() {
        let lp = pos.to_logical::<i32>(scale);
        config.window.x = Some(lp.x);
        config.window.y = Some(lp.y);
    }
    config.window.maximized = window.is_maximized();
}

#[allow(clippy::too_many_arguments)]
fn hide_to_tray(
    window: &Window,
    wv: Option<&DiscordWebView>,
    config: &AppConfig,
    in_voice: bool,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    tracker: &mut ProcessTracker,
    window_visible: &mut bool,
    last_freed_mb: &mut f64,
    total_freed_mb: &mut f64,
) {
    window.set_visible(false);
    *window_visible = false;

    let pid = wv.and_then(|w| w.browser_pid);
    match config.performance.tray_mode {
        TrayMode::Normal => {}
        TrayMode::SmartTrim => {
            if let Some(wv) = wv {
                wv.set_low_memory(true);
            }
            tracker.set_low_priority(pid, true);
            let rep = tracker.trim(pid);
            *last_freed_mb = rep.freed_mb;
            *total_freed_mb += rep.freed_mb;
        }
        TrayMode::DeepSuspend => {
            if let Some(wv) = wv {
                wv.set_low_memory(true);
            }
            tracker.set_low_priority(pid, true);
            let rep = tracker.trim(pid);
            *last_freed_mb = rep.freed_mb;
            *total_freed_mb += rep.freed_mb;
            if !in_voice {
                let p = proxy.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(3));
                    let _ = p.send_event(UserEvent::DelayedSuspendCheck);
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn hibernate_immediately(
    window: &Window,
    wv: Option<&DiscordWebView>,
    config: &AppConfig,
    tray: Option<&AppTray>,
    tracker: &mut ProcessTracker,
    window_visible: &mut bool,
    last_freed_mb: &mut f64,
    total_freed_mb: &mut f64,
) {
    window.set_visible(false);
    *window_visible = false;
    if let Some(wv) = wv {
        wv.set_low_memory(true);
        wv.try_suspend();
        tracker.set_low_priority(wv.browser_pid, true);
        let rep = tracker.trim(wv.browser_pid);
        *last_freed_mb = rep.freed_mb;
        *total_freed_mb += rep.freed_mb;
        if let Some(t) = tray {
            t.refresh_menu(config, rep.after_mb, true);
        }
    }
}

fn restore_from_tray(
    window: &Window,
    hwnd: isize,
    wv: Option<&DiscordWebView>,
    config: &AppConfig,
    tray: Option<&AppTray>,
    tracker: &mut ProcessTracker,
    window_visible: &mut bool,
) {
    let was_suspended = wv.map(|w| w.blocker.is_suspended()).unwrap_or(false);
    if let Some(wv) = wv {
        if was_suspended {
            wv.resume();
        }
        wv.set_low_memory(config.performance.lite_mode);
        tracker.set_low_priority(wv.browser_pid, false);
    }
    window.set_visible(true);
    window.set_minimized(false);
    memory::focus_window_native(hwnd);
    *window_visible = true;

    let snap = tracker.measure(wv.and_then(|w| w.browser_pid));
    update_window_title(window, config, snap.total_mb, false);
    if let Some(t) = tray {
        if was_suspended {
            t.refresh_menu(config, snap.total_mb, false);
        } else {
            t.update_tooltip(config, snap.total_mb, false);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn switch_profile(
    new_id: &str,
    window: &Window,
    config: &mut AppConfig,
    discord_wv: &mut Option<DiscordWebView>,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    blocker: &BlockerState,
    tray: Option<&AppTray>,
    tracker: &mut ProcessTracker,
    in_voice: &mut bool,
    cached_profile_sizes: &mut Vec<ProfileSizeInfo>,
) {
    config.active_profile_id = new_id.to_string();
    let _ = config.save();
    *in_voice = false;

    // Fully destroy the old profile's WebView2 first so RAM stays at 1 account only
    *discord_wv = None;
    tracker.invalidate();

    if let Ok(new_wv) = DiscordWebView::create(window, config, proxy.clone(), blocker.clone()) {
        let snap = tracker.measure(new_wv.browser_pid);
        update_window_title(window, config, snap.total_mb, false);
        if let Some(t) = tray {
            t.refresh_menu(config, snap.total_mb, false);
        }
        *discord_wv = Some(new_wv);
    }
    *cached_profile_sizes = compute_profile_sizes(config);
}

fn compute_profile_sizes(config: &AppConfig) -> Vec<ProfileSizeInfo> {
    config
        .profiles
        .iter()
        .map(|p| ProfileSizeInfo {
            id: p.id.clone(),
            disk_mb: AppConfig::profile_disk_usage_mb(&p.id),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn make_stats(
    snap: memory::MemorySnapshot,
    peak_mb: f64,
    last_freed_mb: f64,
    total_freed_mb: f64,
    blocker: &BlockerState,
    startup_ms: u64,
    binary_size_mb: f64,
    in_voice: bool,
    profile_sizes: &[ProfileSizeInfo],
) -> RuntimeStats {
    RuntimeStats {
        total_mb: snap.total_mb,
        host_mb: snap.host_mb,
        webview_mb: snap.webview_mb,
        private_mb: snap.private_mb,
        peak_mb,
        last_freed_mb,
        total_freed_mb,
        process_count: snap.process_count,
        blocked_requests: blocker.blocked_count(),
        startup_ms,
        binary_size_mb,
        in_voice,
        is_suspended: blocker.is_suspended(),
        data_dir: AppConfig::data_dir().to_string_lossy().into_owned(),
        profile_sizes: profile_sizes.to_vec(),
    }
}

fn push_config_to_js(wv: &DiscordWebView, config: &AppConfig) {
    let css = build_discord_css(config);
    let Ok(cfg_json) = serde_json::to_string(config) else {
        return;
    };
    let Ok(css_json) = serde_json::to_string(&css) else {
        return;
    };
    let script = format!(
        "window.__rustcordBridge && window.__rustcordBridge.sync({cfg_json}, {css_json});"
    );
    wv.eval(&script);
}

fn push_stats_to_js(wv: &DiscordWebView, stats: &RuntimeStats) {
    let Ok(stats_json) = serde_json::to_string(stats) else {
        return;
    };
    let script = format!("window.__rustcordBridge && window.__rustcordBridge.stats({stats_json});");
    wv.eval(&script);
}

fn push_toast(wv: &DiscordWebView, msg: &str, kind: &str) {
    let Ok(msg_json) = serde_json::to_string(msg) else {
        return;
    };
    let Ok(kind_json) = serde_json::to_string(kind) else {
        return;
    };
    let script = format!(
        "window.__rustcordBridge && window.__rustcordBridge.toast({msg_json}, {kind_json});"
    );
    wv.eval(&script);
}
