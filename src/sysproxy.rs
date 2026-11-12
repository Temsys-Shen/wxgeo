use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

pub const CA_NAME: &str = crate::ca::CA_NAME;

fn run(prog: &str, args: &[&str]) -> (i32, String, String) {
    match Command::new(prog).args(args).output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => (127, String::new(), e.to_string()),
    }
}

fn which(c: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(c).exists()))
        .unwrap_or(false)
}

fn env_file() -> PathBuf { crate::config::app_dir().join("proxy-env.sh") }

#[cfg(target_os = "linux")]
fn lrun(prog: &str, args: &[&str]) -> (i32, String, String) {
    let mut cmd = if let Some(u) = crate::config::sudo_user() {
        let mut c = Command::new("sudo");
        c.arg("-u").arg(u).arg(prog);
        c
    } else {
        Command::new(prog)
    };
    match cmd.args(args).output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => (127, String::new(), e.to_string()),
    }
}

pub fn trust_ca() -> Result<()> {
    let cert = crate::ca::ca_pem_path();
    let c = cert.to_string_lossy().to_string();
    #[cfg(target_os = "macos")]
    {
        let (rc, _, err) = run("security", &["add-trusted-cert", "-r", "trustRoot", "-p", "ssl",
            "-p", "basic", "-k", &format!("{}/Library/Keychains/login.keychain-db",
            std::env::var("HOME").unwrap_or_default()), &c]);
        if rc != 0 { return Err(anyhow!("security 失败: {err}")); }
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let (rc, _, err) = run("certutil", &["-addstore", "-f", "-user", "Root", &c]);
        if rc != 0 { return Err(anyhow!("certutil 失败: {err}")); }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let dst = "/usr/local/share/ca-certificates/wxgeo.crt";
        if crate::config::is_root() {
            std::fs::copy(&cert, dst).map_err(|e| anyhow!("复制 CA 失败: {e}"))?;
            run("update-ca-certificates", &[]);
        } else {
            let (rc, _, _) = run("sudo", &["-n", "cp", &c, dst]);
            if rc != 0 {
                std::fs::copy(&cert, dst).map_err(|e| anyhow!("复制 CA 失败（需要 root，请用 sudo wxgeo 运行）: {e}"))?;
            }
            run("sudo", &["-n", "update-ca-certificates"]);
            run("update-ca-certificates", &[]);
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("不支持的平台"))
}

pub fn untrust_ca() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let kc = format!("{}/Library/Keychains/login.keychain-db", std::env::var("HOME").unwrap_or_default());
        run("security", &["delete-certificate", "-c", CA_NAME, &kc]);
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        run("certutil", &["-delstore", "-user", "Root", CA_NAME]);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let dst = "/usr/local/share/ca-certificates/wxgeo.crt";
        if std::path::Path::new(dst).exists() {
            if crate::config::is_root() {
                let _ = std::fs::remove_file(dst);
                run("update-ca-certificates", &[]);
            } else {
                let (rc, _, _) = run("sudo", &["-n", "rm", "-f", dst]);
                if rc != 0 { let _ = std::fs::remove_file(dst); }
                run("sudo", &["-n", "update-ca-certificates"]);
                run("update-ca-certificates", &[]);
            }
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("不支持的平台"))
}

pub fn ca_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        let kc = format!("{}/Library/Keychains/login.keychain-db", std::env::var("HOME").unwrap_or_default());
        let (rc, out, _) = run("security", &["find-certificate", "-c", CA_NAME, &kc]);
        return rc == 0 && out.to_lowercase().contains("keychain");
    }
    #[cfg(target_os = "windows")]
    {
        let (rc, _, _) = run("certutil", &["-user", "-store", "Root", CA_NAME]);
        return rc == 0;
    }
    #[cfg(target_os = "linux")]
    {
        return std::path::Path::new("/usr/local/share/ca-certificates/wxgeo.crt").exists();
    }
    #[allow(unreachable_code)]
    false
}

fn linux_backend() -> &'static str {
    if let Ok(f) = std::env::var("WXGEO_PROXY_BACKEND") {
        let f = f.to_lowercase();
        if f == "gnome" || f == "kde" || f == "env" {
            return match f.as_str() { "gnome" => "gnome", "kde" => "kde", _ => "env" };
        }
    }
    let de = format!(
        "{} {} {}",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        std::env::var("DESKTOP_SESSION").unwrap_or_default(),
        std::env::var("KDE_FULL_SESSION").unwrap_or_default()
    ).to_lowercase();
    let kde = de.contains("kde") || de.contains("plasma");
    let has_kde = which("kwriteconfig6") || which("kwriteconfig5")
        || crate::config::home_dir().join(".config/kioslaverc").exists();
    if kde && has_kde { return "kde"; }
    if which("gsettings") {
        let (_, out, _) = run("gsettings", &["list-schemas"]);
        if out.contains("org.gnome.system.proxy") { return "gnome"; }
    }
    if has_kde { return "kde"; }
    "env"
}

fn write_env(on: bool, port: u16) -> String {
    let p = env_file();
    let _ = std::fs::create_dir_all(crate::config::app_dir());
    let body = if on {
        format!("# 执行 source {} 生效\nexport http_proxy=http://127.0.0.1:{p0}\nexport https_proxy=http://127.0.0.1:{p0}\nexport HTTP_PROXY=http://127.0.0.1:{p0}\nexport HTTPS_PROXY=http://127.0.0.1:{p0}\nexport no_proxy=localhost,127.0.0.1,::1\nexport NO_PROXY=localhost,127.0.0.1,::1\n", p.display(), p0 = port)
    } else {
        "# wxgeo 代理已关闭\nexport http_proxy=\nexport https_proxy=\nexport HTTP_PROXY=\nexport HTTPS_PROXY=\n".to_string()
    };
    let _ = std::fs::write(&p, body);
    crate::config::fix_owner(&p);
    p.to_string_lossy().to_string()
}

pub fn system_proxy(on: bool, port: u16) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let (rc, out, _) = run("networksetup", &["-listallnetworkservices"]);
        if rc != 0 { return Err(anyhow!("networksetup 不可用")); }
        let ps = port.to_string();
        for line in out.lines() {
            let s = line.trim();
            if s.is_empty() || s.starts_with('*') || s.starts_with("An asterisk") { continue; }
            if on {
                run("networksetup", &["-setwebproxy", s, "127.0.0.1", &ps]);
                run("networksetup", &["-setsecurewebproxy", s, "127.0.0.1", &ps]);
            } else {
                run("networksetup", &["-setwebproxystate", s, "off"]);
                run("networksetup", &["-setsecurewebproxystate", s, "off"]);
            }
        }
        return Ok("macOS networksetup".into());
    }
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        let (key, _) = hkcu.create_subkey(path)?;
        if on {
            key.set_value("ProxyEnable", &1u32)?;
            key.set_value("ProxyServer", &format!("127.0.0.1:{}", port))?;
            key.set_value("ProxyOverride", &"localhost;127.0.0.1;<local>")?;
        } else {
            key.set_value("ProxyEnable", &0u32)?;
        }
        return Ok("Windows 注册表 Internet Settings".into());
    }
    #[cfg(target_os = "linux")]
    {
        let b = linux_backend();
        match b {
            "gnome" => {
                if on {
                    lrun("gsettings", &["set", "org.gnome.system.proxy", "mode", "manual"]);
                    for sch in ["org.gnome.system.proxy.http", "org.gnome.system.proxy.https"] {
                        lrun("gsettings", &["set", sch, "host", "127.0.0.1"]);
                        lrun("gsettings", &["set", sch, "port", &port.to_string()]);
                    }
                } else {
                    lrun("gsettings", &["set", "org.gnome.system.proxy", "mode", "none"]);
                }
                return Ok("Linux gsettings（GNOME 系）".into());
            }
            "kde" => {
                let kc = if which("kwriteconfig6") { "kwriteconfig6" } else { "kwriteconfig5" };
                if which(kc) {
                    if on {
                        lrun(kc, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", "1"]);
                        lrun(kc, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpProxy", &format!("http://127.0.0.1:{}", port)]);
                        lrun(kc, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpsProxy", &format!("http://127.0.0.1:{}", port)]);
                        lrun(kc, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "NoProxy", "localhost,127.0.0.1,::1"]);
                    } else {
                        lrun(kc, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", "0"]);
                    }
                    if which("dbus-send") {
                        lrun("dbus-send", &["--type=signal", "/KIO/Scheduler", "org.kde.KIO.Scheduler.reparseSlaveConfiguration", "string:"]);
                    }
                    return Ok("Linux kioslaverc（KDE Plasma）".into());
                }
                return Ok(format!("Linux 环境变量文件 {}", write_env(on, port)));
            }
            _ => return Ok(format!("Linux 环境变量文件 {}", write_env(on, port))),
        }
    }
    #[allow(unreachable_code)]
    Err(anyhow!("不支持的平台"))
}

pub fn system_proxy_on(port: u16) -> bool {
    let _ = port;
    #[cfg(target_os = "macos")]
    {
        let (_, out, _) = run("scutil", &["--proxy"]);
        return out.contains("HTTPEnable : 1");
    }
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        if let Ok(key) = hkcu.open_subkey(path) {
            let v: u32 = key.get_value("ProxyEnable").unwrap_or(0);
            return v == 1;
        }
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        match linux_backend() {
            "gnome" => {
                let (_, out, _) = lrun("gsettings", &["get", "org.gnome.system.proxy", "mode"]);
                return out.contains("manual");
            }
            "kde" => {
                let kr = if which("kreadconfig6") { "kreadconfig6" } else { "kreadconfig5" };
                if which(kr) {
                    let (_, out, _) = lrun(kr, &["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType"]);
                    return out.trim() == "1";
                }
                return false;
            }
            _ => {
                return std::fs::read_to_string(env_file()).map(|s| s.contains("export http_proxy=http://127.0.0.1")).unwrap_or(false);
            }
        }
    }
    #[allow(unreachable_code)]
    false
}
