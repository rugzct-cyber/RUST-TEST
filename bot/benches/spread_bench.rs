use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hft_bot::adapters::types::{Orderbook, OrderbookLevel};
use hft_bot::core::spread::SpreadCalculator;

fn make_orderbook(levels: usize, base_ask: f64, base_bid: f64) -> Orderbook {
    let mut ob = Orderbook::new();
    for i in 0..levels {
        let offset = i as f64 * 0.01;
        ob.asks.push(OrderbookLevel::new(base_ask + offset, 1.0));
        ob.bids.push(OrderbookLevel::new(base_bid - offset, 1.0));
    }
    ob
}

fn bench_spread_calc_simple(c: &mut Criterion) {
    c.bench_function("spread_calc_simple", |b| {
        let ob_a = make_orderbook(1, 42000.0, 41990.0);
        let ob_b = make_orderbook(1, 42005.0, 41985.0);
        let calc = SpreadCalculator::new(0.30, 0.10, "A".into(), "B".into());

        b.iter(|| {
            black_box(calc.calculate(black_box(&ob_a), black_box(&ob_b)));
        });
    });
}

fn bench_spread_calc_10_levels(c: &mut Criterion) {
    c.bench_function("spread_calc_10_levels", |b| {
        let ob_a = make_orderbook(10, 42000.0, 41990.0);
        let ob_b = make_orderbook(10, 42005.0, 41985.0);
        let calc = SpreadCalculator::new(0.30, 0.10, "A".into(), "B".into());

        b.iter(|| {
            black_box(calc.calculate(black_box(&ob_a), black_box(&ob_b)));
        });
    });
}

fn bench_entry_spread(c: &mut Criterion) {
    c.bench_function("entry_spread", |b| {
        b.iter(|| {
            black_box(SpreadCalculator::calculate_entry_spread(
                black_box(42000.0),
                black_box(42150.0),
            ));
        });
    });
}

fn bench_exit_spread(c: &mut Criterion) {
    c.bench_function("exit_spread", |b| {
        b.iter(|| {
            black_box(SpreadCalculator::calculate_exit_spread(
                black_box(42150.0),
                black_box(42000.0),
            ));
        });
    });
}

criterion_group!(
    benches,
    bench_spread_calc_simple,
    bench_spread_calc_10_levels,
    bench_entry_spread,
    bench_exit_spread
);
criterion_main!(benches);
