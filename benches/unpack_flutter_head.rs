use criterion::{black_box, criterion_group, criterion_main, Criterion};
use anni_fetch::{Client, Pack};
use anni_fetch::client::Message::PackData;
use std::io::Cursor;

fn unpack() {
    let client = Client::new("https://github.com/flutter/flutter.git");
    let iter = client.request("fetch", None, &[
        "thin-pack",
        "ofs-delta",
        "deepen 1",
        &client.want_ref("HEAD").expect("failed to get sha1 of HEAD"),
        "done"
    ]).unwrap();
    let mut pack = Vec::new();
    for msg in iter {
        match msg {
            PackData(mut d) => pack.append(&mut d),
            _ => {}
        }
    }
    let mut cursor = Cursor::new(pack);
    Pack::from_reader(&mut cursor).expect("invalid pack file");
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("unpack");
    group.significance_level(0.1).sample_size(10);
    group.bench_function("unpack", |b| b.iter(|| unpack()));
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);