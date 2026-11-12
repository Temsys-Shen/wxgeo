use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
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
        let _ = fs::create_dir_all(app_dir());
        match fs::read_to_string(cfg_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
    pub fn save(&self) {
        let _ = fs::create_dir_all(app_dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(cfg_path(), s);
        }
    }
    pub fn in_scope(&self, host: &str) -> bool {
        if self.all_hosts { return true; }
        self.domains.iter().any(|d| {
            !d.is_empty() && (host == d || host.ends_with(&format!(".{}", d)) || host.ends_with(d.as_str()))
        })
    }
}
