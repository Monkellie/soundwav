mod cava;
mod config;
mod ctl;
mod layout;
mod render;
mod zoo;

use std::sync::{Arc, Mutex};

const USAGE: &str = "soundwav: audio waves wrapped around the focused window (Zoolander)

usage: soundwav [--toml TOML] [--toggle | --quit | --style waves|bars|next | --fresh] [--demo]

  --toml TOML   settings layered over ~/.config/soundwav/config.toml;
                if running with different settings, it restarts with these
  --toggle      stop if running, start if not
  --quit        stop the running instance
  --style S     switch the running instance to waves, bars or next
  --fresh       forget a previous --toggle off
  --demo        synthetic spectrum instead of cava (for testing)

zoolander plugin: ~/.config/zoolander/plugins/soundwav";

const ARGS: &[&str] = &["--toml", "--toggle", "--quit", "--style", "--fresh", "--demo"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }
    let cli = match ctl::parse(&args, ARGS) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("soundwav: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let dir = config::runtime_dir();
    let wait = match ctl::front(&dir, "soundwav", &cli) {
        ctl::Front::Done(code) => std::process::exit(code),
        ctl::Front::Start(w) => w,
    };
    let Some(_lock) = ctl::lock(&dir, wait) else {
        eprintln!("soundwav: already running");
        return;
    };
    let cfg = config::load(cli.inline.as_deref());
    let style_cfg = cfg.clone();
    ctl::serve(&dir, "soundwav", cli.inline.clone(), move |cmd| {
        let arg = cmd.strip_prefix("style ")?;
        Some(config::set_style(&style_cfg, arg).unwrap_or_else(|| format!("bad style {arg}")))
    });
    let audio = Arc::new(Mutex::new(cava::Audio::default()));
    let scene = Arc::new(Mutex::new(None));
    let control = Arc::new(cava::Control::default());
    if cli.demo {
        cava::spawn_demo(cfg.clone(), audio.clone());
    } else {
        cava::spawn(cfg.clone(), audio.clone(), control.clone());
    }
    zoo::spawn(cfg.clone(), scene.clone(), control, cli.demo);
    if let Err(e) = render::run(cfg, audio, scene) {
        eprintln!("soundwav: {e}");
        std::process::exit(1);
    }
}
