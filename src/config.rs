use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn is_root() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
            .unwrap_or(false)
    })
}

pub fn sudo_user() -> Option<String> {
    if is_root() {
        if let Ok(u) = std::env::var("SUDO_USER") {
            if !u.is_empty() { return Some(u); }
        }
    }
    None
}

fn user_home(name: &str) -> Option<PathBuf> {
    let out = Command::new("getent").args(["passwd", name]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    let home = line.trim().split(':').nth(5)?;
    if home.is_empty() { None } else { Some(PathBuf::from(home)) }
}

pub fn home_dir() -> PathBuf {
    if let Some(u) = sudo_user() {
        if let Some(h) = user_home(&u) {
            return h;
        }
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

/// 以 sudo 运行时，把新建文件归还给原用户，避免 root 所有权
pub fn fix_owner(p: &Path) {
    #[cfg(unix)]
    {
        if let (Ok(u), Ok(g)) = (std::env::var("SUDO_UID"), std::env::var("SUDO_GID")) {
            let _ = Command::new("chown")
                .args(["-R", &format!("{}:{}", u, g), &p.to_string_lossy()])
                .status();
        }
    }
    let _ = p;
}

pub fn ensure_app_dirs() -> PathBuf {
    let a = app_dir();
    let _ = fs::create_dir_all(a.join("ca").join("hosts"));
    fix_owner(&a);
    a
}

pub fn app_dir() -> PathBuf { home_dir().join(".wxgeo") }
pub fn ca_dir() -> PathBuf { app_dir().join("ca") }
pub fn host_dir() -> PathBuf { ca_dir().join("hosts") }
pub fn cfg_path() -> PathBuf { app_dir().join("config.json") }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub lat: f64,
    pub lon: f64,
    pub acc: f64,
    pub domains: Vec<String>,
    pub all_hosts: bool,
    pub extra_js: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lat: 30.678232,
            lon: 104.098004,
            acc: 12.0,
            domains: vec!["wjx.top".into(), "wjx.cn".into()],
            all_hosts: false,
            extra_js: String::new(),
            port: 8888,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let _ = ensure_app_dirs();
        match fs::read_to_string(cfg_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
    pub fn save(&self) {
        let _ = ensure_app_dirs();
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(cfg_path(), s);
            fix_owner(&cfg_path());
        }
    }
    pub fn in_scope(&self, host: &str) -> bool {
        if self.all_hosts { return true; }
        self.domains.iter().any(|d| {
            !d.is_empty() && (host == d || host.ends_with(&format!(".{}", d)) || host.ends_with(d.as_str()))
        })
    }
}
