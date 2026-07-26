# stars

<img src="img/stars.svg" align="right" width="150">

**The Hertzsprung-Russell diagram in your terminal. Written in Rust.**

![Rust](https://img.shields.io/badge/language-Rust-f74c00) ![License](https://img.shields.io/badge/license-Unlicense-green) ![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue) ![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-important)

Every named star with a measured parallax, plotted where it belongs on the HR diagram: temperature across, luminosity up. Walk between them with the arrow keys, read each star's numbers and its full Wikipedia article, and lay schematic evolutionary tracks over the plot to see where a star of a given mass goes when it leaves the main sequence. Built on [Crust](https://github.com/isene/crust), part of the [Fe2O3 suite](https://github.com/isene/fe2o3).

## Features

- **461 named stars** from the HYG catalog, plotted in log Teff against log luminosity
- **Spatial navigation**: the arrow keys move to the nearest star in that direction on the diagram, so you can walk up the main sequence and across into the giants
- **Seven color modes** (keys 1-7, `m` for the menu, or Ctrl+←/→): spectral class in true star colors, luminosity class, distance, apparent magnitude, mass, radius, and data source
- **Evolutionary tracks** (`t`): schematic paths for 1, 5 and 15 M☉, with their stages named below the diagram, from ZAMS through the giant branch to a white dwarf or a supernova
- **Honest about its numbers**: measured values come from Wikidata, the rest are derived from the spectral type and absolute magnitude, and mode 7 colors the diagram by which is which
- **Full Wikipedia article** for 439 of the stars, cached locally
- **Ask Claude** (`c`) about the star you are looking at, with its data and article as context
- **Zero idle cost**: event-driven, no timers, no polling
- **Offline**: one fetch, then everything is local

## Install

Download the prebuilt binary from [Releases](https://github.com/isene/stars/releases), or build from source:

```bash
cargo build --release
cp target/release/stars ~/.local/bin/
```

First start builds the catalog (about three minutes), then the app works offline.

## Key Bindings

| Key | Action |
|-----|--------|
| ← ↑ ↓ →, h/j/k/l | Move to the nearest star that way on the diagram |
| Tab, Shift+Tab | Next / previous star by apparent brightness |
| < >, n p | Same as Tab / Shift+Tab |
| 1-7, Ctrl+←/→ | Color mode |
| m | Mode menu |
| t | Evolutionary tracks: off → 1 M☉ → 5 M☉ → 15 M☉ → all |
| J K, Shift+↓/↑ | Scroll the article one line |
| Space, PgUp/PgDn | Scroll the article one page |
| g G | Top / bottom of the article |
| / | Find a star by name |
| c | Ask Claude about this star (follow-ups keep context) |
| C | Toggle the Claude conversation view |
| w | Open the star's Wikipedia page in the browser |
| u | Rebuild the catalog |
| ? | Help |
| ESC | Back to the article (quits from the article view) |
| q | Quit |

## CLI

```
stars [STAR] [--fetch]
```

`STAR` starts at (or, when piped, prints) that star. `--fetch` rebuilds the catalog.

## Where the numbers come from

Positions, magnitudes, spectral types and distances come from the [HYG database](https://github.com/astronexus/HYG-Database) (Hipparcos + Yale + Gliese). Rows without a usable parallax are dropped, since their absolute magnitudes are meaningless.

Effective temperatures, luminosities, radii and masses come from Wikidata where they are published (119 stars have a measured temperature). Everything else is derived: temperature from the spectral type on the standard main-sequence scale, or from the B-V color index via Ballesteros' formula; luminosity from radius and temperature, or from the absolute magnitude with a bolometric correction. The detail panel names the source for every star, and color mode 7 shows it across the whole diagram.

**The evolutionary tracks are schematic.** They show the shape and the order of the stages a star of that mass passes through, not a computed stellar model.

## License

Public domain (Unlicense). Created by [Geir Isene](https://isene.com).
