use std::fs::File;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Cli {
    pub inline: Option<String>,
    pub toggle: bool,
    pub quit: bool,
    pub fresh: bool,
    pub style: Option<String>,
    pub demo: bool,
}

pub fn parse(args: &[String], allow: &[&str]) -> Result<Cli, String> {
    let mut cli = Cli { inline: None, toggle: false, quit: false, fresh: false, style: None, demo: false };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if !allow.contains(&a.as_str()) {
            return Err(format!("unknown argument {a}"));
        }
        match a.as_str() {
            "--toml" => cli.inline = Some(it.next().ok_or("--toml needs a value")?.clone()),
            "--style" => cli.style = Some(it.next().ok_or("--style needs a value")?.clone()),
            "--toggle" => cli.toggle = true,
            "--quit" => cli.quit = true,
            "--fresh" => cli.fresh = true,
            "--demo" => cli.demo = true,
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    Ok(cli)
}

fn sock(dir: &Path) -> PathBuf {
    dir.join("ctl")
}

pub fn send(dir: &Path, msg: &str) -> Option<String> {
    let mut s = UnixStream::connect(sock(dir)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(msg.as_bytes()).ok()?;
    s.shutdown(Shutdown::Write).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out.trim().to_string())
}

pub fn lock(dir: &Path, wait: Duration) -> Option<File> {
    std::fs::create_dir_all(dir).ok()?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock")).ok()?;
    let until = Instant::now() + wait;
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Some(f);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub enum Front {
    Start(Duration),
    Done(i32),
}

pub fn front(dir: &Path, name: &str, cli: &Cli) -> Front {
    let off = dir.join("off");
    if cli.quit {
        return match send(dir, "quit") {
            Some(_) => {
                println!("{name}: stopped");
                Front::Done(0)
            }
            None => {
                println!("{name}: not running");
                Front::Done(1)
            }
        };
    }
    if let Some(s) = &cli.style {
        return match send(dir, &format!("style {s}")) {
            Some(r) => {
                println!("{name}: {r}");
                Front::Done(0)
            }
            None => {
                println!("{name}: not running");
                Front::Done(1)
            }
        };
    }
    if cli.fresh {
        let _ = std::fs::remove_file(&off);
    }
    if cli.toggle {
        if send(dir, "quit").is_some() {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&off, b"");
            println!("{name}: off");
            return Front::Done(0);
        }
        let _ = std::fs::remove_file(&off);
        return Front::Start(Duration::from_secs(3));
    }
    let Some(inline) = &cli.inline else {
        return Front::Start(Duration::ZERO);
    };
    if off.exists() {
        return Front::Done(0);
    }
    match send(dir, &format!("config\n{inline}")).as_deref() {
        None => Front::Start(Duration::from_secs(1)),
        Some("same") => Front::Done(0),
        Some(_) => Front::Start(Duration::from_secs(3)),
    }
}

pub fn serve<F>(dir: &Path, name: &'static str, inline: Option<String>, extra: F)
where
    F: Fn(&str) -> Option<String> + Send + 'static,
{
    let path = sock(dir);
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{name}: control socket {}: {e}", path.display());
            return;
        }
    };
    let own = inline.unwrap_or_default();
    std::thread::spawn(move || {
        for mut c in listener.incoming().flatten() {
            let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
            let mut msg = String::new();
            if c.read_to_string(&mut msg).is_err() {
                continue;
            }
            let (cmd, rest) = msg.split_once('\n').unwrap_or((msg.as_str(), ""));
            let (reply, exit) = match cmd.trim() {
                "quit" => ("bye".to_string(), true),
                "ping" => ("pong".to_string(), false),
                "config" if rest == own => ("same".to_string(), false),
                "config" => ("restart".to_string(), true),
                other => (extra(other).unwrap_or_else(|| format!("unknown command {other}")), false),
            };
            let _ = c.write_all(reply.as_bytes());
            let _ = c.flush();
            drop(c);
            if exit {
                let _ = std::fs::remove_file(&path);
                std::process::exit(0);
            }
        }
    });
}
