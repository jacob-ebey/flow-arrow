use flowarrow::{BuildOptions, build_file_with_options};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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
    let flowarrow_source = flowarrow_source(&left, &right, config.iterations, expected);
    let flowarrow_cpu_source_path = root.join("vector_bench_cpu.flow");
    let flowarrow_gpu_source_path = root.join("vector_bench_gpu.flow");
    fs::write(&flowarrow_cpu_source_path, &flowarrow_source)
        .expect("write CPU FlowArrow benchmark source");
    fs::write(&flowarrow_gpu_source_path, &flowarrow_source)
        .expect("write GPU FlowArrow benchmark source");
    let rust_project_root = root.join("rust_vector_bench");
    let rust_package_name = make_rust_package_name("flowarrow-vector-rust-bench", &root);
    fs::create_dir_all(rust_project_root.join("src")).expect("create Rust benchmark project");
    fs::write(
        rust_project_root.join("Cargo.toml"),
        rust_manifest(&rust_package_name),
    )
    .expect("write Rust benchmark manifest");
    fs::write(
        rust_project_root.join("src/main.rs"),
        rust_source(&left, &right, config.iterations, expected),
    )
    .expect("write Rust benchmark source");

    let flowarrow_cpu_build_start = Instant::now();
    eprintln!("building FlowArrow CPU benchmark...");
    let flowarrow_cpu_build =
        build_file_with_options(&flowarrow_cpu_source_path, &BuildOptions::default())
            .expect("build CPU FlowArrow benchmark");
    let flowarrow_cpu_build_time = flowarrow_cpu_build_start.elapsed();

    let flowarrow_gpu_build = if config.gpu {
        let gpu_options = BuildOptions {
            gpu: true,
            ..BuildOptions::default()
        };
        let build_start = Instant::now();
        eprintln!("building FlowArrow GPU benchmark...");
        let build = build_file_with_options(&flowarrow_gpu_source_path, &gpu_options)
            .expect("build GPU FlowArrow benchmark");
        Some((build, build_start.elapsed()))
    } else {
        None
    };

    let rust_build_start = Instant::now();
    eprintln!("building Rust baseline benchmark...");
    let rust_executable = build_rust_executable(&rust_project_root, &rust_package_name);
    let rust_build_time = rust_build_start.elapsed();

    run_executable_once(&rust_executable, "Rust benchmark executable");
    run_executable_once(
        &flowarrow_cpu_build.executable,
        "FlowArrow CPU benchmark executable",
    );
    if let Some((build, _)) = &flowarrow_gpu_build {
        run_executable_once(&build.executable, "FlowArrow GPU benchmark executable");
    }

    let rust_samples = sample("rust executable", config.samples, || {
        run_executable_once(&rust_executable, "Rust benchmark executable")
    });
    let flowarrow_cpu_samples = sample("FlowArrow CPU executable", config.samples, || {
        run_executable_once(
            &flowarrow_cpu_build.executable,
            "FlowArrow CPU benchmark executable",
        )
    });
    let flowarrow_gpu_samples = flowarrow_gpu_build.as_ref().map(|(build, _)| {
        sample("FlowArrow GPU executable", config.gpu_samples, || {
            run_executable_once(&build.executable, "FlowArrow GPU benchmark executable")
        })
    });

    println!("vector benchmark");
    println!("  len:        {}", config.len);
    println!("  iterations: {}", config.iterations);
    println!("  samples:    {}", config.samples);
    if config.gpu {
        println!("  GPU samples: {}", config.gpu_samples);
    }
    println!("  rust build:     {}", format_duration(rust_build_time));
    println!(
        "  flow CPU build: {}",
        format_duration(flowarrow_cpu_build_time)
    );
    if let Some((_, build_time)) = &flowarrow_gpu_build {
        println!("  flow GPU build: {}", format_duration(*build_time));
    } else {
        println!("  flow GPU build: disabled; pass --gpu to include it");
    }
    println!();
    print_summary("rust exe", &rust_samples);
    print_summary("flow CPU", &flowarrow_cpu_samples);
    if let Some(samples) = &flowarrow_gpu_samples {
        print_summary("flow GPU", samples);
    }
    println!();
    println!(
        "  flow CPU / rust: {:.2}x",
        mean(&flowarrow_cpu_samples).as_secs_f64() / mean(&rust_samples).as_secs_f64()
    );
    if let Some(samples) = &flowarrow_gpu_samples {
        println!(
            "  flow GPU / rust: {:.2}x",
            mean(samples).as_secs_f64() / mean(&rust_samples).as_secs_f64()
        );
        println!(
            "  flow GPU / CPU:  {:.2}x",
            mean(samples).as_secs_f64() / mean(&flowarrow_cpu_samples).as_secs_f64()
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct Config {
    len: usize,
    iterations: usize,
    samples: usize,
    gpu_samples: usize,
    gpu: bool,
}

impl Config {
    fn from_args(args: Vec<String>) -> Self {
        let mut config = Self {
            len: env_usize("FLOWARROW_BENCH_VECTOR_LEN", DEFAULT_LEN),
            iterations: env_usize("FLOWARROW_BENCH_ITERATIONS", DEFAULT_ITERATIONS),
            samples: env_usize("FLOWARROW_BENCH_SAMPLES", DEFAULT_SAMPLES),
            gpu_samples: env_usize("FLOWARROW_BENCH_GPU_SAMPLES", 1),
            gpu: env_bool("FLOWARROW_BENCH_GPU", false),
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
                "--gpu-samples" => {
                    index += 1;
                    config.gpu_samples = parse_usize(args.get(index), "--gpu-samples");
                }
                "--gpu" => {
                    config.gpu = true;
                }
                "--no-gpu" => {
                    config.gpu = false;
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

        if config.len == 0
            || config.iterations == 0
            || config.samples == 0
            || config.gpu_samples == 0
        {
            eprintln!(
                "--len, --iterations, --samples, and --gpu-samples must be greater than zero"
            );
            std::process::exit(2);
        }

        config
    }
}

fn print_help() {
    eprintln!(
        "usage: cargo bench --bench vector -- [--gpu] [--gpu-samples N] [--len N] [--iterations N] [--samples N]\n\
         env: FLOWARROW_BENCH_GPU=1, FLOWARROW_BENCH_GPU_SAMPLES, FLOWARROW_BENCH_VECTOR_LEN, FLOWARROW_BENCH_ITERATIONS, FLOWARROW_BENCH_SAMPLES"
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

fn env_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(value) => match value.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON" => true,
            "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF" => false,
            _ => panic!("{name} must be a boolean"),
        },
        Err(_) => default,
    }
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
import std.math {{ add_f64 as scalar_add, eq_f64 as eq }}
import std.vector {{ dot_f64, squared_distance_f64, squared_norm_f64 }}

node kernel(left: Seq[f64], right: Seq[f64], score: f64) -> (out_left: Seq[f64], out_right: Seq[f64], out_score: f64) {{
    ($left, $right) -> dot_f64 -> $dot
    ($left, $right) -> squared_distance_f64 -> $distance_squared
    $left -> squared_norm_f64 -> $norm_squared
    ($dot, $distance_squared) -> scalar_add -> $partial
    ($partial, $norm_squared) -> scalar_add -> $delta
    ($score, $delta) -> scalar_add -> $out_score
    $left -> $out_left
    $right -> $out_right
}}

node final_score(left: Seq[f64], right: Seq[f64], score: f64) -> out: f64 {{
    $score -> $out
}}

program main(args: Args) -> exit_code: i64 {{
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

fn rust_source(left: &[f64], right: &[f64], iterations: usize, expected: f64) -> String {
    format!(
        r#"
use std::hint::black_box;

static LEFT: &[f64] = &{};
static RIGHT: &[f64] = &{};

fn kernel(left: &[f64], right: &[f64], iterations: usize) -> f64 {{
    let mut score = 0.0;
    for _ in 0..iterations {{
        let mut dot = 0.0;
        let mut squared_distance = 0.0;
        let mut squared_norm = 0.0;
        for (&a, &b) in black_box(left).iter().zip(black_box(right)) {{
            dot += a * b;
            let delta = a - b;
            squared_distance += delta * delta;
            squared_norm += a * a;
        }}
        score += black_box(dot) + black_box(squared_distance) + black_box(squared_norm);
    }}
    score
}}

fn is_close(actual: f64, expected: f64) -> bool {{
    let tolerance = 1e-9 * expected.abs().max(1.0);
    (actual - expected).abs() <= tolerance
}}

fn main() {{
    let score = kernel(
        black_box(LEFT),
        black_box(RIGHT),
        black_box({iterations}usize),
    );
    let expected = black_box({});
    std::process::exit(if is_close(score, expected) {{ 0 }} else {{ 1 }});
}}
"#,
        rust_slice(left),
        rust_slice(right),
        flow_real(expected),
    )
}

fn rust_manifest(package_name: &str) -> String {
    format!(
        r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"

[profile.release]
opt-level = 3
debug = false
lto = "thin"
codegen-units = 1
panic = "abort"
"#
    )
}

fn make_rust_package_name(prefix: &str, root: &Path) -> String {
    let suffix = root
        .file_name()
        .expect("bench temp dir name")
        .to_string_lossy()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>();
    format!("{prefix}-{suffix}")
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

fn rust_slice(values: &[f64]) -> String {
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

fn build_rust_executable(project_root: &PathBuf, package_name: &str) -> PathBuf {
    let target_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/flowarrow-rust-benches");
    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(project_root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("invoke cargo for Rust benchmark executable");
    assert!(
        output.status.success(),
        "cargo build failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    target_dir
        .join("release")
        .join(format!("{package_name}{}", std::env::consts::EXE_SUFFIX))
}

fn run_executable_once(executable: &PathBuf, label: &str) {
    let status = Command::new(executable)
        .status()
        .unwrap_or_else(|error| panic!("run {label}: {error}"));
    assert!(status.success(), "{label} failed with {status}");
}

fn sample(label: &str, samples: usize, mut run: impl FnMut()) -> Vec<Duration> {
    let mut durations = Vec::with_capacity(samples);
    for index in 0..samples {
        eprintln!("running {label} sample {}/{}...", index + 1, samples);
        let start = Instant::now();
        run();
        durations.push(start.elapsed());
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
