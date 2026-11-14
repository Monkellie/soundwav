# soundwav

Audio waves wrapped around the focused window, for the [Zoolander](https://git.pika-os.com/otter-shell/zoolander) Wayland compositor.

soundwav reads your desktop audio through [cava](https://github.com/karlstav/cava) and draws a glowing ring of waves around whatever window has focus. Bass, mids and treble each get their own wave and colour. The ring grows outwards from the window border into free space and never covers the window itself. Where a neighbouring tile sits close by, it draws a smaller, dimmed ring over the neighbour. It follows focus across workspaces and monitors, hides when nothing is playing, and pauses cava while a fullscreen window is up, so games are left alone.

There is also a `bars` style: a classic cava spectrum bent around the window.

## Requirements

- Zoolander 0.1.39 or newer (it reads Zoolander's control socket)
- cava, with PipeWire (via `pipewire-pulse`) or PulseAudio
- a Vulkan GPU driver (rendering goes through wgpu)
- Rust 1.90+ to build

## Install

```sh
git clone https://github.com/Monkellie/soundwav
cd soundwav
./install.sh
```

`install.sh` builds a release binary into `~/.local/bin/soundwav` and copies the Zoolander plugin to `~/.config/zoolander/plugins/soundwav/`.

Then load the plugin from `~/.config/zoolander/init.lua`:

```lua
local ok, soundwav = pcall(require, "plugins.soundwav")
if ok then
    soundwav.setup {
        app_id = "*",
        style = "waves",
    }
end
```

Saving `init.lua` reloads Zoolander and starts soundwav. You don't need to add anything to `exec.lua`: the plugin starts soundwav on login and on every config reload.

## Plugin

Zoolander has no plugin API, so the plugin is a Lua module. It hooks `zl.on("startup")` and `zl.on("config_reloaded")` and passes your settings to soundwav as TOML.

- With the same settings, a reload leaves soundwav running.
- With changed settings, soundwav restarts with the new ones.
- `enabled = false` stops it.

Default keys:

| Key | Action |
| --- | --- |
| `SUPER + ALT + w` | turn soundwav on or off (off stays off across reloads until the next login) |
| `SUPER + ALT + SHIFT + w` | switch between `waves` and `bars` |

Change them with `keys = { toggle = "...", style = "..." }`, or turn them off with `keys = false`.

The setup options that aren't soundwav settings are:

| Option | Meaning |
| --- | --- |
| `keys` | keybinds, see above |
| `bin` | path to the binary (default `$HOME/.local/bin/soundwav`) |
| `env` | extra environment variables, e.g. `{ SOUNDWAV_DEBUG = "1" }` |
| `enabled` | `false` stops soundwav |

The plugin also adds no-animation layer rules for soundwav's surfaces.

## Settings

Every setting can go in the `setup { }` table or in `~/.config/soundwav/config.toml`. Values from the plugin win over the file.

| Setting | Default | Meaning |
| --- | --- | --- |
| `app_id` | `"kitty"` | which windows get the ring: an app id, a comma list, or `"*"` for any focused window |
| `style` | `"waves"` | `"waves"` or `"bars"` |
| `wave_colors` | `["#B44CFF", "#FF5FB7", "#5FD4FF"]` | bass, mids, treble; setting this turns off theme colours for the waves |
| `theme` | `"auto"` | theme colour file: `"auto"` is `~/.config/soundwav/theme.toml`, `"off"` disables it, anything else is a path |
| `theme_spread` | `40.0` | hue distance (degrees) of the bass and treble waves from the theme accent |
| `theme_boost` | `1.25` | saturation multiplier for theme colours |
| `wave_line` | `2.5` | crest line width (px) |
| `wave_fill` | `0.28` | opacity of the fill between the border and the crest |
| `wave_speed` | `1.0` | wave movement speed multiplier |
| `wave_step` | `3.0` | sample spacing along the ring (px); lower is smoother and costs more CPU |
| `corner_blend` | `120.0` | distance (px) over which ring heights blend around corners |
| `colors` | theme file, else cava's `gradient_color_*` | bar colours in `bars` style |
| `bars_per_channel` | `96` | spectrum resolution in `bars` style |
| `bar_width`, `bar_gap` | `5.0`, `3.0` | bar shape in `bars` style |
| `border` | `5.0` | gap between the window border and the ring (px) |
| `corner_radius` | `14.0` | should match your window rounding |
| `min_open` | `60.0` | free space (px) a side needs to get the full outward ring |
| `max_out` | `220.0` | maximum ring height into free space (px) |
| `max_over`, `max_over_frac` | `56.0`, `0.12` | cap for the ring drawn over a close neighbour (px, and fraction of the window's short side) |
| `over_alpha` | `0.8` | opacity of the ring over neighbours |
| `glow` | `6.0` | glow radius |
| `opacity` | `0.95` | overall opacity |
| `fps` | `60` | frame rate of cava and the ring (cava capped at 144) |
| `follow_ms`, `fade_ms` | `90`, `250` | how fast the ring follows focus and fades in or out |
| `layer` | `"top"` | layer-shell layer of the ring |
| `reserve` | `[0, 0, 0, 0]` | extra space to keep free at each screen edge, `[top, right, bottom, left]` px, so the ring has room even around a lone full tile |
| `cava_input`, `cava_source` | `"pulse"`, `"auto"` | passed to cava's `[input]` section |
| `tab_min`, `tab_strip_width`, `tab_strip_offset` | `2`, `30`, `12` | keeps the ring out of the way of [side-tabs](https://github.com/Monkellie/side-tabs) strips |

Zoolander has only one `gaps_out` value, so `reserve` is how you get a bigger ring on some sides than others. It creates invisible exclusive zones on the overlay layer, and drops them on any monitor that has a fullscreen window.

## Wallpaper colours

soundwav can take its colours from a theme file that something else keeps up to date. It checks the file once a second and fades into new colours without restarting. The file can hold:

```toml
accent = "#CD95FF"
wave_colors = ["#7689FF", "#CD95FF", "#FFBEF2"]
colors = ["#4C278A", "#CD95FF", "#FFFEFF"]
```

`wave_colors` sets the three waves directly. With only `accent`, soundwav builds them around it: the accent for the mids, and the hue turned `theme_spread` degrees either way for the bass and treble. `colors` is for the `bars` style. Colours you set in `setup { }` or `config.toml` win over the theme file.

### otter-theme-gen

If `install.sh` finds `~/.config/otter-shell/otter-theme-gen.conf`, it installs `otter-theme-gen/soundwav.toml` as a template and registers it, keeping a backup of the config as `otter-theme-gen.conf.bak-soundwav`. otter-theme-gen then rewrites `~/.config/soundwav/theme.toml` whenever the wallpaper changes, so the ring follows the wallpaper.

otter-theme-gen's `direct` strategy takes one colour from the wallpaper (`primary`); its secondary and tertiary roles are fixed status colours. That's why the template passes only the accent. Edit `~/.config/otter-shell/templates/soundwav.toml` to pick other roles.

To undo it, delete the three `template_soundwav_*` lines from `otter-theme-gen.conf`.

## Command line

```
soundwav [--toml TOML] [--toggle | --quit | --style waves|bars|next | --fresh] [--demo]
```

The plugin runs all of this for you. It's also handy from a terminal or another keybind: `soundwav --style next` or `soundwav --toggle`. A running instance is controlled through `$XDG_RUNTIME_DIR/soundwav/ctl`. `--demo` swaps cava for a synthetic spectrum.

Logs go to `$XDG_RUNTIME_DIR/soundwav.log` when started by the plugin. Set `SOUNDWAV_DEBUG=1` for a status line every few seconds.

## Notes

- On laptops and desktops with both an iGPU and a dGPU, soundwav picks the high-performance adapter that can present to the compositor. CPU renderers (llvmpipe) are refused unless `SOUNDWAV_ALLOW_CPU=1` is set, mainly for headless testing.
- Fractional scaling is handled by rendering at the logical size.
- Expect roughly 2–5% of one CPU core while music plays, and close to 0% when idle.

## License

MIT
