(() => {
  if (window.__RUSTCORD_INITIALIZED__) return;
  window.__RUSTCORD_INITIALIZED__ = true;

  const post = (obj) => {
    try {
      if (window.ipc && typeof window.ipc.postMessage === "function") {
        window.ipc.postMessage(JSON.stringify(obj));
      }
    } catch (_) {}
  };

  let state = {
    config: null,
    stats: {
      total_mb: 0,
      host_mb: 0,
      webview_mb: 0,
      private_mb: 0,
      peak_mb: 0,
      last_freed_mb: 0,
      total_freed_mb: 0,
      process_count: 0,
      blocked_requests: 0,
      startup_ms: 0,
      binary_size_mb: 0,
      in_voice: false,
      is_suspended: false,
      data_dir: "",
      profile_sizes: [],
    },
    ramHistory: [],
    activeTab: "perf",
    modalOpen: false,
    profileMenuOpen: false,
    lastVoice: false,
  };

  let shadow = null;
  let discordStyleEl = null;
  let toastTimer = null;

  // ---------------------------------------------------------------------------
  // Dynamic Discord CSS injector (outside Shadow DOM so it styles Discord)
  // ---------------------------------------------------------------------------
  function applyDiscordCss(cssText) {
    if (!discordStyleEl || !discordStyleEl.isConnected) {
      discordStyleEl = document.getElementById("rustcord-discord-css");
      if (!discordStyleEl) {
        discordStyleEl = document.createElement("style");
        discordStyleEl.id = "rustcord-discord-css";
        (document.head || document.documentElement).appendChild(discordStyleEl);
      }
    }
    if (discordStyleEl.textContent !== cssText) {
      discordStyleEl.textContent = cssText;
    }
  }

  // ---------------------------------------------------------------------------
  // Subtle Web Audio beep for in-game global mic mute confirmation
  // ---------------------------------------------------------------------------
  function playMuteBeep(muted) {
    if (!state.config || !state.config.gamer || !state.config.gamer.mute_beep) return;
    try {
      const ctx = new (window.AudioContext || window.webkitAudioContext)();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      const now = ctx.currentTime;
      if (muted) {
        osc.frequency.setValueAtTime(520, now);
        osc.frequency.exponentialRampToValueAtTime(340, now + 0.09);
      } else {
        osc.frequency.setValueAtTime(380, now);
        osc.frequency.exponentialRampToValueAtTime(640, now + 0.09);
      }
      gain.gain.setValueAtTime(0.08, now);
      gain.gain.exponentialRampToValueAtTime(0.001, now + 0.11);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(now);
      osc.stop(now + 0.12);
      setTimeout(() => ctx.close().catch(() => {}), 220);
    } catch (_) {}
  }

  // ---------------------------------------------------------------------------
  // Voice call detection & reliable cross-language Mic Mute toggle
  // ---------------------------------------------------------------------------
  function detectVoiceActive() {
    const panels = document.querySelector('section[class*="panels_"], [class*="panels_"]');
    if (!panels) return false;
    return Boolean(
      panels.querySelector(
        '[class*="rtcConnectionStatus_"], button[aria-label*="切断"], button[aria-label*="Disconnect" i]'
      )
    );
  }

  function findMuteButton() {
    const buttons = document.querySelectorAll('section[class*="panels_"] button, [class*="panels_"] button');
    for (const btn of buttons) {
      const label = (btn.getAttribute("aria-label") || "").trim();
      if (
        label === "ミュート" ||
        label === "ミュート解除" ||
        label.toLowerCase() === "mute" ||
        label.toLowerCase() === "unmute"
      ) {
        return btn;
      }
    }
    // Fallback: first aria-checked button in panels (Discord orders Mute before Deafen)
    for (const btn of buttons) {
      if (btn.hasAttribute("aria-checked")) {
        const label = (btn.getAttribute("aria-label") || "").toLowerCase();
        if (!label.includes("スピーカー") && !label.includes("deafen")) {
          return btn;
        }
      }
    }
    return null;
  }

  function toggleMicMute() {
    const btn = findMuteButton();
    if (!btn) {
      showToast("🎙 マイクボタンが見つかりません（ログイン後に有効）", "warn");
      return;
    }
    const wasMuted = btn.getAttribute("aria-checked") === "true";
    btn.click();
    const nowMuted = !wasMuted;
    playMuteBeep(nowMuted);
    showToast(nowMuted ? "🔇 マイクをミュートしました" : "🎙 マイクのミュートを解除しました", "info");
  }

  // ---------------------------------------------------------------------------
  // Lightweight GIF / Animated Avatar Autoplay Suppressor (plays on hover)
  // ---------------------------------------------------------------------------
  function freezeAnimatedImg(img) {
    if (!state.config || !state.config.performance.disable_gif_autoplay) return;
    if (!img || img.dataset.rcGifHandled) return;
    const src = img.getAttribute("src") || "";
    if (!src) return;
    const isDiscordCdn =
      src.includes("cdn.discordapp.com") || src.includes("media.discordapp.net");
    if (!isDiscordCdn) return;

    let staticSrc = null;
    if (src.includes("animated=true")) {
      staticSrc = src.replace("animated=true", "animated=false");
    } else if (/\/a_[a-f0-9]+\.gif(\?|$)/i.test(src)) {
      staticSrc = src.replace(/\.gif(\?|$)/i, ".webp$1");
    }
    if (!staticSrc || staticSrc === src) return;

    img.dataset.rcGifHandled = "1";
    img.dataset.rcOrig = src;
    img.dataset.rcStatic = staticSrc;
    img.addEventListener(
      "error",
      () => {
        if (img.dataset.rcOrig && img.src !== img.dataset.rcOrig) {
          img.src = img.dataset.rcOrig;
        }
      },
      { once: true }
    );
    img.addEventListener("mouseenter", () => {
      if (img.dataset.rcOrig) img.src = img.dataset.rcOrig;
    });
    img.addEventListener("mouseleave", () => {
      if (state.config?.performance?.disable_gif_autoplay && img.dataset.rcStatic) {
        img.src = img.dataset.rcStatic;
      }
    });
    img.src = staticSrc;
  }

  const pendingImgNodes = new Set();
  let gifFlushScheduled = false;
  let gifObserver = null;

  function flushPendingImgs() {
    gifFlushScheduled = false;
    if (!state.config || !state.config.performance.disable_gif_autoplay) {
      pendingImgNodes.clear();
      return;
    }
    for (const node of pendingImgNodes) {
      if (!node || !node.isConnected) continue;
      if (node.tagName === "IMG") {
        freezeAnimatedImg(node);
      } else if (node.querySelectorAll) {
        const imgs = node.querySelectorAll('img[src*="animated=true"], img[src*="/a_"][src*=".gif"]');
        for (const img of imgs) freezeAnimatedImg(img);
      }
    }
    pendingImgNodes.clear();
  }

  function scheduleImgFlush() {
    if (gifFlushScheduled) return;
    gifFlushScheduled = true;
    if (typeof requestIdleCallback === "function") {
      requestIdleCallback(() => flushPendingImgs(), { timeout: 600 });
    } else {
      setTimeout(flushPendingImgs, 450);
    }
  }

  function syncGifObserver() {
    const enabled = Boolean(state.config?.performance?.disable_gif_autoplay);
    if (!enabled) {
      if (gifObserver) {
        gifObserver.disconnect();
        gifObserver = null;
      }
      pendingImgNodes.clear();
      return;
    }
    if (gifObserver) return;
    const root = document.getElementById("app-mount");
    if (!root) return;
    // Initial one-time pass on existing DOM
    root
      .querySelectorAll('img[src*="animated=true"], img[src*="/a_"][src*=".gif"]')
      .forEach(freezeAnimatedImg);

    // Lightweight incremental observer: inspects ONLY newly added nodes (zero full-DOM rescans)
    gifObserver = new MutationObserver((mutations) => {
      for (let i = 0; i < mutations.length; i++) {
        const added = mutations[i].addedNodes;
        for (let j = 0; j < added.length; j++) {
          const n = added[j];
          if (n.nodeType === 1) {
            pendingImgNodes.add(n);
          }
        }
      }
      if (pendingImgNodes.size > 0) scheduleImgFlush();
    });
    gifObserver.observe(root, { childList: true, subtree: true });
  }

  // ---------------------------------------------------------------------------
  // Shadow DOM HUD & Settings Modal
  // ---------------------------------------------------------------------------
  const SHADOW_CSS = `
    :host {
      all: initial;
      font-family: "Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo", sans-serif;
      color: #eef1f8;
      user-select: none;
    }
    * { box-sizing: border-box; }

    .rc-bar {
      position: fixed;
      top: 0; left: 0; right: 0;
      height: 30px;
      background: linear-gradient(90deg, #0b0d14 0%, #10131e 55%, #0c0f18 100%);
      border-bottom: 1px solid rgba(255, 255, 255, 0.08);
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 8px;
      z-index: 2147483640;
      font-size: 12px;
      gap: 6px;
    }
    .rc-bar.oled {
      background: #000000;
      border-bottom-color: rgba(255, 255, 255, 0.09);
    }
    .rc-bar.hidden { display: none; }

    .rc-left, .rc-right {
      display: flex;
      align-items: center;
      gap: 5px;
      min-width: 0;
    }

    .rc-brand {
      display: inline-flex;
      align-items: center;
      gap: 5px;
      font-weight: 700;
      letter-spacing: 0.02em;
      padding: 2px 7px;
      border-radius: 6px;
      background: linear-gradient(135deg, rgba(255, 94, 26, 0.18), rgba(88, 101, 242, 0.18));
      border: 1px solid rgba(255, 115, 55, 0.35);
      color: #fff;
      font-size: 11.5px;
    }
    .rc-brand-dot {
      width: 7px; height: 7px;
      border-radius: 50%;
      background: linear-gradient(135deg, #ff6b2b, #5865f2);
      box-shadow: 0 0 6px rgba(255, 107, 43, 0.8);
    }

    .rc-btn {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      height: 22px;
      padding: 0 8px;
      border-radius: 6px;
      border: 1px solid rgba(255, 255, 255, 0.09);
      background: rgba(255, 255, 255, 0.045);
      color: #d6dcf0;
      font-size: 11.5px;
      font-weight: 500;
      cursor: pointer;
      transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
      white-space: nowrap;
      font-family: inherit;
    }
    .rc-btn:hover {
      background: rgba(255, 255, 255, 0.11);
      border-color: rgba(255, 255, 255, 0.2);
      color: #ffffff;
    }
    .rc-btn.active {
      background: rgba(255, 107, 43, 0.22);
      border-color: rgba(255, 107, 43, 0.6);
      color: #ffd6c2;
    }
    .rc-btn.active-blue {
      background: rgba(88, 101, 242, 0.28);
      border-color: rgba(114, 137, 255, 0.65);
      color: #dfe4ff;
    }
    .rc-btn.ram-low { color: #5ce69a; border-color: rgba(92, 230, 154, 0.28); }
    .rc-btn.ram-mid { color: #f6c453; border-color: rgba(246, 196, 83, 0.32); }
    .rc-btn.ram-high { color: #ff7a6b; border-color: rgba(255, 122, 107, 0.4); }

    .rc-voice-pill {
      display: none;
      align-items: center;
      gap: 4px;
      height: 20px;
      padding: 0 7px;
      border-radius: 99px;
      background: rgba(35, 165, 90, 0.2);
      border: 1px solid rgba(35, 165, 90, 0.55);
      color: #7ef2ad;
      font-size: 11px;
      font-weight: 600;
    }
    .rc-voice-pill.on { display: inline-flex; }

    .rc-profile-dot {
      width: 8px; height: 8px;
      border-radius: 50%;
      flex-shrink: 0;
    }

    .rc-floating-pill {
      position: fixed;
      top: 6px;
      right: 14px;
      z-index: 2147483640;
      display: none;
      align-items: center;
      gap: 5px;
      padding: 3px 9px;
      border-radius: 999px;
      background: rgba(14, 17, 26, 0.88);
      border: 1px solid rgba(255, 107, 43, 0.45);
      color: #eef1f8;
      font-size: 11px;
      font-weight: 600;
      cursor: pointer;
      box-shadow: 0 4px 14px rgba(0, 0, 0, 0.45);
    }
    .rc-floating-pill.show { display: inline-flex; }
    .rc-floating-pill:hover { background: rgba(24, 29, 44, 0.96); }

    /* Profile Quick Menu */
    .rc-dropdown {
      position: fixed;
      top: 34px;
      left: 88px;
      min-width: 245px;
      background: #121521;
      border: 1px solid rgba(255, 255, 255, 0.14);
      border-radius: 10px;
      padding: 6px;
      box-shadow: 0 14px 36px rgba(0, 0, 0, 0.62);
      z-index: 2147483645;
      display: none;
      flex-direction: column;
      gap: 3px;
    }
    .rc-dropdown.open { display: flex; }
    .rc-dd-title {
      font-size: 10.5px;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: #8892b0;
      padding: 4px 8px;
    }
    .rc-dd-item {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      padding: 7px 9px;
      border-radius: 7px;
      border: 1px solid transparent;
      background: transparent;
      color: #e6ebf8;
      cursor: pointer;
      font-size: 12.5px;
      text-align: left;
      font-family: inherit;
    }
    .rc-dd-item:hover { background: rgba(255, 255, 255, 0.08); }
    .rc-dd-item.current {
      background: rgba(88, 101, 242, 0.18);
      border-color: rgba(88, 101, 242, 0.45);
    }
    .rc-dd-meta { font-size: 10.5px; color: #8c96b4; }
    .rc-dd-sep { height: 1px; background: rgba(255,255,255,0.08); margin: 3px 0; }

    /* Toast */
    .rc-toast {
      position: fixed;
      bottom: 22px;
      right: 22px;
      z-index: 2147483647;
      padding: 9px 15px;
      border-radius: 10px;
      background: rgba(16, 19, 30, 0.96);
      border: 1px solid rgba(255, 107, 43, 0.55);
      color: #f4f7ff;
      font-size: 12.5px;
      font-weight: 500;
      box-shadow: 0 10px 30px rgba(0, 0, 0, 0.55);
      opacity: 0;
      transform: translateY(10px);
      pointer-events: none;
      transition: opacity 150ms ease, transform 150ms ease;
    }
    .rc-toast.show { opacity: 1; transform: translateY(0); }
    .rc-toast.ok { border-color: rgba(92, 230, 154, 0.65); }
    .rc-toast.warn { border-color: rgba(246, 196, 83, 0.65); }

    /* Settings Modal */
    .rc-overlay {
      position: fixed;
      inset: 0;
      background: rgba(4, 6, 12, 0.72);
      z-index: 2147483646;
      display: none;
      align-items: center;
      justify-content: center;
      padding: 18px;
    }
    .rc-overlay.open { display: flex; }

    .rc-modal {
      width: min(860px, 95vw);
      height: min(630px, 88vh);
      background: #0e111a;
      border: 1px solid rgba(255, 255, 255, 0.13);
      border-radius: 14px;
      box-shadow: 0 24px 68px rgba(0, 0, 0, 0.72);
      display: flex;
      overflow: hidden;
      color: #e9eefb;
    }

    .rc-nav {
      width: 200px;
      background: #090b11;
      border-right: 1px solid rgba(255, 255, 255, 0.08);
      padding: 16px 10px;
      display: flex;
      flex-direction: column;
      gap: 4px;
      flex-shrink: 0;
    }
    .rc-nav-logo {
      padding: 4px 10px 14px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.07);
      margin-bottom: 8px;
    }
    .rc-nav-logo h2 {
      margin: 0;
      font-size: 16px;
      font-weight: 800;
      background: linear-gradient(90deg, #ff7a3d, #8290ff);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
    }
    .rc-nav-logo p {
      margin: 3px 0 0;
      font-size: 11px;
      color: #828ca8;
    }
    .rc-tab {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 9px 11px;
      border-radius: 8px;
      border: none;
      background: transparent;
      color: #9aa5c4;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      text-align: left;
      font-family: inherit;
    }
    .rc-tab:hover { background: rgba(255, 255, 255, 0.06); color: #fff; }
    .rc-tab.active {
      background: linear-gradient(90deg, rgba(255, 107, 43, 0.22), rgba(88, 101, 242, 0.2));
      color: #fff;
      font-weight: 600;
    }
    .rc-nav-foot {
      margin-top: auto;
      padding: 10px 8px 2px;
      border-top: 1px solid rgba(255, 255, 255, 0.07);
      font-size: 11px;
      color: #77819e;
      line-height: 1.5;
    }

    .rc-content {
      flex: 1;
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    .rc-modal-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 14px 20px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    }
    .rc-modal-head h3 { margin: 0; font-size: 15.5px; font-weight: 700; }
    .rc-close {
      width: 30px; height: 30px;
      border-radius: 8px;
      border: 1px solid rgba(255, 255, 255, 0.1);
      background: rgba(255, 255, 255, 0.05);
      color: #c5cde2;
      font-size: 16px;
      cursor: pointer;
    }
    .rc-close:hover { background: rgba(235, 77, 75, 0.25); color: #fff; }

    .rc-body {
      flex: 1;
      overflow-y: auto;
      padding: 16px 20px 22px;
    }
    .rc-body::-webkit-scrollbar { width: 6px; }
    .rc-body::-webkit-scrollbar-thumb { background: #262c40; border-radius: 3px; }

    .rc-stats-grid {
      display: grid;
      grid-template-columns: repeat(4, 1fr);
      gap: 9px;
      margin-bottom: 12px;
    }
    .rc-stat-card {
      background: #141826;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 10px;
      padding: 10px 12px;
    }
    .rc-stat-label { font-size: 10.5px; color: #8b95b3; margin-bottom: 4px; }
    .rc-stat-val { font-size: 18px; font-weight: 800; color: #fff; }
    .rc-stat-sub { font-size: 10.5px; color: #7a85a6; margin-top: 2px; }

    .rc-chart-box {
      background: #121622;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 10px;
      padding: 10px 12px;
      margin-bottom: 14px;
    }
    .rc-chart-head {
      display: flex;
      justify-content: space-between;
      align-items: center;
      font-size: 11.5px;
      color: #9ba5c4;
      margin-bottom: 6px;
    }

    .rc-actions-row {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-bottom: 14px;
    }
    .rc-cta {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 8px 14px;
      border-radius: 8px;
      border: 1px solid rgba(255, 107, 43, 0.55);
      background: linear-gradient(135deg, #ff6b2b, #e04d14);
      color: #fff;
      font-size: 12.5px;
      font-weight: 700;
      cursor: pointer;
      font-family: inherit;
    }
    .rc-cta:hover { filter: brightness(1.09); }
    .rc-cta.secondary {
      background: #1a1f31;
      border-color: rgba(255, 255, 255, 0.14);
      color: #dce3f7;
      font-weight: 600;
    }
    .rc-cta.secondary:hover { background: #232a42; }
    .rc-cta.danger {
      background: rgba(235, 77, 75, 0.16);
      border-color: rgba(235, 77, 75, 0.5);
      color: #ff9c9b;
    }
    .rc-cta.danger:hover { background: rgba(235, 77, 75, 0.28); }

    .rc-section {
      margin-bottom: 16px;
      background: #131724;
      border: 1px solid rgba(255, 255, 255, 0.07);
      border-radius: 10px;
      padding: 6px 14px;
    }
    .rc-section-title {
      font-size: 11.5px;
      font-weight: 700;
      color: #8f9bb9;
      text-transform: uppercase;
      letter-spacing: 0.04em;
      margin: 14px 0 7px;
    }
    .rc-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 14px;
      padding: 10px 0;
      border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    }
    .rc-row:last-child { border-bottom: none; }
    .rc-row-info strong { display: block; font-size: 13px; color: #f0f4ff; font-weight: 600; }
    .rc-row-info span { display: block; font-size: 11.5px; color: #8c96b4; margin-top: 2px; line-height: 1.4; }

    /* Toggle Switch */
    .rc-switch {
      position: relative;
      width: 40px;
      height: 22px;
      flex-shrink: 0;
    }
    .rc-switch input { opacity: 0; width: 0; height: 0; }
    .rc-slider {
      position: absolute;
      inset: 0;
      border-radius: 99px;
      background: #272e45;
      cursor: pointer;
      transition: background 140ms ease;
    }
    .rc-slider::before {
      content: "";
      position: absolute;
      width: 16px; height: 16px;
      left: 3px; top: 3px;
      border-radius: 50%;
      background: #cfd7ee;
      transition: transform 140ms ease, background 140ms ease;
    }
    .rc-switch input:checked + .rc-slider { background: #ff6b2b; }
    .rc-switch input:checked + .rc-slider::before {
      transform: translateX(18px);
      background: #fff;
    }

    .rc-select, .rc-input, .rc-textarea {
      background: #0b0d14;
      border: 1px solid rgba(255, 255, 255, 0.14);
      border-radius: 7px;
      color: #eef1f8;
      padding: 6px 10px;
      font-size: 12.5px;
      font-family: inherit;
    }
    .rc-select:focus, .rc-input:focus, .rc-textarea:focus {
      outline: none;
      border-color: #ff6b2b;
    }
    .rc-textarea {
      width: 100%;
      min-height: 130px;
      font-family: Consolas, "Cascadia Code", monospace;
      font-size: 12px;
      resize: vertical;
      user-select: text;
    }

    .rc-theme-grid {
      display: grid;
      grid-template-columns: repeat(4, 1fr);
      gap: 9px;
      margin-bottom: 14px;
    }
    .rc-theme-card {
      padding: 10px;
      border-radius: 10px;
      border: 2px solid rgba(255, 255, 255, 0.09);
      background: #141826;
      cursor: pointer;
      text-align: left;
      color: #eef1f8;
      font-family: inherit;
    }
    .rc-theme-card.active {
      border-color: #ff6b2b;
      background: rgba(255, 107, 43, 0.12);
    }
    .rc-theme-swatch {
      height: 28px;
      border-radius: 6px;
      margin-bottom: 7px;
      border: 1px solid rgba(255,255,255,0.12);
    }
    .rc-theme-card strong { display: block; font-size: 12px; }
    .rc-theme-card span { display: block; font-size: 10.5px; color: #8c96b4; margin-top: 2px; }

    .rc-range-wrap { display: flex; align-items: center; gap: 10px; }
    .rc-range { accent-color: #ff6b2b; width: 150px; cursor: pointer; }

    .rc-kbd {
      display: inline-block;
      padding: 2px 6px;
      border-radius: 5px;
      background: #1d2236;
      border: 1px solid rgba(255,255,255,0.16);
      font-family: Consolas, monospace;
      font-size: 11px;
      color: #d7e0ff;
    }

    /* Profiles */
    .rc-prof-card {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      padding: 11px 13px;
      border-radius: 10px;
      background: #141826;
      border: 1px solid rgba(255, 255, 255, 0.08);
      margin-bottom: 8px;
    }
    .rc-prof-card.active {
      border-color: rgba(88, 101, 242, 0.65);
      background: rgba(88, 101, 242, 0.11);
    }
    .rc-prof-left { display: flex; align-items: center; gap: 10px; min-width: 0; }
    .rc-prof-avatar {
      width: 34px; height: 34px;
      border-radius: 10px;
      display: flex; align-items: center; justify-content: center;
      font-weight: 800; font-size: 14px; color: #fff;
      flex-shrink: 0;
    }
    .rc-prof-actions { display: flex; gap: 6px; flex-shrink: 0; }

    .rc-form-grid {
      display: grid;
      grid-template-columns: 1fr 120px;
      gap: 8px;
      margin-top: 8px;
    }
    .rc-colors { display: flex; gap: 6px; margin: 8px 0; }
    .rc-color-dot {
      width: 22px; height: 22px;
      border-radius: 50%;
      border: 2px solid transparent;
      cursor: pointer;
    }
    .rc-color-dot.selected { border-color: #fff; transform: scale(1.1); }

    /* Compare Table */
    .rc-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 12.5px;
      margin-bottom: 14px;
    }
    .rc-table th, .rc-table td {
      text-align: left;
      padding: 9px 10px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.07);
    }
    .rc-table th { color: #8c96b4; font-size: 11px; text-transform: uppercase; }
    .rc-win { color: #5ce69a; font-weight: 700; }
    .rc-dim { color: #8c96b4; }
  `;

  function mountUi() {
    if (document.getElementById("rustcord-host")) return;
    const host = document.createElement("div");
    host.id = "rustcord-host";
    host.style.cssText = "position:fixed;top:0;left:0;z-index:2147483640;pointer-events:auto;";
    (document.documentElement || document.body).appendChild(host);
    shadow = host.attachShadow({ mode: "open" });

    shadow.innerHTML = `
      <style>${SHADOW_CSS}</style>
      <div class="rc-bar" id="rcBar">
        <div class="rc-left">
          <span class="rc-brand"><span class="rc-brand-dot"></span>Rustcord</span>
          <button class="rc-btn" id="rcProfileBtn" title="アカウント切替">
            <span class="rc-profile-dot" id="rcProfileDot" style="background:#5865F2"></span>
            <span id="rcProfileName">メイン</span>
            <span>▾</span>
          </button>
          <span class="rc-voice-pill" id="rcVoicePill">🎙 通話保護中</span>
        </div>
        <div class="rc-right">
          <button class="rc-btn ram-low" id="rcRamBtn" title="クリックで今すぐメモリ解放 (Ctrl+Alt+F)">
            ⚡ <span id="rcRamText">-- MB</span>
          </button>
          <span class="rc-btn" id="rcBlockedPill" title="ネイティブ遮断したテレメトリ・解析通信数" style="cursor:default;color:#8da2ff;border-color:rgba(141,162,255,0.28)">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor" style="flex-shrink:0"><path d="M12 2L4 5v6.09c0 5.05 3.41 9.76 8 10.91 4.59-1.15 8-5.86 8-10.91V5l-8-3z"/></svg>
            <span id="rcBlockedText">0</span>
          </span>
          <button class="rc-btn" id="rcLiteBtn" title="軽量モード切替">⚡ 軽量</button>
          <button class="rc-btn" id="rcOledBtn" title="OLED漆黒テーマ切替">🌑 OLED</button>
          <button class="rc-btn" id="rcPinBtn" title="最前面に固定 (Ctrl+Alt+T)">📌</button>
          <button class="rc-btn" id="rcHibernateBtn" title="休止モードにして超省メモリ化 (Ctrl+Alt+H)">💤</button>
          <button class="rc-btn" id="rcSettingsBtn" title="Rustcord 設定 (Ctrl+Shift+,)">⚙️ 設定</button>
          <button class="rc-btn" id="rcHideBarBtn" title="バーを折りたたむ (Ctrl+Shift+B)">▲</button>
        </div>
      </div>

      <button class="rc-floating-pill" id="rcFloatingPill" title="Rustcord バーを表示 / 設定を開く">
        🦀 <span id="rcFloatRam">-- MB</span> ▾
      </button>

      <div class="rc-dropdown" id="rcProfileDropdown"></div>
      <div class="rc-toast" id="rcToast"></div>

      <div class="rc-overlay" id="rcOverlay">
        <div class="rc-modal" id="rcModal">
          <nav class="rc-nav">
            <div class="rc-nav-logo">
              <h2>🦀 Rustcord</h2>
              <p>Ultra-Light Rust Client</p>
            </div>
            <button class="rc-tab active" data-tab="perf">⚡ 軽量化・メモリ</button>
            <button class="rc-tab" data-tab="ui">🎨 見た目・UI調整</button>
            <button class="rc-tab" data-tab="gamer">🎮 ゲーマー・操作</button>
            <button class="rc-tab" data-tab="profiles">👤 マルチアカウント</button>
            <button class="rc-tab" data-tab="about">ℹ️ 比較・診断</button>
            <div class="rc-nav-foot">
              <div>設定開閉: <span class="rc-kbd">Ctrl+Shift+,</span></div>
              <div>バー切替: <span class="rc-kbd">Ctrl+Shift+B</span></div>
            </div>
          </nav>
          <div class="rc-content">
            <div class="rc-modal-head">
              <h3 id="rcTabTitle">⚡ 軽量化・メモリ管理</h3>
              <button class="rc-close" id="rcCloseModal" title="閉じる (Esc)">✕</button>
            </div>
            <div class="rc-body" id="rcBody"></div>
          </div>
        </div>
      </div>
    `;

    wireTopBarEvents();
  }

  function wireTopBarEvents() {
    const $ = (id) => shadow.getElementById(id);

    $("rcRamBtn").addEventListener("click", () => {
      post({ cmd: "flush_ram" });
    });

    $("rcLiteBtn").addEventListener("click", () => {
      if (!state.config) return;
      state.config.performance.lite_mode = !state.config.performance.lite_mode;
      saveConfig();
      showToast(
        state.config.performance.lite_mode
          ? "⚡ 軽量モード ON（アニメ・ブラー停止）"
          : "軽量モード OFF",
        "info"
      );
    });

    $("rcOledBtn").addEventListener("click", () => {
      if (!state.config) return;
      state.config.appearance.theme =
        state.config.appearance.theme === "oled_black" ? "default" : "oled_black";
      saveConfig();
      showToast(
        state.config.appearance.theme === "oled_black"
          ? "🌑 OLED 漆黒テーマを適用しました"
          : "Discord 標準テーマに戻しました",
        "info"
      );
    });

    $("rcPinBtn").addEventListener("click", () => {
      post({ cmd: "toggle_pin" });
    });

    $("rcHibernateBtn").addEventListener("click", () => {
      post({ cmd: "hibernate_now" });
    });

    $("rcSettingsBtn").addEventListener("click", () => openModal("perf"));

    $("rcHideBarBtn").addEventListener("click", () => {
      post({ cmd: "toggle_top_bar", visible: false });
    });

    $("rcFloatingPill").addEventListener("click", () => {
      post({ cmd: "toggle_top_bar", visible: true });
    });

    $("rcProfileBtn").addEventListener("click", (e) => {
      e.stopPropagation();
      state.profileMenuOpen = !state.profileMenuOpen;
      renderProfileDropdown();
    });

    $("rcCloseModal").addEventListener("click", closeModal);
    $("rcOverlay").addEventListener("click", (e) => {
      if (e.target === $("rcOverlay")) closeModal();
    });

    shadow.querySelectorAll(".rc-tab").forEach((btn) => {
      btn.addEventListener("click", () => {
        state.activeTab = btn.dataset.tab;
        shadow.querySelectorAll(".rc-tab").forEach((b) =>
          b.classList.toggle("active", b.dataset.tab === state.activeTab)
        );
        renderModalBody();
      });
    });

    document.addEventListener("click", () => {
      if (state.profileMenuOpen) {
        state.profileMenuOpen = false;
        renderProfileDropdown();
      }
    });

    // In-app keyboard shortcuts
    window.addEventListener("keydown", (e) => {
      if (e.ctrlKey && e.shiftKey && (e.key === "," || e.code === "Comma")) {
        e.preventDefault();
        if (state.modalOpen) closeModal();
        else openModal(state.activeTab);
      } else if (e.ctrlKey && e.shiftKey && e.code === "KeyB") {
        e.preventDefault();
        if (state.config) {
          post({ cmd: "toggle_top_bar", visible: !state.config.appearance.show_top_bar });
        }
      } else if (e.key === "Escape" && state.modalOpen) {
        closeModal();
      }
    });
  }

  function saveConfig() {
    if (!state.config) return;
    post({ cmd: "save_config", config: state.config });
    updateTopBar();
  }

  function openModal(tab) {
    if (tab) state.activeTab = tab;
    state.modalOpen = true;
    state.profileMenuOpen = false;
    renderProfileDropdown();
    const ov = shadow.getElementById("rcOverlay");
    ov.classList.add("open");
    shadow.querySelectorAll(".rc-tab").forEach((b) =>
      b.classList.toggle("active", b.dataset.tab === state.activeTab)
    );
    renderModalBody();
  }

  function closeModal() {
    state.modalOpen = false;
    const ov = shadow?.getElementById("rcOverlay");
    if (ov) ov.classList.remove("open");
  }

  function showToast(text, kind = "info") {
    if (!shadow) return;
    const el = shadow.getElementById("rcToast");
    if (!el) return;
    el.textContent = text;
    el.className = `rc-toast show ${kind}`;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      el.classList.remove("show");
    }, 2800);
  }

  function updateTopBar() {
    if (!shadow || !state.config) return;
    const cfg = state.config;
    const stats = state.stats;
    const $ = (id) => shadow.getElementById(id);

    const bar = $("rcBar");
    const floatPill = $("rcFloatingPill");
    bar.classList.toggle("hidden", !cfg.appearance.show_top_bar);
    bar.classList.toggle("oled", cfg.appearance.theme === "oled_black");
    floatPill.classList.toggle("show", !cfg.appearance.show_top_bar);

    const activeProf =
      cfg.profiles.find((p) => p.id === cfg.active_profile_id) || cfg.profiles[0];
    if (activeProf) {
      $("rcProfileDot").style.background = activeProf.color;
      $("rcProfileName").textContent = activeProf.name;
    }

    const ramText = stats.total_mb > 0 ? `${Math.round(stats.total_mb)} MB` : "-- MB";
    $("rcRamText").textContent = ramText;
    $("rcFloatRam").textContent = ramText;

    const ramBtn = $("rcRamBtn");
    ramBtn.classList.remove("ram-low", "ram-mid", "ram-high");
    if (stats.total_mb < 240) ramBtn.classList.add("ram-low");
    else if (stats.total_mb < 380) ramBtn.classList.add("ram-mid");
    else ramBtn.classList.add("ram-high");

    $("rcBlockedText").textContent = String(stats.blocked_requests || 0);
    $("rcLiteBtn").classList.toggle("active", cfg.performance.lite_mode);
    $("rcOledBtn").classList.toggle("active-blue", cfg.appearance.theme === "oled_black");
    $("rcPinBtn").classList.toggle("active", cfg.gamer.always_on_top);
    $("rcVoicePill").classList.toggle("on", Boolean(stats.in_voice));
  }

  function renderProfileDropdown() {
    if (!shadow || !state.config) return;
    const dd = shadow.getElementById("rcProfileDropdown");
    dd.classList.toggle("open", state.profileMenuOpen);
    if (!state.profileMenuOpen) return;

    const cfg = state.config;
    const sizes = new Map((state.stats.profile_sizes || []).map((s) => [s.id, s.disk_mb]));

    let html = `<div class="rc-dd-title">マルチアカウント切替（完全分離）</div>`;
    for (const p of cfg.profiles) {
      const isCur = p.id === cfg.active_profile_id;
      const disk = sizes.has(p.id) ? `${sizes.get(p.id).toFixed(0)} MB` : "";
      html += `
        <button class="rc-dd-item ${isCur ? "current" : ""}" data-switch="${esc(p.id)}">
          <span style="display:flex;align-items:center;gap:8px;min-width:0">
            <span class="rc-profile-dot" style="background:${esc(p.color)}"></span>
            <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(p.name)}</span>
          </span>
          <span class="rc-dd-meta">${isCur ? "使用中" : disk}</span>
        </button>
      `;
    }
    html += `
      <div class="rc-dd-sep"></div>
      <button class="rc-dd-item" data-open-profiles="1">
        <span>＋ アカウント管理・新規追加</span>
        <span class="rc-dd-meta">⚙️</span>
      </button>
    `;
    dd.innerHTML = html;

    dd.querySelectorAll("[data-switch]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const id = btn.dataset.switch;
        state.profileMenuOpen = false;
        renderProfileDropdown();
        if (id !== cfg.active_profile_id) {
          showToast("👤 アカウントを切り替え中…", "info");
          post({ cmd: "switch_profile", id });
        }
      });
    });

    const openBtn = dd.querySelector("[data-open-profiles]");
    if (openBtn) {
      openBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        openModal("profiles");
      });
    }
  }

  function buildSparklineSvg() {
    const pts = state.ramHistory;
    if (pts.length < 2) {
      return `<div style="height:64px;display:flex;align-items:center;justify-content:center;color:#6f7894;font-size:11.5px;">メモリ推移を計測中…</div>`;
    }
    const w = 560;
    const h = 64;
    const min = Math.max(0, Math.min(...pts) - 20);
    const max = Math.max(...pts, min + 40);
    const span = max - min || 1;
    const coords = pts.map((v, i) => {
      const x = (i / (pts.length - 1)) * w;
      const y = h - 4 - ((v - min) / span) * (h - 10);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    });
    const area = `0,${h} ${coords.join(" ")} ${w},${h}`;
    return `
      <svg viewBox="0 0 ${w} ${h}" width="100%" height="${h}" preserveAspectRatio="none" style="display:block">
        <defs>
          <linearGradient id="rcGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#ff6b2b" stop-opacity="0.38"/>
            <stop offset="100%" stop-color="#5865f2" stop-opacity="0.02"/>
          </linearGradient>
        </defs>
        <polygon points="${area}" fill="url(#rcGrad)"/>
        <polyline points="${coords.join(" ")}" fill="none" stroke="#ff7a3d" stroke-width="2" stroke-linejoin="round"/>
      </svg>
    `;
  }

  function renderModalBody() {
    if (!shadow || !state.config || !state.modalOpen) return;
    const cfg = state.config;
    const st = state.stats;
    const titleEl = shadow.getElementById("rcTabTitle");
    const body = shadow.getElementById("rcBody");

    const titles = {
      perf: "⚡ 軽量化・省メモリ管理",
      ui: "🎨 見た目・UI調整・カスタムCSS",
      gamer: "🎮 ゲーマー向け機能・ホットキー",
      profiles: "👤 マルチアカウント（プロファイル分離）",
      about: "ℹ️ 本家Discord比較・診断",
    };
    titleEl.textContent = titles[state.activeTab] || "Rustcord";

    if (state.activeTab === "perf") {
      body.innerHTML = `
        <div class="rc-stats-grid">
          <div class="rc-stat-card">
            <div class="rc-stat-label">現在の合計 RAM</div>
            <div class="rc-stat-val" id="rcStatTotal">${st.total_mb.toFixed(0)} MB</div>
            <div class="rc-stat-sub" id="rcStatTotalSub">Rust ${st.host_mb.toFixed(1)} + Web ${st.webview_mb.toFixed(0)} MB</div>
          </div>
          <div class="rc-stat-card">
            <div class="rc-stat-label">解放したメモリ累計</div>
            <div class="rc-stat-val" style="color:#5ce69a" id="rcStatFreed">${st.total_freed_mb.toFixed(0)} MB</div>
            <div class="rc-stat-sub" id="rcStatPeakSub">ピーク ${st.peak_mb.toFixed(0)} MB</div>
          </div>
          <div class="rc-stat-card">
            <div class="rc-stat-label">遮断した追跡通信</div>
            <div class="rc-stat-val" style="color:#8da2ff" id="rcStatBlocked">${st.blocked_requests}</div>
            <div class="rc-stat-sub">テレメトリ / 解析カット</div>
          </div>
          <div class="rc-stat-card">
            <div class="rc-stat-label">稼働プロセス数</div>
            <div class="rc-stat-val" id="rcStatProcs">${st.process_count}</div>
            <div class="rc-stat-sub" id="rcStatVoiceSub">${st.in_voice ? "🎙 通話中(保護)" : "通常待機"}</div>
          </div>
        </div>

        <div class="rc-chart-box">
          <div class="rc-chart-head">
            <span>リアルタイム RAM 推移（Rustcord + WebView2 合計 Private Working Set）</span>
            <span id="rcChartSummary">現在 ${st.total_mb.toFixed(1)} MB / Commit ${st.private_mb.toFixed(0)} MB</span>
          </div>
          <div id="rcSparkWrap">${buildSparklineSvg()}</div>
        </div>

        <div class="rc-actions-row">
          <button class="rc-cta" id="btnFlushNow">🧹 今すぐメモリ解放 (EmptyWorkingSet)</button>
          <button class="rc-cta secondary" id="btnHibernateNow">💤 トレイへ格納＆休止モード</button>
        </div>

        <div class="rc-section-title">リアルタイム軽量化</div>
        <div class="rc-section">
          ${toggleRow("perf_lite", "⚡ 軽量モード（アニメーション・ブラー完全停止）", "CSSアニメ・トランジション・ブラー・プロフィール装飾を無効化してCPU/GPU負荷を削減", cfg.performance.lite_mode)}
          ${toggleRow("perf_telemetry", "🛡 テレメトリ・解析通信をネイティブ遮断", "Discordの /science・/metrics・Sentry 送信をRust側WebView2層で0ms遮断", cfg.performance.block_telemetry)}
          ${toggleRow("perf_typing", "👻 ステルス入力（タイピング通知を遮断）", "「〇〇が入力中…」の通信 (/typing) をブロックして通信量とプライバシーを保護", cfg.performance.block_typing)}
          ${toggleRow("perf_gif", "🖼 GIF・動くアイコンの自動再生を抑制", "通常時は静止画表示にし、マウスを乗せた時だけ再生してVRAMとCPUを節約", cfg.performance.disable_gif_autoplay)}
        </div>

        <div class="rc-section-title">バックグラウンド・エンジン最適化</div>
        <div class="rc-section">
          <div class="rc-row">
            <div class="rc-row-info">
              <strong>トレイ格納時の省メモリ方式</strong>
              <span>※ボイス通話中は自動的にディープ休止をスキップし、通話を一切途切れさせません</span>
            </div>
            <select class="rc-select" id="selTrayMode">
              <option value="smart_trim" ${cfg.performance.tray_mode === "smart_trim" ? "selected" : ""}>⚡ スマート省メモリ（通知維持・推奨）</option>
              <option value="deep_suspend" ${cfg.performance.tray_mode === "deep_suspend" ? "selected" : ""}>💤 ディープ休止（WebView2停止・超省電力）</option>
              <option value="normal" ${cfg.performance.tray_mode === "normal" ? "selected" : ""}>通常（最小化のみ）</option>
            </select>
          </div>
          <div class="rc-row">
            <div class="rc-row-info">
              <strong>自動定期メモリ解放</strong>
              <span>一定時間ごとに WebView2 GC＋ワーキングセット解放を実行して肥大化を防止</span>
            </div>
            <select class="rc-select" id="selAutoTrim">
              <option value="0" ${cfg.performance.auto_trim_minutes === 0 ? "selected" : ""}>OFF</option>
              <option value="3" ${cfg.performance.auto_trim_minutes === 3 ? "selected" : ""}>3分ごと</option>
              <option value="5" ${cfg.performance.auto_trim_minutes === 5 ? "selected" : ""}>5分ごと（推奨）</option>
              <option value="10" ${cfg.performance.auto_trim_minutes === 10 ? "selected" : ""}>10分ごと</option>
              <option value="15" ${cfg.performance.auto_trim_minutes === 15 ? "selected" : ""}>15分ごと</option>
            </select>
          </div>
          <div class="rc-row">
            <div class="rc-row-info">
              <strong>🛡 オート RAM ガード（上限自動解放）</strong>
              <span>RAM が指定値を超えたら自動で V8 GC＋メモリ解放を実行（60秒クールダウン付き）</span>
            </div>
            <select class="rc-select" id="selAutoGuard">
              <option value="0" ${(cfg.performance.auto_guard_mb || 0) === 0 ? "selected" : ""}>OFF</option>
              <option value="250" ${cfg.performance.auto_guard_mb === 250 ? "selected" : ""}>250 MB 超で自動解放</option>
              <option value="350" ${cfg.performance.auto_guard_mb === 350 ? "selected" : ""}>350 MB 超で自動解放（推奨）</option>
              <option value="500" ${cfg.performance.auto_guard_mb === 500 ? "selected" : ""}>500 MB 超で自動解放</option>
            </select>
          </div>
          <div class="rc-row">
            <div class="rc-row-info">
              <strong>JSヒープ上限キャップ（再起動 / アカウント切替で反映）</strong>
              <span>V8の最大メモリ上限を抑えて長時間起動時のメモリ膨張を防ぎます</span>
            </div>
            <select class="rc-select" id="selHeap">
              <option value="0" ${cfg.performance.js_heap_limit_mb === 0 ? "selected" : ""}>無制限</option>
              <option value="256" ${cfg.performance.js_heap_limit_mb === 256 ? "selected" : ""}>256 MB（超軽量）</option>
              <option value="384" ${cfg.performance.js_heap_limit_mb === 384 ? "selected" : ""}>384 MB（推奨）</option>
              <option value="512" ${cfg.performance.js_heap_limit_mb === 512 ? "selected" : ""}>512 MB（バランス）</option>
            </select>
          </div>
          ${toggleRow("perf_gpu", "ハードウェアアクセラレーション（GPU使用・次回反映）", "重いゲームでVRAMを節約したい場合はOFFにするとGPU負荷を完全カットできます", cfg.performance.hardware_acceleration)}
        </div>
      `;

      body.querySelector("#btnFlushNow").addEventListener("click", () => post({ cmd: "flush_ram" }));
      body.querySelector("#btnHibernateNow").addEventListener("click", () => {
        closeModal();
        post({ cmd: "hibernate_now" });
      });
      bindToggle(body, "perf_lite", (v) => (cfg.performance.lite_mode = v));
      bindToggle(body, "perf_telemetry", (v) => (cfg.performance.block_telemetry = v));
      bindToggle(body, "perf_typing", (v) => (cfg.performance.block_typing = v));
      bindToggle(body, "perf_gif", (v) => {
        cfg.performance.disable_gif_autoplay = v;
        syncGifObserver();
      });
      bindToggle(body, "perf_gpu", (v) => {
        cfg.performance.hardware_acceleration = v;
        showToast("GPU設定は次回起動またはアカウント切替時に反映されます", "info");
      });
      body.querySelector("#selTrayMode").addEventListener("change", (e) => {
        cfg.performance.tray_mode = e.target.value;
        saveConfig();
      });
      body.querySelector("#selAutoTrim").addEventListener("change", (e) => {
        cfg.performance.auto_trim_minutes = parseInt(e.target.value, 10) || 0;
        saveConfig();
      });
      body.querySelector("#selAutoGuard").addEventListener("change", (e) => {
        cfg.performance.auto_guard_mb = parseInt(e.target.value, 10) || 0;
        saveConfig();
      });
      body.querySelector("#selHeap").addEventListener("change", (e) => {
        cfg.performance.js_heap_limit_mb = parseInt(e.target.value, 10) || 0;
        saveConfig();
        showToast("ヒープ上限は次回起動またはアカウント切替時に反映されます", "info");
      });
    } else if (state.activeTab === "ui") {
      body.innerHTML = `
        <div class="rc-section-title">テーマプリセット（即時反映）</div>
        <div class="rc-theme-grid">
          ${themeCard("default", "Discord 標準", "おなじみの公式配色", "linear-gradient(135deg,#313338,#1e1f22)", cfg.appearance.theme)}
          ${themeCard("oled_black", "OLED 漆黒", "完全 #000000 省電力", "linear-gradient(135deg,#000000,#0c0e14)", cfg.appearance.theme)}
          ${themeCard("rust_midnight", "Rust Midnight", "濃紺 × Rustオレンジ", "linear-gradient(135deg,#0f121b,#ff6b2b)", cfg.appearance.theme)}
          ${themeCard("compact_dark", "コンパクト", "余白圧縮・情報量UP", "linear-gradient(135deg,#1d2028,#5865f2)", cfg.appearance.theme)}
        </div>

        <div class="rc-section-title">不要UI非表示（デブロート）</div>
        <div class="rc-section">
          ${toggleRow("ui_nitro", "Nitro / ショップ / クエストを非表示", "DM一覧の上部にあるNitroやショップ広告タブをすっきり隠します", cfg.appearance.hide_nitro_shop)}
          ${toggleRow("ui_gift", "ギフトボタンを非表示", "メッセージ入力欄の「Nitroをプレゼント」ボタンを隠します", cfg.appearance.hide_gift_button)}
          ${toggleRow("ui_boost", "サーバーブースト・アップセルバナー非表示", "各種プロモーションバナーやブースト目標表示を非表示にします", cfg.appearance.hide_boost_banner)}
          ${toggleRow("ui_nowplaying", "「アクティビティ中」列を非表示", "フレンド画面右側のNow Playing欄を隠してスッキリさせます", cfg.appearance.hide_now_playing)}
          ${toggleRow("ui_help", "ヘルプ（?）ボタンを非表示", "右上のサポートリンクボタンを隠します", cfg.appearance.hide_help_button)}
          ${toggleRow("ui_stickers", "GIF / スタンプ / アプリボタンを非表示", "チャット入力欄をテキスト中心の超シンプル構成にします", cfg.appearance.hide_sticker_gif_buttons)}
          ${toggleRow("ui_members", "メンバーリスト自動折りたたみ", "普段はアイコンだけの細いバーにし、マウスを乗せると展開します", cfg.appearance.auto_collapse_members)}
          ${toggleRow("ui_topbar", "Rustcord トップバーを表示", "OFFにすると100%全画面表示（右上の小さなバッジまたは Ctrl+Shift+B で復帰）", cfg.appearance.show_top_bar)}
        </div>

        <div class="rc-section-title">カスタム CSS（リアルタイム反映）</div>
        <div class="rc-section" style="padding:12px">
          <div class="rc-actions-row" style="margin-bottom:8px">
            <button class="rc-cta secondary" id="snipFont" style="padding:5px 10px;font-size:11.5px">+ 游ゴシック/メイリオ見やすく</button>
            <button class="rc-cta secondary" id="snipSquare" style="padding:5px 10px;font-size:11.5px">+ アイコン角丸四角</button>
            <button class="rc-cta secondary" id="snipClean" style="padding:5px 10px;font-size:11.5px">+ クリア</button>
          </div>
          <textarea class="rc-textarea" id="txtCustomCss" placeholder="/* 例: チャット文字サイズを大きくする */&#10;[class*='markup_'] { font-size: 15px !important; }">${esc(cfg.appearance.custom_css)}</textarea>
          <div style="display:flex;justify-content:flex-end;margin-top:8px">
            <button class="rc-cta" id="btnApplyCss">✨ カスタムCSSを適用・保存</button>
          </div>
        </div>
      `;

      body.querySelectorAll(".rc-theme-card").forEach((card) => {
        card.addEventListener("click", () => {
          cfg.appearance.theme = card.dataset.theme;
          saveConfig();
          renderModalBody();
        });
      });

      bindToggle(body, "ui_nitro", (v) => (cfg.appearance.hide_nitro_shop = v));
      bindToggle(body, "ui_gift", (v) => (cfg.appearance.hide_gift_button = v));
      bindToggle(body, "ui_boost", (v) => (cfg.appearance.hide_boost_banner = v));
      bindToggle(body, "ui_nowplaying", (v) => (cfg.appearance.hide_now_playing = v));
      bindToggle(body, "ui_help", (v) => (cfg.appearance.hide_help_button = v));
      bindToggle(body, "ui_stickers", (v) => (cfg.appearance.hide_sticker_gif_buttons = v));
      bindToggle(body, "ui_members", (v) => (cfg.appearance.auto_collapse_members = v));
      bindToggle(body, "ui_topbar", (v) => (cfg.appearance.show_top_bar = v));

      const cssArea = body.querySelector("#txtCustomCss");
      body.querySelector("#btnApplyCss").addEventListener("click", () => {
        cfg.appearance.custom_css = cssArea.value;
        saveConfig();
        showToast("🎨 カスタムCSSを適用しました", "ok");
      });
      body.querySelector("#snipFont").addEventListener("click", () => {
        cssArea.value += `\nbody, [class*="markup_"] {\n  font-family: "Yu Gothic UI", "Meiryo", sans-serif !important;\n}\n`;
      });
      body.querySelector("#snipSquare").addEventListener("click", () => {
        cssArea.value += `\n[class*="avatar_"], [class*="wrapper_"] svg mask circle {\n  border-radius: 22% !important;\n}\n`;
      });
      body.querySelector("#snipClean").addEventListener("click", () => {
        cssArea.value = "";
        cfg.appearance.custom_css = "";
        saveConfig();
        showToast("カスタムCSSをクリアしました", "info");
      });
    } else if (state.activeTab === "gamer") {
      body.innerHTML = `
        <div class="rc-section-title">ウィンドウ・オーバーレイ設定</div>
        <div class="rc-section">
          ${toggleRow("g_pin", "📌 常に最前面に表示（Always on Top）", "ゲームのウィンドウやブラウザの上に Rustcord を固定表示します", cfg.gamer.always_on_top)}
          <div class="rc-row">
            <div class="rc-row-info">
              <strong>ウィンドウ透過度（ゲーム重ね表示向け）</strong>
              <span>最前面固定と組み合わせると、半透明チャットオーバーレイとして使えます</span>
            </div>
            <div class="rc-range-wrap">
              <input type="range" min="50" max="100" value="${cfg.gamer.window_opacity}" class="rc-range" id="rngOpacity"/>
              <span id="lblOpacity" style="min-width:38px;text-align:right;font-weight:700">${cfg.gamer.window_opacity}%</span>
            </div>
          </div>
          ${toggleRow("g_title_ram", "タイトルバーにリアルタイム RAM を表示", "バーを非表示にしてもウィンドウタイトルでメモリ使用量を常時確認できます", cfg.gamer.show_ram_in_title)}
          ${toggleRow("g_tray_close", "× ボタンでシステムトレイに最小化（常駐）", "誤って通話や通知を切らないよう、ウィンドウを閉じてもトレイに常駐します", cfg.gamer.close_to_tray)}
          ${toggleRow("g_mute_beep", "グローバルミュート時にビープ音を鳴らす", "フルスクリーンゲーム中でもミュートON/OFFが音で分かります", cfg.gamer.mute_beep)}
        </div>

        <div class="rc-section-title">グローバルホットキー（ゲーム中・バックグラウンドでも有効）</div>
        <div class="rc-section">
          ${toggleRow("hk_master", "グローバルホットキー全体を有効化", "OFFにすると他のアプリとキーが一切干渉しなくなります", cfg.gamer.hotkeys_enabled)}
          ${toggleRow("hk_show", '表示 / トレイ格納 <span class="rc-kbd">Ctrl + Alt + D</span>', "Rustcord を瞬時に呼び出し・隠す", cfg.gamer.hotkey_show_hide)}
          ${toggleRow("hk_pin", '最前面固定トグル <span class="rc-kbd">Ctrl + Alt + T</span>', "ワンキーで最前面のON/OFFを切り替え", cfg.gamer.hotkey_always_on_top)}
          ${toggleRow("hk_flush", '今すぐメモリ解放 <span class="rc-kbd">Ctrl + Alt + F</span>', "ゲーム中の一瞬で WebView2 のRAMを解放", cfg.gamer.hotkey_flush_ram)}
          ${toggleRow("hk_mute", 'どこからでもマイクミュート <span class="rc-kbd">Ctrl + Alt + M</span>', "別ウィンドウ操作中でも Discord のマイクを即ミュート", cfg.gamer.hotkey_mic_mute)}
          ${toggleRow("hk_hib", '休止モード切替 <span class="rc-kbd">Ctrl + Alt + H</span>', "通話外なら WebView2 を停止してCPU 0%化", cfg.gamer.hotkey_hibernate)}
        </div>
      `;

      bindToggle(body, "g_pin", (v) => (cfg.gamer.always_on_top = v));
      bindToggle(body, "g_title_ram", (v) => (cfg.gamer.show_ram_in_title = v));
      bindToggle(body, "g_tray_close", (v) => (cfg.gamer.close_to_tray = v));
      bindToggle(body, "g_mute_beep", (v) => (cfg.gamer.mute_beep = v));
      bindToggle(body, "hk_master", (v) => (cfg.gamer.hotkeys_enabled = v));
      bindToggle(body, "hk_show", (v) => (cfg.gamer.hotkey_show_hide = v));
      bindToggle(body, "hk_pin", (v) => (cfg.gamer.hotkey_always_on_top = v));
      bindToggle(body, "hk_flush", (v) => (cfg.gamer.hotkey_flush_ram = v));
      bindToggle(body, "hk_mute", (v) => (cfg.gamer.hotkey_mic_mute = v));
      bindToggle(body, "hk_hib", (v) => (cfg.gamer.hotkey_hibernate = v));

      const rng = body.querySelector("#rngOpacity");
      const lbl = body.querySelector("#lblOpacity");
      rng.addEventListener("input", () => {
        const val = parseInt(rng.value, 10);
        lbl.textContent = `${val}%`;
        cfg.gamer.window_opacity = val;
        post({ cmd: "set_opacity", opacity: val });
      });
      rng.addEventListener("change", () => saveConfig());
    } else if (state.activeTab === "profiles") {
      const sizes = new Map((st.profile_sizes || []).map((s) => [s.id, s.disk_mb]));
      const palette = ["#5865F2", "#FF6B2B", "#23A55A", "#EB459E", "#F0B232", "#00A8FC", "#9B59B6"];
      let selectedColor = "#FF6B2B";

      let cardsHtml = "";
      for (const p of cfg.profiles) {
        const isCur = p.id === cfg.active_profile_id;
        const diskMb = sizes.get(p.id) || 0;
        const chLabel = p.channel === "ptb" ? "PTB" : p.channel === "canary" ? "Canary" : "Stable";
        cardsHtml += `
          <div class="rc-prof-card ${isCur ? "active" : ""}">
            <div class="rc-prof-left">
              <div class="rc-prof-avatar" style="background:${esc(p.color)}">${esc((p.name || "?").slice(0, 1))}</div>
              <div style="min-width:0">
                <div style="font-weight:700;font-size:13.5px;display:flex;align-items:center;gap:6px">
                  <span>${esc(p.name)}</span>
                  <span class="rc-kbd">${chLabel}</span>
                  ${isCur ? `<span style="font-size:10.5px;color:#7ef2ad">● 使用中</span>` : ""}
                </div>
                <div style="font-size:11.5px;color:#8c96b4;margin-top:2px">
                  ${esc(p.note || "独立セッション")} ・ データ容量 ${diskMb.toFixed(1)} MB
                </div>
              </div>
            </div>
            <div class="rc-prof-actions">
              ${!isCur ? `<button class="rc-cta" style="padding:6px 11px;font-size:12px" data-p-switch="${esc(p.id)}">切替</button>` : ""}
              <button class="rc-cta secondary" style="padding:6px 10px;font-size:11.5px" data-p-clean="${esc(p.id)}" title="ログインを維持したままキャッシュだけ削除">🧹 キャッシュ掃除</button>
              ${cfg.profiles.length > 1 ? `<button class="rc-cta danger" style="padding:6px 10px;font-size:11.5px" data-p-del="${esc(p.id)}">削除</button>` : ""}
            </div>
          </div>
        `;
      }

      body.innerHTML = `
        <div style="font-size:12px;color:#98a3c4;margin-bottom:12px;line-height:1.5">
          各プロファイルは完全に独立した Cookie / ログイン情報フォルダで管理されます。切替時に旧アカウントの WebView2 を完全解放するため、<strong>何個登録しても消費メモリは常に1アカウント分だけ</strong>です。
        </div>
        ${cardsHtml}

        <div class="rc-section-title">＋ 新規プロファイル（サブ垢・仕事用など）を作成</div>
        <div class="rc-section" style="padding:12px 14px">
          <div class="rc-form-grid">
            <input class="rc-input" id="inpProfName" placeholder="プロファイル名（例: ゲーム用、サブ垢）" maxlength="30"/>
            <select class="rc-select" id="selProfChannel">
              <option value="stable">Stable (標準)</option>
              <option value="ptb">PTB</option>
              <option value="canary">Canary</option>
            </select>
          </div>
          <input class="rc-input" id="inpProfNote" placeholder="メモ（任意: 例 FPS仲間用）" style="width:100%;margin-top:8px" maxlength="60"/>
          <div class="rc-colors" id="profColors">
            ${palette
              .map(
                (c) =>
                  `<button class="rc-color-dot ${c === selectedColor ? "selected" : ""}" data-color="${c}" style="background:${c}" type="button"></button>`
              )
              .join("")}
          </div>
          <button class="rc-cta" id="btnCreateProfile">＋ プロファイルを作成して追加</button>
        </div>
      `;

      body.querySelectorAll("[data-p-switch]").forEach((btn) => {
        btn.addEventListener("click", () => {
          closeModal();
          showToast("👤 プロファイルを高速切替中…", "info");
          post({ cmd: "switch_profile", id: btn.dataset.pSwitch });
        });
      });

      body.querySelectorAll("[data-p-clean]").forEach((btn) => {
        btn.addEventListener("click", () => {
          post({ cmd: "clean_cache", id: btn.dataset.pClean });
        });
      });

      body.querySelectorAll("[data-p-del]").forEach((btn) => {
        btn.addEventListener("click", () => {
          if (btn.dataset.confirmed === "1") {
            post({ cmd: "delete_profile", id: btn.dataset.pDel });
          } else {
            btn.dataset.confirmed = "1";
            btn.textContent = "本当に削除？";
          }
        });
      });

      body.querySelectorAll(".rc-color-dot").forEach((dot) => {
        dot.addEventListener("click", () => {
          selectedColor = dot.dataset.color;
          body.querySelectorAll(".rc-color-dot").forEach((d) =>
            d.classList.toggle("selected", d.dataset.color === selectedColor)
          );
        });
      });

      body.querySelector("#btnCreateProfile").addEventListener("click", () => {
        const name = body.querySelector("#inpProfName").value.trim() || "サブアカウント";
        const channel = body.querySelector("#selProfChannel").value;
        const note = body.querySelector("#inpProfNote").value.trim();
        post({ cmd: "create_profile", name, color: selectedColor, channel, note });
      });
    } else if (state.activeTab === "about") {
      body.innerHTML = `
        <div class="rc-section-title">本家 Electron 版 Discord との比較（実測値）</div>
        <div class="rc-section" style="padding:4px 12px">
          <table class="rc-table">
            <thead>
              <tr><th>項目</th><th>本家 Discord (Electron)</th><th>🦀 Rustcord (実測)</th></tr>
            </thead>
            <tbody>
              <tr>
                <td>アプリ本体サイズ</td>
                <td class="rc-dim">約 350〜500 MB (Chromium丸ごと内蔵)</td>
                <td class="rc-win">${st.binary_size_mb.toFixed(2)} MB（単一ネイティブEXE）</td>
              </tr>
              <tr>
                <td>ウィンドウ起動速度</td>
                <td class="rc-dim">約 3,000〜6,000 ms</td>
                <td class="rc-win">${st.startup_ms} ms</td>
              </tr>
              <tr>
                <td>合計メモリ使用量</td>
                <td class="rc-dim">約 450〜900+ MB</td>
                <td class="rc-win">${st.total_mb.toFixed(1)} MB（Rust本体 ${st.host_mb.toFixed(1)} MB）</td>
              </tr>
              <tr>
                <td>追跡・テレメトリ通信</td>
                <td class="rc-dim">常時バックグラウンド送信</td>
                <td class="rc-win">ネイティブ遮断（累計 ${st.blocked_requests} 件ブロック）</td>
              </tr>
              <tr>
                <td>マルチアカウント</td>
                <td class="rc-dim">毎回ログイン切替 / 複数起動で倍増</td>
                <td class="rc-win">完全分離プロファイル（常にメモリ1垢分）</td>
              </tr>
              <tr>
                <td>通話・画面共有・DM</td>
                <td class="rc-dim">全対応</td>
                <td class="rc-win">本家と同じ WebRTC で完全対応</td>
              </tr>
            </tbody>
          </table>
        </div>

        <div class="rc-section-title">ツール・メンテナンス</div>
        <div class="rc-actions-row">
          <button class="rc-cta secondary" id="btnOpenDir">📁 データフォルダを開く</button>
          <button class="rc-cta secondary" id="btnReload">🔄 ページ再読み込み</button>
          <button class="rc-cta secondary" id="btnDevtools">🛠 DevTools を開く</button>
          <button class="rc-cta danger" id="btnResetCfg">♻️ 設定を初期値に戻す</button>
        </div>
        <div style="font-size:11.5px;color:#7e88a6;word-break:break-all">
          保存先: ${esc(st.data_dir)}
        </div>
      `;

      body.querySelector("#btnOpenDir").addEventListener("click", () => post({ cmd: "open_data_folder" }));
      body.querySelector("#btnReload").addEventListener("click", () => {
        closeModal();
        post({ cmd: "reload_page" });
      });
      body.querySelector("#btnDevtools").addEventListener("click", () => post({ cmd: "open_dev_tools" }));
      const resetBtn = body.querySelector("#btnResetCfg");
      resetBtn.addEventListener("click", () => {
        if (resetBtn.dataset.confirmed === "1") {
          post({ cmd: "reset_config" });
        } else {
          resetBtn.dataset.confirmed = "1";
          resetBtn.textContent = "クリックで初期化を実行";
        }
      });
    }
  }

  function toggleRow(id, title, desc, checked) {
    return `
      <div class="rc-row">
        <div class="rc-row-info">
          <strong>${title}</strong>
          <span>${desc}</span>
        </div>
        <label class="rc-switch">
          <input type="checkbox" id="${id}" ${checked ? "checked" : ""}/>
          <span class="rc-slider"></span>
        </label>
      </div>
    `;
  }

  function bindToggle(root, id, onChange) {
    const el = root.querySelector(`#${id}`);
    if (!el) return;
    el.addEventListener("change", () => {
      onChange(Boolean(el.checked));
      saveConfig();
    });
  }

  function themeCard(key, name, desc, bg, current) {
    return `
      <button class="rc-theme-card ${current === key ? "active" : ""}" data-theme="${key}" type="button">
        <div class="rc-theme-swatch" style="background:${bg}"></div>
        <strong>${name}</strong>
        <span>${desc}</span>
      </button>
    `;
  }

  function esc(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // ---------------------------------------------------------------------------
  // Bridge invoked from Rust via evaluate_script
  // ---------------------------------------------------------------------------
  window.__rustcordBridge = {
    sync(config, cssText) {
      state.config = config;
      mountUi();
      applyDiscordCss(cssText || "");
      updateTopBar();
      if (state.modalOpen && state.activeTab === "profiles") {
        renderModalBody();
      }
      syncGifObserver();
    },
    stats(stats) {
      state.stats = stats;
      if (stats && stats.total_mb > 0) {
        state.ramHistory.push(stats.total_mb);
        if (state.ramHistory.length > 60) state.ramHistory.shift();
      }
      updateTopBar();
      // Live refresh only numbers & sparkline if perf tab is open (no flicker!)
      if (state.modalOpen && state.activeTab === "perf" && shadow) {
        const t = shadow.getElementById("rcStatTotal");
        const tSub = shadow.getElementById("rcStatTotalSub");
        const f = shadow.getElementById("rcStatFreed");
        const pSub = shadow.getElementById("rcStatPeakSub");
        const b = shadow.getElementById("rcStatBlocked");
        const pr = shadow.getElementById("rcStatProcs");
        const vs = shadow.getElementById("rcStatVoiceSub");
        const cs = shadow.getElementById("rcChartSummary");
        const sp = shadow.getElementById("rcSparkWrap");
        if (t) t.textContent = `${stats.total_mb.toFixed(0)} MB`;
        if (tSub) tSub.textContent = `Rust ${stats.host_mb.toFixed(1)} + Web ${stats.webview_mb.toFixed(0)} MB`;
        if (f) f.textContent = `${stats.total_freed_mb.toFixed(0)} MB`;
        if (pSub) pSub.textContent = `ピーク ${stats.peak_mb.toFixed(0)} MB`;
        if (b) b.textContent = String(stats.blocked_requests);
        if (pr) pr.textContent = String(stats.process_count);
        if (vs) vs.textContent = stats.in_voice ? "🎙 通話中(保護)" : "通常待機";
        if (cs) cs.textContent = `現在 ${stats.total_mb.toFixed(1)} MB / Commit ${stats.private_mb.toFixed(0)} MB`;
        if (sp) sp.innerHTML = buildSparklineSvg();
      }
    },
    toast(msg, kind) {
      showToast(msg, kind || "info");
    },
    openSettings(tab) {
      mountUi();
      openModal(tab || "perf");
    },
    toggleMicMute() {
      toggleMicMute();
    },
  };

  // Boot once DOM is ready
  function boot() {
    mountUi();
    post({ cmd: "ready" });

    // Periodic voice call check scoped strictly to panels (zero full-DOM scan)
    setInterval(() => {
      const active = detectVoiceActive();
      if (active !== state.lastVoice) {
        state.lastVoice = active;
        post({ cmd: "voice_state", active });
      }
    }, 3000);

    // Ultra-lightweight root guard: watches ONLY direct children of <html> (subtree: false)
    const hostObs = new MutationObserver(() => {
      if (!document.getElementById("rustcord-host")) {
        mountUi();
        if (state.config) updateTopBar();
      }
    });
    hostObs.observe(document.documentElement, { childList: true, subtree: false });

    syncGifObserver();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot, { once: true });
  } else {
    boot();
  }
})();
