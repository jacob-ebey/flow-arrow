mod bytes;
mod cli;
mod control;
mod core;
mod int_real;
mod io;
mod math;
mod seq;

const PARTS: &[&str] = &[
    core::C,
    cli::C,
    io::C,
    bytes::C,
    int_real::C,
    math::C,
    seq::C,
    control::C,
];

pub fn emit_preamble(out: &mut String) {
    for part in PARTS {
        out.push_str(part);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_parts_are_ordered_for_single_translation_unit() {
        let mut c = String::new();
        emit_preamble(&mut c);

        let core = c.find("typedef struct FaValue FaValue;").expect("core");
        let cli = c.find("static FaValue fa_args").expect("cli");
        let io = c.find("static FaValue fa_builtin_read_stdin").expect("io");
        let bytes = c
            .find("static FaValue fa_builtin_split_lines")
            .expect("bytes");
        let int_real = c
            .find("static FaValue fa_builtin_format_int")
            .expect("int/real");
        let math = c.find("static FaValue fa_binary_numeric").expect("math");
        let control = c.find("static FaValue fa_map").expect("control");

        assert!(core < cli);
        assert!(cli < io);
        assert!(io < bytes);
        assert!(bytes < int_real);
        assert!(int_real < math);
        assert!(math < control);
    }
}
