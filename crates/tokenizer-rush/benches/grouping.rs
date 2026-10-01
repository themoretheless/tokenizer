//! Compare the original reporting script with group_by; verify every group total.
use std::{collections::BTreeMap, hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};

fn main() {
    if cfg!(debug_assertions) {
        println!("Use cargo bench for optimized grouping measurements");
        return;
    }
    println!(
        "Rush grouping · {} {} · prepared run, 3 warmups, 7 alternating samples",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("rows\tgroups\talgorithm\tmin_us\tmedian_us\tmax_us");
    for (count, groups) in [(100, 10), (1000, 10), (1000, 100)] {
        let records = (0..count)
            .map(|i| format!("{{group:'g{}',amount:{}}}", i % groups, i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let prefix = format!("let rows = [{records}]\n");
        let original = format!(
            "{prefix}let groups = rows | fold([], (known,row) => if any(known, group => group == row.group) {{ known }} else {{ [known,[row.group]] | flat_map(xs => xs) }})\ngroups | map(group => {{group:group,total:rows | filter(row => row.group == group) | fold(0,(sum,row) => sum+row.amount)}})"
        );
        let grouped = format!(
            "{prefix}rows | group_by(row => row.group) | map(group => {{group:group.key,total:group.values | fold(0,(sum,row) => sum+row.amount)}})"
        );
        let programs = [
            Program::compile(&original).unwrap(),
            Program::compile(&grouped).unwrap(),
        ];
        let token = CancellationToken::default();
        let run = |index: usize| programs[index].run(100_000_000, &token, &[]).unwrap();
        let expected = Value::List(
            (0..groups)
                .map(|group| {
                    let total: usize = (group..count).step_by(groups).map(|i| i + 1).sum();
                    Value::Record(BTreeMap::from([
                        ("group".into(), Value::String(format!("g{group}"))),
                        ("total".into(), Value::Number(total as f64)),
                    ]))
                })
                .collect(),
        );
        for index in 0..2 {
            assert_eq!(
                run(index),
                expected,
                "algorithm {index}, rows {count}, groups {groups}"
            );
            for _ in 0..3 {
                black_box(run(index));
            }
        }
        let mut samples = [Vec::new(), Vec::new()];
        for round in 0..7 {
            for index in [round % 2, 1 - round % 2] {
                let start = Instant::now();
                black_box(run(index));
                samples[index].push(start.elapsed().as_secs_f64() * 1e6);
            }
        }
        for (name, mut samples) in ["script", "group_by"].into_iter().zip(samples) {
            samples.sort_by(f64::total_cmp);
            println!(
                "{count}\t{groups}\t{name}\t{:.3}\t{:.3}\t{:.3}",
                samples[0], samples[3], samples[6]
            );
        }
    }
}
