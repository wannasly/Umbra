//! Isolated motion scenes and frame timings. Compiled only with smoke-test.
use super::*;
use std::{cell::Cell, time::Instant};

fn sample(index: usize) -> [f64; 2] {
    let t = index as f64;
    let down = 13000.0
        + 9000.0 * (t * 0.41).sin().powi(2)
        + 180000.0 * ((t * 0.19).cos().max(0.0)).powi(16);
    [down * (0.63 + 0.18 * (t * 0.73).sin()), down]
}

pub(super) fn seed_chart(ui: &MainWindow) {
    UI.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.history = (0..crate::chart::SAMPLES).map(sample).collect();
            state.update_chart(ui);
        }
    });
}

pub(super) fn start(ui: &MainWindow) -> Vec<slint::Timer> {
    let scene = std::env::var("UMBRA_BENCH_SCENE").unwrap_or_else(|_| "dashboard".into());
    assert!(["dashboard", "pages", "resize"].contains(&scene.as_str()));
    let seconds = std::env::var("UMBRA_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(12)
        .clamp(4, 90);
    seed_chart(ui);
    let start = Instant::now();
    let previous = Rc::new(Cell::new(None::<(Instant, bool)>));
    let intervals = Rc::new(RefCell::new(Vec::<f64>::new()));
    let recorded = intervals.clone();
    let weak = ui.as_weak();
    ui.window()
        .set_rendering_notifier(move |rendering, _| {
            if !matches!(rendering, slint::RenderingState::AfterRendering) {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                let now = Instant::now();
                if let Some((before, active)) =
                    previous.replace(Some((now, ui.window().has_active_animations())))
                {
                    if active && start.elapsed() > Duration::from_secs(2) {
                        recorded
                            .borrow_mut()
                            .push(now.duration_since(before).as_secs_f64() * 1000.0);
                    }
                }
            }
        })
        .expect("frame timing notifier");

    let traffic = slint::Timer::default();
    let index = Cell::new(crate::chart::SAMPLES);
    let weak = ui.as_weak();
    traffic.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(ui) = weak.upgrade() {
                UI.with(|state| {
                    if let Some(state) = state.borrow_mut().as_mut() {
                        state.history.pop_front();
                        state.history.push_back(sample(index.get()));
                        index.set(index.get() + 1);
                        state.update_chart(&ui);
                    }
                });
            }
        },
    );

    let navigation = slint::Timer::default();
    let step = Cell::new(0);
    let weak = ui.as_weak();
    let scene_for_timer = scene.clone();
    if scene != "dashboard" {
        navigation.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(600),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let i = step.get();
                    step.set(i + 1);
                    if scene_for_timer == "pages" {
                        ui.global::<App>().set_page(i % 6);
                    } else {
                        let sizes = [
                            (1100.0, 720.0),
                            (720.0, 500.0),
                            (480.0, 960.0),
                            (1280.0, 800.0),
                        ];
                        let (width, height) = sizes[i as usize % sizes.len()];
                        ui.window().set_size(slint::LogicalSize::new(width, height));
                    }
                }
            },
        );
    }
    let finish = slint::Timer::default();
    let report =
        std::path::PathBuf::from(std::env::var_os("UMBRA_DATA_DIR").expect("isolated data"))
            .join("frame-times.json");
    finish.start(
        slint::TimerMode::SingleShot,
        Duration::from_secs(seconds),
        move || {
            let mut values = intervals.borrow().clone();
            values.sort_by(f64::total_cmp);
            let percentile = |p: f64| {
                values
                    .get(((values.len().saturating_sub(1)) as f64 * p).round() as usize)
                    .copied()
                    .unwrap_or(0.0)
            };
            let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
            let result = json!({
                "scene":scene,"build":if cfg!(debug_assertions) {"debug"} else {"release"},
                "frames":values.len(),"fps":1000.0/mean.max(0.001),
                "median_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),
                "over_25ms":values.iter().filter(|v| **v>25.0).count(),
                "intervals_ms":values
            });
            std::fs::write(&report, serde_json::to_vec_pretty(&result).unwrap())
                .expect("frame report");
            eprintln!(
                "MOTION: {} frames; {:.1} fps; p95 {:.2} ms; {}",
                values.len(),
                1000.0 / mean.max(0.001),
                percentile(0.95),
                report.display()
            );
            let _ = slint::quit_event_loop();
        },
    );
    vec![traffic, navigation, finish]
}
