//! Performance benchmarks for the large-working-copy hot paths.
//!
//! Run with: `cargo bench` (or `cargo bench -- --quick` for a fast pass).
//! CI guards the same paths with timed tests (see `perf_tests` modules).

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use std::hint::black_box;
use std::path::Path;
use svnui::components::status_tree::StatusTreeComponent;
use svnui::components::{Context, DrawableComponent};
use svnui::queue::Queue;
use svnui::svn::parser::{parse_diff, parse_status};
use svnui::test_support::gen_status_entries;
use svnui::ui::style::Theme;

fn ctx() -> Context {
    Context {
        queue: Queue::new(),
        theme: Theme::default(),
    }
}

fn bench_status_tree(c: &mut Criterion) {
    let mut group = c.benchmark_group("status_tree");
    for (label, n, wide) in [
        ("wide_10k", 10_000, true),
        ("wide_100k", 100_000, true),
        ("deep_100k", 100_000, false),
    ] {
        let entries = gen_status_entries(n, wide);
        group.bench_function(format!("update_{label}"), |b| {
            let mut comp = StatusTreeComponent::new(&ctx());
            // clone the entries in the untimed setup so the measurement
            // only covers `update` itself
            b.iter_batched(
                || entries.clone(),
                |entries| {
                    comp.update(entries);
                    black_box(comp.visible_len());
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("draw_{label}"), |b| {
            let mut comp = StatusTreeComponent::new(&ctx());
            comp.update(entries.clone());
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            b.iter(|| {
                terminal
                    .draw(|f| comp.draw(f, Rect::new(0, 0, 120, 40)).unwrap())
                    .unwrap();
            });
        });
    }
    group.finish();
}

fn bench_parsers(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsers");

    let mut status_out = String::with_capacity(100_000 * 24);
    for i in 0..100_000 {
        status_out.push_str(&format!("M       src/file_{i:06}.rs\n"));
    }
    group.bench_function("parse_status_100k", |b| {
        b.iter(|| {
            black_box(parse_status(
                black_box(&status_out),
                black_box(Path::new("/")),
            ))
        })
    });

    let mut diff_out = String::with_capacity(50_000 * 16);
    diff_out.push_str("Index: big.rs\n===\n@@ -1 +1,50000 @@\n");
    for i in 0..50_000 {
        diff_out.push_str(&format!("+line {i}\n"));
    }
    group.bench_function("parse_diff_50k", |b| {
        b.iter(|| black_box(parse_diff(black_box(&diff_out))))
    });
    group.finish();
}

criterion_group!(benches, bench_status_tree, bench_parsers);
criterion_main!(benches);
