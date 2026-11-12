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
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
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
    campuses: Vec<campus::Campus>,
}

impl App {
    fn new() -> App {
        let cfg = Config::load();
        let shared: SharedLog = Arc::new(Mutex::new(VecDeque::new()));
        let sysproxy = sysproxy::system_proxy_on(cfg.port);
        App { cfg, shared, proxy: None, sysproxy, campuses: campus::load() }
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
    fn count(&self) -> u64 {
        self.proxy.as_ref().map(|p| p.count.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0)
    }
}

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = config::ensure_app_dirs();
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
        "--plain" => plain(),
        "--update-campus-db" => {
            match campus::update_db() {
                Ok(n) => println!("已更新 {} 条高校校区数据到 {}", n, campus::user_path().display()),
                Err(e) => eprintln!("更新失败: {}", e),
            }
            std::process::exit(0);
        }
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
            if app.sysproxy { "开" } else { "关" }, app.count());
        println!("坐标:{:.6},{:.6}  精度:{:.0}m  域名:{}", app.cfg.lat, app.cfg.lon, app.cfg.acc,
            if app.cfg.all_hosts { "*".to_string() } else { app.cfg.domains.join(",") });
        println!("命令: s 启停  c 坐标  d 域名  a 全站  j 自定义JS  t 信任CA  p 系统代理  r 重启  q 退出");
        let mut line = String::new();
        print!("> ");
        let _ = io::stdout().flush();
        if io::stdin().read_line(&mut line).is_err() { break; }
        match line.trim() {
            "q" => break,
            "s" => { if app.proxy.is_some() { app.stop(); } else { app.start(); } }
            "r" => { app.stop(); app.start(); }
            "c" => {
                let m = ask_line("1 自定义坐标  2 选择高校", "1");
                if m.trim() == "2" {
                    plain_campus(&mut app);
                } else {
                    let v = ask_line("纬度", &app.cfg.lat.to_string());
                    if let Ok(x) = v.parse() { app.cfg.lat = x; }
                    let v = ask_line("经度", &app.cfg.lon.to_string());
                    if let Ok(x) = v.parse() { app.cfg.lon = x; }
                    let v = ask_line("精度", &app.cfg.acc.to_string());
                    if let Ok(x) = v.parse() { app.cfg.acc = x; }
                    app.cfg.save();
                }
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
            _ => {}
        }
    }
    app.stop();
    if app.sysproxy { app.set_sysproxy(false); }
    println!("已退出");
}

fn plain_campus(app: &mut App) {
    let kw = ask_line("搜索高校/校区（前缀优先）", "");
    let hits = campus_filter(&app.campuses, &kw, 30);
    if hits.is_empty() { println!("未找到: {}", kw); return; }
    for (i, c) in hits.iter().enumerate() {
        println!("{:>2}. {}  {}  {:.6},{:.6}", i + 1, c.name, c.city, c.lat, c.lon);
    }
    let sel = ask_line("选择序号（回车取消）", "");
    if let Ok(n) = sel.parse::<usize>() {
        if n >= 1 && n <= hits.len() {
            let c = hits[n - 1];
            app.cfg.lat = c.lat; app.cfg.lon = c.lon; app.cfg.save();
            println!("已选择 {}（{:.6},{:.6}）", c.name, c.lat, c.lon);
        }
    }
}

/// 头匹配优先，结果不足时回退包含匹配
fn campus_filter<'a>(list: &'a [campus::Campus], kw: &str, limit: usize) -> Vec<&'a campus::Campus> {
    let k = kw.trim();
    if k.is_empty() {
        return list.iter().take(limit).collect();
    }
    let mut pref: Vec<&campus::Campus> = list.iter().filter(|c| c.name.starts_with(k)).take(limit).collect();
    if pref.len() < limit {
        for c in list.iter() {
            if pref.len() >= limit { break; }
            if !c.name.starts_with(k) && c.name.contains(k) { pref.push(c); }
        }
    }
    pref
}

// ---------------- TUI (ratatui) ----------------

#[derive(PartialEq)]
enum Mode {
    Main,
    CoordMenu { sel: usize },
    Campus { query: String, sel: usize, offset: usize },
    CoordInput { field: u8, buf: String },
    TextInput { kind: TextKind, buf: String },
}

#[derive(PartialEq, Clone, Copy)]
enum TextKind { Domains, Js }

struct Ui {
    app: App,
    mode: Mode,
    campus_list: Vec<usize>,
}

impl Ui {
    fn new() -> Ui {
        let app = App::new();
        let n = app.campuses.len();
        Ui { app, mode: Mode::Main, campus_list: (0..n).collect() }
    }

    fn refilter(&mut self, query: &str) {
        let k = query.trim();
        let mut out: Vec<usize> = Vec::new();
        if k.is_empty() {
            out.extend(0..self.app.campuses.len());
        } else {
            for (i, c) in self.app.campuses.iter().enumerate() {
                if c.name.starts_with(k) { out.push(i); }
            }
            if out.len() < 50 {
                for (i, c) in self.app.campuses.iter().enumerate() {
                    if !c.name.starts_with(k) && c.name.contains(k) { out.push(i); }
                }
            }
        }
        self.campus_list = out;
    }

    fn run(&mut self) -> Result<()> {
        use crossterm::{event, terminal};
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, terminal::EnterAlternateScreen)?;
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend)?;
        let mut running = true;
        while running {
            term.draw(|f| self.draw(f))?;
            if event::poll(std::time::Duration::from_millis(200))? {
                if let event::Event::Key(k) = event::read()? {
                    running = self.handle(k);
                }
            }
        }
        self.app.stop();
        if self.app.sysproxy { self.app.set_sysproxy(false); }
        crossterm::execute!(term.backend_mut(), terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        term.show_cursor()?;
        Ok(())
    }

    fn handle(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode as K;
        let mode = std::mem::replace(&mut self.mode, Mode::Main);
        match mode {
            Mode::Main => match key.code {
                K::Char('q') => return false,
                K::Char('s') => { if self.app.proxy.is_some() { self.app.stop(); } else { self.app.start(); } }
                K::Char('r') => { self.app.stop(); self.app.start(); }
                K::Char('c') => { self.mode = Mode::CoordMenu { sel: 0 }; return true; }
                K::Char('d') => { self.mode = Mode::TextInput { kind: TextKind::Domains, buf: self.app.cfg.domains.join(",") }; return true; }
                K::Char('a') => { self.app.cfg.all_hosts = !self.app.cfg.all_hosts; self.app.cfg.save(); }
                K::Char('j') => { self.mode = Mode::TextInput { kind: TextKind::Js, buf: self.app.cfg.extra_js.clone() }; return true; }
                K::Char('t') => {
                    if sysproxy::ca_trusted() { let _ = sysproxy::untrust_ca(); self.app.push("已移除 CA 信任".into()); }
                    else { match sysproxy::trust_ca() { Ok(_) => self.app.push("已信任本地 CA".into()), Err(e) => self.app.push(format!("信任失败: {}", e)) } }
                }
                K::Char('p') => { let t = !self.app.sysproxy; self.app.set_sysproxy(t); }
                K::Char('l') => { self.app.shared.lock().unwrap().clear(); }
                _ => {}
            },
            Mode::CoordMenu { sel } => match key.code {
                K::Esc | K::Char('q') => self.mode = Mode::Main,
                K::Up | K::Char('k') => self.mode = Mode::CoordMenu { sel: sel.saturating_sub(1) },
                K::Down | K::Char('j') => self.mode = Mode::CoordMenu { sel: (sel + 1).min(1) },
                K::Enter | K::Char(' ') => {
                    if sel == 1 {
                        self.refilter("");
                        self.mode = Mode::Campus { query: String::new(), sel: 0, offset: 0 };
                    } else {
                        self.mode = Mode::CoordInput { field: 0, buf: format!("{}", self.app.cfg.lat) };
                    }
                }
                _ => self.mode = Mode::CoordMenu { sel },
            },
            Mode::Campus { mut query, mut sel, mut offset } => match key.code {
                K::Esc => self.mode = Mode::CoordMenu { sel: 1 },
                K::Up | K::Char('k') => { if sel > 0 { sel -= 1; } self.mode = Mode::Campus { query, sel, offset }; }
                K::Down | K::Char('j') => { if sel + 1 < self.campus_list.len() { sel += 1; } self.mode = Mode::Campus { query, sel, offset }; }
                K::PageUp => { sel = sel.saturating_sub(10); self.mode = Mode::Campus { query, sel, offset }; }
                K::PageDown => { sel = (sel + 10).min(self.campus_list.len().saturating_sub(1)); self.mode = Mode::Campus { query, sel, offset }; }
                K::Backspace => { query.pop(); self.mode = Mode::Campus { query: query.clone(), sel: 0, offset: 0 }; self.refilter(&query); }
                K::Enter => {
                    if let Some(&idx) = self.campus_list.get(sel) {
                        let c = &self.app.campuses[idx];
                        self.app.cfg.lat = c.lat;
                        self.app.cfg.lon = c.lon;
                        self.app.cfg.save();
                        let msg = format!("已选择 {}（{:.6},{:.6}）", c.name, c.lat, c.lon);
                        self.app.push(msg);
                    }
                    self.mode = Mode::Main;
                }
                K::Char(ch) => { query.push(ch); self.mode = Mode::Campus { query: query.clone(), sel: 0, offset: 0 }; self.refilter(&query); }
                _ => self.mode = Mode::Campus { query, sel, offset },
            },
            Mode::CoordInput { field, mut buf } => match key.code {
                K::Esc => self.mode = Mode::Main,
                K::Backspace => { buf.pop(); self.mode = Mode::CoordInput { field, buf }; }
                K::Char(ch) => { buf.push(ch); self.mode = Mode::CoordInput { field, buf }; }
                K::Enter => {
                    if let Ok(v) = buf.trim().parse::<f64>() {
                        match field {
                            0 => self.app.cfg.lat = v,
                            1 => self.app.cfg.lon = v,
                            _ => self.app.cfg.acc = v,
                        }
                    }
                    if field < 2 {
                        let next = field + 1;
                        let label_val = match next { 1 => format!("{}", self.app.cfg.lon), _ => format!("{}", self.app.cfg.acc) };
                        self.mode = Mode::CoordInput { field: next, buf: label_val };
                    } else {
                        self.app.cfg.save();
                        self.app.push(format!("坐标更新 {:.6},{:.6} 精度 {:.0}m", self.app.cfg.lat, self.app.cfg.lon, self.app.cfg.acc));
                        self.mode = Mode::Main;
                    }
                }
                _ => self.mode = Mode::CoordInput { field, buf },
            },
            Mode::TextInput { kind, mut buf } => match key.code {
                K::Esc => self.mode = Mode::Main,
                K::Backspace => { buf.pop(); self.mode = Mode::TextInput { kind, buf }; }
                K::Char(ch) => { buf.push(ch); self.mode = Mode::TextInput { kind, buf }; }
                K::Enter => {
                    match kind {
                        TextKind::Domains => {
                            self.app.cfg.domains = buf.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                        }
                        TextKind::Js => { self.app.cfg.extra_js = buf.clone(); }
                    }
                    self.app.cfg.save();
                    self.mode = Mode::Main;
                }
                _ => self.mode = Mode::TextInput { kind, buf },
            },
        }
        true
    }

    fn draw(&mut self, f: &mut Frame) {
        match &self.mode {
            Mode::Main => self.draw_main(f),
            Mode::CoordMenu { sel } => { self.draw_main(f); self.draw_coord_menu(f, *sel); }
            Mode::Campus { query, sel, offset } => self.draw_campus(f, query, *sel, *offset),
            Mode::CoordInput { field, buf } => { self.draw_main(f); self.draw_input(f, &coord_label(*field), buf); }
            Mode::TextInput { kind, buf } => { self.draw_main(f); self.draw_input(f, if *kind == TextKind::Domains { "域名(逗号分隔)" } else { "自定义JS" }, buf); }
        }
    }

    fn draw_main(&self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(3), Constraint::Length(2)]).split(area);
        let on = self.app.proxy.is_some();
        let header = vec![
            Line::from(Span::styled(" wxgeo 微信定位伪装 TUI ", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan))),
            Line::from(format!(" 代理:{}  端口:{}  CA:{}  系统代理:{}  请求:{}",
                if on { "运行" } else { "停止" }, self.app.cfg.port,
                if sysproxy::ca_trusted() { "已信任" } else { "未信任" },
                if self.app.sysproxy { "开" } else { "关" }, self.app.count())),
            Line::from(format!(" 坐标:{:.6},{:.6} 精度:{:.0}m  范围:{}",
                self.app.cfg.lat, self.app.cfg.lon, self.app.cfg.acc,
                if self.app.cfg.all_hosts { "*".to_string() } else { self.app.cfg.domains.join(",") })),
        ];
        f.render_widget(Paragraph::new(header), chunks[0]);
        let inner_h = chunks[1].height.saturating_sub(2) as usize;
        let logs = self.app.shared.lock().unwrap();
        let skip = logs.len().saturating_sub(inner_h);
        let lines: Vec<Line> = logs.iter().skip(skip).map(|l| Line::from(l.clone())).collect();
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 日志 ")), chunks[1]);
        let hint = " s 启停  c 坐标  d 域名  a 全站  j JS  t 信任CA  p 系统代理  r 重启  l 清屏  q 退出 ";
        f.render_widget(Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(Color::Black).bg(Color::Gray)))), chunks[2]);
    }

    fn draw_coord_menu(&self, f: &mut Frame, sel: usize) {
        let area = centered_rect(46, 32, f.area());
        f.render_widget(Clear, area);
        let items = ["1. 自定义坐标", "2. 选择高校"];
        let lines: Vec<Line> = items.iter().enumerate().map(|(i, t)| {
            if i == sel { Line::from(Span::styled(format!(" {} ", t), Style::default().bg(Color::Blue).fg(Color::White))) }
            else { Line::from(format!(" {} ", t)) }
        }).collect();
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" 选择坐标方式 ")), area);
        let h = f.area().height.saturating_sub(1);
        f.render_widget(Paragraph::new(Line::from(Span::styled(" ↑↓ 选择   Enter 确认   Esc 返回 ", Style::default().fg(Color::Black).bg(Color::Gray)))), Rect { x: 0, y: h, width: f.area().width, height: 1 });
    }

    fn draw_campus(&self, f: &mut Frame, query: &str, sel: usize, offset: usize) {
        let area = f.area();
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(3), Constraint::Length(2)]).split(area);
        let head = format!(" 筛选: {}    匹配 {} 条", query, self.campus_list.len());
        f.render_widget(Paragraph::new(Line::from(head)).block(Block::default().borders(Borders::ALL).title(" 选择高校/校区 ")), chunks[0]);
        let inner_h = chunks[1].height.saturating_sub(2) as usize;
        let mut off = offset;
        if sel < off { off = sel; }
        if sel >= off + inner_h.max(1) { off = sel + 1 - inner_h.max(1); }
        let mut lines: Vec<Line> = Vec::new();
        for row in 0..inner_h {
            let idx = off + row;
            if idx >= self.campus_list.len() { break; }
            let c = &self.app.campuses[self.campus_list[idx]];
            let text = format!("{:<28} {:<10} {:.6},{:.6}", trunc(&c.name, 28), trunc(&c.city, 10), c.lat, c.lon);
            if idx == sel {
                lines.push(Line::from(Span::styled(text, Style::default().bg(Color::Blue).fg(Color::White))));
            } else {
                lines.push(Line::from(text));
            }
        }
        let title = format!(" {}/{} ", if self.campus_list.is_empty() { 0 } else { sel + 1 }, self.campus_list.len());
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)), chunks[1]);
        let hint = " 输入前缀筛选  ↑↓/jk 选择  PgUp/PgDn 翻页  Enter 确认  Esc 返回 ";
        f.render_widget(Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(Color::Black).bg(Color::Gray)))), chunks[2]);
    }

    fn draw_input(&self, f: &mut Frame, label: &str, buf: &str) {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area);
        let text = format!(" {}: {}_", label, buf);
        f.render_widget(Paragraph::new(Line::from(text)).block(Block::default().borders(Borders::ALL).title(" 输入 ")), area);
        let h = f.area().height.saturating_sub(1);
        f.render_widget(Paragraph::new(Line::from(Span::styled(" Enter 确认   Esc 取消   Backspace 删除 ", Style::default().fg(Color::Black).bg(Color::Gray)))), Rect { x: 0, y: h, width: f.area().width, height: 1 });
    }
}

fn coord_label(field: u8) -> &'static str {
    match field { 0 => "纬度", 1 => "经度", _ => "精度(m)" }
}

fn trunc(s: &str, w: usize) -> String {
    let n: usize = s.chars().map(|c| if (c as u32) > 0x2E80 { 2 } else { 1 }).sum();
    if n <= w { return s.to_string(); }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = if (c as u32) > 0x2E80 { 2 } else { 1 };
        if used + cw > w { break; }
        used += cw;
        out.push(c);
    }
    out
}

fn centered_rect(px: u16, py: u16, r: Rect) -> Rect {
    let vert = Layout::vertical([Constraint::Percentage((100 - py) / 2), Constraint::Percentage(py), Constraint::Percentage((100 - py) / 2)]).split(r);
    let hori = Layout::horizontal([Constraint::Percentage((100 - px) / 2), Constraint::Percentage(px), Constraint::Percentage((100 - px) / 2)]).split(vert[1]);
    hori[1]
}

fn tui() {
    let mut ui = Ui::new();
    if let Err(e) = ui.run() {
        eprintln!("TUI 错误: {}", e);
    }
}
