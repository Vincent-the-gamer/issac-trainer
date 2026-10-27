--[[
  Isaac Trainer · companion mod
  ------------------------------------------------------------------
  Companion mod for the "Isaac Trainer" desktop app. It never writes to
  the game's memory and never patches the executable: everything below is
  the official modding API.

  Why this looks odd
  ------------------
  Repentance's Lua sandbox has no `io` / `os.execute`, so a mod cannot
  read or write plain files. Two channels are left:

    app -> mod : `include()` a Lua file. `include` is Repentance's own
                 loader and it never caches, so re-including the same
                 path re-reads the file from disk every time.
    mod -> app : `Isaac.DebugString()`, which appends to log.txt. The
                 desktop app tails that file.

  Command file (rewritten by the app, at the mod root):
    trainer_cmd.lua   return { seq = <n>, god = 1, ..., act = {"giveitem:1"} }

  Log lines (prefixed `[trainer]`):
    [trainer]S boot=.. cur=.. hp=.. ...      live player state
    [trainer]C|kind|id|name                  one entry of the item table
    [trainer]CDONE <count>                   item table finished
    [trainer]B <boot>                        "mod loaded" banner
    [trainer] <text>                         diagnostics
]]

local mod = RegisterMod("Isaac Trainer Bridge", 1)

-- ---------------------------------------------------------------- channels

local function log(text)
  if Isaac and Isaac.DebugString then
    Isaac.DebugString("[trainer] " .. tostring(text))
  end
end

local hasInclude = (type(include) == "function")
local hasIO = (type(io) == "table" and type(io.open) == "function")

if not hasInclude then
  log("include() unavailable, bridge disabled")
  return
end

-- `include` is given a path relative to the mod folder. Repentance accepts
-- both spellings in different builds, so try both once and keep the winner.
local CMD_PATHS = { "trainer_cmd.lua", "trainer_cmd" }
local cmdPath = nil
local cmdProbed = false

local function readCmd(path)
  local ok, value = pcall(include, path)
  if ok and type(value) == "table" then
    return value
  end
  return nil
end

local function resolveCmdPath()
  for _, path in ipairs(CMD_PATHS) do
    if readCmd(path) then
      return path
    end
  end
  return nil
end

-- ---------------------------------------------------------------- state

local S = {
  god = 0,
  heal = 0,
  maxhp = 0,
  damage = 1,
  tears = 1,
  speed = 1,
  range = 1,
  shotspeed = 1,
  luck = 0,
  infcharge = 0,
  cash = 0,
}

local CF = {
  damage = (CacheFlag and CacheFlag.CACHE_DAMAGE) or 1,
  firedelay = (CacheFlag and CacheFlag.CACHE_FIREDELAY) or 2,
  speed = (CacheFlag and CacheFlag.CACHE_SPEED) or 4,
  range = (CacheFlag and CacheFlag.CACHE_RANGE) or 8,
  shotspeed = (CacheFlag and CacheFlag.CACHE_SHOTSPEED) or 16,
  luck = (CacheFlag and CacheFlag.CACHE_LUCK) or 32,
}

local cur = 0
local boot = 0
local frame = 0
local needsCache = false

local function nowMs()
  if Isaac and Isaac.GetTime then
    local ok, value = pcall(Isaac.GetTime)
    if ok and value then
      return value
    end
  end
  return 0
end

boot = nowMs()

-- ---------------------------------------------------------------- helpers

local function players()
  local list = {}
  local game = Game()
  if not game then
    return list
  end
  local ok, count = pcall(function()
    return game:GetNumPlayers()
  end)
  if not ok or not count or count < 1 then
    count = 1
  end
  for i = 0, count - 1 do
    local player = Isaac.GetPlayer(i)
    if player then
      list[#list + 1] = player
    end
  end
  return list
end

local function refreshCache()
  local flags = CF.damage + CF.firedelay + CF.speed + CF.range + CF.shotspeed + CF.luck
  for _, player in ipairs(players()) do
    pcall(function()
      player:AddCacheFlags(flags)
      player:EvaluateItems()
    end)
  end
end

local function clean(value)
  return (tostring(value or ""):gsub("%s+", "_"):gsub("[=]", "_"))
end

-- ---------------------------------------------------------------- actions

local function forEachPlayer(fn)
  for _, player in ipairs(players()) do
    pcall(fn, player)
  end
end

local function fullHeal(player)
  local maxHp = player:GetMaxHearts()
  local hp = player:GetHearts()
  if maxHp > 0 and hp < maxHp then
    player:AddHearts(maxHp - hp)
  end
  if player:GetSoulHearts() <= 0 and player:GetBlackHearts() <= 0 then
    player:AddSoulHearts(2)
  end
end

local function killRoom()
  for _, entity in ipairs(Isaac.GetRoomEntities()) do
    local ok, hostile = pcall(function()
      return entity:IsActiveEnemy(false) and not entity:IsDead()
    end)
    if ok and hostile then
      pcall(function()
        entity:Kill()
      end)
    end
  end
end

local function applyAction(action)
  local kind, arg = action:match("^([%w_]+):?(.*)$")
  if not kind then
    return
  end
  local n = tonumber(arg)

  if kind == "giveitem" and n then
    forEachPlayer(function(player)
      player:AddCollectible(n, 0, false)
    end)
  elseif kind == "givetrinket" and n then
    forEachPlayer(function(player)
      player:AddTrinket(n)
    end)
  elseif kind == "givecard" and n then
    forEachPlayer(function(player)
      player:AddCard(n)
    end)
  elseif kind == "givepill" and n then
    forEachPlayer(function(player)
      local ok = pcall(function()
        player:AddPill(n)
      end)
      if not ok then
        pcall(function()
          player:AddPill(n, 0)
        end)
      end
    end)
  elseif kind == "heal" then
    forEachPlayer(fullHeal)
  elseif kind == "addhearts" and n then
    forEachPlayer(function(player)
      player:AddMaxHearts(n * 2, true)
    end)
  elseif kind == "addsoul" and n then
    forEachPlayer(function(player)
      player:AddSoulHearts(n * 2)
    end)
  elseif kind == "killall" then
    killRoom()
  elseif kind == "fullcharge" then
    forEachPlayer(function(player)
      for slot = 0, 1 do
        pcall(function()
          local maxCharge = player:GetActiveMaxCharge(slot)
          if maxCharge and maxCharge > 0 then
            player:SetActiveCharge(maxCharge, slot)
          end
        end)
      end
    end)
  end
end

-- ---------------------------------------------------------------- command in

local function applySettings(cmd)
  local function num(value, fallback)
    local parsed = tonumber(value)
    if parsed == nil then
      return fallback
    end
    return parsed
  end

  S.god = num(cmd.god, S.god)
  S.heal = num(cmd.heal, S.heal)
  S.maxhp = num(cmd.maxhp, S.maxhp)
  S.damage = num(cmd.damage, S.damage)
  S.tears = num(cmd.tears, S.tears)
  S.speed = num(cmd.speed, S.speed)
  S.range = num(cmd.range, S.range)
  S.shotspeed = num(cmd.shotspeed, S.shotspeed)
  S.luck = num(cmd.luck, S.luck)
  S.infcharge = num(cmd.infcharge, S.infcharge)
  S.cash = num(cmd.cash, S.cash)
end

local function pollCmd()
  if not cmdProbed then
    cmdProbed = true
    cmdPath = resolveCmdPath()
    log(
      string.format(
        "caps io=%s include=%s cmdfile=%s",
        tostring(hasIO),
        tostring(hasInclude),
        tostring(cmdPath or "none")
      )
    )
  end
  if not cmdPath then
    cmdPath = resolveCmdPath()
    if not cmdPath then
      return
    end
    log("command file linked via include('" .. cmdPath .. "')")
  end

  local cmd = readCmd(cmdPath)
  if not cmd then
    return
  end
  local seq = tonumber(cmd.seq)
  if not seq or seq <= cur then
    return
  end

  -- The very first command after (re)load only carries settings: replaying
  -- queued one-shot actions from an older session would double-give items.
  local firstTouch = (cur == 0)
  cur = seq
  applySettings(cmd)
  needsCache = true

  if not firstTouch and type(cmd.act) == "table" then
    for _, action in ipairs(cmd.act) do
      local ok, err = pcall(applyAction, tostring(action))
      if not ok then
        log("action failed: " .. tostring(action) .. " (" .. tostring(err) .. ")")
      end
    end
  end
end

-- ---------------------------------------------------------------- item db

local ITEMS_PER_TICK = 30
local itemQueue = {}
local itemIdx = 1
local itemTotal = 0
local itemsDone = false

local function buildItemQueue()
  local config = Isaac.GetItemConfig()
  if not config then
    return
  end

  local function push(kind, id, entry)
    if not entry then
      return
    end
    local name = entry.Name
    if not name or name == "" or name == "NULL" then
      return
    end
    name = tostring(name):gsub("[\r\n|]", " ")
    itemQueue[#itemQueue + 1] = kind .. "|" .. id .. "|" .. name
  end

  for id = 1, 800 do
    pcall(function()
      push("C", id, config:GetCollectible(id))
    end)
  end
  for id = 1, 400 do
    pcall(function()
      push("T", id, config:GetTrinket(id))
    end)
  end
  for id = 1, 200 do
    pcall(function()
      push("K", id, config:GetCard(id, false))
    end)
  end
  for id = 1, 120 do
    pcall(function()
      push("P", id, config:GetPillEffect(id))
    end)
  end

  itemTotal = #itemQueue
end

local function pumpItems()
  if itemsDone then
    return
  end
  if #itemQueue == 0 then
    buildItemQueue()
    if #itemQueue == 0 then
      return
    end
    log("item table: " .. itemTotal .. " entries")
  end

  local sent = 0
  while itemIdx <= #itemQueue and sent < ITEMS_PER_TICK do
    Isaac.DebugString("[trainer]C|" .. itemQueue[itemIdx])
    itemIdx = itemIdx + 1
    sent = sent + 1
  end

  if itemIdx > #itemQueue then
    itemsDone = true
    itemQueue = {}
    Isaac.DebugString("[trainer]CDONE " .. itemTotal)
  end
end

-- ---------------------------------------------------------------- state out

local function emitState()
  local game = Game()
  local player = Isaac.GetPlayer(0)
  local parts = {}

  local function kv(key, value)
    parts[#parts + 1] = key .. "=" .. tostring(value)
  end

  kv("boot", boot)
  kv("cur", cur)

  local playing = false
  if game and player then
    local ok, maxHp = pcall(function()
      return player:GetMaxHearts()
    end)
    playing = ok and maxHp and maxHp > 0 or false
  end
  kv("play", playing and 1 or 0)

  if game and player then
    pcall(function()
      kv("hp", math.floor(player:GetHearts()))
    end)
    pcall(function()
      kv("maxhp", math.floor(player:GetMaxHearts()))
    end)
    pcall(function()
      kv("soul", math.floor(player:GetSoulHearts()))
    end)
    pcall(function()
      kv("black", math.floor(player:GetBlackHearts()))
    end)
    pcall(function()
      kv("bone", math.floor(player:GetBoneHearts()))
    end)
    pcall(function()
      kv("coins", math.floor(player:GetNumCoins()))
    end)
    pcall(function()
      kv("bombs", math.floor(player:GetNumBombs()))
    end)
    pcall(function()
      kv("keys", math.floor(player:GetNumKeys()))
    end)

    pcall(function()
      local held = {}
      local count = player:GetCollectibleCount()
      local limit = (count and count > 0) and count or 64
      for i = 0, limit - 1 do
        local id = player:GetCollectible(i)
        if id and id > 0 then
          held[#held + 1] = id
        end
      end
      table.sort(held)
      kv("items", table.concat(held, ","))
    end)

    pcall(function()
      kv("trink", player:GetTrinket(0) or 0)
    end)
    pcall(function()
      kv("act", player:GetActiveItem(0) or 0)
    end)
    pcall(function()
      kv("chg", math.floor(player:GetActiveCharge(0) or 0))
    end)
    pcall(function()
      kv("mchg", math.floor(player:GetActiveMaxCharge(0) or 0))
    end)
    pcall(function()
      kv("dmg", string.format("%.2f", player.Damage))
    end)
    pcall(function()
      kv("fd", string.format("%.2f", player.MaxFireDelay))
    end)
    pcall(function()
      kv("spd", string.format("%.2f", player.MoveSpeed))
    end)
    pcall(function()
      kv("luck", string.format("%.2f", player.Luck))
    end)
  end

  if game then
    pcall(function()
      kv("lvl", game:GetLevel():GetStage())
    end)
    pcall(function()
      kv("lname", clean(game:GetLevel():GetName()))
    end)
    pcall(function()
      kv("room", game:GetRoom():GetCurrentRoomIndex())
    end)
    pcall(function()
      kv("seed", clean(game:GetSeeds():GetStartSeedString()))
    end)
  end

  Isaac.DebugString("[trainer]S " .. table.concat(parts, " "))
end

-- ---------------------------------------------------------------- loops

local TICK = 4 -- ~15 Hz: command poll + persistent effects
local STATE_EVERY = 20 -- ~3 Hz: state line

local function applyContinuous()
  local list = players()
  if #list == 0 then
    return
  end

  if S.heal == 1 then
    for _, player in ipairs(list) do
      pcall(fullHeal, player)
    end
  end

  if S.maxhp > 0 then
    for _, player in ipairs(list) do
      pcall(function()
        local want = S.maxhp * 2
        local have = player:GetMaxHearts()
        if have < want then
          player:AddMaxHearts(want - have, true)
        elseif have > want then
          player:AddMaxHearts(-(have - want), false)
        end
      end)
    end
  end

  if S.cash == 1 then
    for _, player in ipairs(list) do
      pcall(function()
        local coins = player:GetNumCoins()
        if coins < 99 then
          player:AddCoins(99 - coins)
        end
        local bombs = player:GetNumBombs()
        if bombs < 99 then
          player:AddBombs(99 - bombs)
        end
        local keys = player:GetNumKeys()
        if keys < 99 then
          player:AddKeys(99 - keys)
        end
      end)
    end
  end

  if S.infcharge == 1 then
    for _, player in ipairs(list) do
      for slot = 0, 1 do
        pcall(function()
          local maxCharge = player:GetActiveMaxCharge(slot)
          if maxCharge and maxCharge > 0 and player:GetActiveCharge(slot) < maxCharge then
            player:SetActiveCharge(maxCharge, slot)
          end
        end)
      end
    end
  end
end

local function onUpdate()
  frame = frame + 1

  if frame % TICK == 0 then
    pollCmd()
    applyContinuous()
    pumpItems()
    if needsCache then
      needsCache = false
      refreshCache()
    end
  end

  if frame % STATE_EVERY == 0 then
    emitState()
  end
end

-- ---------------------------------------------------------------- callbacks

function mod:postUpdate()
  local ok, err = pcall(onUpdate)
  if not ok then
    log("update error: " .. tostring(err))
  end
end

function mod:entityTakeDmg(entity, amount, flags, source, cooldown)
  if S.god == 1 then
    local ok, isPlayer = pcall(function()
      return entity:ToPlayer() ~= nil
    end)
    if ok and isPlayer then
      return false
    end
  end
end

function mod:evaluateCache(player, flag)
  if flag == CF.damage and S.damage ~= 1 then
    player.Damage = player.Damage * S.damage
  elseif flag == CF.firedelay and S.tears ~= 1 then
    player.MaxFireDelay = player.MaxFireDelay / S.tears
  elseif flag == CF.speed and S.speed ~= 1 then
    player.MoveSpeed = player.MoveSpeed * S.speed
  elseif flag == CF.range and S.range ~= 1 then
    player.TearRange = player.TearRange * S.range
    player.TearHeight = player.TearHeight * S.range
  elseif flag == CF.shotspeed and S.shotspeed ~= 1 then
    player.ShotSpeed = player.ShotSpeed * S.shotspeed
  elseif flag == CF.luck and S.luck ~= 0 then
    player.Luck = player.Luck + S.luck
  end
end

function mod:postNewRoom()
  needsCache = true
end

function mod:postNewLevel()
  needsCache = true
end

function mod:gameStarted()
  needsCache = true
end

mod:AddCallback(ModCallbacks.MC_POST_UPDATE, mod.postUpdate)
mod:AddCallback(ModCallbacks.MC_ENTITY_TAKE_DMG, mod.entityTakeDmg)
mod:AddCallback(ModCallbacks.MC_EVALUATE_CACHE, mod.evaluateCache)
mod:AddCallback(ModCallbacks.MC_POST_NEW_ROOM, mod.postNewRoom)
mod:AddCallback(ModCallbacks.MC_POST_NEW_LEVEL, mod.postNewLevel)
mod:AddCallback(ModCallbacks.MC_POST_GAME_STARTED, mod.gameStarted)

Isaac.DebugString("[trainer]B " .. tostring(boot))
log("mod loaded (bridge v2)")
