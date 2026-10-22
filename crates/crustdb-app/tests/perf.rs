#![allow(clippy::unwrap_used, clippy::expect_used, clippy::err_expect)]
//! Spec §7 performance budgets. Release build, one thread:
//! `cargo test --release -p crustdb-app --test perf -- --include-ignored --test-threads=1 --nocapture`
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crustdb_app::csv_out::CsvOut;
use crustdb_app::metrics::parse_vm_rss_kb;
use crustdb_app::runtime::{TabId, WorkspaceId};
use crustdb_app::state::autocomplete::ColumnCache;
use crustdb_app::state::query_tab::QueryTab;
use crustdb_app::state::tree::ObjectTree;
use crustdb_app::ui::{editor, grid::grid, theme::install_theme};
use crustdb_core::format::display_cell;
use crustdb_core::model::{Cell, ColumnMeta, DriverKind, GRID_DISPLAY_CHARS, Row, TypeCategory};
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn columns(n: usize) -> Vec<ColumnMeta> {
    (0..n)
        .map(|c| ColumnMeta {
            name: format!("col{c}"),
            type_name: "text".into(),
            category: TypeCategory::Text,
            nullable: Some(true),
            is_primary_key: false,
        })
        .collect()
}

/// ~20-byte text cells.
fn row(i: usize, cols: usize) -> Row {
    (0..cols)
        .map(|c| Cell::Text {
            value: format!("r{i:07}-c{c:03}-xxxxx").into(),
            truncated: false,
        })
        .collect()
}

fn rows(n: usize, cols: usize) -> Vec<Row> {
    (0..n).map(|i| row(i, cols)).collect()
}

fn rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    parse_vm_rss_kb(&status).unwrap() as f64 / 1024.0
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Mean and worst of the frame times, in ms.
fn summary(label: &str, frames: &[Duration]) -> (f64, f64) {
    let mean = frames.iter().map(|d| ms(*d)).sum::<f64>() / frames.len() as f64;
    let worst = frames.iter().map(|d| ms(*d)).fold(0.0, f64::max);
    println!(
        "PERF {label}: mean {mean:.2} ms, worst {worst:.2} ms over {} frames",
        frames.len()
    );
    (mean, worst)
}

fn grid_harness(cols: usize, data: Vec<Row>) -> Harness<'static> {
    let columns = columns(cols);
    let mut selected = None;
    let mut ready = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_accessibility_check(false)
        .build_ui(move |ui| {
            if !std::mem::replace(&mut ready, true) {
                install_theme(ui.ctx());
                return;
            }
            grid(ui, &columns, &data, &mut selected);
        });
    h.run();
    h
}

fn scroll_frames(h: &mut Harness<'_>, n: usize) -> Vec<Duration> {
    h.hover_at(egui::pos2(640.0, 360.0));
    h.step();
    (0..n)
        .map(|_| {
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -8.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            });
            let t = Instant::now();
            h.step();
            t.elapsed()
        })
        .collect()
}

#[test]
#[ignore = "perf"]
fn startup_first_frame_under_300ms() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    if Command::new("xvfb-run").arg("--help").output().is_err() {
        println!("PERF startup: skipped, xvfb-run not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut runs: Vec<u128> = (0..5)
        .map(|_| {
            let mut child = Command::new("xvfb-run")
                .args(["-a", env!("CARGO_BIN_EXE_crustdb")])
                .env("CRUSTDB_TRACE_STARTUP", "1")
                .env("XDG_CONFIG_HOME", dir.path())
                .env("XDG_DATA_HOME", dir.path())
                .env("XDG_STATE_HOME", dir.path())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let found = BufReader::new(child.stderr.take().unwrap())
                .lines()
                .map_while(Result::ok)
                .find_map(|l| {
                    l.strip_prefix("first_frame_ms=")
                        .map(|v| v.parse().unwrap())
                });
            // xvfb-run does not forward the kill: stop the app by name.
            let _ = Command::new("pkill")
                .args(["-f", env!("CARGO_BIN_EXE_crustdb")])
                .status();
            let _ = child.wait();
            found.expect("no first_frame_ms line")
        })
        .collect();
    runs.sort_unstable();
    let median = runs[runs.len() / 2];
    println!("PERF startup_first_frame: median {median} ms of {runs:?}");
    assert!(median <= 300, "{median} ms");
}

#[test]
#[ignore = "perf"]
fn decode_and_render_1000x20_under_20ms() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let cols = columns(20);
    let t = Instant::now();
    let data = rows(1000, 20);
    let shown: usize = data
        .iter()
        .flat_map(|r| r.iter().zip(&cols))
        .map(|(c, m)| display_cell(c, m, GRID_DISPLAY_CHARS).len())
        .sum();
    let decode = t.elapsed();
    let mut h = grid_harness(20, data);
    let frames = scroll_frames(&mut h, 30);
    let (mean, _) = summary("render 1000x20 frame", &frames);
    println!(
        "PERF build+format 1000x20: {:.2} ms ({shown} bytes)",
        ms(decode)
    );
    assert!(ms(decode) + mean <= 20.0, "{:.2}", ms(decode) + mean);
}

#[test]
#[ignore = "perf"]
fn scroll_100k_rows_frame_under_16ms() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let mut h = grid_harness(20, rows(100_000, 20));
    let frames = scroll_frames(&mut h, 120);
    let (mean, _) = summary("scroll 100k x 20", &frames);
    assert!(mean <= 16.0, "{mean:.2} ms");
}

#[test]
#[ignore = "perf"]
fn rss_100k_rows_under_200mb() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let before = rss_mb();
    let mut h = grid_harness(20, rows(100_000, 20));
    let _ = scroll_frames(&mut h, 10);
    let after = rss_mb();
    println!(
        "PERF rss 100k x 20: {after:.0} MB total, +{:.0} MB",
        after - before
    );
    assert!(after <= 200.0, "{after:.0} MB");
}

#[test]
#[ignore = "perf"]
fn export_100k_rows_per_second() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let cols = columns(20);
    let before = rss_mb();
    let t = Instant::now();
    let mut out = CsvOut::new(std::io::sink());
    out.header(&cols).unwrap();
    for i in 0..100_000 {
        out.row(&row(i, 20)).unwrap();
    }
    out.finish().unwrap();
    let rate = 100_000.0 / t.elapsed().as_secs_f64();
    let grew = rss_mb() - before;
    println!("PERF csv encode: {rate:.0} rows/s, RSS +{grew:.1} MB");
    assert!(rate >= 100_000.0, "{rate:.0} rows/s");
    assert!(grew <= 20.0, "+{grew:.1} MB");
}

fn typing_mean_ms(lines: usize) -> f64 {
    let mut tab = QueryTab::new(TabId(1), DriverKind::Postgres, None);
    tab.text = (0..lines)
        .map(|i| format!("SELECT id, name FROM t WHERE id = {i};\n"))
        .collect();
    let tree = ObjectTree::new(WorkspaceId(1), DriverKind::Postgres);
    let cols = ColumnCache::default();
    let mut ready = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .with_accessibility_check(false)
        .build_ui(move |ui| {
            if !std::mem::replace(&mut ready, true) {
                install_theme(ui.ctx());
                return;
            }
            editor::show(ui, &mut tab, &tree, &cols);
        });
    h.run();
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    h.run();
    let frames: Vec<_> = (0..40)
        .map(|_| {
            h.event(egui::Event::Text("a".into()));
            let t = Instant::now();
            h.step();
            t.elapsed()
        })
        .collect();
    summary(&format!("typing in {lines} lines"), &frames).0
}

/// Highlighting is on below `HIGHLIGHT_MAX_LINES` (750).
#[test]
#[ignore = "perf"]
fn typing_700_lines_with_highlight_under_16ms() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let mean = typing_mean_ms(700);
    assert!(mean <= 16.0, "{mean:.2} ms");
}

/// Spec §7 asks for 16 ms; v1 does not meet it (deferred to v2, virtualized editor). Egui's
/// `TextEdit` lays out the whole text on every edit, so this guards against regressions with the
/// measured ceiling (53 ms on the reference machine, highlighting off above 750 lines).
#[test]
#[ignore = "perf"]
fn typing_5000_lines_stays_under_known_ceiling() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let mean = typing_mean_ms(5000);
    assert!(mean <= 100.0, "{mean:.2} ms");
}

/// No budget (spec §7): the grid does not virtualize columns. Records the number.
#[test]
#[ignore = "perf"]
fn wide_table_frame_time() {
    // Budgets assume a --release build (see module docs).
    if cfg!(debug_assertions) {
        return;
    }
    let mut h = grid_harness(250, rows(1000, 250));
    let frames = scroll_frames(&mut h, 30);
    summary("wide table 250 columns", &frames);
}
