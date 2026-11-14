#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.config/zoolander/plugins/soundwav"
install -m 755 target/release/soundwav "$HOME/.local/bin/soundwav.new"
mv "$HOME/.local/bin/soundwav.new" "$HOME/.local/bin/soundwav"
install -m 644 zoolander/init.lua zoolander/util.lua "$HOME/.config/zoolander/plugins/soundwav/"
conf="$HOME/.config/otter-shell/otter-theme-gen.conf"
if [ -f "$conf" ]; then
    mkdir -p "$HOME/.config/otter-shell/templates"
    install -m 644 otter-theme-gen/soundwav.toml "$HOME/.config/otter-shell/templates/soundwav.toml"
    if ! grep -q '^template_soundwav_enabled' "$conf"; then
        cp "$conf" "$conf.bak-soundwav"
        printf '%s\n' \
            "template_soundwav_enabled = true" \
            "template_soundwav_target = ~/.config/soundwav/theme.toml" \
            "template_soundwav_source = $HOME/.config/otter-shell/templates/soundwav.toml" >> "$conf"
        echo "otter-theme-gen found: soundwav now follows your wallpaper colours (backup: $conf.bak-soundwav)"
    fi
fi
echo "installed soundwav; load it from ~/.config/zoolander/init.lua, see README.md"
