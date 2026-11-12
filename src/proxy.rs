use anyhow::{anyhow, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ServerConfig, ServerConnection, StreamOwned};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type LogFn = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _m: &[u8],
        _c: &CertificateDer<'_>,
        _d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

fn bind_dual(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let addr6: std::net::SocketAddr = format!("[::]:{}", port).parse().unwrap();
    if let Ok(sock) = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)) {
        let _ = sock.set_only_v6(false);
        let _ = sock.set_reuse_address(true);
        if sock.bind(&addr6.into()).is_ok() && sock.listen(256).is_ok() {
            return Ok(sock.into());
        }
    }
    let addr4: std::net::SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    let _ = sock.set_reuse_address(true);
    sock.bind(&addr4.into())?;
    sock.listen(256)?;
    Ok(sock.into())
}

pub struct ProxyServer {
    pub stop: Arc<AtomicBool>,
    pub count: Arc<AtomicU64>,
}

impl ProxyServer {
    pub fn start(cfg: Arc<crate::config::Config>, log: LogFn) -> Result<Self> {
        let ca = Arc::new(crate::ca::Ca::ensure()?);
        let port = cfg.port;
        let listener = bind_dual(port)
            .map_err(|e| anyhow!("绑定端口 {} 失败: {}", port, e))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicU64::new(0));
        let certs: Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>> = Arc::new(Mutex::new(HashMap::new()));

        let client_cfg = Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth(),
        );

        let st = stop.clone();
        let cnt = count.clone();
        let cf = cfg.clone();
        let lg = log.clone();
        std::thread::spawn(move || {
            lg(format!("代理已启动 端口 {} 范围 {}", port,
                if cf.all_hosts { "*".to_string() } else { cf.domains.join(",") }));
            while !st.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((sock, _)) => {
                        let cf2 = cf.clone();
                        let ca2 = ca.clone();
                        let certs2 = certs.clone();
                        let cnt2 = cnt.clone();
                        let cc2 = client_cfg.clone();
                        let lg2 = lg.clone();
                        std::thread::spawn(move || {
                            let _ = handle_conn(sock, cf2, ca2, certs2, cc2, cnt2, lg2);
                        });
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
            lg("代理已停止".to_string());
        });

        Ok(ProxyServer { stop, count })
    }

    pub fn running(&self) -> bool { !self.stop.load(Ordering::Relaxed) }
    pub fn stop(&self) { self.stop.store(true, Ordering::Relaxed); }
}

fn logf(log: &LogFn, msg: String) {
    log(msg);
}

fn read_head<R: Read>(r: &mut R) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if let Some(pos) = find_sub(&buf, b"\r\n\r\n") {
            let head = buf[..pos].to_vec();
            let rest = buf[pos + 4..].to_vec();
            return Ok(Some((head, rest)));
        }
        let n = r.read(&mut tmp)?;
        if n == 0 {
            if buf.is_empty() { return Ok(None); }
            return Ok(Some((buf, Vec::new())));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 2_000_000 { break; }
    }
    Ok(Some((buf, Vec::new())))
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

fn parse_head(head: &[u8]) -> Option<Request> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = Vec::new();
    for ln in lines {
        if ln.is_empty() { continue; }
        if let Some((k, v)) = ln.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Some(Request { method, path, headers })
}

fn header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

fn read_body<R: Read>(r: &mut R, rest: Vec<u8>, len: usize) -> io::Result<Vec<u8>> {
    let mut body = rest;
    let mut tmp = [0u8; 8192];
    while body.len() < len {
        let n = r.read(&mut tmp)?;
        if n == 0 { break; }
        body.extend_from_slice(&tmp[..n]);
    }
    Ok(body)
}

fn handle_conn(
    mut client: TcpStream,
    cfg: Arc<crate::config::Config>,
    ca: Arc<crate::ca::Ca>,
    certs: Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>>,
    client_cfg: Arc<ClientConfig>,
    count: Arc<AtomicU64>,
    log: LogFn,
) -> Result<()> {
    let _ = client.set_nonblocking(false);
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    let (head, rest) = match read_head(&mut client)? {
        Some(x) => x,
        None => return Ok(()),
    };
    let req = match parse_head(&head) {
        Some(r) => r,
        None => return Ok(()),
    };
    if req.method.eq_ignore_ascii_case("CONNECT") {
        let hp = req.path.clone();
        let (host, port) = split_hostport(&hp);
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        client.flush()?;
        if port == 443 && cfg.in_scope(&host) {
            mitm(client, &host, &cfg, &ca, &certs, &client_cfg, &count, &log)?;
        } else {
            tunnel(client, &host, port, rest)?;
        }
    } else {
        // 绝对形式 http 请求（少见）
        if let Some(rest2) = req.path.strip_prefix("http://") {
            let (host, _) = split_hostport(rest2.split('/').next().unwrap_or(rest2));
            if cfg.in_scope(&host) {
                let _ = rest2;
            }
        }
    }
    Ok(())
}

fn split_hostport(s: &str) -> (String, u16) {
    match s.rsplit_once(':') {
        Some((h, p)) => (h.trim_matches(['[', ']']).to_string(), p.parse().unwrap_or(443)),
        None => (s.to_string(), 443),
    }
}

fn tunnel(mut client: TcpStream, host: &str, port: u16, rest: Vec<u8>) -> Result<()> {
    let mut up = TcpStream::connect((host, port))?;
    up.set_read_timeout(Some(Duration::from_secs(60)))?;
    if !rest.is_empty() { up.write_all(&rest)?; }
    let mut c1 = client.try_clone()?;
    let mut u1 = up.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut c1, &mut u1);
        let _ = u1.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut up, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

fn get_leaf(ca: &crate::ca::Ca, host: &str, certs: &Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>>) -> Result<(Vec<u8>, Vec<u8>)> {
    {
        let m = certs.lock().unwrap();
        if let Some(v) = m.get(host) {
            return Ok(v.clone());
        }
    }
    let (c, k) = ca.leaf(host)?;
    let mut m = certs.lock().unwrap();
    m.insert(host.to_string(), (c.clone(), k.clone()));
    Ok((c, k))
}

fn mitm(
    client: TcpStream,
    host: &str,
    cfg: &Arc<crate::config::Config>,
    ca: &Arc<crate::ca::Ca>,
    certs: &Arc<Mutex<HashMap<String, (Vec<u8>, Vec<u8>)>>>,
    client_cfg: &Arc<ClientConfig>,
    count: &Arc<AtomicU64>,
    log: &LogFn,
) -> Result<()> {
    let (cert_der, key_der) = match get_leaf(ca, host, certs) {
        Ok(v) => v,
        Err(e) => { logf(log, format!("证书失败 {} {}", host, e)); return Ok(()); }
    };
    let certs_v = vec![CertificateDer::from(cert_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
    let sc = ServerConfig::builder().with_no_client_auth().with_single_cert(certs_v, key)?;
    let conn = ServerConnection::new(Arc::new(sc))?;
    let mut tls = StreamOwned::new(conn, client);
    logf(log, format!("MITM 开始 {}", host));

    loop {
        let (head, rest) = match read_head(&mut tls) {
            Ok(Some(x)) => x,
            Ok(None) => { logf(log, format!("MITM {} 读请求结束", host)); break; }
            Err(e) => { logf(log, format!("MITM {} 读请求错误 {}", host, e)); break; }
        };
        let req = match parse_head(&head) {
            Some(r) => r,
            None => { logf(log, format!("MITM {} 请求解析失败", host)); break; }
        };
        logf(log, format!("MITM 请求 {} {} {}", host, req.method, req.path));
        let clen: usize = header(&req, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let body = read_body(&mut tls, rest, clen)?;

        let mut out = format!("{} {} HTTP/1.1\r\n", req.method, req.path);
        for (k, v) in &req.headers {
            let lk = k.to_lowercase();
            if lk == "accept-encoding" || lk == "proxy-connection" || lk == "connection" || lk == "keep-alive" {
                continue;
            }
            out.push_str(&format!("{}: {}\r\n", k, v));
        }
        out.push_str("Connection: close\r\n\r\n");
        let mut reqbytes = out.into_bytes();
        reqbytes.extend_from_slice(&body);

        let tcp = match TcpStream::connect((host, 443)) {
            Ok(t) => t,
            Err(e) => { logf(log, format!("上游失败 {} {}", host, e)); break; }
        };
        tcp.set_read_timeout(Some(Duration::from_secs(20)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(20)))?;
        let name = match ServerName::try_from(host.to_string()) {
            Ok(n) => n,
            Err(_) => break,
        };
        let cconn = ClientConnection::new(client_cfg.clone(), name)?;
        let mut up = StreamOwned::new(cconn, tcp);
        if up.write_all(&reqbytes).is_err() { break; }
        let _ = up.flush();
        let mut data = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            match up.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        if data.is_empty() { logf(log, format!("上游空响应 {}", host)); break; }
        count.fetch_add(1, Ordering::Relaxed);
        let resp = maybe_inject(&data, cfg, log, host, &req.path);
        if tls.write_all(&resp).is_err() { break; }
        let _ = tls.flush();
    }
    Ok(())
}

fn dechunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let pos = find_sub(body, b"\r\n")?;
        let line = std::str::from_utf8(&body[..pos]).ok()?;
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        body = &body[pos + 2..];
        if size == 0 { break; }
        if body.len() < size + 2 { return None; }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
    Some(out)
}

fn maybe_inject(data: &[u8], cfg: &crate::config::Config, log: &LogFn, host: &str, path: &str) -> Vec<u8> {
    let pos = match find_sub(data, b"\r\n\r\n") {
        Some(p) => p,
        None => return data.to_vec(),
    };
    let head = String::from_utf8_lossy(&data[..pos]).to_string();
    let raw_body = &data[pos + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("HTTP/1.1 200 OK").to_string();
    let mut hdrs: Vec<(String, String)> = Vec::new();
    for ln in lines {
        if let Some((k, v)) = ln.split_once(':') {
            hdrs.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let get = |n: &str| -> Option<String> {
        hdrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone())
    };
    let ctype = get("content-type").unwrap_or_default().to_lowercase();
    let is_chunked = get("transfer-encoding").map(|v| v.to_lowercase().contains("chunked")).unwrap_or(false);
    let encoding = get("content-encoding").unwrap_or_default().to_lowercase();
    let mut body = if is_chunked {
        match dechunk(raw_body) { Some(b) => b, None => raw_body.to_vec() }
    } else {
        raw_body.to_vec()
    };
    let mut injected = false;
    if ctype.contains("text/html") && encoding.is_empty() {
        let text = String::from_utf8_lossy(&body).to_string();
        let inj = crate::inject::build(cfg);
        let lower = text.to_lowercase();
        let newtext = if let Some(i) = lower.find("<head") {
            if let Some(j) = text[i..].find('>') {
                let j = i + j + 1;
                format!("{}{}{}", &text[..j], inj, &text[j..])
            } else {
                format!("{}{}", inj, text)
            }
        } else if let Some(i) = lower.find("<html") {
            if let Some(j) = text[i..].find('>') {
                let j = i + j + 1;
                format!("{}{}{}", &text[..j], inj, &text[j..])
            } else {
                format!("{}{}", inj, text)
            }
        } else {
            format!("{}{}", inj, text)
        };
        body = newtext.into_bytes();
        injected = true;
    }
    let scode = status.split_whitespace().nth(1).unwrap_or("?").to_string();
    logf(log, format!("{} {} {} {} len {}", if injected { "注入" } else { "透传" }, host, scode, &path[..path.len().min(45)], body.len()));

    let mut out = format!("{}\r\n", status);
    for (k, v) in &hdrs {
        let lk = k.to_lowercase();
        if lk == "content-length" || lk == "transfer-encoding" || lk == "connection" || lk == "content-encoding" {
            continue;
        }
        out.push_str(&format!("{}: {}\r\n", k, v));
    }
    out.push_str(&format!("Content-Length: {}\r\nConnection: keep-alive\r\n\r\n", body.len()));
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&body);
    bytes
}
