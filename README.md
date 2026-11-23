<div align="center">
  <img src="assets/rustcord_256.png" width="112" height="112" alt="Rustcord Logo" />
  <h1>🦀 Rustcord</h1>
  <p><strong>本家 Electron 版より圧倒的に軽い、Rust + Windows WebView2 製の超軽量 Discord デスクトップクライアント</strong></p>
  <p>
    <a href="https://github.com/koutyann115/rustcord/releases/latest">
      <img src="https://img.shields.io/github/v/release/koutyann115/rustcord?color=ff6b2b&style=for-the-badge" alt="Latest Release" />
    </a>
    <img src="https://img.shields.io/badge/Binary-~1.05%20MB-5ce69a?style=for-the-badge" alt="Binary Size" />
    <img src="https://img.shields.io/badge/Built%20with-Rust-f74c00?style=for-the-badge&logo=rust&logoColor=white" alt="Rust" />
    <img src="https://img.shields.io/badge/Platform-Windows%2010%2F11-5865F2?style=for-the-badge" alt="Windows" />
    <a href="LICENSE">
      <img src="https://img.shields.io/badge/License-MIT-blue?style=for-the-badge" alt="MIT License" />
    </a>
  </p>
</div>

---

## Rustcord とは？

本家デスクトップ版 Discord は **Electron（Chromium 丸ごと内蔵＋Node.js）** で動いているため、インストール容量が約 350〜500 MB、起動も重くメモリも膨らみがちです。

**Rustcord** は Electron を完全に排除し、**Rust ネイティブシェル（約 1.05 MB の単一 EXE）＋ Windows 標準の Edge WebView2 ＋ Win32 メモリ制御** で動作します。  
**本家 Discord のアカウント・サーバー・DM・ボイス通話・画面共有・通知がそのまま全機能使えて**、さらに本家にはない軽量化・カスタマイズ・ゲーマー向け機能・マルチアカウント分離を備えています。

| 項目 | 本家 Discord (Electron) | 🦀 Rustcord |
|---|---|---|
| **アプリ本体サイズ** | 約 350〜500 MB | **約 `1.05 MB`**（単一ポータブル EXE） |
| **ネイティブホスト層** | Electron + Node.js (~100–200 MB) | **Rust ホスト (`~3–5 MB`)** |
| **テレメトリ通信** | 常時送信 | WebView2 ネットワーク層で **0ms ネイティブ遮断** |
| **省メモリ・休止機能** | なし | **WebView2 `MemoryUsageLevel::Low` + `EmptyWorkingSet` + 通話保護付き `TrySuspend`** |
| **マルチアカウント** | 毎回切替 / 複数起動で負荷倍増 | **完全分離プロファイル**（切替時に旧 WebView2 破棄・常にメモリ 1 垢分） |
| **通話・画面共有・DM** | 全対応 | **公式 WebRTC で全対応** |

---

## 📥 ダウンロード＆起動方法

1. [**Releases ページ**](https://github.com/koutyann115/rustcord/releases/latest) から **`Rustcord.exe`** をダウンロード
2. ダブルクリックでそのまま起動（インストール不要・Windows 10 / 11 の標準 WebView2 を利用）
3. 本家 Discord と同じく QR コード・メール・パスキーでログインすればすぐ使えます

### CLI 起動オプション（ショートカット作成に便利）
```powershell
Rustcord.exe --profile ゲーム用   # 指定したプロファイルで直接起動
Rustcord.exe --settings          # 起動と同時に設定パネルを開く
```

---

## ✨ 主な機能

### ⚡ 省メモリ・軽量化特化
- **正確なリアルタイム RAM 計測**: Windows タスクマネージャーと同じ **Private Working Set** を Rustcord＋配下の WebView2 プロセスツリー単位で集計し、タイトルバー・上部 HUD・リアルタイム推移グラフに表示
- **多層メモリ最適化**:
  - WebView2 公式の `MemoryUsageLevel::Low`（V8 GC ＋ Blink/Skia キャッシュ破棄）
  - Win32 `K32EmptyWorkingSet` ワーキングセット解放
  - V8 `--optimize-for-size` / JSヒープ上限キャップ / GPU メモリ上限設定
  - 軽量モード時の画面外メッセージ描画スキップ（`content-visibility: auto`）
- **🛡 オート RAM ガード**: 設定した閾値（デフォルト `350 MB`）を超えると自動で V8 GC＋メモリ解放を実行（60 秒クールダウン付き）
- **3段階のトレイ格納モード（通話保護付き）**:
  - `⚡ スマート省メモリ（推奨）`: 通知・通話を維持したまま RAM トリム＋プロセス優先度低下
  - `💤 ディープ休止`: WebView2 ネイティブ `TrySuspend` で CPU ほぼ 0% 化（**ボイス通話中は自動検知して休止を保護スキップ**）
  - `通常`: 標準のトレイ最小化
- **ネイティブ・テレメトリ遮断**: `/science`・`/metrics`・Sentry を COM 層で 0ms 遮断（遮断件数カウンター表示）、ステルス入力（`/typing` 遮断）、GIF／動くアバターの自動再生抑制（ホバー時のみ再生）

### 🎨 見た目・UI 調整
- **4種のテーマプリセット**: `Discord 標準` / `OLED 漆黒 (#000000)` / `Rust Midnight` / `コンパクト省スペース`
- **不要 UI の個別非表示**: Nitro・ショップ・クエスト・ギフトボタン・ブーストバナー・Now Playing 列・ヘルプ・GIF/スタンプボタン・メンバーリスト自動折りたたみ
- **カスタム CSS エディタ**: リアルタイム反映＆スニペット付き

### 🎮 ゲーマー向け機能 & ショートカット
- **最前面固定（Always on Top）＋ ウィンドウ透過度（50〜100%）**: ゲームの上に半透明チャットを重ねて表示可能
- **2重起動時の自動復帰**: トレイ収納中・休止中に `Rustcord.exe` を再クリックすると既存ウィンドウを瞬時に前面表示
- **プライバシー保護**: マイク・通知・クリップボードは公式 Discord オリジンのみ自動許可、カメラは確認あり

| ショートカット | 動作 | 適用範囲 |
|---|---|---|
| `Ctrl + Alt + D` | Rustcord 表示 / トレイ格納 | グローバル（ゲーム中でも有効） |
| `Ctrl + Alt + T` | 最前面固定 ON/OFF | グローバル |
| `Ctrl + Alt + F` | 今すぐメモリ解放 | グローバル |
| `Ctrl + Alt + M` | どこからでもマイクミュート（ビープ音付き） | グローバル |
| `Ctrl + Alt + H` | 休止モード切替 | グローバル |
| `Ctrl + Shift + ,` | Rustcord 設定パネル開閉 | アプリ内 |
| `Ctrl + Shift + B` | 上部 Rustcord バー表示/折りたたみ | アプリ内 |

### 👤 マルチアカウント（完全プロファイル分離）
- アカウントごとに独立したデータフォルダ（`%LOCALAPPDATA%\Rustcord\profiles\<id>`）を管理
- 切替時に旧アカウントの WebView2 を完全破棄してから新アカウントを起動するため、何個登録しても消費メモリは常に 1 アカウント分
- プロファイルごとに `Stable` / `PTB` / `Canary` チャンネルを選択可能
- **ログイン維持キャッシュ掃除**: Cookie / LocalStorage / IndexedDB を保持したまま肥大化したキャッシュのみ安全に削除

---

## 🛠️ ソースからビルドする

### 必要環境
- Windows 10 / 11（Edge WebView2 ランタイム標準搭載）
- Rust 1.85+（`edition = "2024"`）および Visual Studio C++ Build Tools

### ビルド手順
```powershell
git clone https://github.com/koutyann115/rustcord.git
cd rustcord
cargo build --release
```
生成先: `target\release\rustcord.exe`

---

## ⚠️ 免責事項 / Disclaimer

- Rustcord は非公式のオープンソースプロジェクトであり、**Discord Inc. とは一切関係ありません**。
- 本アプリは Windows 標準の WebView2 上で公式の Discord Web アプリ（`discord.com/app`）を表示する軽量デスクトップシェルであり、ユーザートークンの抽出や自動操作（セルフボット）などは一切行いません。
- 設定およびプロファイルデータはすべてローカル（`%LOCALAPPDATA%\Rustcord`）にのみ保存され、外部サーバーへの送信は一切ありません。

---

## 📄 License

[MIT License](LICENSE)
