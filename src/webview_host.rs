use crate::config::{AppConfig, PerformanceConfig};
use crate::frontend::INJECT_JS;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{
    MemoryUsageLevel, NewWindowResponse, Theme, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, ICoreWebView2_3,
};
#[cfg(windows)]
use webview2_com::{
    take_pwstr, PermissionRequestedEventHandler, TrySuspendCompletedHandler,
    WebResourceRequestedEventHandler,
};
#[cfg(windows)]
use windows_core::{w, Interface, PWSTR};

#[derive(Debug)]
pub enum UserEvent {
    IpcMessage(String),
    TrayMenu(String),
    TrayClick,
    GlobalHotkey(u32),
    TickStats,
    AutoTrimTick,
    DelayedSuspendCheck,
    PostBootOptimize,
    WakeFromSecondInstance,
}

#[derive(Clone)]
pub struct BlockerState {
    pub block_telemetry: Arc<AtomicBool>,
    pub block_typing: Arc<AtomicBool>,
    pub blocked_requests: Arc<AtomicU64>,
    pub is_suspended: Arc<AtomicBool>,
}

impl BlockerState {
    pub fn from_config(cfg: &PerformanceConfig) -> Self {
        Self {
            block_telemetry: Arc::new(AtomicBool::new(cfg.block_telemetry)),
            block_typing: Arc::new(AtomicBool::new(cfg.block_typing)),
            blocked_requests: Arc::new(AtomicU64::new(0)),
            is_suspended: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn update(&self, cfg: &PerformanceConfig) {
        self.block_telemetry
            .store(cfg.block_telemetry, Ordering::Relaxed);
        self.block_typing.store(cfg.block_typing, Ordering::Relaxed);
    }

    pub fn blocked_count(&self) -> u64 {
        self.blocked_requests.load(Ordering::Relaxed)
    }

    pub fn is_suspended(&self) -> bool {
        self.is_suspended.load(Ordering::Relaxed)
    }
}

pub struct DiscordWebView {
    pub webview: WebView,
    // Keep WebContext alive for the entire lifetime of this WebView
    _context: WebContext,
    pub browser_pid: Option<u32>,
    pub blocker: BlockerState,
}

impl DiscordWebView {
    pub fn create(
        window: &Window,
        config: &AppConfig,
        proxy: EventLoopProxy<UserEvent>,
        blocker: BlockerState,
    ) -> Result<Self, String> {
        let profile = config.active_profile();
        let profile_dir = AppConfig::profile_dir(&profile.id);
        let _ = std::fs::create_dir_all(&profile_dir);

        let mut web_context = WebContext::new(Some(profile_dir));
        let additional_args = build_browser_args(&config.performance);
        let target_url = profile.channel.url();

        let proxy_ipc = proxy.clone();
        let builder = WebViewBuilder::new_with_web_context(&mut web_context)
            .with_url(target_url)
            .with_background_color((11, 13, 20, 255))
            .with_theme(Theme::Dark)
            .with_devtools(true)
            .with_browser_accelerator_keys(true)
            .with_additional_browser_args(additional_args)
            .with_initialization_script(INJECT_JS)
            .with_ipc_handler(move |req| {
                let body = req.body().clone();
                let _ = proxy_ipc.send_event(UserEvent::IpcMessage(body));
            })
            .with_new_window_req_handler(|uri, _features| {
                if is_internal_popup(&uri) {
                    NewWindowResponse::Allow
                } else {
                    if uri.starts_with("http://") || uri.starts_with("https://") {
                        let _ = open::that(&uri);
                    }
                    NewWindowResponse::Deny
                }
            });

        let webview = builder
            .build(window)
            .map_err(|e| format!("WebView2 initialization failed: {e}"))?;

        blocker.is_suspended.store(false, Ordering::Relaxed);
        let browser_pid = configure_native_webview2(&webview, &blocker);
        if config.performance.lite_mode {
            let _ = webview.set_memory_usage_level(MemoryUsageLevel::Low);
        }

        Ok(Self {
            webview,
            _context: web_context,
            browser_pid,
            blocker,
        })
    }

    pub fn set_low_memory(&self, low: bool) {
        let level = if low {
            MemoryUsageLevel::Low
        } else {
            MemoryUsageLevel::Normal
        };
        let _ = self.webview.set_memory_usage_level(level);
    }

    /// Triggers WebView2's native Low memory usage target (forces V8 GC & Blink/Skia cache purge).
    pub fn pulse_memory_purge(&self, keep_low: bool) {
        let _ = self.webview.set_memory_usage_level(MemoryUsageLevel::Low);
        if !keep_low {
            let _ = self.webview.set_memory_usage_level(MemoryUsageLevel::Normal);
        }
    }

    pub fn eval(&self, script: &str) {
        let _ = self.webview.evaluate_script(script);
    }

    pub fn open_devtools(&self) {
        #[cfg(windows)]
        unsafe {
            if let Ok(settings) = self.webview.webview().Settings() {
                let _ = settings.SetAreDevToolsEnabled(true);
            }
        }
        self.webview.open_devtools();
    }

    pub fn reload(&self) {
        let _ = self.webview.evaluate_script("window.location.reload();");
    }

    pub fn try_suspend(&self) -> bool {
        #[cfg(windows)]
        {
            let controller = self.webview.controller();
            let core = self.webview.webview();
            unsafe {
                let _ = controller.SetIsVisible(false);
                if let Ok(core3) = core.cast::<ICoreWebView2_3>() {
                    let suspended_flag = self.blocker.is_suspended.clone();
                    let handler = TrySuspendCompletedHandler::create(Box::new(
                        move |_hr, is_successful| {
                            if is_successful {
                                suspended_flag.store(true, Ordering::Relaxed);
                            }
                            Ok(())
                        },
                    ));
                    if core3.TrySuspend(&handler).is_ok() {
                        self.blocker.is_suspended.store(true, Ordering::Relaxed);
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn resume(&self) {
        #[cfg(windows)]
        {
            let controller = self.webview.controller();
            let core = self.webview.webview();
            unsafe {
                if let Ok(core3) = core.cast::<ICoreWebView2_3>() {
                    let _ = core3.Resume();
                }
                let _ = controller.SetIsVisible(true);
            }
            self.blocker.is_suspended.store(false, Ordering::Relaxed);
        }
    }
}

fn build_browser_args(perf: &PerformanceConfig) -> String {
    let mut args = vec![
        "--enable-features=QuickIntensiveWakeUpThrottlingAfterLoading".to_string(),
        "--disable-features=Translate,OptimizationHints,MediaRouter,SpareRendererForSitePerProcess,BackForwardCache,Prerender2,PaintHolding,AutofillServerCommunication,msEdgeSidebarV2,msEdgeMiniMenu,msEdgeCollections".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-component-extensions-with-background-pages".to_string(),
        "--disable-default-apps".to_string(),
        "--disable-domain-reliability".to_string(),
        "--disable-extensions".to_string(),
        "--disable-sync".to_string(),
        "--no-first-run".to_string(),
        "--no-pings".to_string(),
        "--process-per-site".to_string(),
        "--gpu-program-cache-size-kb=1024".to_string(),
        "--disable-gpu-shader-disk-cache".to_string(),
        "--force-gpu-mem-available-mb=256".to_string(),
        "--max-active-webgl-contexts=2".to_string(),
        format!(
            "--renderer-process-limit={}",
            if perf.lite_mode { 1 } else { 2 }
        ),
    ];

    let mut v8_flags = vec!["--optimize-for-size".to_string()];
    if perf.js_heap_limit_mb >= 192 {
        v8_flags.push(format!("--max-old-space-size={}", perf.js_heap_limit_mb));
    }
    args.push(format!("--js-flags=\"{}\"", v8_flags.join(" ")));

    if !perf.hardware_acceleration {
        args.push("--disable-gpu".to_string());
        args.push("--disable-gpu-compositing".to_string());
    }

    args.join(" ")
}

fn is_internal_popup(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    lower.starts_with("https://discord.com/popout")
        || lower.starts_with("https://ptb.discord.com/popout")
        || lower.starts_with("https://canary.discord.com/popout")
        || lower.starts_with("https://discord.com/oauth2")
        || lower.starts_with("https://accounts.google.com/")
        || lower.starts_with("https://appleid.apple.com/")
        || lower == "about:blank"
}

#[cfg(windows)]
fn is_discord_origin(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    lower.starts_with("https://discord.com/")
        || lower.starts_with("https://ptb.discord.com/")
        || lower.starts_with("https://canary.discord.com/")
}

#[cfg(windows)]
fn configure_native_webview2(webview: &WebView, blocker: &BlockerState) -> Option<u32> {
    let core = webview.webview();
    let env = webview.environment();
    let mut browser_pid: u32 = 0;

    unsafe {
        let _ = core.BrowserProcessId(&mut browser_pid);

        // Clean UI settings: hide noisy bottom-left URL preview & hide DevTools from context menu until explicitly opened
        if let Ok(settings) = core.Settings() {
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetAreDefaultContextMenusEnabled(true);
            let _ = settings.SetIsZoomControlEnabled(true);
            let _ = settings.SetIsBuiltInErrorPageEnabled(true);
            let _ = settings.SetAreDevToolsEnabled(cfg!(debug_assertions));
        }

        // Auto-allow Microphone, Notifications, Clipboard on official Discord origins.
        // Camera is intentionally left to WebView2's confirmation prompt for privacy.
        let perm_handler = PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
            if let Some(args) = args {
                let mut uri_ptr = PWSTR::null();
                if args.Uri(&mut uri_ptr).is_ok() {
                    let uri = take_pwstr(uri_ptr);
                    if is_discord_origin(&uri) {
                        let mut kind = Default::default();
                        if args.PermissionKind(&mut kind).is_ok()
                            && (kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
                                || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
                                || kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ)
                        {
                            let _ = args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW);
                        }
                    }
                }
            }
            Ok(())
        }));
        let mut perm_token = 0i64;
        let _ = core.add_PermissionRequested(&perm_handler, &mut perm_token);

        // Native network-level filter ONLY for telemetry & typing endpoints (zero overhead on chats/voice)
        let _ = core.AddWebResourceRequestedFilter(
            w!("*discord.com/api/*/science*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        );
        let _ = core.AddWebResourceRequestedFilter(
            w!("*discord.com/api/*/metrics*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        );
        let _ = core.AddWebResourceRequestedFilter(
            w!("*sentry.io/*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        );
        let _ = core.AddWebResourceRequestedFilter(
            w!("*discord.com/api/*/typing*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        );

        let block_telemetry = blocker.block_telemetry.clone();
        let block_typing = blocker.block_typing.clone();
        let blocked_requests = blocker.blocked_requests.clone();
        let env_clone = env.clone();

        let res_handler = WebResourceRequestedEventHandler::create(Box::new(move |_sender, args| {
            let Some(args) = args else { return Ok(()) };
            let Ok(req) = args.Request() else {
                return Ok(());
            };
            let mut uri_ptr = PWSTR::null();
            if req.Uri(&mut uri_ptr).is_err() {
                return Ok(());
            }
            let uri = take_pwstr(uri_ptr).to_ascii_lowercase();

            let is_typing = uri.contains("/typing");
            let is_telemetry =
                uri.contains("/science") || uri.contains("/metrics") || uri.contains("sentry.io");

            let should_block = (is_telemetry && block_telemetry.load(Ordering::Relaxed))
                || (is_typing && block_typing.load(Ordering::Relaxed));

            if should_block {
                if let Ok(resp) = env_clone.CreateWebResourceResponse(
                    None,
                    204,
                    w!("No Content"),
                    w!("Access-Control-Allow-Origin: *\r\nContent-Length: 0"),
                ) {
                    let _ = args.SetResponse(&resp);
                    blocked_requests.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(())
        }));

        let mut res_token = 0i64;
        let _ = core.add_WebResourceRequested(&res_handler, &mut res_token);
    }

    if browser_pid != 0 {
        Some(browser_pid)
    } else {
        None
    }
}
