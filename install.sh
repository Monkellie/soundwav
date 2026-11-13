#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.config/zoolander/plugins/soundwav"
install -m 755 target/release/soundwav "$HOME/.local/bin/soundwav.new"
mv "$HOME/.local/bin/soundwav.new" "$HOME/.local/bin/soundwav"
install -m 644 zoolander/init.lua zoolander/util.lua "$HOME/.config/zoolander/plugins/soundwav/"
echo "installed soundwav; load it from ~/.config/zoolander/init.lua, see README.md"
