use super::*;

const MODULE: &str = "std.math";

macro_rules! math_node {
    ($constant:ident, $name:literal, $runtime:literal, $input:literal, $output:literal) => {
        pub const $constant: StdSymbol = runtime_node(MODULE, $name, $runtime, $input, $output);
    };
}

macro_rules! math_reducible_node {
    (
        $constant:ident,
        $name:literal,
        $runtime:literal,
        $input:literal,
        $output:literal,
        $reduce_input:literal,
        $reduce_output:literal
    ) => {
        pub const $constant: StdSymbol = runtime_reducible_node(
            MODULE,
            $name,
            $runtime,
            $input,
            $output,
            $reduce_input,
            $reduce_output,
        );
    };
}

math_reducible_node!(
    ADD_I32,
    "add_i32",
    "add",
    "(i32,i32)",
    "Faultable[i32]",
    "(i32,i32)",
    "Faultable[i32]"
);
math_reducible_node!(
    ADD_I64,
    "add_i64",
    "add",
    "(i64,i64)",
    "Faultable[i64]",
    "(i64,i64)",
    "Faultable[i64]"
);
math_reducible_node!(
    ADD_F32,
    "add_f32",
    "add",
    "(f32,f32)",
    "f32",
    "(f32,f32)",
    "f32"
);
math_reducible_node!(
    ADD_F64,
    "add_f64",
    "add",
    "(f64,f64)",
    "f64",
    "(f64,f64)",
    "f64"
);

math_node!(SUB_I32, "sub_i32", "sub", "(i32,i32)", "Faultable[i32]");
math_node!(SUB_I64, "sub_i64", "sub", "(i64,i64)", "Faultable[i64]");
math_node!(SUB_F32, "sub_f32", "sub", "(f32,f32)", "f32");
math_node!(SUB_F64, "sub_f64", "sub", "(f64,f64)", "f64");

math_node!(MUL_I32, "mul_i32", "mul", "(i32,i32)", "Faultable[i32]");
math_node!(MUL_I64, "mul_i64", "mul", "(i64,i64)", "Faultable[i64]");
math_node!(MUL_F32, "mul_f32", "mul", "(f32,f32)", "f32");
math_node!(MUL_F64, "mul_f64", "mul", "(f64,f64)", "f64");

math_node!(DIV_I32, "div_i32", "div", "(i32,i32)", "Faultable[i32]");
math_node!(DIV_I64, "div_i64", "div", "(i64,i64)", "Faultable[i64]");
math_node!(DIV_F32, "div_f32", "div", "(f32,f32)", "Faultable[f32]");
math_node!(DIV_F64, "div_f64", "div", "(f64,f64)", "Faultable[f64]");

math_node!(REM_I32, "rem_i32", "rem", "(i32,i32)", "Faultable[i32]");
math_node!(REM_I64, "rem_i64", "rem", "(i64,i64)", "Faultable[i64]");
math_node!(REM_F32, "rem_f32", "rem", "(f32,f32)", "Faultable[f32]");
math_node!(REM_F64, "rem_f64", "rem", "(f64,f64)", "Faultable[f64]");

math_node!(NEG_I32, "neg_i32", "neg", "i32", "Faultable[i32]");
math_node!(NEG_I64, "neg_i64", "neg", "i64", "Faultable[i64]");
math_node!(NEG_F32, "neg_f32", "neg", "f32", "f32");
math_node!(NEG_F64, "neg_f64", "neg", "f64", "f64");

math_node!(ABS_I32, "abs_i32", "abs", "i32", "Faultable[i32]");
math_node!(ABS_I64, "abs_i64", "abs", "i64", "Faultable[i64]");
math_node!(ABS_F32, "abs_f32", "abs", "f32", "f32");
math_node!(ABS_F64, "abs_f64", "abs", "f64", "f64");

math_node!(SQRT_F32, "sqrt_f32", "sqrt", "f32", "Faultable[f32]");
math_node!(SQRT_F64, "sqrt_f64", "sqrt", "f64", "Faultable[f64]");
math_node!(EXP_F32, "exp_f32", "exp", "f32", "f32");
math_node!(EXP_F64, "exp_f64", "exp", "f64", "f64");
math_node!(SIN_F32, "sin_f32", "sin", "f32", "f32");
math_node!(SIN_F64, "sin_f64", "sin", "f64", "f64");
math_node!(COS_F32, "cos_f32", "cos", "f32", "f32");
math_node!(COS_F64, "cos_f64", "cos", "f64", "f64");

math_node!(EQ_I32, "eq_i32", "eq", "(i32,i32)", "Bool");
math_node!(EQ_I64, "eq_i64", "eq", "(i64,i64)", "Bool");
math_node!(EQ_F32, "eq_f32", "eq", "(f32,f32)", "Bool");
math_node!(EQ_F64, "eq_f64", "eq", "(f64,f64)", "Bool");

math_node!(LT_I32, "lt_i32", "lt", "(i32,i32)", "Bool");
math_node!(LT_I64, "lt_i64", "lt", "(i64,i64)", "Bool");
math_node!(LT_F32, "lt_f32", "lt", "(f32,f32)", "Bool");
math_node!(LT_F64, "lt_f64", "lt", "(f64,f64)", "Bool");

math_node!(GT_I32, "gt_i32", "gt", "(i32,i32)", "Bool");
math_node!(GT_I64, "gt_i64", "gt", "(i64,i64)", "Bool");
math_node!(GT_F32, "gt_f32", "gt", "(f32,f32)", "Bool");
math_node!(GT_F64, "gt_f64", "gt", "(f64,f64)", "Bool");

math_node!(LE_I32, "le_i32", "le", "(i32,i32)", "Bool");
math_node!(LE_I64, "le_i64", "le", "(i64,i64)", "Bool");
math_node!(LE_F32, "le_f32", "le", "(f32,f32)", "Bool");
math_node!(LE_F64, "le_f64", "le", "(f64,f64)", "Bool");

math_node!(GE_I32, "ge_i32", "ge", "(i32,i32)", "Bool");
math_node!(GE_I64, "ge_i64", "ge", "(i64,i64)", "Bool");
math_node!(GE_F32, "ge_f32", "ge", "(f32,f32)", "Bool");
math_node!(GE_F64, "ge_f64", "ge", "(f64,f64)", "Bool");

math_reducible_node!(
    MIN_I32,
    "min_i32",
    "min",
    "(i32,i32)",
    "i32",
    "(i32,i32)",
    "i32"
);
math_reducible_node!(
    MIN_I64,
    "min_i64",
    "min",
    "(i64,i64)",
    "i64",
    "(i64,i64)",
    "i64"
);
math_reducible_node!(
    MIN_F32,
    "min_f32",
    "min",
    "(f32,f32)",
    "f32",
    "(f32,f32)",
    "f32"
);
math_reducible_node!(
    MIN_F64,
    "min_f64",
    "min",
    "(f64,f64)",
    "f64",
    "(f64,f64)",
    "f64"
);

math_reducible_node!(
    MAX_I32,
    "max_i32",
    "max",
    "(i32,i32)",
    "i32",
    "(i32,i32)",
    "i32"
);
math_reducible_node!(
    MAX_I64,
    "max_i64",
    "max",
    "(i64,i64)",
    "i64",
    "(i64,i64)",
    "i64"
);
math_reducible_node!(
    MAX_F32,
    "max_f32",
    "max",
    "(f32,f32)",
    "f32",
    "(f32,f32)",
    "f32"
);
math_reducible_node!(
    MAX_F64,
    "max_f64",
    "max",
    "(f64,f64)",
    "f64",
    "(f64,f64)",
    "f64"
);
