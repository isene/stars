//! One-time data build: the HYG catalog for the star list, Wikidata for
//! measured temperatures / luminosities / radii / masses, and Wikipedia
//! for the article text. Runs on first start, `--fetch`, or the `u` key;
//! the UI loop never touches the network.

use crate::data::{self, Src, Star};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

const HYG_URL: &str =
    "https://raw.githubusercontent.com/astronexus/HYG-Database/main/hyg/CURRENT/hygdata_v41.csv";
const SPARQL_URL: &str = "https://query.wikidata.org/sparql";
const UA: &str = "stars/0.1 (https://github.com/isene/stars)";

/// HYG marks "no usable parallax" with this distance; those rows carry
/// meaningless absolute magnitudes and must not be plotted.
const NO_PARALLAX: f64 = 99999.0;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(180))
        .user_agent(UA)
        .build()
}

/// Split one CSV line, honoring double-quoted fields.
fn csv_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub fn fetch_all() -> Result<Vec<Star>, String> {
    println!("Fetching the HYG star catalog (33 MB) …");
    // Streamed line by line: the whole file is far past ureq's
    // into_string limit, and only the named rows are of any use.
    let reader = std::io::BufReader::new(
        agent()
            .get(HYG_URL)
            .call()
            .map_err(|e| format!("hyg: {e}"))?
            .into_reader(),
    );
    let mut lines = std::io::BufRead::lines(reader).map_while(Result::ok);
    let header_line = lines.next().ok_or("hyg: empty")?;
    let header: Vec<String> = csv_split(&header_line)
        .into_iter()
        .map(|h| h.trim_matches('"').to_string())
        .collect();
    let col = |name: &str| -> Option<usize> { header.iter().position(|h| h == name) };
    let (c_proper, c_dist, c_mag, c_absmag, c_spect, c_ci) = (
        col("proper").ok_or("hyg: no proper column")?,
        col("dist").ok_or("hyg: no dist column")?,
        col("mag").ok_or("hyg: no mag column")?,
        col("absmag").ok_or("hyg: no absmag column")?,
        col("spect").ok_or("hyg: no spect column")?,
        col("ci").ok_or("hyg: no ci column")?,
    );
    let (c_bf, c_con, c_hip, c_hd) = (col("bf"), col("con"), col("hip"), col("hd"));

    let mut stars: Vec<Star> = Vec::new();
    for line in lines {
        let f = csv_split(&line);
        if f.len() < header.len() {
            continue;
        }
        let name = f[c_proper].trim().to_string();
        if name.is_empty() {
            continue;
        }
        let dist: f64 = f[c_dist].parse().unwrap_or(NO_PARALLAX);
        if dist >= NO_PARALLAX {
            continue; // no parallax: absmag would be fiction
        }
        let get = |i: Option<usize>| -> String {
            i.and_then(|i| f.get(i)).map(|s| s.trim().to_string()).unwrap_or_default()
        };
        let spectral = f[c_spect].trim().to_string();
        stars.push(Star {
            lum_class: data::luminosity_class(&spectral),
            name,
            designation: get(c_bf),
            constellation: get(c_con),
            hip: get(c_hip),
            hd: get(c_hd),
            spectral,
            dist_pc: dist,
            mag: f[c_mag].parse().unwrap_or(0.0),
            absmag: f[c_absmag].parse().unwrap_or(0.0),
            color_index: f[c_ci].parse().ok(),
            teff: 0.0,
            teff_src: Src::Unknown,
            lum: 0.0,
            lum_src: Src::Unknown,
            radius: None,
            mass: None,
            article: String::new(),
            source: String::new(),
        });
    }
    stars.sort_by(|a, b| a.mag.partial_cmp(&b.mag).unwrap_or(std::cmp::Ordering::Equal));
    println!("  {} named stars with a usable parallax", stars.len());

    println!("Fetching measured values from Wikidata …");
    let measured = match fetch_wikidata(&stars) {
        Ok(m) => {
            println!("  {} stars with a published effective temperature", m.len());
            m
        }
        Err(e) => {
            eprintln!("  (wikidata unavailable: {e})");
            HashMap::new()
        }
    };

    for s in stars.iter_mut() {
        let m = measured.get(&s.name);
        s.radius = m.and_then(|m| m.radius);
        s.mass = m.and_then(|m| m.mass);

        // Temperature: published value, else the spectral type, else color.
        match m.and_then(|m| m.teff) {
            Some(t) => {
                s.teff = t;
                s.teff_src = Src::Measured;
            }
            None => match data::teff_from_spectral(&s.spectral) {
                Some(t) => {
                    s.teff = t;
                    s.teff_src = Src::Spectral;
                }
                None => {
                    if let Some(ci) = s.color_index {
                        s.teff = data::teff_from_color(ci);
                        s.teff_src = Src::Color;
                    }
                }
            },
        }

        // Luminosity: published, else radius + Teff, else absolute magnitude.
        match m.and_then(|m| m.lum) {
            Some(l) => {
                s.lum = l;
                s.lum_src = Src::Measured;
            }
            None => match (s.radius, s.teff > 0.0) {
                (Some(r), true) => {
                    s.lum = data::lum_from_radius(r, s.teff);
                    s.lum_src = Src::Radius;
                }
                _ if s.teff > 0.0 => {
                    s.lum = data::lum_from_absmag(s.absmag, s.teff);
                    s.lum_src = Src::Magnitude;
                }
                _ => {}
            },
        }
    }
    // The Sun is the calibration point, not something to interpolate.
    if let Some(sun) = stars.iter_mut().find(|s| s.name == "Sol") {
        sun.name = "Sun".to_string();
        sun.teff = data::SUN_TEFF;
        sun.teff_src = Src::Measured;
        sun.lum = 1.0;
        sun.lum_src = Src::Measured;
        sun.radius = Some(1.0);
        sun.mass = Some(1.0);
    }
    stars.retain(|s| s.teff > 0.0 && s.lum > 0.0);

    let total = stars.len();
    println!("Fetching the Wikipedia article for all {total} stars …");
    let stars = Arc::new(Mutex::new(stars));
    let next = Arc::new(Mutex::new(0usize));
    let done = Arc::new(Mutex::new(0usize));
    let failed = Arc::new(Mutex::new(0usize));
    let mut workers = Vec::new();
    for _ in 0..3 {
        let stars = Arc::clone(&stars);
        let next = Arc::clone(&next);
        let done = Arc::clone(&done);
        let failed = Arc::clone(&failed);
        workers.push(std::thread::spawn(move || {
            let agent = agent();
            loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                if i >= total {
                    break;
                }
                let name = {
                    let s = stars.lock().unwrap();
                    s[i].name.clone()
                };
                let fetched = fetch_article(&agent, &name);
                {
                    let mut s = stars.lock().unwrap();
                    match fetched {
                        Ok((title, text)) => {
                            s[i].source = format!(
                                "https://en.wikipedia.org/wiki/{}",
                                title.replace(' ', "_")
                            );
                            s[i].article = text;
                        }
                        Err(_) => *failed.lock().unwrap() += 1,
                    }
                }
                // Stay under Wikipedia's burst threshold.
                std::thread::sleep(std::time::Duration::from_millis(60));
                let mut d = done.lock().unwrap();
                *d += 1;
                print!("\r  [{:3}/{}] {:<22}", *d, total, name);
                std::io::stdout().flush().ok();
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    let missed = *failed.lock().unwrap();
    println!("\r  {} articles fetched, {missed} without one{:20}", total - missed, "");
    let stars = Arc::try_unwrap(stars)
        .map_err(|_| "worker thread leaked")?
        .into_inner()
        .unwrap();
    Ok(stars)
}

#[derive(Default, Clone)]
struct Measured {
    teff: Option<f64>,
    lum: Option<f64>,
    radius: Option<f64>,
    mass: Option<f64>,
}

/// One SPARQL POST asking for every star name at once. GET would blow the
/// URL length limit; POST keeps it to a single round trip.
fn fetch_wikidata(stars: &[Star]) -> Result<HashMap<String, Measured>, String> {
    let mut values = String::new();
    for s in stars {
        if !s.name.contains('"') && !s.name.contains('\\') {
            values.push_str(&format!("\"{}\"@en ", s.name));
        }
    }
    let query = format!(
        "SELECT ?name ?temp ?lum ?radius ?mass WHERE {{ VALUES ?name {{ {values} }} \
         ?s rdfs:label ?name ; wdt:P6879 ?temp . \
         OPTIONAL {{ ?s wdt:P2060 ?lum }} OPTIONAL {{ ?s wdt:P2120 ?radius }} \
         OPTIONAL {{ ?s wdt:P2067 ?mass }} }}"
    );
    let json: serde_json::Value = agent()
        .post(SPARQL_URL)
        .set("Accept", "application/sparql-results+json")
        .send_form(&[("query", &query)])
        .map_err(|e| format!("{e}"))?
        .into_json()
        .map_err(|e| format!("parse: {e}"))?;

    let mut out: HashMap<String, Measured> = HashMap::new();
    let rows = json["results"]["bindings"]
        .as_array()
        .ok_or("no bindings")?;
    for r in rows {
        let name = match r["name"]["value"].as_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let num = |k: &str| -> Option<f64> { r[k]["value"].as_str().and_then(|v| v.parse().ok()) };
        let e = out.entry(name).or_default();
        // Several sources per star: first one wins, but fill any gaps.
        if e.teff.is_none() {
            e.teff = num("temp");
        }
        if e.lum.is_none() {
            e.lum = num("lum");
        }
        if e.radius.is_none() {
            e.radius = num("radius");
        }
        if e.mass.is_none() {
            e.mass = num("mass");
        }
    }
    Ok(out)
}

/// Full plain-text article via the Wikipedia TextExtracts API.
fn fetch_article(agent: &ureq::Agent, name: &str) -> Result<(String, String), String> {
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts&explaintext=1&redirects=1&format=json&formatversion=2&titles={}",
        urlencode(name)
    );
    let mut last = String::new();
    // Wikipedia throttles bursts (429/503). Back off and try again rather
    // than dropping the article: a silent miss looks like "no article".
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400 << attempt));
        }
        match agent.get(&url).call() {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(json) => {
                    let page = &json["query"]["pages"][0];
                    let title = page["title"].as_str().unwrap_or(name).to_string();
                    let text = page["extract"].as_str().unwrap_or("").trim().to_string();
                    if text.is_empty() {
                        return Err("empty extract".into());
                    }
                    return Ok((title, text));
                }
                Err(e) => last = e.to_string(),
            },
            Err(ureq::Error::Status(code, _)) if code == 429 || code >= 500 => {
                last = format!("http {code}");
            }
            Err(e) => {
                last = e.to_string();
                break; // 404 and friends will not improve with waiting
            }
        }
    }
    Err(last)
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
