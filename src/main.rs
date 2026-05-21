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
                .ok_or_else(|| "usage: flowarrow run <path.flow> [args...]".to_string())?;
            flowarrow::run_file_with_args(PathBuf::from(path).as_path(), args)
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
        Some("typecheck") => {
            let path = args
                .next()
                .ok_or_else(|| "usage: flowarrow typecheck <path.flow>".to_string())?;
            if args.next().is_some() {
                return Err("usage: flowarrow typecheck <path.flow>".to_string());
            }
            flowarrow::typecheck_file(PathBuf::from(path).as_path())?;
            Ok(0)
        }
        Some("fmt") => {
            let path = args
                .next()
                .ok_or_else(|| "usage: flowarrow fmt <path.flow> [--check|--stdout]".to_string())?;
            let mut check = false;
            let mut stdout = false;
            for flag in args {
                match flag.as_str() {
                    "--check" => check = true,
                    "--stdout" => stdout = true,
                    other => return Err(format!("unknown fmt option `{other}`")),
                }
            }
            if check && stdout {
                return Err("flowarrow fmt accepts only one of --check or --stdout".to_string());
            }
            let path = PathBuf::from(path);
            if check {
                flowarrow::check_format_file(path.as_path())?;
            } else if stdout {
                let source = std::fs::read_to_string(&path)
                    .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
                print!("{}", flowarrow::format_source(&source)?);
            } else {
                flowarrow::format_file(path.as_path())?;
            }
            Ok(0)
        }
        Some("graph") => {
            let mut compact = false;
            let mut path = None;
            for arg in args {
                match arg.as_str() {
                    "--compact" => compact = true,
                    other if other.starts_with("--") => {
                        return Err(format!("unknown graph option `{other}`"));
                    }
                    _ if path.is_none() => path = Some(arg),
                    other => return Err(format!("unknown graph option `{other}`")),
                }
            }
            let path =
                path.ok_or_else(|| "usage: flowarrow graph [--compact] <path.flow>".to_string())?;
            let path = PathBuf::from(path);
            let graph = if compact {
                flowarrow::mermaid_file_compact(path.as_path())?
            } else {
                flowarrow::mermaid_file(path.as_path())?
            };
            print!("{graph}");
            Ok(0)
        }
        _ => Err("usage: flowarrow <run|build|typecheck|fmt|graph> ...".to_string()),
    }
}
