mod cava;
mod config;
mod layout;
mod gpu;
mod render;
mod reserve;
mod theme;
mod waves;
mod zoo;

use config::StyleCmd;
use std::sync::{Arc, Mutex};
use zoo_ipc::{Daemon, Front, Spec};

pub const DAEMON: Daemon = Daemon::new("soundwav");

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

const SPEC: Spec = Spec { commands: &[("--style", "style")], flags: &[DEMO] };
const DEMO: &str = "--demo";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }
    let cli = match DAEMON.parse(&args, &SPEC) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("soundwav: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let wait = match DAEMON.front(&cli) {
        Front::Done(code) => std::process::exit(code),
        Front::Start(w) => w,
    };
    let Some(_lock) = DAEMON.lock(wait) else {
        eprintln!("soundwav: already running");
        return;
    };
    let demo = cli.has(DEMO);
    let cfg = config::load(cli.inline());
    let style_cfg = cfg.clone();
    DAEMON.serve(cli.inline(), move |cmd| {
        let arg = cmd.strip_prefix("style ")?;
        Some(match StyleCmd::parse(arg) {
            Some(c) => config::set_style(&style_cfg, c).name().to_string(),
            None => format!("bad style {arg}"),
        })
    });
    let audio = Arc::new(Mutex::new(cava::Audio::default()));
    let scene = Arc::new(Mutex::new(None));
    let control = Arc::new(cava::Control::default());
    if demo {
        cava::spawn_demo(cfg.clone(), audio.clone());
    } else {
        cava::spawn(cfg.clone(), audio.clone(), control.clone());
    }
    zoo::spawn(cfg.clone(), scene.clone(), control, demo);
    if let Err(e) = render::run(cfg, audio, scene) {
        eprintln!("soundwav: {e}");
        std::process::exit(1);
    }
}
