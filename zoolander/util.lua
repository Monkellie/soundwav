local M = {}

local named = {
    space = zl.KEY_SPACE,
    tab = zl.KEY_TAB,
    ["return"] = zl.KEY_RETURN,
    escape = zl.KEY_ESCAPE,
    backspace = zl.KEY_BACKSPACE,
    left = zl.KEY_LEFT,
    right = zl.KEY_RIGHT,
    up = zl.KEY_UP,
    down = zl.KEY_DOWN,
    comma = string.byte(","),
    period = string.byte("."),
    slash = string.byte("/"),
    backslash = string.byte("\\"),
    bracketleft = string.byte("["),
    bracketright = string.byte("]"),
    equal = string.byte("="),
    minus = string.byte("-"),
}

function M.keys(spec)
    local mods, key = 0, nil
    for token in spec:gmatch("[^+]+") do
        local v = token:match("^%s*(.-)%s*$"):lower()
        if v == "shift" then mods = mods + zl.MOD_SHIFT
        elseif v == "ctrl" or v == "control" then mods = mods + zl.MOD_CTRL
        elseif v == "alt" then mods = mods + zl.MOD_ALT
        elseif v == "super" or v == "logo" then mods = mods + zl.MOD_LOGO
        else key = v end
    end
    local sym = key and (named[key] or (#key == 1 and string.byte(key)))
    assert(sym, "unknown key in " .. tostring(spec))
    return mods, sym
end

local function sh(s)
    return "'" .. (tostring(s):gsub("'", "'\\''")) .. "'"
end

local function value(v)
    local t = type(v)
    if t == "string" then
        return '"' .. (v:gsub("\\", "\\\\"):gsub('"', '\\"'):gsub("\n", "\\n")) .. '"'
    elseif t == "boolean" then
        return tostring(v)
    elseif t == "number" then
        if v == math.floor(v) and math.abs(v) < 2 ^ 53 then return string.format("%d", v) end
        return tostring(v)
    elseif t == "table" then
        local out = {}
        for i, x in ipairs(v) do out[i] = value(x) end
        return "[" .. table.concat(out, ", ") .. "]"
    end
    error("unsupported setting value " .. tostring(v))
end

local reserved = { bin = true, env = true, keys = true, enabled = true }

function M.toml(opts)
    local names = {}
    for k in pairs(opts) do
        if not reserved[k] then names[#names + 1] = k end
    end
    table.sort(names)
    local lines = {}
    for i, k in ipairs(names) do lines[i] = k .. " = " .. value(opts[k]) end
    return table.concat(lines, "\n")
end

function M.daemon(name, opts, actions)
    opts = opts or {}
    pcall(zl.add_layer_no_animation_rule, "^(" .. name .. ")$")
    pcall(zl.add_layer_no_animation_rule, "^(" .. name .. "-.*)$")
    local env = {}
    for k, v in pairs(opts.env or {}) do env[#env + 1] = k .. "=" .. sh(v) end
    table.sort(env)
    local prefix = #env > 0 and (table.concat(env, " ") .. " ") or ""
    local bin = opts.bin or ('"$HOME/.local/bin/' .. name .. '"')
    local log = '"${XDG_RUNTIME_DIR:-/tmp}/' .. name .. '.log"'
    local base = prefix .. bin .. " --toml " .. sh(M.toml(opts))
    local function run(extra)
        return zl.exec_command(base .. extra .. " >>" .. log .. " 2>&1 &")
    end
    local function plain(extra)
        return zl.exec_command(prefix .. bin .. extra .. " >>" .. log .. " 2>&1 &")
    end
    if opts.enabled == false then
        zl.on("startup", function() plain(" --quit") end)
        zl.on("config_reloaded", function() plain(" --quit") end)
        return
    end
    zl.on("startup", function() run(" --fresh") end)
    zl.on("config_reloaded", function() run("") end)
    local keys = opts.keys
    if keys == false then return end
    for action, spec in pairs(keys or {}) do
        local fn = actions[action]
        if fn and spec then
            local mods, sym = M.keys(spec)
            pcall(zl.bind, mods, sym, function() fn(run, plain) end, { description = name .. ": " .. action })
        end
    end
end

return M
