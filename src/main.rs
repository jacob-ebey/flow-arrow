use std::env;
use std::io::Read;
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
            let mut path = None;
            let mut options = flowarrow::BuildOptions::default();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--target" => {
                        let target = args
                            .next()
                            .ok_or_else(|| "--target requires a target triple".to_string())?;
                        options.target = target.parse()?;
                    }
                    "--crate-type" => {
                        let crate_type = args
                            .next()
                            .ok_or_else(|| "--crate-type requires `bin` or `cdylib`".to_string())?;
                        options.crate_type = crate_type.parse()?;
                    }
                    "--compiler-flag" | "--cflag" => {
                        let flag = args
                            .next()
                            .ok_or_else(|| format!("{flag} requires a compiler flag"))?;
                        if let Some(optimization) =
                            flowarrow::BuildOptimization::from_clang_flag(&flag)
                        {
                            options.optimization = optimization;
                        } else {
                            options.compiler_flags.push(flag);
                        }
                    }
                    "--linker-flag" | "--ldflag" => {
                        let flag = args
                            .next()
                            .ok_or_else(|| format!("{flag} requires a linker flag"))?;
                        options.linker_flags.push(flag);
                    }
                    "--emit-llvm" => {
                        let out = args
                            .next()
                            .ok_or_else(|| "--emit-llvm requires an output path".to_string())?;
                        options.emit_llvm = Some(PathBuf::from(out));
                    }
                    "--" => {
                        options.compiler_flags.extend(args);
                        break;
                    }
                    opt if flowarrow::BuildOptimization::from_clang_flag(opt).is_some() => {
                        options.optimization =
                            flowarrow::BuildOptimization::from_clang_flag(opt).expect("checked");
                    }
                    other if other.starts_with("--") => {
                        return Err(format!("unknown build option `{other}`"));
                    }
                    _ if path.is_none() => path = Some(flag),
                    other => return Err(format!("unknown build option `{other}`")),
                }
            }
            let path = path.ok_or_else(|| {
                "usage: flowarrow build [--target <target>] [--crate-type <bin|cdylib>] [-O0|-O1|-O2|-O3|-Os|-Oz] [--compiler-flag <flag>] [--linker-flag <flag>] [--emit-llvm <path.ll>] <path.flow> [-- <compiler flags...>]"
                    .to_string()
            })?;
            flowarrow::build_file_with_options(PathBuf::from(path).as_path(), &options)?;
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
            let mut path = None;
            let mut check = false;
            let mut stdout = false;
            let mut stdin = false;
            for flag in args {
                match flag.as_str() {
                    "--check" => check = true,
                    "--stdout" => stdout = true,
                    "--stdin" => stdin = true,
                    other if other.starts_with("--") => {
                        return Err(format!("unknown fmt option `{other}`"));
                    }
                    _ if path.is_none() => path = Some(flag),
                    other => return Err(format!("unknown fmt option `{other}`")),
                }
            }
            if check && stdout {
                return Err("flowarrow fmt accepts only one of --check or --stdout".to_string());
            }
            if stdin && path.is_some() {
                return Err("flowarrow fmt accepts either <path.flow> or --stdin".to_string());
            }
            if stdin {
                let mut source = String::new();
                std::io::stdin()
                    .read_to_string(&mut source)
                    .map_err(|error| format!("failed to read stdin: {error}"))?;
                let formatted = flowarrow::format_source(&source)?;
                if check {
                    if formatted == source {
                        Ok(0)
                    } else {
                        Err("stdin is not formatted".to_string())
                    }
                } else {
                    print!("{formatted}");
                    Ok(0)
                }
            } else {
                let path = path.ok_or_else(|| {
                    "usage: flowarrow fmt <path.flow> [--check|--stdout] | flowarrow fmt --stdin [--check|--stdout]".to_string()
                })?;
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
        Some("lsp") => {
            if args.next().is_some() {
                return Err("usage: flowarrow lsp".to_string());
            }
            flowarrow::run_lsp_server()
        }
        _ => Err("usage: flowarrow <run|build|typecheck|fmt|graph|lsp> ...".to_string()),
    }
}
