-- File-system helpers implementing the atomic write sequence from file-bridge v1:
-- 1. write X.tmp  2. remove old X.json  3. rename X.tmp -> X.json

local M = {}

function M.defaultFs()
  return {
    open = io.open,
    remove = os.remove,
    rename = os.rename,
  }
end

local function trimTrailingSeparator(path)
  if path == "/" or path == "\\" then return path end
  if path:match("^[A-Za-z]:[/\\]$") then return path end
  return (path:gsub("[/\\]+$", ""))
end

function M.join(directory, name)
  local clean = trimTrailingSeparator(directory)
  local separator = clean:find("\\", 1, true) and not clean:find("/", 1, true)
    and "\\" or "/"
  return clean .. separator .. name
end

function M.temporaryPath(path)
  local base = path:match("^(.*)%.[^%.\\/]*$")
  return (base or path) .. ".tmp"
end

function M.directoryExists(path, fs)
  if type(path) ~= "string" or path == "" then return false end
  local actual = fs or M.defaultFs()
  local clean = trimTrailingSeparator(path)
  local ok, _, code = actual.rename(clean, clean)
  return ok == true or code == 13
end

function M.fileExists(path, fs)
  if type(path) ~= "string" or path == "" then return false end
  local actual = fs or M.defaultFs()
  local file = actual.open(path, "rb")
  if not file then return false end
  pcall(function() file:close() end)
  return true
end

function M.readFile(path, fs)
  local actual = fs or M.defaultFs()
  local file, err = actual.open(path, "rb")
  if not file then return nil, err end
  local body = file:read("*a")
  file:close()
  return body, nil
end

function M.writeAtomic(path, content, fs)
  local actual = fs or M.defaultFs()
  local temporary = M.temporaryPath(path)
  local file, openErr = actual.open(temporary, "wb")
  if not file then
    error("无法打开临时文件：" .. tostring(openErr))
  end
  local ok, writeErr = file:write(content)
  if not ok then
    pcall(function() file:close() end)
    actual.remove(temporary)
    error("写临时文件失败：" .. tostring(writeErr))
  end
  local closed, closeErr = file:close()
  if not closed then
    actual.remove(temporary)
    error("关闭临时文件失败：" .. tostring(closeErr))
  end
  actual.remove(path)
  local renamed, renameErr = actual.rename(temporary, path)
  if not renamed then
    error("改名输出文件失败：" .. tostring(renameErr))
  end
end

return M
