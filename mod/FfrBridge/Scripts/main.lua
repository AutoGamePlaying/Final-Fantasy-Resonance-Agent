-- FfrBridge v1: read-only file-bridge mod for FINAL FANTASY RESONANCE (AutoGamePlaying).
-- Samples v0 metadata and v1 topic states (field, battle, ui, party) on the game thread
-- every 500 ms and exchanges JSON files in <ue4ss>/ffr-out/bridge/ following AGP protocol v1.

local TAG = "[FfrBridge] "

local function dirname(path)
  return path:match("^(.*)[/\\][^/\\]*$")
end

local function locate()
  local info = debug.getinfo(1, "S")
  local source = info and info.source or ""
  local script = source:sub(1, 1) == "@" and source:sub(2) or ""
  local scripts = dirname(script)
  local mod = scripts and dirname(scripts) or nil
  local mods = mod and dirname(mod) or nil
  local ue4ss = mods and dirname(mods) or nil
  if ue4ss and ue4ss ~= "" then
    ue4ss = ue4ss:gsub("/", "\\")
    scripts = scripts:gsub("/", "\\")
    return ue4ss, scripts
  end
  if type(IterateGameDirectories) ~= "function" then return nil end
  local ok, directories = pcall(IterateGameDirectories)
  if not ok or not directories then return nil end
  local okPath, win64 = pcall(function()
    local directory = directories.Game.Binaries.Win64
    return directory.__absolute_path or tostring(directory)
  end)
  if not okPath or type(win64) ~= "string" or win64 == "" then return nil end
  ue4ss = win64:gsub("/", "\\") .. "\\ue4ss"
  return ue4ss, ue4ss .. "\\Mods\\FfrBridge\\Scripts"
end

local ue4ss, scripts = locate()
if not ue4ss then
  print(TAG .. "error 无法确定 UE4SS 目录\n")
  return
end

package.path = scripts .. "\\?.lua;" .. package.path
local state = require("ffrbridge.state")
local protocol = require("ffrbridge.protocol")

local topics = state.loadTopics()
local BRIDGE = ue4ss .. "\\ffr-out\\bridge"
local bridge = protocol.create({ directory = BRIDGE, tag = TAG, topics = topics })
local inFlight = false

print(
  TAG
    .. "loaded v="
    .. state.BRIDGE_VERSION
    .. " bridge="
    .. BRIDGE
    .. " session="
    .. bridge.session
    .. " commands="
    .. tostring(#bridge.commands)
    .. "\n"
)

LoopAsync(500, function()
  if inFlight then return false end
  if not bridge.hasDirectory() then return false end
  inFlight = true
  local ok, err = pcall(ExecuteInGameThread, function()
    local tickOk, tickErr = pcall(bridge.tick)
    inFlight = false
    if not tickOk then bridge.logError("tick", tickErr) end
  end)
  if not ok then
    inFlight = false
    bridge.logError("schedule", err)
  end
  return false
end)
