//! Schematic post-main-sequence evolutionary tracks.
//!
//! These are teaching aids, NOT computed stellar models. Each track is a
//! short list of waypoints in (log Teff, log L/L☉) taken from the standard
//! textbook picture of how a star of that mass moves across the HR diagram:
//! the shape and the order of the stages are right, the exact path is not
//! a fit to any particular set of isochrones. The app says so in `?`.

pub struct Track {
    pub mass: &'static str,
    pub color: (u8, u8, u8),
    /// (log Teff, log L, stage name) in the order the star passes through.
    pub stages: &'static [(f64, f64, &'static str)],
}

pub const TRACKS: [Track; 3] = [
    Track {
        mass: "1 M☉",
        color: (255, 200, 90),
        stages: &[
            (3.762, 0.00, "ZAMS"),
            (3.755, 0.18, "main sequence"),
            (3.740, 0.30, "turn-off"),
            (3.700, 0.48, "subgiant"),
            (3.650, 1.00, "base of the red giant branch"),
            (3.590, 2.00, "red giant branch"),
            (3.540, 3.40, "helium flash"),
            (3.690, 1.70, "horizontal branch"),
            (3.640, 2.30, "early AGB"),
            (3.560, 3.60, "thermally pulsing AGB"),
            (4.000, 3.70, "post-AGB"),
            (4.900, 3.40, "planetary nebula nucleus"),
            (5.000, 1.00, "white dwarf, hot"),
            (4.300, -1.50, "white dwarf, cooling"),
            (3.900, -3.50, "white dwarf, cold"),
        ],
    },
    Track {
        mass: "5 M☉",
        color: (140, 220, 255),
        stages: &[
            (4.230, 2.75, "ZAMS"),
            (4.190, 2.90, "main sequence"),
            (4.130, 3.05, "turn-off"),
            (4.000, 3.10, "Hertzsprung gap"),
            (3.750, 3.15, "crossing the gap"),
            (3.620, 3.25, "red giant"),
            (3.680, 3.30, "blue loop"),
            (3.800, 3.35, "Cepheid strip"),
            (3.640, 3.45, "back to the red"),
            (3.540, 4.00, "AGB"),
        ],
    },
    Track {
        mass: "15 M☉",
        color: (255, 130, 190),
        stages: &[
            (4.480, 4.25, "ZAMS"),
            (4.440, 4.40, "main sequence"),
            (4.380, 4.50, "turn-off"),
            (4.200, 4.55, "blue supergiant"),
            (4.000, 4.60, "crossing"),
            (3.700, 4.70, "yellow supergiant"),
            (3.560, 4.90, "red supergiant"),
            (3.540, 5.05, "pre-supernova"),
        ],
    },
];

impl Track {
    /// Waypoints joined by straight segments, sampled densely enough to
    /// draw as a continuous line in the terminal grid.
    pub fn polyline(&self) -> Vec<(f64, f64)> {
        let mut pts = Vec::new();
        for w in self.stages.windows(2) {
            let (x0, y0, _) = w[0];
            let (x1, y1, _) = w[1];
            let steps = 40;
            for i in 0..=steps {
                let f = i as f64 / steps as f64;
                pts.push((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f));
            }
        }
        pts
    }
}
