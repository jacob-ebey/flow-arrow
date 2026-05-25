use flowarrow::{BuildOptions, build_file_with_options};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_ROWS: usize = 16;
const DEFAULT_INNER: usize = 16;
const DEFAULT_COLS: usize = 16;
const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_SAMPLES: usize = 10;

fn main() {
    let config = Config::from_args(env::args().skip(1).collect());
    let left = matrix_values(config.rows, config.inner, 1);
    let right = matrix_values(config.inner, config.cols, 2);
    let vector = vector_values(config.inner, 3);
    let expected = native_kernel(
        &left,
        &right,
        &vector,
        config.rows,
        config.inner,
        config.cols,
        config.iterations,
    );

    let root = temp_root();
    fs::create_dir_all(&root).expect("create bench temp dir");
    let flowarrow_source = flowarrow_source(&left, &right, &vector, config, expected);
    let flowarrow_cpu_source_path = root.join("matrix_bench_cpu.flow");
    let flowarrow_gpu_source_path = root.join("matrix_bench_gpu.flow");
    fs::write(&flowarrow_cpu_source_path, &flowarrow_source)
        .expect("write CPU FlowArrow benchmark source");
    fs::write(&flowarrow_gpu_source_path, &flowarrow_source)
        .expect("write GPU FlowArrow benchmark source");
    let rust_project_root = root.join("rust_matrix_bench");
    let rust_package_name = make_rust_package_name("flowarrow-matrix-rust-bench", &root);
    fs::create_dir_all(rust_project_root.join("src")).expect("create Rust benchmark project");
    fs::write(
        rust_project_root.join("Cargo.toml"),
        rust_manifest(&rust_package_name),
    )
    .expect("write Rust benchmark manifest");
    fs::write(
        rust_project_root.join("src/main.rs"),
        rust_source(&left, &right, &vector, config, expected),
    )
    .expect("write Rust benchmark source");

    let flowarrow_cpu_build_start = Instant::now();
    eprintln!("building FlowArrow CPU benchmark...");
    let flowarrow_cpu_build =
        build_file_with_options(&flowarrow_cpu_source_path, &BuildOptions::default())
            .expect("build CPU FlowArrow benchmark");
    let flowarrow_cpu_build_time = flowarrow_cpu_build_start.elapsed();

    let flowarrow_gpu_build = if config.gpu {
        let mut gpu_options = BuildOptions::default();
        gpu_options.gpu = true;
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

    println!("matrix benchmark");
    println!(
        "  shape:      {}x{} * {}x{}",
        config.rows, config.inner, config.inner, config.cols
    );
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
    rows: usize,
    inner: usize,
    cols: usize,
    iterations: usize,
    samples: usize,
    gpu_samples: usize,
    gpu: bool,
}

impl Config {
    fn from_args(args: Vec<String>) -> Self {
        let mut config = Self {
            rows: env_usize("FLOWARROW_BENCH_MATRIX_ROWS", DEFAULT_ROWS),
            inner: env_usize("FLOWARROW_BENCH_MATRIX_INNER", DEFAULT_INNER),
            cols: env_usize("FLOWARROW_BENCH_MATRIX_COLS", DEFAULT_COLS),
            iterations: env_usize("FLOWARROW_BENCH_ITERATIONS", DEFAULT_ITERATIONS),
            samples: env_usize("FLOWARROW_BENCH_SAMPLES", DEFAULT_SAMPLES),
            gpu_samples: env_usize("FLOWARROW_BENCH_GPU_SAMPLES", 1),
            gpu: env_bool("FLOWARROW_BENCH_GPU", false),
        };

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--rows" => {
                    index += 1;
                    config.rows = parse_usize(args.get(index), "--rows");
                }
                "--inner" => {
                    index += 1;
                    config.inner = parse_usize(args.get(index), "--inner");
                }
                "--cols" => {
                    index += 1;
                    config.cols = parse_usize(args.get(index), "--cols");
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
                    eprintln!("unknown matrix benchmark option `{other}`");
                    print_help();
                    std::process::exit(2);
                }
            }
            index += 1;
        }

        if config.rows == 0
            || config.inner == 0
            || config.cols == 0
            || config.iterations == 0
            || config.samples == 0
            || config.gpu_samples == 0
        {
            eprintln!(
                "--rows, --inner, --cols, --iterations, --samples, and --gpu-samples must be greater than zero"
            );
            std::process::exit(2);
        }

        config
    }
}

fn print_help() {
    eprintln!(
        "usage: cargo bench --bench matrix -- [--gpu] [--gpu-samples N] [--rows N] [--inner N] [--cols N] [--iterations N] [--samples N]\n\
         env: FLOWARROW_BENCH_GPU=1, FLOWARROW_BENCH_GPU_SAMPLES, FLOWARROW_BENCH_MATRIX_ROWS, FLOWARROW_BENCH_MATRIX_INNER, FLOWARROW_BENCH_MATRIX_COLS, FLOWARROW_BENCH_ITERATIONS, FLOWARROW_BENCH_SAMPLES"
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

fn vector_values(len: usize, seed: usize) -> Vec<f64> {
    (0..len)
        .map(|index| ((index + seed) % 11 + 1) as f64)
        .collect()
}

fn matrix_values(rows: usize, cols: usize, seed: usize) -> Vec<f64> {
    (0..rows * cols)
        .map(|index| ((index + seed) % 13 + 1) as f64)
        .collect()
}

fn native_kernel(
    left: &[f64],
    right: &[f64],
    vector: &[f64],
    rows: usize,
    inner: usize,
    cols: usize,
    iterations: usize,
) -> f64 {
    let mut score = 0.0;
    for _ in 0..iterations {
        let mut product_sum = 0.0;
        for row in 0..rows {
            let mut row_product_sum = 0.0;
            for col in 0..cols {
                let mut dot = 0.0;
                for k in 0..inner {
                    dot += left[row * inner + k] * right[k * cols + col];
                }
                row_product_sum += dot;
            }
            product_sum += row_product_sum;
        }

        let mut matvec_sum = 0.0;
        for row in 0..rows {
            let mut dot = 0.0;
            for k in 0..inner {
                dot += left[row * inner + k] * vector[k];
            }
            matvec_sum += dot;
        }

        let mut row_sum_total = 0.0;
        for row in 0..rows {
            let mut row_sum = 0.0;
            for col in 0..inner {
                row_sum += left[row * inner + col];
            }
            row_sum_total += row_sum;
        }

        score += product_sum + matvec_sum + row_sum_total;
    }
    score
}

fn flowarrow_source(
    left: &[f64],
    right: &[f64],
    vector: &[f64],
    config: Config,
    expected: f64,
) -> String {
    format!(
        r#"
import std.cli {{ Args }}
import std.math {{ add as scalar_add, eq }}
import std.vector {{ sum as vector_sum }}
import std.matrix {{ matmul, matvec, row_sums, sum as matrix_sum }}

node kernel(left: Seq[Seq[Real]], right: Seq[Seq[Real]], vector: Seq[Real], score: Real) -> (out_left: Seq[Seq[Real]], out_right: Seq[Seq[Real]], out_vector: Seq[Real], out_score: Real) {{
    ($left, $right) -> matmul -> $product
    $product -> matrix_sum -> $product_sum
    ($left, $vector) -> matvec -> $mv
    $mv -> vector_sum -> $matvec_sum
    $left -> row_sums -> vector_sum -> $row_sum_total
    ($product_sum, $matvec_sum) -> scalar_add -> $partial
    ($partial, $row_sum_total) -> scalar_add -> $delta
    ($score, $delta) -> scalar_add -> $out_score
    $left -> $out_left
    $right -> $out_right
    $vector -> $out_vector
}}

node final_score(left: Seq[Seq[Real]], right: Seq[Seq[Real]], vector: Seq[Real], score: Real) -> out: Real {{
    $score -> $out
}}

program main(args: Args) -> exit_code: Int {{
    {} -> $left
    {} -> $right
    {} -> $vector
    ($left, $right, $vector, 0.0) -> repeat<{}> kernel -> final_score -> $score
    ($score, {}) -> eq -> $ok
    ($ok, 0, 1) -> select -> $exit_code
}}
"#,
        flow_matrix(left, config.rows, config.inner),
        flow_matrix(right, config.inner, config.cols),
        flow_seq(vector),
        config.iterations,
        flow_real(expected),
    )
}

fn rust_source(
    left: &[f64],
    right: &[f64],
    vector: &[f64],
    config: Config,
    expected: f64,
) -> String {
    format!(
        r#"
use std::hint::black_box;

static LEFT: &[f64] = &{};
static RIGHT: &[f64] = &{};
static VECTOR: &[f64] = &{};

fn kernel(left: &[f64], right: &[f64], vector: &[f64], rows: usize, inner: usize, cols: usize, iterations: usize) -> f64 {{
    let mut score = 0.0;
    for _ in 0..iterations {{
        let mut product_sum = 0.0;
        for row in 0..rows {{
            for col in 0..cols {{
                let mut dot = 0.0;
                for k in 0..inner {{
                    dot += black_box(left)[row * inner + k] * black_box(right)[k * cols + col];
                }}
                product_sum += dot;
            }}
        }}

        let mut matvec_sum = 0.0;
        for row in 0..rows {{
            let mut dot = 0.0;
            for k in 0..inner {{
                dot += black_box(left)[row * inner + k] * black_box(vector)[k];
            }}
            matvec_sum += dot;
        }}

        let mut row_sum_total = 0.0;
        for row in 0..rows {{
            for col in 0..inner {{
                row_sum_total += black_box(left)[row * inner + col];
            }}
        }}

        score += black_box(product_sum) + black_box(matvec_sum) + black_box(row_sum_total);
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
        black_box(VECTOR),
        black_box({}usize),
        black_box({}usize),
        black_box({}usize),
        black_box({}usize),
    );
    let expected = black_box({});
    std::process::exit(if is_close(score, expected) {{ 0 }} else {{ 1 }});
}}
"#,
        rust_slice(left),
        rust_slice(right),
        rust_slice(vector),
        config.rows,
        config.inner,
        config.cols,
        config.iterations,
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

fn flow_matrix(values: &[f64], rows: usize, cols: usize) -> String {
    let mut out = String::from("[");
    for row in 0..rows {
        if row > 0 {
            out.push_str(", ");
        }
        out.push('[');
        for col in 0..cols {
            if col > 0 {
                out.push_str(", ");
            }
            out.push_str(&flow_real(values[row * cols + col]));
        }
        out.push(']');
    }
    out.push(']');
    out
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
        "flowarrow-matrix-bench-{}-{unique}",
        std::process::id()
    ))
}
