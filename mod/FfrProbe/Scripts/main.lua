-- FfrProbe: read-only feasibility probe for FINAL FANTASY RESONANCE (AutoGamePlaying).
-- Every 5 s, on the game thread, log the world, player controller, pawn, game state and the
-- first distinct actor classes. Never presses keys or writes game state. Stops after 5 minutes.
local TAG = "[FfrProbe] "
local TICKS = 60
local tick = 0
local lastWorld = ""

local function name(obj)
  if obj == nil then return "nil" end
  local ok, valid = pcall(function() return obj:IsValid() end)
  if not ok or not valid then return "invalid" end
  local ok2, full = pcall(function() return obj:GetFullName() end)
  return ok2 and tostring(full) or "?"
end

local function className(obj)
  local ok, cls = pcall(function() return obj:GetClass() end)
  if not ok or cls == nil then return "?" end
  return name(cls)
end

local function sample()
  local world = FindFirstOf("World")
  local worldName = name(world)
  print(TAG .. "tick " .. tick .. " world=" .. worldName)
  local pc = FindFirstOf("PlayerController")
  print(TAG .. "  pc=" .. name(pc))
  if pc ~= nil and pc:IsValid() then
    local okPawn, pawn = pcall(function() return pc.Pawn end)
    if okPawn and pawn ~= nil and pawn:IsValid() then
      print(TAG .. "  pawn=" .. name(pawn))
      local okLoc, loc = pcall(function() return pawn:K2_GetActorLocation() end)
      if okLoc and loc ~= nil then
        print(string.format("%s  pawnLocation=%.1f,%.1f,%.1f", TAG, loc.X, loc.Y, loc.Z))
      end
    end
  end
  print(TAG .. "  gameState=" .. className(FindFirstOf("GameStateBase")))
  print(TAG .. "  gameMode=" .. className(FindFirstOf("GameModeBase")))
  print(TAG .. "  gameInstance=" .. className(FindFirstOf("GameInstance")))
  if worldName ~= lastWorld then
    lastWorld = worldName
    local seen, count = {}, 0
    local actors = FindAllOf("Actor") or {}
    print(TAG .. "  actors=" .. #actors)
    for _, actor in ipairs(actors) do
      local c = className(actor)
      if not seen[c] then
        seen[c] = true
        count = count + 1
        print(TAG .. "  actorClass " .. c)
        if count >= 40 then break end
      end
    end
  end
end

print(TAG .. "loaded")
LoopAsync(5000, function()
  tick = tick + 1
  ExecuteInGameThread(function()
    local ok, err = pcall(sample)
    if not ok then print(TAG .. "error " .. tostring(err)) end
  end)
  return tick >= TICKS
end)
