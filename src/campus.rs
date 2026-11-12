use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Campus {
    pub name: String,
    #[serde(default)]
    pub city: String,
    pub lat: f64,
    pub lon: f64,
}

const BUNDLED: &str = include_str!("../assets/campuses.json");

pub fn user_path() -> PathBuf { crate::config::app_dir().join("campuses.json") }

pub fn load() -> Vec<Campus> {
    let mut v: Vec<Campus> = serde_json::from_str(BUNDLED).unwrap_or_default();
    if let Ok(s) = std::fs::read_to_string(user_path()) {
        if let Ok(mut u) = serde_json::from_str::<Vec<Campus>>(&s) {
            v.append(&mut u);
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|c| seen.insert((c.name.clone(), (c.lat * 1e4).round() as i64, (c.lon * 1e4).round() as i64)));
    v
}

pub fn search<'a>(list: &'a [Campus], kw: &str, limit: usize) -> Vec<&'a Campus> {
    let k = kw.trim().to_lowercase();
    let mut out: Vec<&Campus> = Vec::new();
    for c in list {
        let hit = if k.is_empty() {
            true
        } else {
            c.name.to_lowercase().contains(&k) || c.city.to_lowercase().contains(&k)
        };
        if hit {
            out.push(c);
            if out.len() >= limit { break; }
        }
    }
    out
}

const RQ_UNI: &str = r#"SELECT ?item ?itemLabel ?coord ?adminLabel WHERE {
  ?item wdt:P31/wdt:P279* wd:Q3918 . ?item wdt:P17 wd:Q148 . ?item wdt:P625 ?coord .
  OPTIONAL { ?item wdt:P131 ?admin . }
  SERVICE wikibase:label { bd:serviceParam wikibase:language "zh-hans,zh,en". }
}"#;

const RQ_CAMPUS: &str = r#"SELECT ?item ?name ?coord ?uniName ?cityName WHERE {
  ?item wdt:P31/wdt:P279* wd:Q209465 . ?item wdt:P625 ?coord .
  { ?item wdt:P17 wd:Q148 } UNION { ?item wdt:P361 ?u0 . ?u0 wdt:P17 wd:Q148 }
  ?item rdfs:label ?name . FILTER(lang(?name)="zh-hans"||lang(?name)="zh")
  OPTIONAL { ?item wdt:P361 ?u1 . ?u1 rdfs:label ?uniName . FILTER(lang(?uniName)="zh-hans"||lang(?uniName)="zh") }
  OPTIONAL { ?item wdt:P131 ?c1 . ?c1 rdfs:label ?cityName . FILTER(lang(?cityName)="zh-hans"||lang(?cityName)="zh") }
}"#;

fn cjk(s: &str) -> bool { s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) }

fn parse_point(s: &str) -> Option<(f64, f64)> {
    let a = s.find("Point(")? + 6;
    let b = s[a..].find(')')? + a;
    let mut it = s[a..b].split_whitespace();
    let lon: f64 = it.next()?.parse().ok()?;
    let lat: f64 = it.next()?.parse().ok()?;
    Some((lon, lat))
}

fn val<'a>(row: &'a serde_json::Value, k: &str) -> String {
    row.get(k).and_then(|x| x.get("value")).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn fetch(query: &str, tag: &str) -> Result<Vec<Campus>> {
    let dir = std::env::temp_dir();
    let qf = dir.join(format!("wxgeo_{}.rq", tag));
    std::fs::write(&qf, query)?;
    let ua = "wxgeo/0.1 (+https://github.com/Temsys-Shen/wxgeo; contact: shenshichao920@icloud.com)";
    let mut text = String::new();
    let mut last_err = String::new();
    for attempt in 0..3 {
        let out = std::process::Command::new("curl")
            .args(["-s", "-m", "150", "-H", &format!("User-Agent: {}", ua),
                   "-H", "Accept: application/sparql-results+json",
                   "--data-urlencode", &format!("query@{}", qf.to_string_lossy()),
                   "https://query.wikidata.org/sparql"])
            .output()
            .map_err(|e| anyhow!("调用 curl 失败: {e}"))?;
        text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.trim_start().starts_with('{') { break; }
        last_err = text.chars().take(160).collect();
        std::thread::sleep(std::time::Duration::from_secs(8 * (attempt + 1)));
    }
    let j: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow!("查询失败（可能被限流，稍后重试）: {} / {}", e, last_err))?;
    let mut rows = Vec::new();
    if let Some(arr) = j.get("results").and_then(|r| r.get("bindings")).and_then(|b| b.as_array()) {
        for r in arr {
            let mut name = val(r, "name");
            if name.is_empty() { name = val(r, "itemLabel"); }
            let coord = val(r, "coord");
            if let Some((lon, lat)) = parse_point(&coord) {
                if !(73.0 < lon && lon < 136.0 && 17.0 < lat && lat < 54.0) { continue; }
                if !cjk(&name) { continue; }
                let city = {
                    let c = val(r, "cityName");
                    if c.is_empty() { val(r, "adminLabel") } else { c }
                };
                rows.push(Campus { name: name.trim().to_string(), city: city.trim().to_string(), lat: (lat * 1e6).round() / 1e6, lon: (lon * 1e6).round() / 1e6 });
            }
        }
    }
    Ok(rows)
}

pub fn update_db() -> Result<usize> {
    let mut all = fetch(RQ_UNI, "uni")?;
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut camps = fetch(RQ_CAMPUS, "campus")?;
    all.append(&mut camps);
    // 合并已有用户数据
    let mut existing: Vec<Campus> = std::fs::read_to_string(user_path())
        .ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    all.append(&mut existing);
    // 去重
    let mut seen = std::collections::HashSet::new();
    all.retain(|c| {
        let k = (c.name.clone(), (c.lat * 1e4).round() as i64, (c.lon * 1e4).round() as i64);
        seen.insert(k)
    });
    all.sort_by(|a, b| a.name.cmp(&b.name).then(a.lat.partial_cmp(&b.lat).unwrap_or(std::cmp::Ordering::Equal)));
    let _ = std::fs::create_dir_all(crate::config::app_dir());
    std::fs::write(user_path(), serde_json::to_string(&all)?)?;
    crate::config::fix_owner(&user_path());
    Ok(all.len())
}
