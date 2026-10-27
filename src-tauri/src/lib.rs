mod bridge;
mod game;
mod mod_install;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use bridge::{ItemEntry, Settings, Snapshot};

#[derive(Serialize, Clone)]
struct UiState {
    info: game::GameInfo,
    snapshot: Snapshot,
}

/// Shared state. The UI thread only mutates this; a single watcher thread owns
/// all I/O with the game (one command file on disk, one log file being read).
pub struct App {
    settings: Mutex<Settings>,
    /// One-shot actions waiting for the mod to be reachable.
    pending: Mutex<Vec<String>>,
    /// Settings changed and not yet published.
    dirty: AtomicBool,
    /// Sequence number of the last command file written.
    seq: AtomicU64,
    /// Sequence number the mod reports as applied.
    acked: AtomicU64,
    /// Identifies the current game session, so a restart forces a resync.
    boot: Mutex<String>,
    items: Mutex<Vec<ItemEntry>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
}

// ------------------------------------------------------------- commands

#[tauri::command]
fn detect() -> game::GameInfo {
    game::detect()
}

#[tauri::command]
fn get_settings(app: State<App>) -> Settings {
    app.settings.lock().map(|s| s.clone()).unwrap_or_default()
}

#[tauri::command]
fn push_settings(app: State<App>, settings: Settings) -> Result<(), String> {
    {
        let mut guard = app.settings.lock().map_err(|_| "设置锁失效".to_string())?;
        *guard = settings.clone();
    }
    let mut cfg = game::load_config();
    cfg.settings = Some(settings);
    let _ = game::save_config(&cfg);

    app.dirty.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn do_action(app: State<App>, action: String) -> Result<(), String> {
    let action = action.trim().to_string();
    if action.is_empty() {
        return Ok(());
    }
    app.pending
        .lock()
        .map_err(|_| "指令队列锁失效".to_string())?
        .push(action);
    app.dirty.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn install_mod(app: State<App>) -> Result<game::GameInfo, String> {
    mod_install::install()?;
    app.dirty.store(true, Ordering::SeqCst);
    Ok(game::detect())
}

#[tauri::command]
fn uninstall_mod() -> Result<game::GameInfo, String> {
    mod_install::uninstall()?;
    Ok(game::detect())
}

#[tauri::command]
fn get_items(app: State<App>) -> Vec<ItemEntry> {
    app.items.lock().map(|v| v.clone()).unwrap_or_default()
}

#[tauri::command]
fn set_option(key: String, enabled: bool) -> Result<game::GameInfo, String> {
    let game_dir = game::find_game_dir().ok_or("找不到游戏目录（isaac-ng.exe）")?;
    let ini = game::options_ini(&game_dir);
    if !ini.exists() {
        return Err(format!("找不到 options.ini: {}", ini.display()));
    }
    let ini_key = match key.as_str() {
        "mods" => "EnableMods",
        "console" => "EnableDebugConsole",
        other => return Err(format!("未知选项: {other}")),
    };
    game::ini_set(&ini, ini_key, if enabled { "1" } else { "0" })
        .map_err(|e| format!("写入 options.ini 失败: {e}"))?;
    Ok(game::detect())
}

#[tauri::command]
fn set_game_dir(path: String) -> Result<game::GameInfo, String> {
    let p = Path::new(&path);
    if !p.join("isaac-ng.exe").exists() {
        return Err("该目录下没有 isaac-ng.exe".to_string());
    }
    let mut cfg = game::load_config();
    cfg.game_dir = Some(path);
    game::save_config(&cfg).map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(game::detect())
}

#[tauri::command]
fn open_folder(which: String) -> Result<(), String> {
    let game_dir = game::find_game_dir().ok_or("找不到游戏目录（isaac-ng.exe）")?;
    let target = match which.as_str() {
        "game" => game_dir.clone(),
        "mods" => game::mods_dir(&game_dir),
        "mod" | "bridge" => bridge::mod_dir(&game::mods_dir(&game_dir)),
        "save" => game::save_dir(&game_dir),
        _ => game_dir.clone(),
    };
    std::process::Command::new("explorer")
        .arg(target.as_os_str())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn launch_game() -> Result<(), String> {
    let url = format!("steam://rungameid/{}", game::STEAM_APP_ID);
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url.as_str()])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------- bootstrap

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cfg = game::load_config();
    let settings = cfg.settings.clone().unwrap_or_default();

    tauri::Builder::default()
        .manage(App {
            settings: Mutex::new(settings),
            pending: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            acked: AtomicU64::new(0),
            boot: Mutex::new(String::new()),
            items: Mutex::new(Vec::new()),
        })
        .invoke_handler(tauri::generate_handler![
            detect,
            get_settings,
            push_settings,
            do_action,
            install_mod,
            uninstall_mod,
            get_items,
            set_option,
            set_game_dir,
            open_folder,
            launch_game
        ])
        .setup(|app| {
            // Install the companion mod on first launch so the user only has
            // to tick one checkbox in the game's Mods menu.
            if let Some(dir) = game::find_game_dir() {
                if mod_install::is_installed(&dir) {
                    // Keep the installed copy in sync with this build.
                    let _ = mod_install::sync();
                } else {
                    let _ = mod_install::install();
                }
                // The mod probes this file with include() at load time; having
                // it present from the start means the probe succeeds.
                let _ = bridge::ensure_cmd_file(&game::mods_dir(&dir));
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let state = handle.state::<App>();
                let mut info = game::detect();
                let mut last_detect = Instant::now();
                let mut reader: Option<bridge::Reader> = None;
                let mut items_len: usize = 0;
                let mut boot_seen = String::new();

                loop {
                    if last_detect.elapsed() > Duration::from_secs(2) {
                        info = game::detect();
                        last_detect = Instant::now();
                    }

                    // Follow `<save dir>/log.txt`. A moved save dir rebuilds
                    // the reader, which replays the log and reconstructs state.
                    match info.save_dir.as_deref() {
                        Some(save) => {
                            let log = Path::new(save).join("log.txt");
                            let stale = reader
                                .as_ref()
                                .map(|r| r.log_path() != log)
                                .unwrap_or(true);
                            if stale {
                                reader = Some(bridge::Reader::new(log));
                                items_len = 0;
                            }
                        }
                        None => reader = None,
                    }

                    let mut snapshot = Snapshot::default();
                    let mut fresh_items: Option<Vec<ItemEntry>> = None;

                    if let Some(r) = reader.as_mut() {
                        r.poll();
                        snapshot = r.snapshot.clone();
                        if !info.game_running {
                            snapshot.connected = false;
                        }

                        // A new game session re-reads our settings file at boot
                        // but skips queued actions, so publish again.
                        if !snapshot.boot.is_empty() && snapshot.boot != boot_seen {
                            boot_seen = snapshot.boot.clone();
                            if let Ok(mut guard) = state.boot.lock() {
                                *guard = boot_seen.clone();
                            }
                            state.dirty.store(true, Ordering::SeqCst);
                        }

                        state.acked.store(snapshot.cur, Ordering::SeqCst);

                        if snapshot.item_count as usize != items_len {
                            items_len = snapshot.item_count as usize;
                            fresh_items = Some(r.items());
                        }
                    }

                    if let Some(list) = fresh_items {
                        if let Ok(mut slot) = state.items.lock() {
                            *slot = list;
                        }
                    }

                    publish(&state, &info);

                    let _ = handle.emit(
                        "ui-state",
                        UiState {
                            info: info.clone(),
                            snapshot,
                        },
                    );
                    std::thread::sleep(Duration::from_millis(120));
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Write the command file when the UI has something new and the mod is ready
/// for it. Only one command file is outstanding at a time: the next one is
/// written after the mod acks the previous sequence number.
fn publish(state: &App, info: &game::GameInfo) {
    let written = state.seq.load(Ordering::SeqCst);
    let acked = state.acked.load(Ordering::SeqCst);
    let has_pending = state
        .pending
        .lock()
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    // While the game is closed there is nothing to ack, so overwriting the
    // file is always fine — the mod reads it at its next load.
    let ready = acked >= written || !info.game_running;
    if !ready || !(state.dirty.load(Ordering::SeqCst) || has_pending) {
        return;
    }

    let game_dir = match info.game_dir.as_deref() {
        Some(dir) => dir,
        None => return,
    };
    let mods = game::mods_dir(Path::new(game_dir));
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let actions: Vec<String> = state
        .pending
        .lock()
        .map(|mut p| std::mem::take(&mut *p))
        .unwrap_or_default();
    let next = now_ms().max(written + 1);

    match bridge::write_cmd(&mods, next, &settings, &actions) {
        Ok(()) => {
            state.seq.store(next, Ordering::SeqCst);
            state.dirty.store(false, Ordering::SeqCst);
        }
        Err(_) => {
            // Keep the actions for the next attempt.
            if let Ok(mut pending) = state.pending.lock() {
                let mut restored = actions;
                restored.extend(std::mem::take(&mut *pending));
                *pending = restored;
            }
        }
    }
}
