import { invoke } from "@tauri-apps/api/core";
import type { GameInfo, ItemEntry, Settings } from "./types";

export const api = {
  detect: () => invoke<GameInfo>("detect"),
  getSettings: () => invoke<Settings>("get_settings"),
  pushSettings: (settings: Settings) => invoke<void>("push_settings", { settings }),
  action: (action: string) => invoke<void>("do_action", { action }),
  installMod: () => invoke<GameInfo>("install_mod"),
  uninstallMod: () => invoke<GameInfo>("uninstall_mod"),
  getItems: () => invoke<ItemEntry[]>("get_items"),
  setOption: (key: "mods" | "console", enabled: boolean) =>
    invoke<GameInfo>("set_option", { key, enabled }),
  setGameDir: (path: string) => invoke<GameInfo>("set_game_dir", { path }),
  openFolder: (which: "game" | "mods" | "mod" | "save") => invoke<void>("open_folder", { which }),
  launchGame: () => invoke<void>("launch_game"),
};
