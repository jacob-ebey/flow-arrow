use flowarrow::build_file;
use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_LEN: usize = 512;
const DEFAULT_ITERATIONS: usize = 100;
const DEFAULT_SAMPLES: usize = 10;

fn main() {
    let config = Config::from_args(env::args().skip(1).collect());
    let left = vector_values(config.len, 0.25, 0.5);
    let right = vector_values(config.len, 1.5, 0.25);
    let expected = native_kernel(&left, &right, config.iterations);

    let root = temp_root();
    fs::create_dir_all(&root).expect("create bench temp dir");
    let source_path = root.join("vector_bench.flow");
    fs::write(
        &source_path,
        flowarrow_source(&left, &right, config.iterations, expected),
    )
    .expect("write FlowArrow benchmark source");

    let build_start = Instant::now();
    let build = build_file(&source_path, None).expect("build FlowArrow benchmark");
    let build_time = build_start.elapsed();

    run_flowarrow_once(&build.executable);
    black_box(native_kernel(&left, &right, config.iterations));

    let native_samples = sample(config.samples, || {
        black_box(native_kernel(
            black_box(&left),
            black_box(&right),
            black_box(config.iterations),
        ));
    });
    let flowarrow_samples = sample(config.samples, || run_flowarrow_once(&build.executable));

    println!("vector benchmark");
    println!("  len:        {}", config.len);
    println!("  iterations: {}", config.iterations);
    println!("  samples:    {}", config.samples);
    println!("  build:      {}", format_duration(build_time));
    println!();
    print_summary("rust native", &native_samples);
    print_summary("flowarrow", &flowarrow_samples);
    println!();
    println!(
        "  mean ratio: {:.2}x",
        mean(&flowarrow_samples).as_secs_f64() / mean(&native_samples).as_secs_f64()
    );
}

#[derive(Debug, Clone, Copy)]
struct Config {
    len: usize,
    iterations: usize,
    samples: usize,
}

impl Config {
    fn from_args(args: Vec<String>) -> Self {
        let mut config = Self {
            len: env_usize("FLOWARROW_BENCH_VECTOR_LEN", DEFAULT_LEN),
            iterations: env_usize("FLOWARROW_BENCH_ITERATIONS", DEFAULT_ITERATIONS),
            samples: env_usize("FLOWARROW_BENCH_SAMPLES", DEFAULT_SAMPLES),
        };

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--len" => {
                    index += 1;
                    config.len = parse_usize(args.get(index), "--len");
                }
                "--iterations" => {
                    index += 1;
                    config.iterations = parse_usize(args.get(index), "--iterations");
                }
                "--samples" => {
                    index += 1;
                    config.samples = parse_usize(args.get(index), "--samples");
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                "--bench" => {}
                other => {
                    eprintln!("unknown vector benchmark option `{other}`");
                    print_help();
                    std::process::exit(2);
                }
            }
            index += 1;
        }

        if config.len == 0 || config.iterations == 0 || config.samples == 0 {
            eprintln!("--len, --iterations, and --samples must be greater than zero");
            std::process::exit(2);
        }

        config
    }
}

fn print_help() {
    eprintln!(
        "usage: cargo bench --bench vector -- [--len N] [--iterations N] [--samples N]\n\
         env: FLOWARROW_BENCH_VECTOR_LEN, FLOWARROW_BENCH_ITERATIONS, FLOWARROW_BENCH_SAMPLES"
    );
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{name} must be a positive integer"))
        })
        .unwrap_or(default)
}

fn parse_usize(value: Option<&String>, flag: &str) -> usize {
    value
        .unwrap_or_else(|| {
            eprintln!("{flag} requires a value");
            std::process::exit(2);
        })
        .parse::<usize>()
        .unwrap_or_else(|_| {
            eprintln!("{flag} requires a positive integer");
            std::process::exit(2);
        })
}

fn vector_values(len: usize, offset: f64, step: f64) -> Vec<f64> {
    (0..len)
        .map(|index| offset + (index as f64 + 1.0) * step)
        .collect()
}

fn native_kernel(left: &[f64], right: &[f64], iterations: usize) -> f64 {
    let mut score = 0.0;
    for _ in 0..iterations {
        let mut dot = 0.0;
        let mut squared_distance = 0.0;
        let mut squared_norm = 0.0;
        for (&a, &b) in left.iter().zip(right) {
            dot += a * b;
            let delta = a - b;
            squared_distance += delta * delta;
            squared_norm += a * a;
        }
        score += dot + squared_distance + squared_norm;
    }
    score
}

fn flowarrow_source(left: &[f64], right: &[f64], iterations: usize, expected: f64) -> String {
    format!(
        r#"
import std.cli {{ Args }}
import std.math {{ add as scalar_add, eq }}
import std.vector {{ dot, squared_distance, squared_norm }}

node kernel(left: Seq[Real], right: Seq[Real], score: Real) -> (out_left: Seq[Real], out_right: Seq[Real], out_score: Real) {{
    ($left, $right) -> dot -> $dot
    ($left, $right) -> squared_distance -> $distance_squared
    $left -> squared_norm -> $norm_squared
    ($dot, $distance_squared) -> scalar_add -> $partial
    ($partial, $norm_squared) -> scalar_add -> $delta
    ($score, $delta) -> scalar_add -> $out_score
    $left -> $out_left
    $right -> $out_right
}}

node final_score(left: Seq[Real], right: Seq[Real], score: Real) -> out: Real {{
    $score -> $out
}}

program main(args: Args) -> exit_code: Int {{
    {} -> $left
    {} -> $right
    ($left, $right, 0.0) -> repeat<{iterations}> kernel -> final_score -> $score
    ($score, {}) -> eq -> $ok
    ($ok, 0, 1) -> select -> $exit_code
}}
"#,
        flow_seq(left),
        flow_seq(right),
        flow_real(expected),
    )
}

fn flow_seq(values: &[f64]) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&flow_real(*value));
    }
    out.push(']');
    out
}

fn flow_real(value: f64) -> String {
    format!("{value:.17}")
}

fn run_flowarrow_once(executable: &PathBuf) {
    let status = Command::new(executable)
        .status()
        .expect("run FlowArrow benchmark executable");
    assert!(
        status.success(),
        "FlowArrow benchmark executable failed with {status}"
    );
}

fn sample(mut samples: usize, mut run: impl FnMut()) -> Vec<Duration> {
    let mut durations = Vec::with_capacity(samples);
    while samples > 0 {
        let start = Instant::now();
        run();
        durations.push(start.elapsed());
        samples -= 1;
    }
    durations
}

fn print_summary(label: &str, samples: &[Duration]) {
    println!(
        "  {label:<11} mean={} min={} max={}",
        format_duration(mean(samples)),
        format_duration(*samples.iter().min().expect("samples")),
        format_duration(*samples.iter().max().expect("samples")),
    );
}

fn mean(samples: &[Duration]) -> Duration {
    let nanos = samples.iter().map(Duration::as_nanos).sum::<u128>() / samples.len() as u128;
    Duration::from_nanos(nanos.try_into().unwrap_or(u64::MAX))
}

fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos < 1_000 {
        format!("{nanos}ns")
    } else if nanos < 1_000_000 {
        format!("{:.2}us", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2}ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.2}s", nanos as f64 / 1_000_000_000.0)
    }
}

fn temp_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    env::temp_dir().join(format!(
        "flowarrow-vector-bench-{}-{unique}",
        std::process::id()
    ))
}
