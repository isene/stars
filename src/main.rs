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

use crust::style;
use crust::{Crust, Cursor, Input, Pane, Popup};
use data::{Src, Star};
use std::collections::HashMap;
use std::io::Write;

// Diagram geometry. The plot area is a fixed box; the article pane and
// the property block flow around it.
const PLOT_X: u16 = 8; // first column of the plot area
const PLOT_Y: u16 = 3; // first row of the plot area (row 2 stays blank)
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

const RUST_RGB: (u8, u8, u8) = (247, 76, 0);
const HEAD_RGB: (u8, u8, u8) = (247, 140, 60);
const RESET: &str = style::RESET;
const ERR_RGB: (u8, u8, u8) = (255, 120, 120);
const ASK_RGB: (u8, u8, u8) = (120, 200, 255);

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
    /// Which star of the current cell is selected (Tab cycles).
    cell_ix: usize,
    /// Plot cell → the stars that land in it, brightest first.
    cells: HashMap<(u16, u16), Vec<usize>>,
    mode: usize,
    /// 0 = off, else index into tracks::TRACKS + 1, last = all.
    track: usize,
    view: View,
    menu_ix: usize,
    chat: Vec<(String, String)>,
}

impl App {
    /// The cell the selected star sits in.
    fn cur_cell(&self) -> (u16, u16) {
        cell_index(&self.stars[self.sel])
    }
    /// Every star in the current cell, brightest first.
    fn cur_cell_stars(&self) -> &[usize] {
        static EMPTY: &[usize] = &[];
        self.cells.get(&self.cur_cell()).map(|v| v.as_slice()).unwrap_or(EMPTY)
    }
    fn build_cells(&mut self) {
        self.cells.clear();
        for (i, s) in self.stars.iter().enumerate() {
            self.cells.entry(cell_index(s)).or_default().push(i);
        }
        // Stars arrive sorted by apparent magnitude, so each cell's list
        // is already brightest-first.
    }
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
        cell_ix: 0,
        cells: HashMap::new(),
        mode: 0,
        track: 0,
        view: View::Article,
        menu_ix: 0,
        chat: Vec::new(),
    };
    app.build_cells();
    app.cell_ix = app.cur_cell_stars().iter().position(|&i| i == app.sel).unwrap_or(0);

    Crust::init();
    Crust::set_app_identity("Stars");
    let (mut cols, mut rows) = Crust::terminal_size();
    // Margins: two blank columns on the left, three on the right, so the
    // article never runs into the terminal's edges.
    let mut detail = Pane::new(3, DETAIL_Y, cols.saturating_sub(5), rows.saturating_sub(DETAIL_Y).max(1), 253, 0);
    // Park the scroll markers out in the right margin, clear of the text.
    detail.scroll_x = Some(cols);
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
            // Tab walks the stars sharing the cursor's cell.
            "TAB" => cycle_cell(&mut app, true, &mut detail, cols),
            "S-TAB" => cycle_cell(&mut app, false, &mut detail, cols),
            // Brightness tour: the sky's most prominent stars in order.
            ">" | "n" => {
                let t = (app.sel + 1).min(app.stars.len() - 1);
                select(&mut app, t, &mut detail, cols);
            }
            "<" | "p" => {
                let t = app.sel.saturating_sub(1);
                select(&mut app, t, &mut detail, cols);
            }
            // Everything in this cell, as a pick list.
            "ENTER" => {
                let list = app.cur_cell_stars().to_vec();
                if !list.is_empty() {
                    let (col, row) = app.cur_cell();
                    let (teff, lum) = cell_range(col, row);
                    let hint = format!(
                        " {} stars near {:.0} K / {} L☉ · last column: {} · ↓↑ walk · ENTER picks",
                        list.len(),
                        teff,
                        fmt_num(lum),
                        MODE_NAMES[app.mode]
                    );
                    let at = app.cell_ix;
                    pick_list(&mut app, &list, at, &hint, &mut detail, &mut status, cols, rows);
                }
            }
            // The whole catalog, ordered by whatever the color mode is
            // asking: brightest, nearest, heaviest, hottest.
            "L" => {
                let list = mode_order(&app);
                let start = list.iter().position(|&i| i == app.sel).unwrap_or(0);
                let hint = format!(
                    " all {} stars by {} · ↓↑ walk · ENTER picks",
                    list.len(),
                    MODE_NAMES[app.mode]
                );
                pick_list(&mut app, &list, start, &hint, &mut detail, &mut status, cols, rows);
            }
            // The same list, on disk.
            "e" => {
                let list = mode_order(&app);
                match export_csv(&app, &list) {
                    Ok(p) => status.say(&style::dim(&format!(
                        " {} stars by {} → {p}",
                        list.len(),
                        MODE_NAMES[app.mode]
                    ))),
                    Err(e) => status.say(&style::rgb(
                        &format!(" export failed: {e}"),
                        Some(ERR_RGB),
                        None,
                        "",
                    )),
                }
            }
            "1" | "2" | "3" | "4" | "5" | "6" | "7" => {
                app.mode = key.parse::<usize>().unwrap() - 1;
                app.view = View::Article;
                draw_header(&app, cols);
                redraw_diagram(&app, cols);
                set_detail(&app, &mut detail, cols);
            }
            "C-RIGHT" => {
                app.mode = (app.mode + 1) % MODE_NAMES.len();
                draw_header(&app, cols);
                redraw_diagram(&app, cols);
            }
            "C-LEFT" => {
                app.mode = (app.mode + MODE_NAMES.len() - 1) % MODE_NAMES.len();
                draw_header(&app, cols);
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
                print!("{}", Cursor::hide_seq());
                std::io::stdout().flush().ok();
                match q.as_deref().map(|q| find(&app.stars, q)) {
                    Some(Some(i)) => {
                        select(&mut app, i, &mut detail, cols);
                        status.say(&help_line());
                    }
                    Some(None) => status.say(&style::rgb("no match", Some(ERR_RGB), None, "")),
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
                print!("{}", Cursor::hide_seq());
                std::io::stdout().flush().ok();
                match q {
                    Some(q) if !q.trim().is_empty() => {
                        status.say(&style::rgb(" asking claude…", Some(ASK_RGB), None, ""));
                        match ask_claude(&app, q.trim()) {
                            Ok(a) if !a.is_empty() => {
                                app.chat.push((q.trim().to_string(), a));
                                app.view = View::Chat;
                                set_detail(&app, &mut detail, cols);
                                status.say(&help_line());
                            }
                            Ok(_) => status.say(&style::rgb("claude returned nothing", Some(ERR_RGB), None, "")),
                            Err(e) => status.say(&style::rgb(&format!("claude: {e}"), Some(ERR_RGB), None, "")),
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
                            style::rgb(&format!("could not save cache: {e}"), Some(ERR_RGB), None, "")
                        } else {
                            let cur = app.stars[app.sel].name.clone();
                            app.stars = s;
                            app.sel = app.stars.iter().position(|x| x.name == cur).unwrap_or(0);
                            "catalog updated".to_string()
                        }
                    }
                    Err(e) => style::rgb(&format!("fetch failed: {e}"), Some(ERR_RGB), None, ""),
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
                detail.w = cols.saturating_sub(5);
                detail.scroll_x = Some(cols);
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

/// Which plot cell a star falls in: (column, row), both 0-based.
fn cell_index(s: &Star) -> (u16, u16) {
    let (x, y) = plot_pos(s);
    let cx = (x * (PLOT_W - 1) as f64).round() as u16;
    let cy = ((1.0 - y) * (PLOT_H - 1) as f64).round() as u16;
    (cx.min(PLOT_W - 1), cy.min(PLOT_H - 1))
}

/// Temperature and luminosity a cell covers, for the "empty cell" readout.
fn cell_range(col: u16, row: u16) -> (f64, f64) {
    let fx = col as f64 / (PLOT_W - 1) as f64;
    let fy = 1.0 - row as f64 / (PLOT_H - 1) as f64;
    let teff = 10f64.powf(LOG_T_HOT + fx * (LOG_T_COOL - LOG_T_HOT));
    let lum = 10f64.powf(LOG_L_MIN + fy * (LOG_L_MAX - LOG_L_MIN));
    (teff, lum)
}

/// Walk one cell at a time: scan along the row (or column) for the next
/// cell holding a star. If that line is empty, fall back to the nearest
/// occupied cell on that side, so a step never dead-ends.
fn step(app: &mut App, dir: (i32, i32), detail: &mut Pane, cols: u16) {
    let (cc, cr) = app.cur_cell();
    let (mut c, mut r) = (cc as i32, cr as i32);
    loop {
        c += dir.0;
        r -= dir.1; // screen rows grow downward, luminosity grows up
        if c < 0 || r < 0 || c >= PLOT_W as i32 || r >= PLOT_H as i32 {
            break;
        }
        if let Some(list) = app.cells.get(&(c as u16, r as u16)) {
            let target = list[0];
            select(app, target, detail, cols);
            return;
        }
    }
    // Nothing in that row / column: take the closest cell in that half.
    let mut best: Option<(f64, usize)> = None;
    for (&(x, y), list) in app.cells.iter() {
        let (dx, dy) = (x as f64 - cc as f64, cr as f64 - y as f64);
        let along = dx * dir.0 as f64 + dy * dir.1 as f64;
        if along <= 0.0 {
            continue;
        }
        let across = (dx * dir.1 as f64).abs() + (dy * dir.0 as f64).abs();
        let cost = along + across * 2.0;
        if best.map_or(true, |(b, _)| cost < b) {
            best = Some((cost, list[0]));
        }
    }
    if let Some((_, i)) = best {
        select(app, i, detail, cols);
    }
}

/// Cycle through the stars sharing the selected star's cell.
fn cycle_cell(app: &mut App, forward: bool, detail: &mut Pane, cols: u16) {
    let list = app.cur_cell_stars().to_vec();
    if list.len() < 2 {
        return;
    }
    let at = list.iter().position(|&i| i == app.sel).unwrap_or(0);
    let next = if forward {
        (at + 1) % list.len()
    } else {
        (at + list.len() - 1) % list.len()
    };
    app.cell_ix = next;
    select(app, list[next], detail, cols);
}

/// A popup of stars to walk and pick from, colored and captioned by the
/// ACTIVE color mode, so the list answers the question the diagram is
/// currently being asked. `start` is the row to open on.
#[allow(clippy::too_many_arguments)]
fn pick_list(
    app: &mut App,
    list: &[usize],
    start: usize,
    hint: &str,
    detail: &mut Pane,
    status: &mut Pane,
    cols: u16,
    rows: u16,
) {
    if list.is_empty() {
        return;
    }
    let lines: Vec<String> = list
        .iter()
        .map(|&i| {
            let s = &app.stars[i];
            let name: String = s.name.chars().take(20).collect();
            format!(
                " {} {:<10} {:>7.0} K  {:>9} L☉  {}",
                style::rgb(&format!("{name:<20}"), Some(star_rgb(app, s)), None, ""),
                s.spectral,
                s.teff,
                fmt_num(s.lum),
                style::rgb(
                    &format!("{:<22}", mode_value_str(app, s)),
                    Some(star_rgb(app, s)),
                    None,
                    "b"
                )
            )
        })
        .collect();
    let w = 96.min(cols.saturating_sub(4));
    let h = (lines.len() as u16).min(rows.saturating_sub(6)).max(1);
    let mut pop = Popup::centered(w, h, 253, 236);
    // Open on the star we are already showing, so the list is a place to
    // walk from rather than a fresh start.
    pop.pane.index = start.min(list.len() - 1);
    status.say(&style::dim(hint));
    let picked = pop.modal(&lines.join("\n"));
    pop.dismiss(&mut [detail, status]);
    Crust::clear_screen();
    if let Some(ix) = picked {
        if let Some(&i) = list.get(ix) {
            if i != app.sel {
                app.chat.clear();
            }
            app.sel = i;
            app.view = View::Article;
            app.cell_ix = app
                .cur_cell_stars()
                .iter()
                .position(|&j| j == app.sel)
                .unwrap_or(0);
        }
    }
    draw_all(app, detail, status, cols, rows);
}

/// One CSV field, quoted only when it has to be.
fn csv(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        String::new()
    }
}

/// Write the given stars to CSV, in the order they are listed, with every
/// column the app knows — including which numbers were measured and which
/// this program derived.
fn export_csv(app: &App, list: &[usize]) -> Result<String, String> {
    let slug = MODE_NAMES[app.mode].replace(' ', "-");
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = std::path::Path::new(&home).join(format!("stars-by-{slug}.csv"));
    let mut out = String::with_capacity(list.len() * 160);
    out.push_str(
        "name,designation,constellation,spectral,lum_class,teff_k,teff_source,\
         luminosity_lsun,lum_source,radius_rsun,mass_msun,distance_ly,\
         apparent_mag,absolute_mag,color_index,hip,hd,url\n",
    );
    for &i in list {
        let s = &app.stars[i];
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv(&s.name),
            csv(&s.designation),
            csv(&s.constellation),
            csv(&s.spectral),
            csv(&s.lum_class),
            num(s.teff),
            csv(s.teff_src.label()),
            num(s.lum),
            csv(s.lum_src.label()),
            s.radius.map(num).unwrap_or_default(),
            s.mass.map(num).unwrap_or_default(),
            num(s.dist_ly()),
            num(s.mag),
            num(s.absmag),
            s.color_index.map(num).unwrap_or_default(),
            csv(&s.hip),
            csv(&s.hd),
            csv(&s.source),
        ));
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// The whole catalog in the order the current color mode implies:
/// hottest, most luminous, nearest, brightest, heaviest, largest.
fn mode_order(app: &App) -> Vec<usize> {
    let mut ix: Vec<usize> = (0..app.stars.len()).collect();
    let key = |s: &Star| -> (i64, i64) {
        let big = |v: f64| -(v * 1000.0) as i64; // descending
        match app.mode {
            1 => (
                data::lum_class_group(&s.lum_class) as i64,
                big(s.lum.max(0.0).log10().max(-9.0) + 10.0),
            ),
            2 => ((s.dist_ly() * 1000.0) as i64, 0),
            3 => ((s.mag * 1000.0) as i64, 0),
            4 => (s.mass.map_or(i64::MAX, |m| big(m)), 0),
            5 => (s.radius.map_or(i64::MAX, |r| big(r)), 0),
            6 => (s.teff_src as i64, 0),
            // Spectral class: hottest first, which walks O through M.
            _ => (big(s.teff), 0),
        }
    };
    ix.sort_by(|&a, &b| {
        key(&app.stars[a])
            .cmp(&key(&app.stars[b]))
            .then_with(|| app.stars[a].name.cmp(&app.stars[b].name))
    });
    ix
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
    // Keep Tab's in-cell cursor on the star we just jumped to.
    app.cell_ix = app
        .cur_cell_stars()
        .iter()
        .position(|&i| i == app.sel)
        .unwrap_or(0);
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

/// The color a star of this temperature shows, interpolated across the
/// spectral classes so the temperature axis reads as a real spectrum.
fn teff_rgb(teff: f64) -> (u8, u8, u8) {
    const STOPS: [(f64, (u8, u8, u8)); 7] = [
        (40000.0, (120, 150, 255)),
        (20000.0, (160, 195, 255)),
        (9700.0, (225, 235, 255)),
        (7200.0, (255, 245, 200)),
        (5800.0, (255, 215, 90)),
        (4400.0, (255, 150, 60)),
        (3000.0, (255, 90, 60)),
    ];
    let t = teff.clamp(3000.0, 40000.0);
    for w in STOPS.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t0 && t >= t1 {
            let f = (t0.ln() - t.ln()) / (t0.ln() - t1.ln());
            return (
                (c0.0 as f64 + (c1.0 as f64 - c0.0 as f64) * f) as u8,
                (c0.1 as f64 + (c1.1 as f64 - c0.1 as f64) * f) as u8,
                (c0.2 as f64 + (c1.2 as f64 - c0.2 as f64) * f) as u8,
            );
        }
    }
    STOPS[0].1
}

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

/// What the active color mode encodes for this star, as text. Used in
/// the cell popup so the list reads in the same terms as the diagram.
fn mode_value_str(app: &App, s: &Star) -> String {
    match app.mode {
        1 => {
            let g = data::lum_class_group(&s.lum_class);
            if s.lum_class.is_empty() {
                "class unknown".to_string()
            } else {
                format!("{} {}", s.lum_class, LUMCLASS_LEGEND[g].0)
            }
        }
        2 => format!("{:.1} ly", s.dist_ly()),
        3 => format!("mag {:.2}", s.mag),
        4 => match s.mass {
            Some(m) => format!("{} M☉", fmt_num(m)),
            None => "mass unknown".to_string(),
        },
        5 => match s.radius {
            Some(r) => format!("{} R☉", fmt_num(r)),
            None => "radius unknown".to_string(),
        },
        6 => format!("T {}", s.teff_src.label()),
        _ => format!("class {}", s.class()),
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
    Cursor::at(col, row)
}

fn legend_string(app: &App) -> String {
    let mut s = String::new();
    if app.track > 0 {
        if app.track == tracks::TRACKS.len() + 1 {
            s.push_str(&style::rgb("tracks: all", Some((200, 200, 255)), None, "b"));
            s.push_str("  ");
        } else {
            let t = &tracks::TRACKS[app.track - 1];
            let (r, g, b) = t.color;
            s.push_str(&style::rgb(&format!("track {}", t.mass), Some((r, g, b)), None, "b"));
            s.push_str("  ");
        }
    }
    s.push_str(&style::bold(&format!("{} {}", app.mode + 1, MODE_NAMES[app.mode])));
    s.push(' ');
    match app.mode {
        0 => {
            for (lbl, c) in CLASS_LEGEND {
                let (r, g, b) = class_rgb(c);
                s.push_str(&style::rgb(lbl, Some((r, g, b)), None, "b"));
            }
        }
        1 => {
            for (lbl, rgb) in LUMCLASS_LEGEND.iter().take(7) {
                s.push_str(&style::rgb(lbl, Some(*rgb), None, ""));
                s.push(' ');
            }
        }
        6 => {
            for (lbl, (r, g, b)) in SRC_LEGEND {
                s.push_str(&style::rgb(lbl, Some((r, g, b)), None, ""));
                s.push(' ');
            }
        }
        _ => {
            s.push_str(&style::dim("low "));
            for i in 0..14 {
                let (r, g, b) = gradient(i as f64 / 13.0);
                s.push_str(&style::rgb("█", Some((r, g, b)), None, ""));
            }
            s.push_str(&style::dim(" high"));
        }
    }
    s
}

fn draw_header(app: &App, cols: u16) {
    let s = &app.stars[app.sel];
    let (r, g, b) = class_rgb(s.class());
    let bar_bg: (u8, u8, u8) = (38, 38, 38);
    let here = app.cur_cell_stars().len();
    let crowd = if here > 1 {
        style::rgb(&format!(" +{}", here - 1), Some((255, 170, 80)), None, "b")
    } else {
        String::new()
    };
    let info = format!(
        " {}  {}{}  {}  {}",
        style::rgb("stars", Some(RUST_RGB), None, "b"),
        style::bold(&s.name),
        crowd,
        style::rgb(
            if s.spectral.is_empty() { "—" } else { &s.spectral },
            Some((r, g, b)),
            None,
            ""
        ),
        style::dim(&s.designation)
    );
    let iw = crust::display_width(&info);
    let content = if cols >= SIDE_MIN && iw < SIDE_X as usize - 1 {
        format!("{info}{}{}", " ".repeat(SIDE_X as usize - 1 - iw), legend_string(app))
    } else {
        format!("{info}   {}", legend_string(app))
    };
    let pad = (cols as usize).saturating_sub(crust::display_width(&content));
    // Re-arm the bar background after every reset inside the content.
    let bar = style::rgb("", None, Some(bar_bg), "");
    let armed = bar.trim_end_matches(RESET).to_string();
    let line = content.replace(RESET, &format!("{RESET}{armed}"));
    print!(
        "{}{}",
        move_to(1, 1),
        style::rgb(&format!("{line}{}", " ".repeat(pad)), None, Some(bar_bg), "")
    );
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
        print!("{}{}", move_to(PLOT_Y, 2), style::dim("terminal too narrow for the diagram"));
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
                    s.push_str(&style::rgb("·", Some((r, g, b)), None, ""));
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
        let attrs = if st.mag < 2.0 { "b" } else { "" };
        s.push_str(&move_to(row, col));
        s.push_str(&style::rgb(&glyph.to_string(), Some((r, g, b)), None, attrs));
        let _ = i;
    }

    // The selected star last, inverted so it is always findable.
    let sel = &app.stars[app.sel];
    let (x, y) = plot_pos(sel);
    if let Some((row, col)) = cell_of(x, y) {
        let (r, g, b) = star_rgb(app, sel);
        s.push_str(&move_to(row, col));
        s.push_str(&style::rgb("◉", Some((r, g, b)), None, "br"));
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
    // Y axis: log luminosity, ticked every other row. The rule brightens
    // with the axis, dim grey at the faint end to white at the luminous.
    for r in 0..PLOT_H {
        let frac = 1.0 - r as f64 / (PLOT_H - 1) as f64;
        let ll = LOG_L_MIN + frac * (LOG_L_MAX - LOG_L_MIN);
        let v = (60.0 + 195.0 * frac) as u8;
        s.push_str(&move_to(PLOT_Y + r, PLOT_X - 1));
        s.push_str(&style::rgb("│", Some((v, v, v)), None, ""));
        if r % 2 == 0 {
            s.push_str(&move_to(PLOT_Y + r, 1));
            s.push_str(&style::dim(&format!("{ll:>5.1}")));
        }
    }
    // X axis along the bottom, tinted with the color a star of that
    // temperature actually shines: blue at the hot end, red at the cool.
    s.push_str(&move_to(PLOT_Y + PLOT_H, PLOT_X - 1));
    // Dark slate corner: a bright one pulls the eye away from the plot.
    s.push_str(&style::rgb("└", Some((70, 80, 100)), None, ""));
    for c in 0..PLOT_W {
        let f = c as f64 / (PLOT_W - 1) as f64;
        let teff = 10f64.powf(LOG_T_HOT + f * (LOG_T_COOL - LOG_T_HOT));
        let (r, g, b) = teff_rgb(teff);
        s.push_str(&style::rgb("─", Some((r, g, b)), None, ""));
    }
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
    s.push_str(&style::dim(&row));
    // Axis names, both on the caption row so the row under the header
    // bar stays empty.
    s.push_str(&move_to(PLOT_Y + PLOT_H + 2, 1));
    s.push_str(&style::dim("↑ log L/L☉   ← hotter     effective temperature (K)     cooler →"));
    print!("{s}");
    std::io::stdout().flush().ok();
}

fn help_line() -> String {
    style::dim("←↓↑→ cell · Tab in-cell · ⏎ cell list · L all · e csv · 1-7/m color · t tracks · / find · c claude · ? help · q")
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

/// The star itself: what it is and what it puts out.
fn star_rows(s: &Star) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    if !s.spectral.is_empty() {
        v.push(("spectral".into(), s.spectral.clone()));
    }
    if !s.lum_class.is_empty() {
        let g = data::lum_class_group(&s.lum_class);
        v.push(("class".into(), format!("{} ({})", s.lum_class, LUMCLASS_LEGEND[g].0)));
    }
    v.push(("temperature".into(), format!("{:.0} K", s.teff)));
    v.push(("luminosity".into(), format!("{} L☉", fmt_num(s.lum))));
    if let Some(r) = s.radius {
        v.push(("radius".into(), format!("{} R☉", fmt_num(r))));
    }
    if let Some(m) = s.mass {
        v.push(("mass".into(), format!("{} M☉", fmt_num(m))));
    }
    v
}

/// How it appears from here.
fn sky_rows(s: &Star) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    v.push(("distance".into(), format!("{:.1} ly", s.dist_ly())));
    v.push(("".into(), format!("{:.1} pc", s.dist_pc)));
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

const LBL_W: usize = 13;

/// One label/value row of a boxed table: dim label, gutter, plain value.
/// The gutter is explicit because the longest labels ("apparent mag",
/// "constellation") fill the label column exactly and would otherwise
/// run straight into their value.
const GUTTER: usize = 2;

fn table_cell(entry: Option<&(String, String)>, cell: usize) -> String {
    match entry {
        Some((l, v)) => format!(
            " {}{}{} ",
            style::dim(&format!("{l:<LBL_W$}")),
            " ".repeat(GUTTER),
            fit(v, cell.saturating_sub(LBL_W + GUTTER + 2))
        ),
        None => " ".repeat(cell),
    }
}

/// Top rule of a boxed column: `┌─ Title ───────┐` (or `┬` between columns).
fn table_top(title: &str, cell: usize, opener: &str, closer: &str) -> String {
    let rule = "─".repeat(cell.saturating_sub(3 + title.chars().count()));
    format!(
        "{}{}{}",
        style::dim(&format!("{opener}─ ")),
        style::bold(title),
        style::dim(&format!(" {rule}{closer}"))
    )
}

/// Two labelled columns in a box-drawn table, same shape as elements'.
fn prop_table(lt: &str, left: &[(String, String)], rt: &str, right: &[(String, String)], cell: usize) -> String {
    let bar = style::dim("│");
    let mut s = format!(
        "{}{}\n",
        table_top(lt, cell, "┌", "┬"),
        table_top(rt, cell, "", "┐")
    );
    for i in 0..left.len().max(right.len()) {
        s.push_str(&format!(
            "{bar}{}{bar}{}{bar}\n",
            table_cell(left.get(i), cell),
            table_cell(right.get(i), cell)
        ));
    }
    let rule = "─".repeat(cell);
    s.push_str(&style::dim(&format!("└{rule}┴{rule}┘")));
    s.push('\n');
    s
}

/// One labelled column, for side panels too narrow to hold two.
fn prop_table_single(title: &str, rows: &[(String, String)], cell: usize) -> String {
    let bar = style::dim("│");
    let mut s = format!("{}\n", table_top(title, cell, "┌", "┐"));
    for r in rows {
        s.push_str(&format!("{bar}{}{bar}\n", table_cell(Some(r), cell)));
    }
    s.push_str(&style::dim(&format!("└{}┘", "─".repeat(cell))));
    s.push('\n');
    s
}

fn fit(v: &str, w: usize) -> String {
    let n = v.chars().count();
    if n <= w {
        format!("{v}{}", " ".repeat(w - n))
    } else {
        let mut t: String = v.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn draw_side(app: &App, cols: u16) {
    if cols < SIDE_MIN {
        return;
    }
    let avail = (cols - SIDE_X + 1) as usize;
    let s = &app.stars[app.sel];
    let (r, g, b) = class_rgb(s.class());
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{}  {}",
        style::rgb(&s.name, Some((r, g, b)), None, "b"),
        style::dim(&s.designation)
    ));
    lines.push(String::new());

    // Two columns when there is room, otherwise stack them.
    let table = if avail >= 76 {
        let cell = ((avail - 3) / 2).min(38);
        prop_table("Star", &star_rows(s), "Sky", &sky_rows(s), cell)
    } else {
        let cell = avail.saturating_sub(2).min(40);
        format!(
            "{}{}",
            prop_table_single("Star", &star_rows(s), cell),
            prop_table_single("Sky", &sky_rows(s), cell)
        )
    };
    lines.extend(table.lines().map(|l| l.to_string()));
    lines.push(String::new());
    lines.push(format!(
        "{}T {}, L {}",
        style::dim("source        "),
        s.teff_src.label(),
        s.lum_src.label()
    ));
    // How crowded the cursor's cell is, so Tab has an obvious purpose.
    let n = app.cur_cell_stars().len();
    if n > 1 {
        lines.push(format!(
            "{}{n} stars  {}",
            style::dim("this cell     "),
            style::dim("(Tab cycles, Enter lists)")
        ));
    }

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
    let all = app.track == tracks::TRACKS.len() + 1;
    let mut s = String::new();
    for (i, t) in tracks::TRACKS.iter().enumerate() {
        if !all && app.track != i + 1 {
            continue;
        }
        let (r, g, b) = t.color;
        s.push_str(&format!(
            "{} {}\n",
            style::rgb(&format!("Evolutionary track, {}", t.mass), Some(HEAD_RGB), None, "b"),
            style::dim("(schematic)")
        ));
        let stages: Vec<String> = t
            .stages
            .iter()
            .map(|(_, _, name)| style::rgb(name, Some((r, g, b)), None, ""))
            .collect();
        s.push_str(&stages.join(&format!(" {} ", style::dim("→"))));
        s.push('\n');
    }
    s
}

fn modes_text(app: &App) -> String {
    let mut s = format!("{}\n\n", style::rgb("Color modes", Some(HEAD_RGB), None, "b"));
    for (i, name) in MODE_NAMES.iter().enumerate() {
        let marker = if i == app.mode { "●" } else { " " };
        let line = format!("  {marker} {:>2}  {name}", i + 1);
        if i == app.menu_ix {
            s.push_str(&format!("{}\n", style::reverse(&crust::pad_display(&line, 34))));
        } else {
            s.push_str(&format!("{line}\n"));
        }
    }
    s.push_str(&format!("\n{}\n", style::dim("j/k move · ENTER pick · 1-7 direct · Ctrl+←/→ cycle · ESC back")));
    s
}

fn help_text() -> String {
    format!(
        "{}\n\n\
         \x20 ← ↑ ↓ → / h j k l   walk the diagram a cell at a time\n\
         \x20 Tab / Shift-Tab     cycle the stars sharing the cursor's cell\n\
         \x20 ENTER               list every star in the cell and pick one\n\
         \x20 L                   list the WHOLE catalog, ordered by the color mode\n\
         \x20 < > or n p          previous / next star by apparent brightness\n\
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
         \x20 e                   export that same ordered list to ~/stars-by-<mode>.csv\n\
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
         the stages a star of that mass passes through, not a computed model.",
        style::rgb("stars — keys", Some(RUST_RGB), None, "b")
    )
}

fn detail_text(s: &Star, side: bool) -> String {
    let (r, g, b) = class_rgb(s.class());
    // A blank line so the pane's first row is never flush against the
    // diagram's caption row.
    let mut out = String::from("\n");
    if !side {
        out.push_str(&format!(
            "{}  {}  {}\n\n",
            style::rgb(&s.name, Some((r, g, b)), None, "b"),
            s.spectral,
            style::dim(&s.designation)
        ));
        for (k, v) in star_rows(s).into_iter().chain(sky_rows(s)) {
            out.push_str(&format!("{}{v}\n", style::dim(&format!("{k:<14}"))));
        }
        out.push_str(&format!(
            "{}T {}, L {}\n\n",
            style::dim(&format!("{:<14}", "source")),
            s.teff_src.label(),
            s.lum_src.label()
        ));
    }
    if s.article.is_empty() {
        out.push_str(&format!("{}\n", style::dim("No Wikipedia article cached for this star.")));
    } else {
        out.push_str(&format!("{}\n", style::rgb("Wikipedia article", Some(HEAD_RGB), None, "b")));
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
    // crust folds the extract's math blocks into single inline
    // expressions and clears the debris left by dropped templates.
    let a = crust::text::clean_wiki_extract(a);
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
                2 => style::rgb(title, Some(HEAD_RGB), None, "b"),
                3 => format!("  {}", style::rgb(title, Some((250, 200, 130)), None, "b")),
                _ => format!("    {}", style::rgb(title, Some((200, 170, 140)), None, "b")),
            });
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
    let mut out = format!("{}\n\n", style::rgb(&format!("Claude — {}", s.name), Some(HEAD_RGB), None, "b"));
    if app.chat.is_empty() {
        out.push_str(&format!("{}\n", style::dim("Press c to ask a question about this star.")));
        return out;
    }
    for (q, a) in &app.chat {
        out.push_str(&format!("{}\n\n{a}\n\n", style::rgb(&format!("? {q}"), Some(ASK_RGB), None, "b")));
    }
    out.push_str(&format!("{}\n", style::dim("c: ask a follow-up · ESC: back to the article")));
    out
}
