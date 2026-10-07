-- Globals that Factorio's engine provides to the settings and data stages.
-- This file is our own code (not from the game).

-- Factorio removes these for determinism and sandboxing.
io, os, dofile, loadfile, coroutine = nil, nil, nil, nil, nil
if debug then
  debug = { getinfo = debug.getinfo, traceback = debug.traceback }
end

-- Minimal stand-in for the `serpent` serialiser Factorio bundles. Data-stage code mostly
-- uses it for error messages and log output, so readable output is all that matters.
local function sorted_keys(t)
  local keys = {}
  for k in pairs(t) do keys[#keys + 1] = k end
  table.sort(keys, function(a, b)
    local ta, tb = type(a), type(b)
    if ta ~= tb then return ta < tb end
    if ta == "number" or ta == "string" then return a < b end
    return tostring(a) < tostring(b)
  end)
  return keys
end

local function dump(v, indent, opts, depth, seen)
  local tv = type(v)
  if tv == "string" then return string.format("%q", v) end
  if tv ~= "table" then return tostring(v) end
  if seen[v] then return '"<cycle>"' end
  if opts.maxlevel and depth >= opts.maxlevel then return "{...}" end
  seen[v] = true
  local parts = {}
  local n = #v
  for i = 1, n do parts[#parts + 1] = dump(v[i], indent, opts, depth + 1, seen) end
  for _, k in ipairs(sorted_keys(v)) do
    if not (type(k) == "number" and k >= 1 and k <= n and k % 1 == 0) then
      local key = (type(k) == "string" and k:match("^[%a_][%w_]*$")) and k or ("[" .. dump(k, indent, opts, depth + 1, seen) .. "]")
      parts[#parts + 1] = key .. " = " .. dump(v[k], indent, opts, depth + 1, seen)
    end
  end
  seen[v] = nil
  if indent and #parts > 0 then
    local pad = string.rep(indent, depth + 1)
    return "{\n" .. pad .. table.concat(parts, ",\n" .. pad) .. "\n" .. string.rep(indent, depth) .. "}"
  end
  return "{" .. table.concat(parts, ", ") .. "}"
end

serpent = {
  block = function(v, opts) return dump(v, "  ", opts or {}, 0, {}) end,
  line = function(v, opts) return dump(v, nil, opts or {}, 0, {}) end,
  dump = function(v, opts) return "do local _ = " .. dump(v, nil, opts or {}, 0, {}) .. "; return _; end" end,
  load = function(s) local f = load(s); if f then return true, f() end return false end,
}
serpent.serialize = serpent.line

function table_size(t)
  local n = 0
  for _ in pairs(t) do n = n + 1 end
  return n
end
