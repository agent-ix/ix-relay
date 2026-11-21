-- Helpers every named-focus script starts with. The bounds (MAX_RECORD_FIELDS and the other
-- upper-case locals) are declared above this file by the relay from `named_focus::limits`.
local function decode(raw)
  if not raw then return nil end
  local ok, value = pcall(cjson.decode, raw)
  if ok and type(value) == 'table' then return value end
  return nil
end
local function has_prefix(text, prefix)
  return string.sub(text, 1, string.len(prefix)) == prefix
end
