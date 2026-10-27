import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import {
  DEFAULT_SETTINGS,
  EMPTY_SNAPSHOT,
  type GameInfo,
  type ItemEntry,
  type Settings,
  type Snapshot,
  type UiState,
} from "./types";

const KINDS = ["C", "T", "K", "P"] as const;
const KIND_LABEL: Record<string, string> = {
  C: "道具",
  T: "饰品",
  K: "卡牌",
  P: "药丸",
};
const ACTION_FOR_KIND: Record<string, string> = {
  C: "giveitem",
  T: "givetrinket",
  K: "givecard",
  P: "givepill",
};

// ---------------------------------------------------------------- ui atoms

function Card(props: {
  title: string;
  hint?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <section className={"card " + (props.className ?? "")}>
      <header className="card-head">
        <h2>{props.title}</h2>
        {props.hint && <span className="card-hint">{props.hint}</span>}
      </header>
      {props.children}
    </section>
  );
}

function Toggle(props: {
  label: string;
  hint?: string;
  value: boolean;
  accent?: "red" | "gold";
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      type="button"
      className={
        "toggle" + (props.value ? " on" : "") + (props.accent === "gold" ? " gold" : "")
      }
      onClick={() => props.onChange(!props.value)}
    >
      <span className="toggle-text">
        <span className="toggle-label">{props.label}</span>
        {props.hint && <span className="toggle-hint">{props.hint}</span>}
      </span>
      <span className="switch" aria-hidden="true">
        <span className="knob" />
      </span>
    </button>
  );
}

function Slider(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format?: (v: number) => string;
  onChange: (v: number) => void;
}) {
  const fmt = props.format ?? ((v: number) => v.toFixed(2));
  const changed = props.value !== 1;
  return (
    <div className="slider-row">
      <div className="slider-top">
        <span className="slider-label">{props.label}</span>
        <span className={"slider-value" + (changed ? " active" : "")}>{fmt(props.value)}</span>
      </div>
      <input
        type="range"
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        onChange={(e) => props.onChange(Number(e.target.value))}
      />
    </div>
  );
}

function Pill(props: { ok: boolean; label: string; detail?: string }) {
  return (
    <div className={"pill" + (props.ok ? " ok" : "")} title={props.detail}>
      <span className="dot" />
      {props.label}
    </div>
  );
}

function Hearts({ snap }: { snap: Snapshot }) {
  const containers = Math.max(0, Math.min(12, Math.floor(snap.max_hp / 2)));
  const filled = Math.max(0, Math.min(containers, Math.floor(snap.hp / 2)));
  const soul = Math.min(12, Math.floor(snap.soul / 2));
  const black = Math.min(12, Math.floor(snap.black / 2));
  const bone = Math.min(12, Math.floor(snap.bone / 2));

  if (containers === 0 && soul === 0 && black === 0 && bone === 0) {
    return <div className="hearts-empty">不在游戏中</div>;
  }

  return (
    <div className="hearts">
      {Array.from({ length: containers }).map((_, i) => (
        <span key={"r" + i} className={"heart red" + (i < filled ? "" : " empty")}>
          {i < filled ? "♥" : "♡"}
        </span>
      ))}
      {Array.from({ length: soul }).map((_, i) => (
        <span key={"s" + i} className="heart soul">
          ♡
        </span>
      ))}
      {Array.from({ length: black }).map((_, i) => (
        <span key={"b" + i} className="heart black">
          ♡
        </span>
      ))}
      {Array.from({ length: bone }).map((_, i) => (
        <span key={"o" + i} className="heart bone">
          ♡
        </span>
      ))}
    </div>
  );
}

function Stat(props: { label: string; value: string }) {
  return (
    <div className="stat">
      <span className="stat-label">{props.label}</span>
      <span className="stat-value">{props.value}</span>
    </div>
  );
}

// ---------------------------------------------------------------- app

export default function App() {
  const [info, setInfo] = useState<GameInfo | null>(null);
  const [snap, setSnap] = useState<Snapshot>(EMPTY_SNAPSHOT);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [items, setItems] = useState<ItemEntry[]>([]);
  const [kind, setKind] = useState<string>("C");
  const [query, setQuery] = useState("");
  const [toast, setToast] = useState<{ text: string; bad?: boolean } | null>(null);
  const [manualDir, setManualDir] = useState("");

  const toastTimer = useRef<number | undefined>(undefined);
  const hydrated = useRef(false);

  const notify = useCallback((text: string, bad = false) => {
    setToast({ text, bad });
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2800);
  }, []);

  // live state from the rust side
  useEffect(() => {
    const unlisten = listen<UiState>("ui-state", (e) => {
      setInfo(e.payload.info);
      setSnap(e.payload.snapshot);
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  // initial config
  useEffect(() => {
    void (async () => {
      try {
        const [s, i] = await Promise.all([api.getSettings(), api.detect()]);
        setSettings({ ...DEFAULT_SETTINGS, ...s });
        setInfo(i);
        setManualDir(i.game_dir ?? "");
        hydrated.current = true;
        await api.pushSettings({ ...DEFAULT_SETTINGS, ...s });
      } catch (err) {
        notify(String(err), true);
      }
    })();
  }, [notify]);

  // debounced settings push
  useEffect(() => {
    if (!hydrated.current) return;
    const t = window.setTimeout(() => {
      api.pushSettings(settings).catch((e) => notify(String(e), true));
    }, 120);
    return () => window.clearTimeout(t);
  }, [settings, notify]);

  // Item database: the mod streams it once per game session.
  useEffect(() => {
    if (!snap.items_done) return;
    let cancelled = false;
    api
      .getItems()
      .then((list) => {
        if (!cancelled && list.length > 0) setItems(list);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [snap.items_done, snap.boot]);

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((prev) => ({ ...prev, [key]: value }));

  const nameById = useMemo(() => {
    const map = new Map<number, string>();
    for (const it of items) {
      if (it.kind === "C") map.set(it.id, it.name);
    }
    return map;
  }, [items]);

  const visibleItems = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = items.filter((it) => {
      if (it.kind !== kind) return false;
      if (!q) return true;
      return it.name.toLowerCase().includes(q) || String(it.id) === q;
    });
    return list.slice(0, 400);
  }, [items, kind, query]);

  const give = async (entry: ItemEntry) => {
    try {
      await api.action(`${ACTION_FOR_KIND[entry.kind]}:${entry.id}`);
      notify(`已给予 ${KIND_LABEL[entry.kind]} · ${entry.name}`);
    } catch (e) {
      notify(String(e), true);
    }
  };

  const runAction = async (action: string, label: string) => {
    try {
      await api.action(action);
      notify(label);
    } catch (e) {
      notify(String(e), true);
    }
  };

  const toggleOption = async (key: "mods" | "console", value: boolean) => {
    try {
      const next = await api.setOption(key, value);
      setInfo(next);
      notify(key === "mods" ? "已更新 EnableMods（需重启游戏）" : "已更新 EnableDebugConsole（需重启游戏）");
    } catch (e) {
      notify(String(e), true);
    }
  };

  const connected = snap.connected;

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <div className="sigil">✞</div>
          <div>
            <h1>ISAAC TRAINER</h1>
            <p>
              {info?.exe_version ? `Repentance+ ${info.exe_version}` : "未检测到游戏"}
              <span className="sep">·</span>
              Lua Bridge
            </p>
          </div>
        </div>
        <div className="pills">
          <Pill ok={!!info?.game_dir} label={info?.game_running ? "游戏运行中" : "游戏未运行"} />
          <Pill ok={!!info?.mod_installed} label="Mod 已安装" />
          <Pill ok={connected} label={connected ? "已连接" : "未连接"} />
        </div>
      </header>

      {!connected && (
        <div className="onboard">
          <strong>还没连上游戏。</strong>
          {snap.note && <p className="onboard-note">Mod 回报：{snap.note}</p>}
          <ol>
            <li>
              确认 <code>mods/isaac_trainer</code> 已安装（右下角 <em>重新安装 Mod</em>）。
            </li>
            <li>
              启动游戏 → 主菜单 <em>Mods</em> → 勾选 <code>Isaac Trainer Bridge</code>。
            </li>
            <li>进入存档（哪怕只是走到主菜单之后进一局），本页状态会自动变绿。</li>
          </ol>
          <div className="onboard-actions">
            <button className="btn primary" onClick={() => void api.launchGame()}>
              启动游戏
            </button>
            <button className="btn" onClick={() => void api.openFolder("mods")}>
              打开 mod 目录
            </button>
          </div>
        </div>
      )}

      <main>
        <div className="col">
          <Card title="作弊开关" hint="全局，实时生效">
            <div className="toggle-grid">
              <Toggle
                label="无敌"
                hint="拦截一切伤害"
                value={settings.god}
                onChange={(v) => update("god", v)}
              />
              <Toggle
                label="保持满血"
                hint="每帧回满红心"
                value={settings.heal}
                onChange={(v) => update("heal", v)}
              />
              <Toggle
                label="金币 / 炸弹 / 钥匙 常满"
                hint="维持在 99"
                value={settings.cash}
                onChange={(v) => update("cash", v)}
              />
              <Toggle
                label="无限主动道具充能"
                hint="两个主动槽都拉满"
                value={settings.inf_charge}
                onChange={(v) => update("inf_charge", v)}
              />
            </div>

            <div className="row gap">
              <label className="field">
                <span>红心上限</span>
                <select
                  value={settings.max_hp}
                  onChange={(e) => update("max_hp", Number(e.target.value))}
                >
                  <option value={0}>不修改</option>
                  {Array.from({ length: 12 }).map((_, i) => (
                    <option key={i + 1} value={i + 1}>
                      {i + 1} 红心
                    </option>
                  ))}
                </select>
              </label>
              <button className="btn" onClick={() => void runAction("heal", "已回满血")}>
                回满血
              </button>
              <button className="btn" onClick={() => void runAction("addhearts:1", "红心上限 +1")}>
                红心 +1
              </button>
              <button className="btn" onClick={() => void runAction("addsoul:1", "获得 1 魂心")}>
                魂心 +1
              </button>
            </div>
          </Card>

          <Card title="属性倍率" hint="1.00 = 原版数值">
            <div className="sliders">
              <Slider
                label="伤害"
                value={settings.damage}
                min={0.2}
                max={10}
                step={0.1}
                onChange={(v) => update("damage", v)}
              />
              <Slider
                label="射速"
                value={settings.tears}
                min={0.2}
                max={5}
                step={0.1}
                onChange={(v) => update("tears", v)}
              />
              <Slider
                label="移速"
                value={settings.speed}
                min={0.4}
                max={4}
                step={0.1}
                onChange={(v) => update("speed", v)}
              />
              <Slider
                label="射程"
                value={settings.range}
                min={0.4}
                max={4}
                step={0.1}
                onChange={(v) => update("range", v)}
              />
              <Slider
                label="弹速"
                value={settings.shotspeed}
                min={0.4}
                max={4}
                step={0.1}
                onChange={(v) => update("shotspeed", v)}
              />
              <Slider
                label="幸运 (固定值)"
                value={settings.luck}
                min={-10}
                max={50}
                step={1}
                format={(v) => (v >= 0 ? `+${v}` : String(v))}
                onChange={(v) => update("luck", v)}
              />
            </div>
            <button
              className="btn subtle"
              onClick={() =>
                setSettings((prev) => ({
                  ...prev,
                  damage: 1,
                  tears: 1,
                  speed: 1,
                  range: 1,
                  shotspeed: 1,
                  luck: 0,
                }))
              }
            >
              重置属性
            </button>
          </Card>

          <Card title="物品" hint={items.length ? `已加载 ${items.length} 条` : "等待游戏连接"}>
            <div className="tabs">
              {KINDS.map((k) => (
                <button
                  key={k}
                  className={"tab" + (kind === k ? " active" : "")}
                  onClick={() => setKind(k)}
                >
                  {KIND_LABEL[k]}
                </button>
              ))}
            </div>
            <input
              className="search"
              placeholder="搜索名称或输入 ID…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <div className="item-list">
              {visibleItems.length === 0 && (
                <div className="item-empty">
                  {items.length === 0 ? "连接游戏后自动加载物品表" : "没有匹配项"}
                </div>
              )}
              {visibleItems.map((it) => (
                <button key={it.kind + it.id} className="item" onClick={() => void give(it)}>
                  <span className="item-id">#{it.id}</span>
                  <span className="item-name">{it.name}</span>
                  <span className="item-add">+</span>
                </button>
              ))}
            </div>
          </Card>
        </div>

        <div className="col">
          <Card title="实时状态" hint={connected ? "来自游戏内" : "离线"}>
            <Hearts snap={snap} />
            <div className="stats">
              <Stat label="金币" value={String(snap.coins)} />
              <Stat label="炸弹" value={String(snap.bombs)} />
              <Stat label="钥匙" value={String(snap.keys)} />
              <Stat label="楼层" value={snap.level_name || (snap.level ? `#${snap.level}` : "—")} />
              <Stat label="房间" value={snap.room ? `#${snap.room}` : "—"} />
              <Stat label="种子" value={snap.seed || "—"} />
              <Stat label="伤害" value={snap.damage ? snap.damage.toFixed(2) : "—"} />
              <Stat
                label="射速"
                value={snap.fire_delay ? `${(30 / snap.fire_delay).toFixed(2)}/s` : "—"}
              />
              <Stat label="移速" value={snap.speed ? snap.speed.toFixed(2) : "—"} />
              <Stat label="幸运" value={snap.luck ? snap.luck.toFixed(1) : "—"} />
              <Stat
                label="主动道具"
                value={
                  snap.active
                    ? `#${snap.active}${snap.max_charge ? ` (${snap.charge}/${snap.max_charge})` : ""}`
                    : "—"
                }
              />
              <Stat label="饰品" value={snap.trinket ? `#${snap.trinket}` : "—"} />
            </div>

            <div className="held">
              <span className="held-title">持有道具 ({snap.items.length})</span>
              <div className="chips">
                {snap.items.map((id) => (
                  <span key={id} className="chip" title={`ID ${id}`}>
                    {nameById.get(id) ?? `#${id}`}
                  </span>
                ))}
                {snap.items.length === 0 && <span className="chip muted">无</span>}
              </div>
            </div>
          </Card>

          <Card title="快捷操作">
            <div className="actions">
              <button className="btn danger" onClick={() => void runAction("killall", "已清空房间")}>
                秒杀全屏敌人
              </button>
              <button className="btn" onClick={() => void runAction("fullcharge", "主动道具已充满")}>
                充满主动道具
              </button>
              <button className="btn" onClick={() => void api.openFolder("mod")}>
                mod 目录
              </button>
              <button className="btn" onClick={() => void api.openFolder("save")}>
                存档目录
              </button>
            </div>
          </Card>

          <Card title="设置" hint="改选项需要重启游戏">
            <div className="toggle-grid single">
              <Toggle
                label="启用游戏 Mods"
                hint="写入 options.ini: EnableMods"
                accent="gold"
                value={!!info?.mods_enabled}
                onChange={(v) => void toggleOption("mods", v)}
              />
              <Toggle
                label="启用调试控制台"
                hint="写入 options.ini: EnableDebugConsole，进游戏按 ~"
                accent="gold"
                value={!!info?.console_enabled}
                onChange={(v) => void toggleOption("console", v)}
              />
            </div>

            <div className="path-box">
              <span className="path-label">游戏目录</span>
              <code>{info?.game_dir ?? "未找到"}</code>
            </div>

            <div className="row gap">
              <input
                className="search"
                placeholder="手动指定游戏目录（含 isaac-ng.exe）"
                value={manualDir}
                onChange={(e) => setManualDir(e.target.value)}
              />
              <button
                className="btn"
                onClick={async () => {
                  try {
                    const next = await api.setGameDir(manualDir);
                    setInfo(next);
                    notify("游戏目录已保存");
                  } catch (e) {
                    notify(String(e), true);
                  }
                }}
              >
                应用
              </button>
            </div>

            <div className="row gap">
              <button
                className="btn"
                onClick={async () => {
                  try {
                    const next = await api.installMod();
                    setInfo(next);
                    notify("Mod 已安装（记得在游戏里勾选）");
                  } catch (e) {
                    notify(String(e), true);
                  }
                }}
              >
                重新安装 Mod
              </button>
              <button
                className="btn"
                onClick={async () => {
                  try {
                    const next = await api.uninstallMod();
                    setInfo(next);
                    notify("Mod 已卸载");
                  } catch (e) {
                    notify(String(e), true);
                  }
                }}
              >
                卸载 Mod
              </button>
            </div>
          </Card>
        </div>
      </main>

      {toast && <div className={"toast" + (toast.bad ? " bad" : "")}>{toast.text}</div>}
    </div>
  );
}
