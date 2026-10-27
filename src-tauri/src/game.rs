//! Locating the game, its save folder and its option file, plus a couple of
//! cheap integrity checks (installed version string, process presence).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub const STEAM_APP_ID: &str = "250900";
pub const GAME_FOLDER: &str = "The Binding of Isaac Rebirth";
pub const SAVE_FOLDER: &str = "Binding of Isaac Repentance";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    #[serde(default)]
    pub game_dir: Option<String>,
    #[serde(default)]
    pub settings: Option<crate::bridge::Settings>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct GameInfo {
    pub game_dir: Option<String>,
    pub save_dir: Option<String>,
    pub bridge_dir: Option<String>,
    pub exe_version: Option<String>,
    pub game_running: bool,
    pub mods_enabled: bool,
    pub console_enabled: bool,
    pub mod_installed: bool,
    pub save_path_custom: bool,
}

// ------------------------------------------------------------------ config

pub fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(Path::new(&appdata).join("isaac-trainer").join("config.json"))
}

pub fn load_config() -> Config {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no APPDATA"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg).unwrap_or_default();
    fs::write(path, text)
}

// ------------------------------------------------------------- discovery

fn steam_root() -> Option<PathBuf> {
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
    }
    for guess in [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Steam",
        r"E:\Steam",
    ] {
        let p = PathBuf::from(guess);
        if p.join("steamapps").exists() {
            return Some(p);
        }
    }
    None
}

fn library_roots(steam: &Path) -> Vec<PathBuf> {
    let mut roots = vec![steam.to_path_buf()];
    let vdf = steam.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = fs::read_to_string(vdf) {
        let mut rest = text.as_str();
        while let Some(pos) = rest.find("\"path\"") {
            let after = &rest[pos + 6..];
            if let Some(open) = after.find('"') {
                let tail = &after[open + 1..];
                if let Some(close) = tail.find('"') {
                    let raw = tail[..close].replace("\\\\", "\\");
                    let p = PathBuf::from(raw);
                    if p.exists() && !roots.iter().any(|r| r == &p) {
                        roots.push(p);
                    }
                    rest = &tail[close + 1..];
                    continue;
                }
            }
            break;
        }
    }
    roots
}

pub fn find_game_dir() -> Option<PathBuf> {
    let cfg = load_config();
    if let Some(dir) = cfg.game_dir {
        let p = PathBuf::from(dir);
        if p.join("isaac-ng.exe").exists() {
            return Some(p);
        }
    }

    if let Ok(dir) = std::env::var("ISAAC_DIR") {
        let p = PathBuf::from(dir);
        if p.join("isaac-ng.exe").exists() {
            return Some(p);
        }
    }

    if let Some(steam) = steam_root() {
        for root in library_roots(&steam) {
            let candidate = root.join("steamapps").join("common").join(GAME_FOLDER);
            if candidate.join("isaac-ng.exe").exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn documents_dir() -> PathBuf {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders")
        .and_then(|k| k.get_value::<String, _>("Personal"));
    if let Ok(raw) = key {
        let expanded = expand_env(&raw);
        let p = PathBuf::from(expanded);
        if p.exists() {
            return p;
        }
    }
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    Path::new(&home).join("Documents")
}

fn expand_env(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find('%') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos + 1..];
        match tail.find('%') {
            Some(end) => {
                let name = &tail[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(&rest[pos..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The game writes `savedatapath.txt` next to the executable; if present it is
/// authoritative (it also tells us the mods folder, which may live elsewhere).
pub fn save_dir(game_dir: &Path) -> PathBuf {
    if let Ok(text) = fs::read_to_string(game_dir.join("savedatapath.txt")) {
        for line in text.lines() {
            if let Some(rest) = line.split("Save Data Path:").nth(1) {
                let raw = rest.trim().trim_end_matches(['/', '\\']);
                if !raw.is_empty() {
                    return PathBuf::from(raw);
                }
            }
        }
    }
    documents_dir().join("My Games").join(SAVE_FOLDER)
}

pub fn mods_dir(game_dir: &Path) -> PathBuf {
    if let Ok(text) = fs::read_to_string(game_dir.join("savedatapath.txt")) {
        for line in text.lines() {
            if let Some(rest) = line.split("Modding Data Path:").nth(1) {
                let raw = rest.trim().trim_end_matches(['/', '\\']);
                if !raw.is_empty() {
                    return PathBuf::from(raw);
                }
            }
        }
    }
    game_dir.join("mods")
}

pub fn options_ini(game_dir: &Path) -> PathBuf {
    save_dir(game_dir).join("options.ini")
}

// ----------------------------------------------------------------- ini

pub fn ini_get(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

pub fn ini_set(path: &Path, key: &str, value: &str) -> io::Result<()> {
    let text = fs::read_to_string(path).unwrap_or_else(|_| "[Options]\r\n".to_string());
    let mut out = String::with_capacity(text.len() + 32);
    let mut replaced = false;

    for line in text.lines() {
        if let Some((k, _)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push_str("\r\n");
                replaced = true;
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }

    if !replaced {
        if let Some(pos) = out.find("[Options]") {
            let insert_at = pos + "[Options]".len();
            let tail = out.split_off(insert_at);
            out.push_str("\r\n");
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push_str(&tail);
        } else {
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push_str("\r\n");
        }
    }

    fs::write(path, out)
}

// ------------------------------------------------------------- version

fn exe_version(game_dir: &Path) -> Option<String> {
    static CACHE: OnceLock<Mutex<Option<(PathBuf, SystemTime, String)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));

    let exe = game_dir.join("isaac-ng.exe");
    let mtime = fs::metadata(&exe).ok()?.modified().ok()?;

    if let Ok(guard) = cache.lock() {
        if let Some((p, t, v)) = guard.as_ref() {
            if p == &exe && *t == mtime {
                return Some(v.clone());
            }
        }
    }

    let bytes = fs::read(&exe).ok()?;
    let marker = b"Binding of Isaac: Repentance v";
    let pos = bytes.windows(marker.len()).position(|w| w == marker)?;
    let start = pos + marker.len();
    let end = bytes[start..]
        .iter()
        .take(24)
        .position(|&c| !(0x20..0x7f).contains(&c))
        .map(|i| start + i)
        .unwrap_or(start + 24);
    let version = String::from_utf8_lossy(&bytes[start..end]).to_string();

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((exe, mtime, version.clone()));
    }
    Some(version)
}

// ------------------------------------------------------------- process

#[cfg(windows)]
pub fn is_game_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut found = false;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case("isaac-ng.exe") {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub fn is_game_running() -> bool {
    false
}

// ------------------------------------------------------------- detect

pub fn detect() -> GameInfo {
    let mut info = GameInfo::default();
    let game_dir = match find_game_dir() {
        Some(d) => d,
        None => {
            info.game_running = is_game_running();
            return info;
        }
    };

    let save = save_dir(&game_dir);
    let mods = mods_dir(&game_dir);
    let bridge = crate::bridge::mod_dir(&mods);

    info.exe_version = exe_version(&game_dir);
    info.save_path_custom = game_dir.join("savedatapath.txt").exists();
    info.mod_installed = mods_dir(&game_dir)
        .join("isaac_trainer")
        .join("main.lua")
        .exists();

    let ini = options_ini(&game_dir);
    info.mods_enabled = ini_get(&ini, "EnableMods")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    info.console_enabled = ini_get(&ini, "EnableDebugConsole")
        .map(|v| v.trim() == "1")
        .unwrap_or(false);

    info.game_running = is_game_running();
    info.game_dir = Some(game_dir.display().to_string());
    info.save_dir = Some(save.display().to_string());
    info.bridge_dir = Some(bridge.display().to_string());
    info
}
