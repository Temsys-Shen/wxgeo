#![allow(dead_code)]
mod ca;
mod campus;
mod config;
mod inject;
mod proxy;
mod sysproxy;

use anyhow::Result;
use config::Config;
use proxy::{LogFn, ProxyServer};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

type SharedLog = Arc<Mutex<VecDeque<String>>>;

fn now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

fn make_logger(shared: SharedLog, echo: bool) -> LogFn {
    Arc::new(move |m: String| {
        let line = format!("{} {}", now(), m);
        if echo {
            println!("{}", line);
        }
        let mut g = shared.lock().unwrap();
        g.push_back(line);
        while g.len() > 800 {
            g.pop_front();
        }
    })
}

struct App {
    cfg: Config,
    shared: SharedLog,
    proxy: Option<ProxyServer>,
    sysproxy: bool,
}

impl App {
    fn new() -> App {
        let cfg = Config::load();
        let shared: SharedLog = Arc::new(Mutex::new(VecDeque::new()));
        let sysproxy = sysproxy::system_proxy_on(cfg.port);
        App { cfg, shared, proxy: None, sysproxy }
    }
    fn logger(&self) -> LogFn { make_logger(self.shared.clone(), false) }
    fn start(&mut self) {
        if self.proxy.is_some() { return; }
        match ProxyServer::start(Arc::new(self.cfg.clone()), self.logger()) {
            Ok(p) => self.proxy = Some(p),
            Err(e) => self.push(format!("启动失败: {}", e)),
        }
    }
    fn stop(&mut self) {
        if let Some(p) = self.proxy.take() { p.stop(); }
    }
    fn push(&self, m: String) {
        let mut g = self.shared.lock().unwrap();
        g.push_back(format!("{} {}", now(), m));
        while g.len() > 800 { g.pop_front(); }
    }
    fn set_sysproxy(&mut self, on: bool) {
        match sysproxy::system_proxy(on, self.cfg.port) {
            Ok(m) => { self.sysproxy = on; self.push(format!("系统代理{}（{}）", if on { "开启" } else { "关闭" }, m)); }
            Err(e) => self.push(format!("系统代理失败: {}", e)),
        }
    }
}

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = std::fs::create_dir_all(config::app_dir());
    if cfg!(target_os = "linux") && !config::is_root() {
        eprintln!("提示: Linux 下安装 CA 需要 root 权限，建议直接执行 sudo wxgeo；工具会自动沿用原用户配置，并以原用户身份设置桌面代理。");
    }
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "--selftest" => {
            let url = args.get(2).cloned().unwrap_or_else(|| "https://www.wjx.top/app/checkin.aspx?activity=398036887".to_string());
            std::process::exit(selftest(&url));
        }
        "--headless" => headless(),
        "--update-campus-db" => {
            match campus::update_db() {
                Ok(n) => println!("已更新 {} 条高校校区数据到 {}", n, campus::user_path().display()),
                Err(e) => eprintln!("更新失败: {}", e),
            }
            std::process::exit(0);
        }
        "--plain" => plain(),
        "--help" | "-h" => {
            println!("wxgeo {}\n用法: wxgeo [--selftest [URL] | --headless | --plain | --update-campus-db]", env!("CARGO_PKG_VERSION"));
        }
        _ => tui(),
    }
}

fn selftest(url: &str) -> i32 {
    let cfg = Config::load();
    let shared: SharedLog = Arc::new(Mutex::new(VecDeque::new()));
    let log = make_logger(shared, true);
    println!("CA 目录: {}", ca::ca_pem_path().display());
    let proxy = match ProxyServer::start(Arc::new(cfg.clone()), log) {
        Ok(p) => p,
        Err(e) => { eprintln!("启动失败: {}", e); return 1; }
    };
    std::thread::sleep(std::time::Duration::from_millis(500));
    let out_file = std::env::temp_dir().join("wxgeo_selftest.html");
    let status = std::process::Command::new("curl")
        .args(["-s", "-m", "20", "--cacert", &ca::ca_pem_path().to_string_lossy(),
               "-x", &format!("http://127.0.0.1:{}", cfg.port),
               "-A", "Mozilla/5.0 (iPhone) MicroMessenger/8.0",
               "-o", &out_file.to_string_lossy(), "-w", "%{http_code}", url])
        .output();
    let code = match &status {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(e) => format!("err:{e}"),
    };
    let injected = std::fs::read_to_string(&out_file)
        .map(|s| s.contains(&format!("LAT={}", cfg.lat)))
        .unwrap_or(false);
    println!("HTTP: {} 注入: {}", code, injected);
    proxy.stop();
    if code == "200" && injected { 0 } else { 1 }
}

fn headless() {
    let cfg = Config::load();
    let shared: SharedLog = Arc::new(Mutex::new(VecDeque::new()));
    let log = make_logger(shared, true);
    match ProxyServer::start(Arc::new(cfg), log) {
        Ok(_p) => {
            println!("按 Ctrl-C 停止");
            loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
        }
        Err(e) => eprintln!("启动失败: {}", e),
    }
}

fn ask_line(label: &str, default: &str) -> String {
    print!("{} [{}]: ", label, default);
    let _ = io::stdout().flush();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
    let s = s.trim().to_string();
    if s.is_empty() { default.to_string() } else { s }
}

fn plain() {
    let mut app = App::new();
    loop {
        let on = app.proxy.is_some();
        println!("\n==== wxgeo ====");
        println!("代理:{}  端口:{}  CA:{}  系统代理:{}  请求:{}",
            if on { "运行" } else { "停止" }, app.cfg.port,
            if sysproxy::ca_trusted() { "已信任" } else { "未信任" },
            if app.sysproxy { "开" } else { "关" },
            app.proxy.as_ref().map(|p| p.count.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0));
        println!("坐标:{:.6},{:.6}  精度:{:.0}m  域名:{}", app.cfg.lat, app.cfg.lon, app.cfg.acc,
            if app.cfg.all_hosts { "*".to_string() } else { app.cfg.domains.join(",") });
        println!("命令: s 启停  c 坐标  d 域名  a 全站  j 自定义JS  t 信任CA  p 系统代理  u 选校区  r 重启  q 退出");
        let mut line = String::new();
        print!("> ");
        let _ = io::stdout().flush();
        if io::stdin().read_line(&mut line).is_err() { break; }
        match line.trim() {
            "q" => break,
            "s" => { if app.proxy.is_some() { app.stop(); } else { app.start(); } }
            "r" => { app.stop(); app.start(); }
            "c" => {
                let v = ask_line("纬度", &app.cfg.lat.to_string());
                if let Ok(x) = v.parse() { app.cfg.lat = x; }
                let v = ask_line("经度", &app.cfg.lon.to_string());
                if let Ok(x) = v.parse() { app.cfg.lon = x; }
                let v = ask_line("精度", &app.cfg.acc.to_string());
                if let Ok(x) = v.parse() { app.cfg.acc = x; }
                app.cfg.save();
            }
            "d" => {
                let v = ask_line("域名逗号分隔", &app.cfg.domains.join(","));
                app.cfg.domains = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                app.cfg.save();
            }
            "a" => { app.cfg.all_hosts = !app.cfg.all_hosts; app.cfg.save(); }
            "j" => { app.cfg.extra_js = ask_line("自定义JS单行", &app.cfg.extra_js); app.cfg.save(); }
            "t" => {
                if sysproxy::ca_trusted() { let _ = sysproxy::untrust_ca(); app.push("已移除 CA 信任".into()); }
                else { match sysproxy::trust_ca() { Ok(_) => app.push("已信任本地 CA".into()), Err(e) => app.push(format!("信任失败: {}", e)) } }
            }
            "p" => { let target = !app.sysproxy; app.set_sysproxy(target); }
            "u" => {
                let kw = ask_line("搜索高校/校区", "");
                let list = campus::load();
                let hits = campus::search(&list, &kw, 20);
                if hits.is_empty() {
                    println!("未找到: {}", kw);
                } else {
                    for (i, c) in hits.iter().enumerate() {
                        println!("{:>2}. {} {} {:.6},{:.6}", i + 1, c.name, c.city, c.lat, c.lon);
                    }
                    let sel = ask_line("选择序号（回车取消）", "");
                    if let Ok(n) = sel.parse::<usize>() {
                        if n >= 1 && n <= hits.len() {
                            let c = hits[n - 1];
                            app.cfg.lat = c.lat;
                            app.cfg.lon = c.lon;
                            app.cfg.save();
                            println!("已选择 {}（{:.6},{:.6}）", c.name, c.lat, c.lon);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    app.stop();
    if app.sysproxy { app.set_sysproxy(false); }
    println!("已退出");
}

fn tui() {
    use crossterm::{cursor, event, execute, terminal};
    let mut app = App::new();
    let mut out = io::stdout();
    let _ = terminal::enable_raw_mode();
    let _ = execute!(out, terminal::EnterAlternateScreen, cursor::Hide);

    let res = (|| -> Result<()> {
        loop {
            draw(&mut out, &app)?;
            if event::poll(std::time::Duration::from_millis(200))? {
                if let event::Event::Key(k) = event::read()? {
                    use event::KeyCode::*;
                    match k.code {
                        Char('q') => break,
                        Char('s') => { if app.proxy.is_some() { app.stop(); } else { app.start(); } }
                        Char('r') => { app.stop(); app.start(); }
                        Char('c') => {
                            let v = tui_prompt(&mut out, "纬度", &app.cfg.lat.to_string())?;
                            if let Ok(x) = v.parse() { app.cfg.lat = x; }
                            let v = tui_prompt(&mut out, "经度", &app.cfg.lon.to_string())?;
                            if let Ok(x) = v.parse() { app.cfg.lon = x; }
                            let v = tui_prompt(&mut out, "精度", &app.cfg.acc.to_string())?;
                            if let Ok(x) = v.parse() { app.cfg.acc = x; }
                            app.cfg.save();
                        }
                        Char('d') => {
                            let v = tui_prompt(&mut out, "域名逗号分隔", &app.cfg.domains.join(","))?;
                            app.cfg.domains = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                            app.cfg.save();
                        }
                        Char('a') => { app.cfg.all_hosts = !app.cfg.all_hosts; app.cfg.save(); }
                        Char('j') => { let v = tui_prompt(&mut out, "自定义JS", &app.cfg.extra_js)?; app.cfg.extra_js = v; app.cfg.save(); }
                        Char('t') => {
                            if sysproxy::ca_trusted() { let _ = sysproxy::untrust_ca(); app.push("已移除 CA 信任".into()); }
                            else { match sysproxy::trust_ca() { Ok(_) => app.push("已信任本地 CA".into()), Err(e) => app.push(format!("信任失败: {}", e)) } }
                        }
                        Char('p') => { let t = !app.sysproxy; app.set_sysproxy(t); }
                        Char('l') => { app.shared.lock().unwrap().clear(); }
                        Char('u') => {
                            let kw = tui_prompt(&mut out, "搜索高校/校区", "")?;
                            let list = campus::load();
                            let hits = campus::search(&list, &kw, 20);
                            if hits.is_empty() {
                                app.push(format!("未找到: {}", kw));
                            } else {
                                app.push(format!("搜索结果 {} 条:", hits.len()));
                                for (i, c) in hits.iter().enumerate() {
                                    app.push(format!("{:>2}. {} {} {:.6},{:.6}", i + 1, c.name, c.city, c.lat, c.lon));
                                }
                                let sel = tui_prompt(&mut out, "选择序号（回车取消）", "")?;
                                if let Ok(n) = sel.parse::<usize>() {
                                    if n >= 1 && n <= hits.len() {
                                        let c = hits[n - 1];
                                        app.cfg.lat = c.lat;
                                        app.cfg.lon = c.lon;
                                        app.cfg.save();
                                        app.push(format!("已选择 {}（{:.6},{:.6}）", c.name, c.lat, c.lon));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    })();

    app.stop();
    if app.sysproxy { app.set_sysproxy(false); }
    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
    if let Err(e) = res { eprintln!("TUI 错误: {}", e); }
}

fn draw(out: &mut io::Stdout, app: &App) -> Result<()> {
    use crossterm::{cursor, execute, style::Print, terminal};
    let (w, h) = terminal::size().unwrap_or((100, 30));
    let w = w as usize;
    execute!(out, cursor::MoveTo(0, 0), terminal::Clear(terminal::ClearType::All))?;
    let on = app.proxy.is_some();
    let line1 = " wxgeo 微信定位伪装 TUI ";
    let line2 = format!(" 代理:{}  端口:{}  CA:{}  系统代理:{}  请求:{} ",
        if on { "运行" } else { "停止" }, app.cfg.port,
        if sysproxy::ca_trusted() { "已信任" } else { "未信任" },
        if app.sysproxy { "开" } else { "关" },
        app.proxy.as_ref().map(|p| p.count.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0));
    let line3 = format!(" 坐标:{:.6},{:.6} 精度:{:.0}m  范围:{} ",
        app.cfg.lat, app.cfg.lon, app.cfg.acc,
        if app.cfg.all_hosts { "*".to_string() } else { app.cfg.domains.join(",") });
    for (i, l) in [line1, line2.as_str(), line3.as_str()].iter().enumerate() {
        execute!(out, cursor::MoveTo(0, i as u16), Print(trunc(l, w)))?;
    }
    let logs = app.shared.lock().unwrap();
    let body = (h as usize).saturating_sub(4);
    let skip = logs.len().saturating_sub(body);
    for (i, l) in logs.iter().skip(skip).enumerate() {
        execute!(out, cursor::MoveTo(0, (3 + i) as u16), Print(trunc(l, w)))?;
    }
    let foot = " s 启停  c 坐标  d 域名  a 全站  j 自定义JS  t 信任CA  p 系统代理  u 选校区  r 重启  l 清屏  q 退出 ";
    execute!(out, cursor::MoveTo(0, h.saturating_sub(1)), Print(trunc(foot, w)))?;
    out.flush()?;
    Ok(())
}

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w { s.to_string() } else { s.chars().take(w).collect() }
}

fn tui_prompt(out: &mut io::Stdout, label: &str, default: &str) -> Result<String> {
    use crossterm::{cursor, event, execute, style::Print, terminal};
    let (w, h) = terminal::size().unwrap_or((100, 30));
    let mut buf = String::new();
    loop {
        let line = format!("{} [{}]: {}", label, default, buf);
        execute!(out, cursor::MoveTo(0, h.saturating_sub(2)), terminal::Clear(terminal::ClearType::CurrentLine), Print(trunc(&line, w as usize)), cursor::Show)?;
        out.flush()?;
        if let event::Event::Key(k) = event::read()? {
            use event::KeyCode::*;
            match k.code {
                Enter => { execute!(out, cursor::Hide)?; return Ok(if buf.is_empty() { default.to_string() } else { buf }); }
                Esc => { execute!(out, cursor::Hide)?; return Ok(default.to_string()); }
                Backspace => { buf.pop(); }
                Char(c) => buf.push(c),
                _ => {}
            }
        }
    }
}
