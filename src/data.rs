//! Star data model, the on-disk cache (~/.stars/stars.json), and the
//! astrophysics needed to place a star on the Hertzsprung-Russell diagram.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a derived quantity came from. Shown in the detail panel and
/// available as a color mode, so a measured value is never mistaken for
/// one this program worked out from a spectral type.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Src {
    /// Published value (Wikidata).
    Measured,
    /// From the spectral type, via the main-sequence temperature scale.
    Spectral,
    /// From the B-V color index (Ballesteros' formula).
    Color,
    /// From radius and temperature (Stefan-Boltzmann).
    Radius,
    /// From absolute magnitude plus a bolometric correction.
    Magnitude,
    Unknown,
}

impl Src {
    pub fn label(&self) -> &'static str {
        match self {
            Src::Measured => "measured",
            Src::Spectral => "from spectral type",
            Src::Color => "from color index",
            Src::Radius => "from radius + Teff",
            Src::Magnitude => "from absolute magnitude",
            Src::Unknown => "unknown",
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Star {
    pub name: String,
    /// Bayer / Flamsteed designation, e.g. "Alp CMa".
    #[serde(default)]
    pub designation: String,
    /// Three-letter constellation abbreviation.
    #[serde(default)]
    pub constellation: String,
    #[serde(default)]
    pub hip: String,
    #[serde(default)]
    pub hd: String,
    #[serde(default)]
    pub spectral: String,
    /// Luminosity class parsed out of the spectral type ("V", "III", "Ia").
    #[serde(default)]
    pub lum_class: String,
    pub dist_pc: f64,
    /// Apparent visual magnitude.
    pub mag: f64,
    pub absmag: f64,
    pub color_index: Option<f64>,
    pub teff: f64,
    pub teff_src: Src,
    /// Bolometric luminosity in solar units.
    pub lum: f64,
    pub lum_src: Src,
    pub radius: Option<f64>,
    pub mass: Option<f64>,
    #[serde(default)]
    pub article: String,
    #[serde(default)]
    pub source: String,
}

impl Star {
    pub fn dist_ly(&self) -> f64 {
        self.dist_pc * 3.261_563_8
    }
    /// Spectral class letter (O B A F G K M), or '?' if unparseable.
    pub fn class(&self) -> char {
        self.spectral
            .chars()
            .find(|c| "OBAFGKMWLTYSCR".contains(*c))
            .unwrap_or('?')
    }
}

pub fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".stars").join("stars.json")
}

pub fn load() -> Option<Vec<Star>> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    let stars: Vec<Star> = serde_json::from_str(&raw).ok()?;
    if stars.is_empty() {
        None
    } else {
        Some(stars)
    }
}

pub fn save(stars: &[Star]) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Atomic write: a killed fetch must never truncate a good cache.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(stars)?)?;
    std::fs::rename(tmp, path)
}

// ───────────────────────── astrophysics ──────────────────────────────

pub const SUN_TEFF: f64 = 5772.0;

/// Main-sequence effective temperature scale, anchored on the standard
/// spectral-type table. Values between anchors are interpolated in
/// log T, which follows the real scale closely.
const TEFF_ANCHORS: [(f64, f64); 19] = [
    (3.0, 44000.0),  // O3
    (5.0, 41000.0),  // O5
    (9.0, 33000.0),  // O9
    (10.0, 31000.0), // B0
    (12.0, 20600.0), // B2
    (15.0, 15200.0), // B5
    (18.0, 12300.0), // B8
    (20.0, 9700.0),  // A0
    (25.0, 8080.0),  // A5
    (30.0, 7220.0),  // F0
    (35.0, 6510.0),  // F5
    (40.0, 5940.0),  // G0
    (42.0, 5770.0),  // G2
    (45.0, 5660.0),  // G5
    (50.0, 5280.0),  // K0
    (55.0, 4410.0),  // K5
    (60.0, 3850.0),  // M0
    (65.0, 3060.0),  // M5
    (69.0, 2400.0),  // M9
];

/// Bolometric correction to visual magnitude, by effective temperature.
/// Standard textbook table, interpolated linearly.
const BC_TABLE: [(f64, f64); 21] = [
    (2500.0, -4.10),
    (3000.0, -2.73),
    (3500.0, -1.60),
    (4000.0, -1.02),
    (4500.0, -0.60),
    (5000.0, -0.31),
    (5500.0, -0.14),
    (5772.0, -0.08),
    (6000.0, -0.04),
    (6500.0, -0.02),
    (7000.0, -0.01),
    (7500.0, -0.03),
    (8000.0, -0.10),
    (9000.0, -0.22),
    (10000.0, -0.35),
    (12000.0, -0.75),
    (15000.0, -1.30),
    (20000.0, -2.00),
    (25000.0, -2.50),
    (30000.0, -3.10),
    (40000.0, -4.00),
];

/// Position on the O..M scale: O0 = 0, B0 = 10, … M9 = 69.
fn spectral_position(spectral: &str) -> Option<f64> {
    let bytes = spectral.as_bytes();
    let idx = bytes.iter().position(|b| b"OBAFGKM".contains(b))?;
    let class = bytes[idx] as char;
    let base = "OBAFGKM".find(class)? as f64 * 10.0;
    // Optional numeric subclass, possibly fractional ("B0.5").
    let rest = &spectral[idx + 1..];
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let sub: f64 = num.parse().unwrap_or(5.0);
    Some(base + sub.clamp(0.0, 9.9))
}

pub fn teff_from_spectral(spectral: &str) -> Option<f64> {
    let pos = spectral_position(spectral)?;
    let first = TEFF_ANCHORS[0];
    let last = TEFF_ANCHORS[TEFF_ANCHORS.len() - 1];
    if pos <= first.0 {
        return Some(first.1);
    }
    if pos >= last.0 {
        return Some(last.1);
    }
    for w in TEFF_ANCHORS.windows(2) {
        let (p0, t0) = w[0];
        let (p1, t1) = w[1];
        if pos >= p0 && pos <= p1 {
            let f = if p1 > p0 { (pos - p0) / (p1 - p0) } else { 0.0 };
            return Some((t0.ln() + f * (t1.ln() - t0.ln())).exp());
        }
    }
    None
}

/// Ballesteros' formula: effective temperature from the B-V color index.
pub fn teff_from_color(ci: f64) -> f64 {
    4600.0 * (1.0 / (0.92 * ci + 1.7) + 1.0 / (0.92 * ci + 0.62))
}

pub fn bolometric_correction(teff: f64) -> f64 {
    let first = BC_TABLE[0];
    let last = BC_TABLE[BC_TABLE.len() - 1];
    if teff <= first.0 {
        return first.1;
    }
    if teff >= last.0 {
        return last.1;
    }
    for w in BC_TABLE.windows(2) {
        let (t0, b0) = w[0];
        let (t1, b1) = w[1];
        if teff >= t0 && teff <= t1 {
            return b0 + (teff - t0) / (t1 - t0) * (b1 - b0);
        }
    }
    0.0
}

/// Bolometric luminosity in solar units from absolute visual magnitude.
pub fn lum_from_absmag(absmag: f64, teff: f64) -> f64 {
    let mbol = absmag + bolometric_correction(teff);
    10f64.powf((4.74 - mbol) / 2.5)
}

/// Stefan-Boltzmann: L/L☉ = (R/R☉)² (T/T☉)⁴.
pub fn lum_from_radius(radius: f64, teff: f64) -> f64 {
    radius * radius * (teff / SUN_TEFF).powi(4)
}

/// Luminosity class out of a spectral type string ("B8Ia" → "Ia").
pub fn luminosity_class(spectral: &str) -> String {
    // Longest first so "III" wins over "II" and "I".
    for pat in ["VIII", "VII", "VI", "III", "IV", "Iab", "Ia", "Ib", "II", "V", "I"] {
        if let Some(p) = spectral.find(pat) {
            // Must not be part of a longer roman numeral run.
            let after = spectral[p + pat.len()..].chars().next();
            if !matches!(after, Some('I') | Some('V')) {
                return pat.to_string();
            }
        }
    }
    String::new()
}

/// Coarse grouping of luminosity class for coloring and labels.
pub fn lum_class_group(lc: &str) -> usize {
    match lc {
        "Ia" | "Iab" | "Ib" | "I" => 0, // supergiant
        "II" => 1,                      // bright giant
        "III" => 2,                     // giant
        "IV" => 3,                      // subgiant
        "V" => 4,                       // main sequence
        "VI" => 5,                      // subdwarf
        "VII" | "VIII" => 6,            // white dwarf
        _ => 7,                         // unknown
    }
}
