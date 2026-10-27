export type GameInfo = {
  game_dir: string | null;
  save_dir: string | null;
  bridge_dir: string | null;
  exe_version: string | null;
  game_running: boolean;
  mods_enabled: boolean;
  console_enabled: boolean;
  mod_installed: boolean;
  save_path_custom: boolean;
};

export type Snapshot = {
  connected: boolean;
  age_ms: number;
  cur: number;
  boot: string;
  note: string;
  items_done: boolean;
  item_count: number;
  playing: boolean;
  hp: number;
  max_hp: number;
  soul: number;
  black: number;
  bone: number;
  coins: number;
  bombs: number;
  keys: number;
  level: number;
  level_name: string;
  room: number;
  seed: string;
  items: number[];
  trinket: number;
  active: number;
  charge: number;
  max_charge: number;
  damage: number;
  fire_delay: number;
  speed: number;
  luck: number;
};

export type Settings = {
  god: boolean;
  heal: boolean;
  max_hp: number;
  damage: number;
  tears: number;
  speed: number;
  range: number;
  shotspeed: number;
  luck: number;
  inf_charge: boolean;
  cash: boolean;
};

export type ItemEntry = {
  kind: string;
  id: number;
  name: string;
};

export type UiState = {
  info: GameInfo;
  snapshot: Snapshot;
};

export const DEFAULT_SETTINGS: Settings = {
  god: false,
  heal: false,
  max_hp: 0,
  damage: 1,
  tears: 1,
  speed: 1,
  range: 1,
  shotspeed: 1,
  luck: 0,
  inf_charge: false,
  cash: false,
};

export const EMPTY_SNAPSHOT: Snapshot = {
  connected: false,
  age_ms: 0,
  cur: 0,
  boot: "",
  note: "",
  items_done: false,
  item_count: 0,
  playing: false,
  hp: 0,
  max_hp: 0,
  soul: 0,
  black: 0,
  bone: 0,
  coins: 0,
  bombs: 0,
  keys: 0,
  level: 0,
  level_name: "",
  room: 0,
  seed: "",
  items: [],
  trinket: 0,
  active: 0,
  charge: 0,
  max_charge: 0,
  damage: 0,
  fire_delay: 0,
  speed: 0,
  luck: 0,
};
