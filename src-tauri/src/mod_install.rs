//! Installs / removes the companion Lua mod next to the game.

use std::fs;
use std::io;
use std::path::Path;

use crate::bridge;
use crate::game;

const MAIN_LUA: &str = include_str!("../../lua/isaac_trainer/main.lua");
const METADATA: &str = include_str!("../../lua/isaac_trainer/metadata.xml");

pub fn mod_dir() -> Result<std::path::PathBuf, String> {
    let game_dir = game::find_game_dir().ok_or("找不到游戏目录（isaac-ng.exe）")?;
    Ok(bridge::mod_dir(&game::mods_dir(&game_dir)))
}

/// Write the Lua files unconditionally: the app ships its own copy of the mod,
/// so an upgraded app must overwrite an older one already sitting in `mods/`.
pub fn sync() -> Result<(), String> {
    let dir = mod_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建 mod 目录失败: {e}"))?;

    fs::write(dir.join("main.lua"), MAIN_LUA).map_err(|e| format!("写入 main.lua 失败: {e}"))?;
    fs::write(dir.join("metadata.xml"), METADATA)
        .map_err(|e| format!("写入 metadata.xml 失败: {e}"))?;

    // Left over from the old io-based bridge; harmless but misleading.
    let legacy = dir.join("paths.lua");
    if legacy.exists() {
        let _ = fs::remove_file(legacy);
    }

    let mods = dir.parent().unwrap_or(Path::new("."));
    bridge::ensure_cmd_file(mods).map_err(|e| format!("写入 trainer_cmd.lua 失败: {e}"))?;
    Ok(())
}

pub fn install() -> Result<(), String> {
    sync()?;

    // The global mod switch must be on before the mod can load at all.
    let game_dir = game::find_game_dir().ok_or("找不到游戏目录（isaac-ng.exe）")?;
    let ini = game::options_ini(&game_dir);
    if ini.exists() {
        let _ = game::ini_set(&ini, "EnableMods", "1");
    }
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let dir = mod_dir()?;
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("删除 mod 目录失败: {e}"))?;
    }
    Ok(())
}

pub fn is_installed(game_dir: &Path) -> bool {
    game::mods_dir(game_dir)
        .join(bridge::MOD_DIR)
        .join("main.lua")
        .exists()
}

#[allow(dead_code)]
fn touch_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}
