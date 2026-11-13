local util = require("plugins.soundwav.util")

local M = {}

local defaults = {
    keys = {
        toggle = "SUPER + ALT + w",
        style = "SUPER + ALT + SHIFT + w",
    },
}

function M.setup(opts)
    opts = opts or {}
    if opts.keys == nil then opts.keys = defaults.keys end
    util.daemon("soundwav", opts, {
        toggle = function(run) run(" --toggle") end,
        style = function(_, plain) plain(" --style next") end,
    })
end

return M
