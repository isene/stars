//! stars — Hertzsprung-Russell diagram explorer for the Fe2O3 suite.
//!
//! Plots every named star with a usable parallax on the physical HR
//! diagram (log effective temperature against log luminosity), lets you
//! walk between them, and shows each star's properties plus its full
//! Wikipedia article from a local cache (~/.stars/stars.json). Schematic
//! evolutionary tracks can be laid over the diagram to show where a star
//! of a given mass goes when it leaves the main sequence.
//!
//! The network is touched exactly once, on first start, `--fetch`, or the
//! `u` key. The UI loop blocks on input and never wakes on its own.

mod data;
mod fetch;
mod tracks;

use crust::{Crust, Input, Pane};
use data::{Src, Star};
use std::io::Write;

// Diagram geometry. The plot area is a fixed box; the article pane and
// the property block flow around it.
const PLOT_X: u16 = 8; // first column of the plot area
const PLOT_Y: u16 = 3; // first row of the plot area
const PLOT_W: u16 = 62;
const PLOT_H: u16 = 17;
const SIDE_X: u16 = PLOT_X + PLOT_W + 4; // property block column
const SIDE_MIN: u16 = SIDE_X + 44; // width needed for the side layout
const DETAIL_Y: u16 = PLOT_Y + PLOT_H + 3; // first row of the article pane

// Axis ranges, chosen to hold every star in the catalog with room for
// the tracks: 45000 K down to 2000 K, 10^-5 to 10^6 solar luminosities.
const LOG_T_HOT: f64 = 4.653; // log10(45000)
const LOG_T_COOL: f64 = 3.301; // log10(2000)
const LOG_L_MIN: f64 = -5.0;
const LOG_L_MAX: f64 = 6.2;

const RUST: &str = "\x1b[1;38;2;247;76;0m";
const RESET: &str = "\x1b[0m";

const MODE_NAMES: [&str; 7] = [
    "spectral class",
    "luminosity class",
    "distance",
    "apparent magnitude",
    "mass",
    "radius",
    "data source",
];

#[derive(PartialEq, Clone, Copy)]
enum View {
    Article,
    Help,
    Chat,
    Modes,
}

struct App {
    stars: Vec<Star>,
    sel: usize,
    mode: usize,
    /// 0 = off, else index into tracks::TRACKS + 1, last = all.
    track: usize,
    view: View,
    menu_ix: usize,
    chat: Vec<(String, String)>,
}

fn main() {
    let mut force_fetch = false;
    let mut start: Option<String> = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--fetch" => force_fetch = true,
            "-h" | "--help" => {
                println!("stars — Hertzsprung-Russell diagram explorer (Fe2O3 suite)");
                println!();
                println!("Usage: stars [STAR] [--fetch]");
                println!();
                println!("  STAR        start at a star (name, or part of one)");
                println!("  --fetch     rebuild the local catalog from HYG + Wikidata + Wikipedia");
                println!("  -v          print version");
                println!();
                println!("Data is cached at ~/.stars/stars.json; the UI works offline.");
                return;
            }
            "-v" | "--version" => {
                println!("stars {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => start = Some(other.to_string()),
        }
    }

    let stars = if force_fetch { None } else { data::load() };
    let stars = match stars {
        Some(s) => s,
        None => {
            println!("stars: building the local catalog (one-time fetch) …");
            match fetch::fetch_all() {
                Ok(s) => {
                    if let Err(e) = data::save(&s) {
                        eprintln!("stars: could not save cache: {e}");
                    }
                    s
                }
                Err(e) => {
                    eprintln!("stars: fetch failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    };

    let mut sel = stars.iter().position(|s| s.name == "Sun").unwrap_or(0);
    if let Some(q) = start {
        match find(&stars, &q) {
            Some(i) => sel = i,
            None => {
                eprintln!("stars: no star matches '{q}'");
                std::process::exit(1);
            }
        }
    }

    // Piped or scripted: print the star as plain text and exit.
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        let text = detail_text(&stars[sel], false);
        if std::io::stdout().is_terminal() {
            println!("{text}");
        } else {
            println!("{}", crust::strip_ansi(&text));
        }
        return;
    }

    let mut app = App {
        stars,
        sel,
        mode: 0,
        track: 0,
        view: View::Article,
        menu_ix: 0,
        chat: Vec::new(),
    };

    Crust::init();
    Crust::set_app_identity("Stars");
    let (mut cols, mut rows) = Crust::terminal_size();
    let mut detail = Pane::new(1, DETAIL_Y, cols, rows.saturating_sub(DETAIL_Y).max(1), 253, 0);
    let mut status = Pane::new(1, rows, cols, 1, 250, 236);
    status.scroll = false;

    draw_all(&app, &mut detail, &mut status, cols, rows);

    loop {
        let key = match Input::getchr(None) {
            Some(k) => k,
            None => continue,
        };
        match key.as_str() {
            "q" => break,
            "ESC" => {
                if app.view == View::Article {
                    break;
                }
                app.view = View::Article;
                set_detail(&app, &mut detail, cols);
            }
            "UP" | "k" | "DOWN" | "j" if app.view == View::Modes => {
                let up = key == "UP" || key == "k";
                let n = MODE_NAMES.len();
                app.menu_ix = if up { (app.menu_ix + n - 1) % n } else { (app.menu_ix + 1) % n };
                set_detail(&app, &mut detail, cols);
            }
            "ENTER" if app.view == View::Modes => {
                app.mode = app.menu_ix;
                app.view = View::Article;
                redraw_diagram(&app, cols);
                set_detail(&app, &mut detail, cols);
            }
            // Spatial movement: nearest star in that direction on the plot.
            "LEFT" | "h" => step(&mut app, (-1, 0), &mut detail, cols),
            "RIGHT" | "l" => step(&mut app, (1, 0), &mut detail, cols),
            "UP" | "k" => step(&mut app, (0, 1), &mut detail, cols),
            "DOWN" | "j" => step(&mut app, (0, -1), &mut detail, cols),
            // Brightness tour: the sky's most prominent stars in order.
            "TAB" | ">" | "n" => {
                let t = (app.sel + 1).min(app.stars.len() - 1);
                select(&mut app, t, &mut detail, cols);
            }
            "S-TAB" | "<" | "p" => {
                let t = app.sel.saturating_sub(1);
                select(&mut app, t, &mut detail, cols);
            }
            "1" | "2" | "3" | "4" | "5" | "6" | "7" => {
                app.mode = key.parse::<usize>().unwrap() - 1;
                app.view = View::Article;
                redraw_diagram(&app, cols);
                set_detail(&app, &mut detail, cols);
            }
            "C-RIGHT" => {
                app.mode = (app.mode + 1) % MODE_NAMES.len();
                redraw_diagram(&app, cols);
            }
            "C-LEFT" => {
                app.mode = (app.mode + MODE_NAMES.len() - 1) % MODE_NAMES.len();
                redraw_diagram(&app, cols);
            }
            "m" => {
                app.view = if app.view == View::Modes { View::Article } else { View::Modes };
                app.menu_ix = app.mode;
                set_detail(&app, &mut detail, cols);
            }
            "t" => {
                app.track = (app.track + 1) % (tracks::TRACKS.len() + 2);
                draw_header(&app, cols);
                redraw_diagram(&app, cols);
                if app.view == View::Article {
                    set_detail(&app, &mut detail, cols);
                }
            }
            "J" | "S-DOWN" => detail.linedown(),
            "K" | "S-UP" => detail.lineup(),
            " " | "PgDOWN" => detail.pagedown(),
            "PgUP" => detail.pageup(),
            "g" | "HOME" => detail.top(),
            "G" | "END" => detail.bottom(),
            "/" => {
                let q = status.ask_or_cancel("Find star: ", "");
                print!("\x1b[?25l");
                std::io::stdout().flush().ok();
                match q.as_deref().map(|q| find(&app.stars, q)) {
                    Some(Some(i)) => {
                        select(&mut app, i, &mut detail, cols);
                        status.say(&help_line());
                    }
                    Some(None) => status.say("\x1b[38;2;255;120;120mno match\x1b[0m"),
                    None => status.say(&help_line()),
                }
            }
            "w" => {
                let url = &app.stars[app.sel].source;
                if !url.is_empty() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(url)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
            }
            "c" => {
                let prompt = if app.chat.is_empty() {
                    format!("Ask Claude about {}: ", app.stars[app.sel].name)
                } else {
                    "Follow-up: ".to_string()
                };
                let q = status.ask_or_cancel(&prompt, "");
                print!("\x1b[?25l");
                std::io::stdout().flush().ok();
                match q {
                    Some(q) if !q.trim().is_empty() => {
                        status.say("\x1b[38;2;120;200;255m asking claude…\x1b[0m");
                        match ask_claude(&app, q.trim()) {
                            Ok(a) if !a.is_empty() => {
                                app.chat.push((q.trim().to_string(), a));
                                app.view = View::Chat;
                                set_detail(&app, &mut detail, cols);
                                status.say(&help_line());
                            }
                            Ok(_) => status.say("\x1b[38;2;255;120;120mclaude returned nothing\x1b[0m"),
                            Err(e) => status.say(&format!("\x1b[38;2;255;120;120mclaude: {e}\x1b[0m")),
                        }
                    }
                    _ => status.say(&help_line()),
                }
            }
            "C" => {
                app.view = if app.view == View::Chat { View::Article } else { View::Chat };
                set_detail(&app, &mut detail, cols);
            }
            "u" => {
                Crust::cleanup();
                println!("stars: rebuilding the catalog …");
                let result = fetch::fetch_all();
                let msg = match result {
                    Ok(s) => {
                        if let Err(e) = data::save(&s) {
                            format!("\x1b[38;2;255;120;120mcould not save cache: {e}\x1b[0m")
                        } else {
                            let cur = app.stars[app.sel].name.clone();
                            app.stars = s;
                            app.sel = app.stars.iter().position(|x| x.name == cur).unwrap_or(0);
                            "catalog updated".to_string()
                        }
                    }
                    Err(e) => format!("\x1b[38;2;255;120;120mfetch failed: {e}\x1b[0m"),
                };
                Crust::init();
                Crust::set_app_identity("Stars");
                draw_all(&app, &mut detail, &mut status, cols, rows);
                status.say(&msg);
            }
            "?" => {
                app.view = if app.view == View::Help { View::Article } else { View::Help };
                set_detail(&app, &mut detail, cols);
            }
            "RESIZE" => {
                let (c, r) = Crust::terminal_size();
                cols = c;
                rows = r;
                detail.w = cols;
                detail.h = rows.saturating_sub(DETAIL_Y).max(1);
                status.y = rows;
                status.w = cols;
                draw_all(&app, &mut detail, &mut status, cols, rows);
            }
            _ => {}
        }
    }

    Crust::cleanup();
}

// ─────────────────────────── selection ───────────────────────────────

fn find(stars: &[Star], q: &str) -> Option<usize> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    stars
        .iter()
        .position(|s| s.name.to_lowercase() == q)
        .or_else(|| stars.iter().position(|s| s.name.to_lowercase().starts_with(&q)))
        .or_else(|| stars.iter().position(|s| s.name.to_lowercase().contains(&q)))
        .or_else(|| {
            stars
                .iter()
                .position(|s| s.designation.to_lowercase().contains(&q))
        })
}

/// Plot coordinates in the diagram's own units (0..1 each way).
fn plot_pos(s: &Star) -> (f64, f64) {
    let lt = s.teff.max(1.0).log10();
    let ll = s.lum.max(1e-12).log10();
    let x = (lt - LOG_T_HOT) / (LOG_T_COOL - LOG_T_HOT);
    let y = (ll - LOG_L_MIN) / (LOG_L_MAX - LOG_L_MIN);
    (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
}

/// Move to the nearest star in the given direction on the diagram.
/// `dir` is (dx, dy) with y pointing up in luminosity.
fn step(app: &mut App, dir: (i32, i32), detail: &mut Pane, cols: u16) {
    let (cx, cy) = plot_pos(&app.stars[app.sel]);
    let mut best: Option<(f64, usize)> = None;
    for (i, s) in app.stars.iter().enumerate() {
        if i == app.sel {
            continue;
        }
        let (x, y) = plot_pos(s);
        let (dx, dy) = (x - cx, y - cy);
        // Progress along the requested axis, in diagram units.
        let along = dx * dir.0 as f64 + dy * dir.1 as f64;
        if along <= 1e-6 {
            continue;
        }
        // Prefer the closest star, penalising sideways drift so the
        // cursor tracks a line rather than wandering across the plot.
        let across = (dx * dir.1 as f64).abs() + (dy * dir.0 as f64).abs();
        let cost = along + across * 3.0;
        if best.map_or(true, |(b, _)| cost < b) {
            best = Some((cost, i));
        }
    }
    if let Some((_, i)) = best {
        select(app, i, detail, cols);
    }
}

fn select(app: &mut App, new: usize, detail: &mut Pane, cols: u16) {
    if new == app.sel && app.view == View::Article {
        return;
    }
    if new != app.sel {
        app.chat.clear();
    }
    app.sel = new;
    app.view = View::Article;
    draw_header(app, cols);
    redraw_diagram(app, cols);
    draw_side(app, cols);
    set_detail(app, detail, cols);
}

// ───────────────────────────── colors ────────────────────────────────

/// True-ish colors for the spectral classes, from blue O to red M.
fn class_rgb(c: char) -> (u8, u8, u8) {
    match c {
        // Saturated a little past the true colors: on a black terminal
        // the real A/F whites are indistinguishable from each other.
        'O' => (120, 150, 255),
        'B' => (160, 195, 255),
        'A' => (225, 235, 255),
        'F' => (255, 245, 200),
        'G' => (255, 215, 90),
        'K' => (255, 150, 60),
        'M' => (255, 90, 60),
        'C' | 'S' | 'R' => (255, 90, 60), // carbon stars
        'W' => (200, 210, 255),           // Wolf-Rayet
        'L' | 'T' | 'Y' => (170, 80, 90), // brown dwarfs
        _ => (150, 150, 150),
    }
}

const CLASS_LEGEND: [(&str, char); 7] = [
    ("O", 'O'), ("B", 'B'), ("A", 'A'), ("F", 'F'), ("G", 'G'), ("K", 'K'), ("M", 'M'),
];

const LUMCLASS_LEGEND: [(&str, (u8, u8, u8)); 8] = [
    ("supergiant", (255, 120, 200)),
    ("bright giant", (255, 150, 120)),
    ("giant", (255, 200, 80)),
    ("subgiant", (200, 230, 120)),
    ("main seq", (120, 220, 255)),
    ("subdwarf", (140, 160, 200)),
    ("white dwarf", (230, 230, 255)),
    ("unknown", (130, 130, 130)),
];

const SRC_LEGEND: [(&str, (u8, u8, u8)); 3] = [
    ("measured", (120, 220, 140)),
    ("from spectrum", (255, 200, 80)),
    ("from magnitude", (170, 150, 200)),
];

fn gradient(t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: (u8, u8, u8), b: (u8, u8, u8), t: f64| {
        (
            (a.0 as f64 + (b.0 as f64 - a.0 as f64) * t) as u8,
            (a.1 as f64 + (b.1 as f64 - a.1 as f64) * t) as u8,
            (a.2 as f64 + (b.2 as f64 - a.2 as f64) * t) as u8,
        )
    };
    if t < 0.5 {
        lerp((70, 130, 255), (250, 220, 90), t * 2.0)
    } else {
        lerp((250, 220, 90), (255, 80, 60), t * 2.0 - 1.0)
    }
}

fn star_rgb(app: &App, s: &Star) -> (u8, u8, u8) {
    match app.mode {
        1 => LUMCLASS_LEGEND[data::lum_class_group(&s.lum_class)].1,
        2 => {
            // Distance, log-scaled: most named stars sit within 1 kpc.
            let d = (s.dist_pc.max(0.1)).log10() / 3.0;
            gradient(d)
        }
        3 => {
            // Apparent magnitude: bright (negative) is hot on the ramp.
            let t = ((6.0 - s.mag) / 8.0).clamp(0.0, 1.0);
            gradient(t)
        }
        4 => match s.mass {
            Some(m) => gradient((m.log10() + 1.0) / 2.3),
            None => (90, 90, 90),
        },
        5 => match s.radius {
            Some(r) => gradient((r.log10() + 2.0) / 5.0),
            None => (90, 90, 90),
        },
        6 => match (s.teff_src, s.lum_src) {
            (Src::Measured, Src::Measured) => SRC_LEGEND[0].1,
            (Src::Measured, _) | (Src::Spectral, Src::Radius) => SRC_LEGEND[1].1,
            (Src::Spectral, _) | (Src::Color, _) => SRC_LEGEND[2].1,
            _ => (110, 110, 110),
        },
        _ => class_rgb(s.class()),
    }
}

// ─────────────────────────── rendering ───────────────────────────────

fn move_to(row: u16, col: u16) -> String {
    format!("\x1b[{};{}H", row, col)
}

fn legend_string(app: &App) -> String {
    let mut s = String::new();
    if app.track > 0 {
        if app.track == tracks::TRACKS.len() + 1 {
            s.push_str("\x1b[1;38;2;200;200;255mtracks: all\x1b[0m  ");
        } else {
            let t = &tracks::TRACKS[app.track - 1];
            let (r, g, b) = t.color;
            s.push_str(&format!("\x1b[1;38;2;{r};{g};{b}mtrack {}\x1b[0m  ", t.mass));
        }
    }
    s.push_str(&format!("\x1b[1m{} {}\x1b[0m ", app.mode + 1, MODE_NAMES[app.mode]));
    match app.mode {
        0 => {
            for (lbl, c) in CLASS_LEGEND {
                let (r, g, b) = class_rgb(c);
                s.push_str(&format!("\x1b[1;38;2;{r};{g};{b}m{lbl}\x1b[0m"));
            }
        }
        1 => {
            for (lbl, (r, g, b)) in LUMCLASS_LEGEND.iter().take(7) {
                s.push_str(&format!("\x1b[38;2;{r};{g};{b}m{lbl}\x1b[0m "));
            }
        }
        6 => {
            for (lbl, (r, g, b)) in SRC_LEGEND {
                s.push_str(&format!("\x1b[38;2;{r};{g};{b}m{lbl}\x1b[0m "));
            }
        }
        _ => {
            s.push_str("\x1b[2mlow \x1b[0m");
            for i in 0..14 {
                let (r, g, b) = gradient(i as f64 / 13.0);
                s.push_str(&format!("\x1b[38;2;{r};{g};{b}m█\x1b[0m"));
            }
            s.push_str("\x1b[2m high\x1b[0m");
        }
    }
    s
}

fn draw_header(app: &App, cols: u16) {
    let s = &app.stars[app.sel];
    let (r, g, b) = class_rgb(s.class());
    let bg = "\x1b[48;5;236m";
    let info = format!(
        " {RUST}stars{RESET}  \x1b[1m{}{RESET}  \x1b[38;2;{r};{g};{b}m{}{RESET}  \x1b[2m{}\x1b[0m",
        s.name,
        if s.spectral.is_empty() { "—" } else { &s.spectral },
        s.designation
    );
    let iw = crust::display_width(&info);
    let content = if cols >= SIDE_MIN && iw < SIDE_X as usize - 1 {
        format!("{info}{}{}", " ".repeat(SIDE_X as usize - 1 - iw), legend_string(app))
    } else {
        format!("{info}   {}", legend_string(app))
    };
    let line = content.replace(RESET, &format!("{RESET}{bg}"));
    let pad = (cols as usize).saturating_sub(crust::display_width(&content));
    print!("{}{bg}{line}{}{RESET}", move_to(1, 1), " ".repeat(pad));
    std::io::stdout().flush().ok();
}

/// Cell for a diagram position, or None if it falls outside the plot.
fn cell_of(x: f64, y: f64) -> Option<(u16, u16)> {
    let cx = (x * (PLOT_W - 1) as f64).round() as i32;
    let cy = ((1.0 - y) * (PLOT_H - 1) as f64).round() as i32;
    if cx < 0 || cy < 0 || cx >= PLOT_W as i32 || cy >= PLOT_H as i32 {
        return None;
    }
    Some((PLOT_Y + cy as u16, PLOT_X + cx as u16))
}

fn redraw_diagram(app: &App, cols: u16) {
    if cols < PLOT_X + PLOT_W {
        print!("{}\x1b[2mterminal too narrow for the diagram\x1b[0m", move_to(PLOT_Y, 2));
        std::io::stdout().flush().ok();
        return;
    }
    let mut s = String::new();
    // Clear the plot box.
    let blank = " ".repeat(PLOT_W as usize);
    for r in 0..PLOT_H {
        s.push_str(&move_to(PLOT_Y + r, PLOT_X));
        s.push_str(&blank);
    }

    // Schematic evolutionary tracks go underneath the stars.
    if app.track > 0 {
        for (ti, tr) in tracks::TRACKS.iter().enumerate() {
            let show = app.track == tracks::TRACKS.len() + 1 || app.track == ti + 1;
            if !show {
                continue;
            }
            let (r, g, b) = tr.color;
            for (lt, ll) in tr.polyline() {
                let x = (lt - LOG_T_HOT) / (LOG_T_COOL - LOG_T_HOT);
                let y = (ll - LOG_L_MIN) / (LOG_L_MAX - LOG_L_MIN);
                if let Some((row, col)) = cell_of(x, y) {
                    s.push_str(&move_to(row, col));
                    s.push_str(&format!("\x1b[38;2;{r};{g};{b}m·\x1b[0m"));
                }
            }
        }
    }

    // Stars. Later (fainter) stars must not overwrite the brighter ones,
    // so paint in reverse order: the list is sorted brightest first.
    let mut painted: Vec<(u16, u16)> = Vec::new();
    for (i, st) in app.stars.iter().enumerate().rev() {
        let (x, y) = plot_pos(st);
        let (row, col) = match cell_of(x, y) {
            Some(c) => c,
            None => continue,
        };
        let (r, g, b) = star_rgb(app, st);
        let glyph = if painted.contains(&(row, col)) { '*' } else { '·' };
        painted.push((row, col));
        let bold = if st.mag < 2.0 { "1;" } else { "" };
        s.push_str(&move_to(row, col));
        s.push_str(&format!("\x1b[{bold}38;2;{r};{g};{b}m{glyph}\x1b[0m"));
        let _ = i;
    }

    // The selected star last, inverted so it is always findable.
    let sel = &app.stars[app.sel];
    let (x, y) = plot_pos(sel);
    if let Some((row, col)) = cell_of(x, y) {
        let (r, g, b) = star_rgb(app, sel);
        s.push_str(&move_to(row, col));
        s.push_str(&format!("\x1b[7;1;38;2;{r};{g};{b}m◉\x1b[0m"));
    }
    print!("{s}");
    std::io::stdout().flush().ok();
}

/// Axes, ticks and labels around the plot box (static, drawn once).
fn draw_axes(cols: u16) {
    if cols < PLOT_X + PLOT_W {
        return;
    }
    let mut s = String::new();
    // Y axis: log luminosity, ticked every other row.
    for r in 0..PLOT_H {
        let frac = 1.0 - r as f64 / (PLOT_H - 1) as f64;
        let ll = LOG_L_MIN + frac * (LOG_L_MAX - LOG_L_MIN);
        s.push_str(&move_to(PLOT_Y + r, PLOT_X - 1));
        s.push_str("\x1b[2m│\x1b[0m");
        if r % 2 == 0 {
            s.push_str(&move_to(PLOT_Y + r, 1));
            s.push_str(&format!("\x1b[2m{:>5.1}\x1b[0m", ll));
        }
    }
    // X axis along the bottom.
    s.push_str(&move_to(PLOT_Y + PLOT_H, PLOT_X - 1));
    s.push_str(&format!("\x1b[2m└{}\x1b[0m", "─".repeat(PLOT_W as usize)));
    // Temperature ticks at round values.
    let ticks: [f64; 7] = [40000.0, 20000.0, 10000.0, 7000.0, 5000.0, 3500.0, 2500.0];
    s.push_str(&move_to(PLOT_Y + PLOT_H + 1, 1));
    let mut row = " ".repeat((PLOT_X + PLOT_W) as usize);
    for t in ticks {
        let x = (t.log10() - LOG_T_HOT) / (LOG_T_COOL - LOG_T_HOT);
        let col = PLOT_X as usize + (x * (PLOT_W - 1) as f64).round() as usize;
        let label = if t >= 10000.0 {
            format!("{:.0}k", t / 1000.0)
        } else {
            format!("{:.1}k", t / 1000.0)
        };
        let start = col.saturating_sub(label.len() / 2);
        if start + label.len() < row.len() {
            row.replace_range(start..start + label.len(), &label);
        }
    }
    s.push_str(&format!("\x1b[2m{row}\x1b[0m"));
    // Axis names.
    s.push_str(&move_to(PLOT_Y + PLOT_H + 2, PLOT_X));
    s.push_str("\x1b[2m← hotter        effective temperature (K)        cooler →\x1b[0m");
    s.push_str(&move_to(PLOT_Y - 1, 1));
    s.push_str("\x1b[2mlog L/L☉\x1b[0m");
    print!("{s}");
    std::io::stdout().flush().ok();
}

fn help_line() -> String {
    "\x1b[2m←↓↑→ move · Tab brightest · 1-7/m color · t tracks · / find · c claude · ? help · q quit\x1b[0m".to_string()
}

fn draw_all(app: &App, detail: &mut Pane, status: &mut Pane, cols: u16, _rows: u16) {
    Crust::clear_screen();
    draw_header(app, cols);
    draw_axes(cols);
    redraw_diagram(app, cols);
    draw_side(app, cols);
    status.invalidate();
    status.say(&help_line());
    detail.invalidate();
    set_detail(app, detail, cols);
}

fn set_detail(app: &App, detail: &mut Pane, cols: u16) {
    let side = cols >= SIDE_MIN;
    let text = match app.view {
        View::Help => help_text(),
        View::Chat => chat_text(app),
        View::Modes => modes_text(app),
        // With a track laid over the diagram, name its stages above the
        // article so the line on screen can be read as a story.
        View::Article if app.track > 0 => {
            format!("{}\n{}", track_text(app), detail_text(&app.stars[app.sel], side))
        }
        View::Article => detail_text(&app.stars[app.sel], side),
    };
    detail.set_text(&text);
    detail.ix = 0;
    detail.refresh();
}

fn fmt_num(v: f64) -> String {
    if v >= 10000.0 || (v != 0.0 && v.abs() < 0.001) {
        format!("{v:.3e}")
    } else if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

/// The star's numbers, as label/value rows.
fn prop_rows(s: &Star) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    if !s.spectral.is_empty() {
        v.push(("spectral".into(), s.spectral.clone()));
    }
    if !s.lum_class.is_empty() {
        let g = data::lum_class_group(&s.lum_class);
        v.push((
            "class".into(),
            format!("{} ({})", s.lum_class, LUMCLASS_LEGEND[g].0),
        ));
    }
    v.push(("temperature".into(), format!("{:.0} K", s.teff)));
    v.push(("luminosity".into(), format!("{} L☉", fmt_num(s.lum))));
    if let Some(r) = s.radius {
        v.push(("radius".into(), format!("{} R☉", fmt_num(r))));
    }
    if let Some(m) = s.mass {
        v.push(("mass".into(), format!("{} M☉", fmt_num(m))));
    }
    v.push((
        "distance".into(),
        format!("{:.1} ly  ({:.1} pc)", s.dist_ly(), s.dist_pc),
    ));
    v.push(("apparent mag".into(), format!("{:.2}", s.mag)));
    v.push(("absolute mag".into(), format!("{:.2}", s.absmag)));
    if let Some(ci) = s.color_index {
        v.push(("color B-V".into(), format!("{ci:.3}")));
    }
    if !s.constellation.is_empty() {
        v.push(("constellation".into(), s.constellation.clone()));
    }
    if !s.hip.is_empty() {
        v.push(("HIP".into(), s.hip.clone()));
    }
    v
}

fn draw_side(app: &App, cols: u16) {
    if cols < SIDE_MIN {
        return;
    }
    let avail = (cols - SIDE_X + 1) as usize;
    let s = &app.stars[app.sel];
    let dim = "\x1b[2m";
    let mut lines: Vec<String> = Vec::new();
    let (r, g, b) = class_rgb(s.class());
    lines.push(format!("\x1b[1;38;2;{r};{g};{b}m{}{RESET}", s.name));
    lines.push(String::new());
    for (k, v) in prop_rows(s) {
        lines.push(format!("{dim}{k:<14}{RESET}{v}"));
    }
    lines.push(String::new());
    lines.push(format!(
        "{dim}{:<14}{RESET}T {}, L {}",
        "source",
        s.teff_src.label(),
        s.lum_src.label()
    ));

    let blank = " ".repeat(avail);
    let mut out = String::new();
    let rows = PLOT_H + 3;
    for r in 0..rows {
        out.push_str(&move_to(PLOT_Y + r, SIDE_X));
        out.push_str(&blank);
    }
    for (i, l) in lines.iter().take(rows as usize).enumerate() {
        out.push_str(&move_to(PLOT_Y + i as u16, SIDE_X));
        out.push_str(&crust::truncate_ansi(l, avail));
    }
    print!("{out}");
    std::io::stdout().flush().ok();
}

/// The stages of the active track, in the order the star passes through.
fn track_text(app: &App) -> String {
    let head = "\x1b[1;38;2;247;140;60m";
    let all = app.track == tracks::TRACKS.len() + 1;
    let mut s = String::new();
    for (i, t) in tracks::TRACKS.iter().enumerate() {
        if !all && app.track != i + 1 {
            continue;
        }
        let (r, g, b) = t.color;
        s.push_str(&format!("{head}Evolutionary track, {}{RESET} \x1b[2m(schematic)\x1b[0m\n", t.mass));
        let stages: Vec<String> = t
            .stages
            .iter()
            .map(|(_, _, name)| format!("\x1b[38;2;{r};{g};{b}m{name}\x1b[0m"))
            .collect();
        s.push_str(&stages.join(" \x1b[2m→\x1b[0m "));
        s.push('\n');
    }
    s
}

fn modes_text(app: &App) -> String {
    let head = "\x1b[1;38;2;247;140;60m";
    let mut s = format!("{head}Color modes{RESET}\n\n");
    for (i, name) in MODE_NAMES.iter().enumerate() {
        let marker = if i == app.mode { "●" } else { " " };
        let line = format!("  {marker} {:>2}  {name}", i + 1);
        if i == app.menu_ix {
            s.push_str(&format!("\x1b[7m{}\x1b[0m\n", crust::pad_display(&line, 34)));
        } else {
            s.push_str(&format!("{line}\n"));
        }
    }
    s.push_str("\n\x1b[2mj/k move · ENTER pick · 1-7 direct · Ctrl+←/→ cycle · ESC back\x1b[0m\n");
    s
}

fn help_text() -> String {
    format!(
        "{RUST}stars — keys{RESET}\n\n\
         \x20 ← ↑ ↓ → / h j k l   move to the nearest star that way on the diagram\n\
         \x20 Tab / Shift-Tab     next / previous star by apparent brightness\n\
         \x20 < > or n p          same as Tab / Shift-Tab\n\
         \x20 1-7, Ctrl+← →       color mode: 1 spectral class · 2 luminosity class ·\n\
         \x20                     3 distance · 4 apparent magnitude · 5 mass ·\n\
         \x20                     6 radius · 7 data source\n\
         \x20 m                   mode menu\n\
         \x20 t                   evolutionary tracks: off → 1 M☉ → 5 M☉ → 15 M☉ → all\n\
         \x20 J K / Shift-↓ ↑     scroll the article one line\n\
         \x20 Space, PgDn/PgUp    scroll the article one page\n\
         \x20 g G                 top / bottom of the article\n\
         \x20 /                   find a star by name\n\
         \x20 c                   ask Claude about this star (follow-ups keep context)\n\
         \x20 C                   toggle the Claude conversation view\n\
         \x20 w                   open the star's Wikipedia page in the browser\n\
         \x20 u                   rebuild the catalog\n\
         \x20 ?                   toggle this help\n\
         \x20 ESC                 back to the article (quits from the article view)\n\
         \x20 q                   quit\n\n\
         The diagram plots log effective temperature (hot on the left, the way\n\
         Hertzsprung and Russell drew it) against log luminosity in solar units.\n\
         Stars come from the HYG catalog, temperatures / luminosities / radii /\n\
         masses from Wikidata where published, and the rest are derived from the\n\
         spectral type and absolute magnitude. Mode 7 colors by which is which.\n\n\
         The evolutionary tracks are SCHEMATIC: they show the shape and order of\n\
         the stages a star of that mass passes through, not a computed model."
    )
}

fn detail_text(s: &Star, side: bool) -> String {
    let (r, g, b) = class_rgb(s.class());
    let dim = "\x1b[2m";
    let head = "\x1b[1;38;2;247;140;60m";
    let mut out = String::new();
    if !side {
        out.push_str(&format!(
            "\x1b[1;38;2;{r};{g};{b}m{}{RESET}  {}  \x1b[2m{}\x1b[0m\n\n",
            s.name, s.spectral, s.designation
        ));
        for (k, v) in prop_rows(s) {
            out.push_str(&format!("{dim}{k:<14}{RESET}{v}\n"));
        }
        out.push_str(&format!(
            "{dim}{:<14}{RESET}T {}, L {}\n\n",
            "source",
            s.teff_src.label(),
            s.lum_src.label()
        ));
    }
    if s.article.is_empty() {
        out.push_str("\x1b[2mNo Wikipedia article cached for this star.\x1b[0m\n");
    } else {
        out.push_str(&format!("{head}Wikipedia article{RESET}\n"));
        out.push_str(&style_article(&s.article));
    }
    out
}

const TAIL_SECTIONS: [&str; 9] = [
    "see also", "references", "notes", "citations", "sources",
    "further reading", "external links", "bibliography", "explanatory notes",
];

/// Same article cleanup as elements: heading hierarchy, math dumps
/// collapsed, reference tail dropped, blank line between paragraphs.
fn style_article(a: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in a.lines() {
        let t = line.trim();
        if t.len() > 4 && t.starts_with("==") && t.ends_with("==") {
            let level = t.chars().take_while(|c| *c == '=').count();
            let title = t.trim_matches(|c: char| c == '=' || c == ' ');
            if TAIL_SECTIONS.contains(&title.to_lowercase().as_str()) {
                break;
            }
            out.push(match level {
                2 => format!("\x1b[1;38;2;247;140;60m{title}{RESET}"),
                3 => format!("  \x1b[1;38;2;250;200;130m{title}{RESET}"),
                _ => format!("    \x1b[1;38;2;200;170;140m{title}{RESET}"),
            });
        } else if let Some(p) = line.find("{\\displaystyle").or_else(|| line.find("{\\textstyle")) {
            while matches!(out.last(), Some(l) if l.is_empty() || l.starts_with(' ')) {
                out.pop();
            }
            let rest = &line[p..];
            let inner = rest.find(' ').map(|i| rest[i + 1..].trim_end()).unwrap_or("");
            let inner = inner.strip_suffix('}').unwrap_or(inner).trim();
            if !inner.is_empty() {
                out.push(format!("    \x1b[38;2;150;200;255m{inner}\x1b[0m"));
            }
        } else {
            if !line.trim().is_empty()
                && matches!(out.last(), Some(l) if !l.trim().is_empty() && !l.contains('\x1b'))
            {
                out.push(String::new());
            }
            out.push(line.to_string());
        }
    }
    let mut s = String::with_capacity(a.len() + 2048);
    let mut blank = false;
    for l in out {
        let empty = l.trim().is_empty();
        if empty && blank {
            continue;
        }
        blank = empty;
        s.push_str(&l);
        s.push('\n');
    }
    s
}

// ─────────────────────────── claude chat ─────────────────────────────

fn claude_run(prompt: &str, input: &str) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("claude")
        .args(["-p", prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "claude not on PATH".to_string(),
            _ => format!("spawn: {e}"),
        })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input.as_bytes()).map_err(|e| format!("stdin: {e}"))?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("(no message)").chars().take(80).collect());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ask_claude(app: &App, question: &str) -> Result<String, String> {
    let s = &app.stars[app.sel];
    let mut ctx = format!(
        "Star: {} ({}), spectral type {}, {:.0} K, {} L☉, {:.1} light years away.\n",
        s.name,
        s.designation,
        s.spectral,
        s.teff,
        fmt_num(s.lum),
        s.dist_ly()
    );
    if let Some(r) = s.radius {
        ctx.push_str(&format!("Radius: {r} R☉\n"));
    }
    if let Some(m) = s.mass {
        ctx.push_str(&format!("Mass: {m} M☉\n"));
    }
    if !s.article.is_empty() {
        ctx.push_str("\nWikipedia article (may be truncated):\n");
        let art: String = s.article.chars().take(12000).collect();
        ctx.push_str(&art);
    }
    if !app.chat.is_empty() {
        ctx.push_str("\n\nEarlier in this conversation:\n");
        for (q, a) in &app.chat {
            ctx.push_str(&format!("User: {q}\nYou: {a}\n\n"));
        }
    }
    ctx.push_str(&format!("\n\nUser's question: {question}\n"));
    let prompt = format!(
        "You are an astrophysics tutor answering inside a terminal Hertzsprung-Russell \
         diagram app. The user is looking at {}. Answer from the reference material and \
         your own knowledge. Plain text only, no markdown headings. Keep it tight: a few \
         short paragraphs at most. Do not use any tools; just answer.",
        s.name
    );
    claude_run(&prompt, &ctx)
}

fn chat_text(app: &App) -> String {
    let s = &app.stars[app.sel];
    let head = "\x1b[1;38;2;247;140;60m";
    let mut out = format!("{head}Claude — {}{RESET}\n\n", s.name);
    if app.chat.is_empty() {
        out.push_str("\x1b[2mPress c to ask a question about this star.\x1b[0m\n");
        return out;
    }
    for (q, a) in &app.chat {
        out.push_str(&format!("\x1b[1;38;2;120;200;255m? {q}{RESET}\n\n{a}\n\n"));
    }
    out.push_str("\x1b[2mc: ask a follow-up · ESC: back to the article\x1b[0m\n");
    out
}
