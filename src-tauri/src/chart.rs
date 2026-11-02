//! Smooth geometry for a sixty-second scrolling traffic window.
//! Curves are rasterized on a worker; Slint only translates the cached texture.
use std::{collections::VecDeque, fmt::Write};

pub const SAMPLES: usize = 61;
pub const WIDTH: f64 = 1000.0;
pub const HEIGHT: f64 = 104.0;
pub const STEP: f64 = WIDTH / (SAMPLES - 2) as f64;

pub struct Curves {
    pub down: String,
    pub up: String,
    pub down_area: String,
    pub up_area: String,
    pub peak: f64,
    pub idle: bool,
}

fn finite(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn points(history: &VecDeque<[f64; 2]>, channel: usize, peak: f64) -> Vec<(f64, f64)> {
    // Right-align real samples, rather than filling only half the chart during
    // the first thirty seconds of a connection.
    let padding = SAMPLES.saturating_sub(history.len());
    (0..SAMPLES)
        .map(|index| {
            let sample = index
                .checked_sub(padding)
                .and_then(|i| history.get(i + history.len().saturating_sub(SAMPLES)))
                .map_or(0.0, |sample| finite(sample[channel]));
            (
                index as f64 * STEP,
                HEIGHT - (sample / peak).min(1.0).sqrt() * (HEIGHT - 10.0),
            )
        })
        .collect()
}

fn smooth_path(points: &[(f64, f64)]) -> String {
    let mut path = format!("M {:.2} {:.2}", points[0].0, points[0].1);
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        let c1 = (
            p1.0 + (p2.0 - p0.0) / 6.0,
            (p1.1 + (p2.1 - p0.1) / 6.0).clamp(10.0, HEIGHT),
        );
        let c2 = (
            p2.0 - (p3.0 - p1.0) / 6.0,
            (p2.1 - (p3.1 - p1.1) / 6.0).clamp(10.0, HEIGHT),
        );
        let _ = write!(
            path,
            " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
        );
    }
    path
}

pub fn curves(history: &VecDeque<[f64; 2]>, previous_peak: f64) -> Curves {
    let raw_peak = history
        .iter()
        .flatten()
        .copied()
        .map(finite)
        .fold(0.0, f64::max);
    let peak = raw_peak.max(finite(previous_peak) * 0.94).max(1.0);
    let down = smooth_path(&points(history, 1, peak));
    let up = smooth_path(&points(history, 0, peak));
    let area = |line: &str| {
        format!(
            "{line} L {:.2} {HEIGHT} L 0 {HEIGHT} Z",
            STEP * (SAMPLES - 1) as f64
        )
    };
    Curves {
        down_area: area(&down),
        up_area: area(&up),
        down,
        up,
        peak,
        idle: raw_peak == 0.0,
    }
}

pub fn svg(chart: &Curves, scale: f32, down: &str, up: &str) -> String {
    let scale = scale.clamp(1.0, 3.0) as f64;
    let width = WIDTH + STEP;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {width} {HEIGHT}">
<defs>
<linearGradient id="down" x1="0" y1="0" x2="0" y2="1"><stop stop-color="{down}" stop-opacity=".2"/><stop offset="1" stop-color="{down}" stop-opacity=".015"/></linearGradient>
<linearGradient id="up" x1="0" y1="0" x2="0" y2="1"><stop stop-color="{up}" stop-opacity=".1"/><stop offset="1" stop-color="{up}" stop-opacity="0"/></linearGradient>
<filter id="glow" x="-10%" y="-50%" width="120%" height="200%"><feGaussianBlur stdDeviation="1.4"/></filter>
</defs>
<path d="{}" fill="url(#down)"/><path d="{}" fill="url(#up)"/>
<path d="{}" fill="none" stroke="{down}" stroke-width="3" stroke-opacity=".45" filter="url(#glow)"/>
<path d="{}" fill="none" stroke="{down}" stroke-width="1.8" stroke-linejoin="round"/>
<path d="{}" fill="none" stroke="{up}" stroke-width="1.5" stroke-opacity=".65" stroke-linejoin="round"/>
</svg>"##,
        (width * scale).ceil() as u32,
        (HEIGHT * scale).ceil() as u32,
        chart.down_area,
        chart.up_area,
        chart.down,
        chart.down,
        chart.up
    )
}

pub fn rasterize(svg: &str) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    slint::Image::load_from_svg_data(svg.as_bytes())
        .ok()?
        .to_rgba8_premultiplied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_history_is_right_aligned_and_covers_the_whole_scroll_window() {
        let history = VecDeque::from([[100.0, 400.0], [200.0, 800.0]]);
        let pts = points(&history, 1, 800.0);
        assert_eq!(pts.len(), SAMPLES);
        assert!(pts[..59].iter().all(|p| p.1 == HEIGHT));
        assert!((pts[59].0 - WIDTH).abs() < 1e-9);
        assert!(
            pts[60].0 > WIDTH,
            "one sample enters from beyond the right edge"
        );
        assert_eq!(pts[60].1, 10.0);
        let chart = curves(&history, 1.0);
        assert_eq!(chart.down.matches(" C ").count(), 60);
        assert!(!chart.idle);
    }

    #[test]
    fn scale_decays_gently_and_invalid_samples_never_reach_the_renderer() {
        let history = VecDeque::from([[f64::NAN, f64::INFINITY], [-10.0, 0.0]]);
        let chart = curves(&history, 1000.0);
        assert!(chart.idle);
        assert_eq!(chart.peak, 940.0);
        assert!(!chart.down.contains("NaN"));
        assert!(!chart.up.contains("inf"));
        assert!(points(&history, 0, chart.peak)
            .iter()
            .all(|p| p.1 == HEIGHT));
    }

    #[test]
    fn chart_can_be_rasterized_off_the_ui_thread_at_desktop_dpi() {
        let chart = curves(&VecDeque::from([[1000.0, 2000.0]; SAMPLES]), 1.0);
        let svg = svg(&chart, 1.25, "#a0e8f0", "#73d5e3");
        std::thread::spawn(move || {
            let image = rasterize(&svg).expect("off-thread SVG rendering");
            assert_eq!(image.height(), 130);
            assert!(image.width() > 1250);
            assert!(image.as_slice().iter().any(|p| p.a > 0));
        })
        .join()
        .unwrap();
    }
}
