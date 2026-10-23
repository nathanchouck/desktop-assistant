//! Search latency with the real providers, built the way the app builds
//! them, over a synthetic file index. Ignored by default (it prints numbers
//! rather than checking them):
//!
//! `cargo test latency -- --ignored --nocapture`
//!
//! Debug builds are several times slower than release, so treat the
//! numbers as an upper bound. Off Windows there are no apps, Everything or
//! media, so this covers the in-process providers and the built-in index.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ai::ProviderId;
use crate::paths::AppPaths;
use crate::search::{AskRow, Frecency, PROVIDER_BUDGET};
use crate::settings::Settings;
use crate::state::AppState;

const RUNS: usize = 20;
const FOLDERS: usize = 200;
const FILES_PER_FOLDER: usize = 100;

const QUERIES: [&str; 12] = [
    "fire",
    "2+2*3",
    "5 mi to km",
    ":smile",
    "clip",
    "kill chr",
    "restart",
    "yt lofi",
    "settings",
    "report 17",
    "what is the tallest mountain",
    "ext:pdf orbit",
];

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "prints timings; run by hand"]
async fn search_latency_with_real_providers() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    for f in 0..FOLDERS {
        let folder = home.join(format!("Project {f}"));
        std::fs::create_dir_all(&folder).unwrap();
        for i in 0..FILES_PER_FOLDER {
            let ext = ["pdf", "docx", "txt", "png"][i % 4];
            std::fs::write(folder.join(format!("report {i} orbit notes.{ext}")), b"").unwrap();
        }
    }
    let paths = AppPaths::with_roots(&dir.path().join("config"), &dir.path().join("data"));
    let mut settings = Settings::default();
    settings.search.index_folders = vec![home.display().to_string()];

    let built = Instant::now();
    let state = AppState::new(paths, settings);
    println!("app state (databases, providers): {:?}", built.elapsed());

    let indexed = Instant::now();
    state.files.ensure_started();
    while state.files.status().incomplete || state.files.status().scanning {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    println!(
        "file index: {} entries in {:?}",
        state.files.status().entries,
        indexed.elapsed()
    );

    let store = Arc::clone(&state.store);
    let frecency: Frecency = Arc::new(move |id| store.frecency(id));
    let ask = AskRow::Ask(ProviderId::Claude);
    let mut all = Vec::new();
    for query in QUERIES {
        let mut times = Vec::with_capacity(RUNS);
        let mut late_times = Vec::new();
        for _ in 0..RUNS {
            let started = Instant::now();
            let outcome = state
                .search
                .search(query, 8, ask, Arc::clone(&frecency))
                .await;
            times.push(started.elapsed());
            if let Some(late) = outcome.complete {
                late.await;
                late_times.push(started.elapsed());
            }
        }
        times.sort();
        late_times.sort();
        let late = if late_times.is_empty() {
            String::new()
        } else {
            format!(
                "   all results p50 {:.1} ms",
                percentile(&late_times, 0.5).as_secs_f64() * 1000.0
            )
        };
        println!(
            "{query:<32} p50 {:>6.1} ms   p95 {:>6.1} ms{late}",
            percentile(&times, 0.5).as_secs_f64() * 1000.0,
            percentile(&times, 0.95).as_secs_f64() * 1000.0
        );
        all.extend(times);
    }
    all.sort();
    let p95 = percentile(&all, 0.95);
    println!(
        "all queries: p50 {:.1} ms, p95 {:.1} ms (budget {:?} for providers)",
        percentile(&all, 0.5).as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        PROVIDER_BUDGET
    );
}
