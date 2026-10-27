//! Bridge to the companion Lua mod.
//!
//! Repentance's Lua sandbox has no `io`, so a plain file protocol is out.
//! Two channels are left and both are used here:
//!
//! * app -> mod: the command file `trainer_cmd.lua` next to the mod's
//!   `main.lua`. The mod loads it with `include()`, which Repentance
//!   deliberately never caches, so rewriting the file is enough to deliver a
//!   new command.
//! * mod -> app: `Isaac.DebugString()`, which appends to the game's
//!   `log.txt`. We tail that file by byte offset and pick the `[trainer]`
//!   lines out of it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const MOD_DIR: &str = "isaac_trainer";
pub const CMD_FILE: &str = "trainer_cmd.lua";

/// Marker every line of the mod starts with in `log.txt`.
const MARK: &str = "[trainer]";

/// How long a state line stays "fresh". The mod emits ~3 per second, so this
/// only trips when the game is paused in a menu that stops `MC_POST_UPDATE`
/// or when the game exits.
const FRESH_MS: u64 = 3500;

/// Everything the UI can tune. Multipliers are 1.0 = untouched,
/// `max_hp` is in red heart *containers* (0 = leave alone).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub god: bool,
    pub heal: bool,
    pub max_hp: i64,
    pub damage: f64,
    pub tears: f64,
    pub speed: f64,
    pub range: f64,
    pub shotspeed: f64,
    pub luck: f64,
    pub inf_charge: bool,
    pub cash: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            god: false,
            heal: false,
            max_hp: 0,
            damage: 1.0,
            tears: 1.0,
            speed: 1.0,
            range: 1.0,
            shotspeed: 1.0,
            luck: 0.0,
            inf_charge: false,
            cash: false,
        }
    }
}

impl Settings {
    fn lua_body(&self) -> String {
        let b = |v: bool| if v { 1 } else { 0 };
        format!(
            concat!(
                "  god = {},\n",
                "  heal = {},\n",
                "  maxhp = {},\n",
                "  damage = {:.4},\n",
                "  tears = {:.4},\n",
                "  speed = {:.4},\n",
                "  range = {:.4},\n",
                "  shotspeed = {:.4},\n",
                "  luck = {:.4},\n",
                "  infcharge = {},\n",
                "  cash = {},\n"
            ),
            b(self.god),
            b(self.heal),
            self.max_hp,
            self.damage,
            self.tears,
            self.speed,
            self.range,
            self.shotspeed,
            self.luck,
            b(self.inf_charge),
            b(self.cash),
        )
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub connected: bool,
    pub age_ms: u64,
    /// Last command sequence the mod applied; doubles as the ack.
    pub cur: u64,
    /// Identifies one game session (the mod reports `Isaac.GetTime()`).
    pub boot: String,
    /// Last diagnostic line the mod wrote, shown in the UI when offline.
    pub note: String,
    pub items_done: bool,
    pub item_count: i64,
    pub playing: bool,
    pub hp: i64,
    pub max_hp: i64,
    pub soul: i64,
    pub black: i64,
    pub bone: i64,
    pub coins: i64,
    pub bombs: i64,
    pub keys: i64,
    pub level: i64,
    pub level_name: String,
    pub room: i64,
    pub seed: String,
    pub items: Vec<i64>,
    pub trinket: i64,
    pub active: i64,
    pub charge: i64,
    pub max_charge: i64,
    pub damage: f64,
    pub fire_delay: f64,
    pub speed: f64,
    pub luck: f64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ItemEntry {
    pub kind: String,
    pub id: i64,
    pub name: String,
}

// ------------------------------------------------------------- outbound

pub fn mod_dir(mods_dir: &Path) -> PathBuf {
    mods_dir.join(MOD_DIR)
}

pub fn cmd_path(mods_dir: &Path) -> PathBuf {
    mod_dir(mods_dir).join(CMD_FILE)
}

/// Create the command file if it is missing, so the mod's `include()` probe at
/// load time succeeds even before the user touches the UI.
pub fn ensure_cmd_file(mods_dir: &Path) -> io::Result<()> {
    let path = cmd_path(mods_dir);
    if path.exists() {
        return Ok(());
    }
    write_cmd(mods_dir, 1, &Settings::default(), &[])
}

/// Publish settings (and any queued one-shot actions) to the mod.
pub fn write_cmd(
    mods_dir: &Path,
    seq: u64,
    settings: &Settings,
    actions: &[String],
) -> io::Result<()> {
    let mut out = String::with_capacity(512);
    out.push_str("-- written by isaac-trainer; safe to delete\nreturn {\n");
    out.push_str(&format!("  seq = {},\n", seq));
    out.push_str(&settings.lua_body());
    if !actions.is_empty() {
        out.push_str("  act = { ");
        for (i, action) in actions.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&lua_string(action));
        }
        out.push_str(" },\n");
    }
    out.push_str("}\n");

    let dir = mod_dir(mods_dir);
    fs::create_dir_all(&dir)?;
    atomic_write(&dir.join(CMD_FILE), &out)
}

fn lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Write to a temp file then rename: `include()` must never see a half-written
/// chunk. The game may hold the previous file open for a few milliseconds, so
/// retry and fall back to a direct write.
fn atomic_write(path: &Path, data: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;

    let mut last: Option<io::Error> = None;
    for _ in 0..6 {
        match fs::rename(&tmp, path) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(8));
            }
        }
    }
    match fs::write(path, data) {
        Ok(_) => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(_) => Err(last.unwrap_or_else(|| io::Error::other("rename failed"))),
    }
}

// ------------------------------------------------------------- inbound

/// Incremental reader for the game log.
pub struct LogTail {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
    skip_partial: bool,
}

impl LogTail {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            pending: Vec::new(),
            skip_partial: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// New complete lines since the previous call.
    pub fn poll(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return out,
        };
        let len = match file.metadata() {
            Ok(meta) => meta.len(),
            Err(_) => return out,
        };

        // A fresh log file (game restart) or a rotated one: start over.
        if len < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        // A very old log can be tens of megabytes; only the tail matters.
        const CAP: u64 = 16 * 1024 * 1024;
        if self.offset == 0 && len > CAP {
            self.offset = len - CAP;
            self.skip_partial = true;
        }
        if len == self.offset {
            return out;
        }

        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return out;
        }
        let mut chunk = Vec::with_capacity((len - self.offset) as usize);
        if file.take(len - self.offset).read_to_end(&mut chunk).is_err() {
            return out;
        }
        self.offset = len;
        self.pending.extend_from_slice(&chunk);

        let mut start = 0;
        for i in 0..self.pending.len() {
            if self.pending[i] == b'\n' {
                let raw = &self.pending[start..i];
                let line = String::from_utf8_lossy(raw);
                let line = line.trim_end_matches('\r').to_string();
                start = i + 1;
                if self.skip_partial {
                    continue;
                }
                out.push(line);
            }
        }
        if start > 0 {
            self.pending.drain(..start);
        }
        if self.skip_partial && start > 0 {
            self.skip_partial = false;
        }
        // Guard against a corrupt file with no newlines at all.
        if self.pending.len() > 1024 * 1024 {
            self.pending.clear();
        }
        out
    }
}

/// Keeps the last state the mod reported plus the item id -> name table.
pub struct Reader {
    tail: LogTail,
    pub snapshot: Snapshot,
    items: BTreeMap<(char, i64), String>,
    last_state: Option<Instant>,
}

impl Reader {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            tail: LogTail::new(log_path),
            snapshot: Snapshot::default(),
            items: BTreeMap::new(),
            last_state: None,
        }
    }

    pub fn log_path(&self) -> &Path {
        self.tail.path()
    }

    /// Read everything new and fold it into the snapshot.
    pub fn poll(&mut self) {
        for line in self.tail.poll() {
            let pos = match line.find(MARK) {
                Some(p) => p + MARK.len(),
                None => continue,
            };
            let body = &line[pos..];

            if let Some(rest) = body.strip_prefix('S') {
                self.apply_state(rest.trim());
            } else if let Some(rest) = body.strip_prefix("C|") {
                self.apply_item(rest);
            } else if body.starts_with("CDONE") {
                self.snapshot.items_done = true;
                self.snapshot.item_count = self.items.len() as i64;
            } else if let Some(rest) = body.strip_prefix('B') {
                // Banner emitted once per game session, before the item table.
                let boot = rest.trim().to_string();
                if !boot.is_empty() && boot != self.snapshot.boot {
                    self.reset_items();
                    self.snapshot.boot = boot;
                    // Diagnostics from an older session are dead weight.
                    self.snapshot.note.clear();
                }
            } else {
                // Anything else is a diagnostic line for the UI.
                let text = body.trim_start();
                if !text.is_empty() {
                    self.snapshot.note = text.chars().take(400).collect();
                }
            }
        }

        let age = self
            .last_state
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(3600));
        self.snapshot.age_ms = age.as_millis() as u64;
        self.snapshot.connected = age.as_millis() as u64 <= FRESH_MS;
    }

    pub fn reset_items(&mut self) {
        self.items.clear();
        self.snapshot.items_done = false;
        self.snapshot.item_count = 0;
    }

    pub fn items(&self) -> Vec<ItemEntry> {
        let rank = |k: char| match k {
            'C' => 0,
            'T' => 1,
            'K' => 2,
            _ => 3,
        };
        let mut out: Vec<ItemEntry> = self
            .items
            .iter()
            .map(|((kind, id), name)| ItemEntry {
                kind: kind.to_string(),
                id: *id,
                name: name.clone(),
            })
            .collect();
        out.sort_by_key(|e| {
            let k = e.kind.chars().next().unwrap_or('Z');
            (rank(k), e.id)
        });
        out
    }

    fn apply_item(&mut self, rest: &str) {
        let mut parts = rest.splitn(3, '|');
        let kind = match parts.next().and_then(|k| k.trim().chars().next()) {
            Some(k) => k,
            None => return,
        };
        let id = match parts.next().and_then(|v| v.trim().parse::<i64>().ok()) {
            Some(v) => v,
            None => return,
        };
        let name = parts.next().unwrap_or("").trim();
        if name.is_empty() {
            return;
        }
        self.items.insert((kind, id), name.to_string());
        self.snapshot.item_count = self.items.len() as i64;
    }

    fn apply_state(&mut self, body: &str) {
        if body.is_empty() {
            return;
        }
        let mut seen_boot = None;

        for token in body.split_whitespace() {
            let (key, value) = match token.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let int = |v: &str| v.parse::<i64>().unwrap_or(0);
            let float = |v: &str| v.parse::<f64>().unwrap_or(0.0);

            match key {
                "boot" => seen_boot = Some(value.to_string()),
                "cur" => self.snapshot.cur = value.parse::<u64>().unwrap_or(0),
                "play" => self.snapshot.playing = int(value) == 1,
                "hp" => self.snapshot.hp = int(value),
                "maxhp" => self.snapshot.max_hp = int(value),
                "soul" => self.snapshot.soul = int(value),
                "black" => self.snapshot.black = int(value),
                "bone" => self.snapshot.bone = int(value),
                "coins" => self.snapshot.coins = int(value),
                "bombs" => self.snapshot.bombs = int(value),
                "keys" => self.snapshot.keys = int(value),
                "lvl" => self.snapshot.level = int(value),
                "lname" => self.snapshot.level_name = value.replace('_', " "),
                "room" => self.snapshot.room = int(value),
                "seed" => self.snapshot.seed = value.to_string(),
                "trink" => self.snapshot.trinket = int(value),
                "act" => self.snapshot.active = int(value),
                "chg" => self.snapshot.charge = int(value),
                "mchg" => self.snapshot.max_charge = int(value),
                "dmg" => self.snapshot.damage = float(value),
                "fd" => self.snapshot.fire_delay = float(value),
                "spd" => self.snapshot.speed = float(value),
                "luck" => self.snapshot.luck = float(value),
                "items" => {
                    self.snapshot.items = value
                        .split(',')
                        .filter_map(|v| v.trim().parse::<i64>().ok())
                        .filter(|v| *v > 0)
                        .collect();
                }
                _ => {}
            }
        }

        if let Some(boot) = seen_boot {
            if boot != self.snapshot.boot {
                self.reset_items();
                self.snapshot.boot = boot;
                self.snapshot.note.clear();
            }
        }
        self.last_state = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_log(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("isaac-trainer-test-{name}.log"));
        let _ = fs::remove_file(&path);
        fs::write(&path, "").unwrap();
        path
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.flush().unwrap();
    }

    const STATE: &str = "[INFO] - Lua Debug: [trainer]S boot=777 cur=99 play=1 hp=6 maxhp=8 \
soul=2 black=0 bone=0 coins=42 bombs=99 keys=1 lvl=2 lname=Basement_II room=84 seed=ABCD1234 \
trink=0 act=105 chg=1 mchg=6 dmg=3.50 fd=10.00 spd=1.00 luck=0.00 items=1,105\r\n";

    #[test]
    fn parses_state_items_and_note() {
        let path = temp_log("parse");
        let mut reader = Reader::new(path.clone());
        reader.poll();

        append(&path, "[INFO] - Lua Debug: [trainer]B 777\r\n");
        append(&path, "[INFO] - Lua Debug: [trainer]caps io=false include=true cmdfile=trainer_cmd.lua\r\n");
        append(&path, "[INFO] - Lua Debug: [trainer]C|C|1|The Sad Onion\r\n");
        append(&path, "[INFO] - Lua Debug: [trainer]C|T|1|Swallowed Penny\r\n");
        append(&path, "[INFO] - Lua Debug: [trainer]CDONE 2\r\n");
        append(&path, STATE);
        reader.poll();

        assert_eq!(reader.snapshot.boot, "777");
        assert_eq!(reader.snapshot.cur, 99);
        assert_eq!(reader.snapshot.coins, 42);
        assert_eq!(reader.snapshot.bombs, 99);
        assert_eq!(reader.snapshot.level_name, "Basement II");
        assert_eq!(reader.snapshot.items, vec![1, 105]);
        assert_eq!(reader.snapshot.item_count, 2);
        assert!(reader.snapshot.items_done);
        assert!(reader.snapshot.connected);
        assert!(reader.snapshot.note.contains("io=false"));

        let items = reader.items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "C");
        assert_eq!(items[0].name, "The Sad Onion");
        assert_eq!(items[1].kind, "T");
    }

    #[test]
    fn partial_lines_wait_for_their_newline() {
        let path = temp_log("partial");
        let mut reader = Reader::new(path.clone());
        append(&path, "[INFO] - Lua Debug: [trainer]S boot=5 cu");
        reader.poll();
        assert_eq!(reader.snapshot.boot, "");
        assert_eq!(reader.snapshot.cur, 0);

        append(&path, "r=7 coins=3\r\n");
        reader.poll();
        assert_eq!(reader.snapshot.boot, "5");
        assert_eq!(reader.snapshot.cur, 7);
        assert_eq!(reader.snapshot.coins, 3);
    }

    #[test]
    fn new_session_clears_item_table() {
        let path = temp_log("boot");
        let mut reader = Reader::new(path.clone());
        append(
            &path,
            "[trainer]B 100\n[trainer]C|C|1|One\n[trainer]CDONE 1\n[trainer]S boot=100 cur=5\r\n",
        );
        reader.poll();
        assert_eq!(reader.snapshot.item_count, 1);
        assert!(reader.snapshot.items_done);

        append(&path, "[trainer] something went wrong\r\n");
        reader.poll();
        assert!(reader.snapshot.note.contains("something went wrong"));

        append(&path, "[trainer]B 200\r\n");
        reader.poll();
        assert_eq!(reader.snapshot.boot, "200");
        assert_eq!(reader.snapshot.item_count, 0);
        assert!(!reader.snapshot.items_done);
        assert!(reader.snapshot.note.is_empty());
    }

    #[test]
    fn truncated_log_restarts_from_zero() {
        let path = temp_log("truncate");
        let mut reader = Reader::new(path.clone());
        append(&path, "[trainer]S boot=1 cur=50 coins=7\r\n");
        reader.poll();
        assert_eq!(reader.snapshot.cur, 50);

        fs::write(&path, "[trainer]S boot=1 cur=3 coins=1\r\n").unwrap();
        reader.poll();
        assert_eq!(reader.snapshot.cur, 3);
        assert_eq!(reader.snapshot.coins, 1);
    }

    #[test]
    fn command_file_is_valid_lua_shape() {
        let mut dir = std::env::temp_dir();
        dir.push("isaac-trainer-test-cmd");
        let mods = dir.join("mods");
        fs::create_dir_all(&mods).unwrap();

        let mut settings = Settings::default();
        settings.god = true;
        settings.max_hp = 3;
        settings.damage = 2.5;
        write_cmd(&mods, 1700000000000, &settings, &["giveitem:1".to_string()]).unwrap();

        let text = fs::read_to_string(cmd_path(&mods)).unwrap();
        assert!(text.contains("seq = 1700000000000"));
        assert!(text.contains("god = 1"));
        assert!(text.contains("maxhp = 3"));
        assert!(text.contains("damage = 2.5000"));
        assert!(text.contains("act = { \"giveitem:1\" }"));
        assert!(text.trim_end().ends_with('}'));
    }
}
