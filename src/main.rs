use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run_cli() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("flowarrow: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<u8, String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {
            let path = args
                .next()
                .ok_or_else(|| "usage: flowarrow run <path.flow>".to_string())?;
            if args.next().is_some() {
                return Err("usage: flowarrow run <path.flow>".to_string());
            }
            flowarrow::run_file(PathBuf::from(path).as_path())
        }
        Some("build") => {
            let path = args.next().ok_or_else(|| {
                "usage: flowarrow build <path.flow> [--emit-llvm <path.ll>]".to_string()
            })?;
            let mut emit_llvm = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--emit-llvm" => {
                        let out = args
                            .next()
                            .ok_or_else(|| "--emit-llvm requires an output path".to_string())?;
                        emit_llvm = Some(PathBuf::from(out));
                    }
                    other => return Err(format!("unknown build option `{other}`")),
                }
            }
            flowarrow::build_file(PathBuf::from(path).as_path(), emit_llvm.as_deref())?;
            Ok(0)
        }
        _ => Err("usage: flowarrow <run|build> ...".to_string()),
    }
}
