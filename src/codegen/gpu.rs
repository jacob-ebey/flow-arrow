use super::Ty;
use crate::module_resolver::{ResolvedSymbolOrigin, SymbolId};
use crate::stdlib::Effect;
use crate::typecheck::{
    TypedCallable, TypedEndpoint, TypedEndpointKind, TypedModule, TypedStageKind, TypedSymbolKind,
};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GpuPlan {
    map_kernels: HashMap<MapKernelKey, GpuMapKernel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MapKernelKey {
    callable: String,
    input: Ty,
    output: Ty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GpuMapKernel {
    pub id: String,
    pub callable: String,
    pub input: Ty,
    pub output: Ty,
    pub scalar: GpuScalarKind,
    pub map_expr: String,
    pub wgsl: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GpuScalarKind {
    I32,
    F32,
    F64,
}

impl GpuScalarKind {
    pub(super) fn map_function(self) -> &'static str {
        match self {
            Self::I32 => "faGpuMapI32",
            Self::F32 => "faGpuMapF32",
            Self::F64 => "faGpuMapF64",
        }
    }
}

#[derive(Debug, Clone)]
struct SymbolNames {
    callables_by_name: HashMap<String, TypedCallable>,
    callables_by_id: HashMap<SymbolId, TypedCallable>,
    builtins_by_id: HashMap<SymbolId, String>,
}

#[derive(Debug, Clone, PartialEq)]
enum GpuExpr {
    Var(String),
    Real(f64),
    Int(i64),
    Bool(bool),
    Tuple(Vec<GpuExpr>),
    Unary {
        op: GpuUnaryOp,
        value: Box<GpuExpr>,
    },
    Binary {
        op: GpuBinaryOp,
        left: Box<GpuExpr>,
        right: Box<GpuExpr>,
    },
    Select {
        pred: Box<GpuExpr>,
        when_true: Box<GpuExpr>,
        when_false: Box<GpuExpr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpuUnaryOp {
    Neg,
    Abs,
    Sqrt,
    Exp,
    Sin,
    Cos,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GpuBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Xor,
}

impl GpuPlan {
    pub(super) fn empty() -> Self {
        Self {
            map_kernels: HashMap::new(),
        }
    }

    pub(super) fn analyze(module: &TypedModule) -> Self {
        let names = SymbolNames::new(module);
        let mut map_kernels = HashMap::new();
        for callable in &module.callables {
            if callable.effect != Effect::Pure {
                continue;
            }
            let [input] = callable.inputs.as_slice() else {
                continue;
            };
            let [output] = callable.outputs.as_slice() else {
                continue;
            };
            let scalar = match (&input.ty, &output.ty) {
                (Ty::I32, Ty::I32) => GpuScalarKind::I32,
                (Ty::F32, Ty::F32) => GpuScalarKind::F32,
                (Ty::F64, Ty::F64) => GpuScalarKind::F64,
                _ => continue,
            };
            if input.ty != output.ty {
                continue;
            }
            if let Ok(expr) = ScalarKernelBuilder::new(&names).lower_callable(callable) {
                let id = format!("fa_gpu_map_{}", sanitize_gpu_ident(&callable.name));
                let map_expr = expr.wgsl(scalar);
                let wgsl = map_wgsl(&id, scalar, &expr);
                let key = MapKernelKey {
                    callable: callable.name.clone(),
                    input: input.ty.clone(),
                    output: output.ty.clone(),
                };
                map_kernels.insert(
                    key,
                    GpuMapKernel {
                        id,
                        callable: callable.name.clone(),
                        input: input.ty.clone(),
                        output: output.ty.clone(),
                        scalar,
                        map_expr,
                        wgsl,
                    },
                );
            }
        }
        Self { map_kernels }
    }

    pub(super) fn kernel_for_map(
        &self,
        callable: &str,
        input: &Ty,
        output: &Ty,
    ) -> Option<&GpuMapKernel> {
        self.map_kernels.get(&MapKernelKey {
            callable: callable.to_string(),
            input: input.clone(),
            output: output.clone(),
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.map_kernels.is_empty()
    }

    pub(super) fn emit_c_manifest(&self) -> String {
        if self.map_kernels.is_empty() {
            return String::new();
        }
        let mut kernels = self.map_kernels.values().collect::<Vec<_>>();
        kernels.sort_by(|left, right| left.id.cmp(&right.id));
        let mut out = String::new();
        out.push_str("\n/* FlowArrow GPU kernels. Host backends dispatch these WGSL kernels through WebGPU/wgpu-compatible runtimes. */\n");
        out.push_str("typedef struct { const char *id; const char *wgsl; } FaGpuKernel;\n");
        for kernel in &kernels {
            out.push_str(&format!(
                "static const char {}_wgsl[] = {};\n",
                kernel.id,
                c_string_literal(&kernel.wgsl)
            ));
        }
        out.push_str("static const FaGpuKernel fa_gpu_kernels[] = {\n");
        for kernel in &kernels {
            out.push_str(&format!(
                "    {{ \"{}\", {}_wgsl }},\n",
                kernel.id, kernel.id
            ));
        }
        out.push_str("};\n");
        out.push_str(&format!(
            "static const size_t fa_gpu_kernel_count = {};\n",
            kernels.len()
        ));
        out
    }
}

impl SymbolNames {
    fn new(module: &TypedModule) -> Self {
        let mut callables_by_name = HashMap::new();
        let mut callables_by_id = HashMap::new();
        for callable in &module.callables {
            callables_by_name.insert(callable.name.clone(), callable.clone());
            if let Some(id) = callable.id {
                callables_by_id.insert(id, callable.clone());
            }
        }

        let mut builtins_by_id = HashMap::new();
        for symbol in &module.symbols {
            let TypedSymbolKind::Callable(callable) = &symbol.kind else {
                continue;
            };
            if callable.origin != ResolvedSymbolOrigin::StdlibBuiltin {
                continue;
            }
            builtins_by_id.insert(symbol.id, callable.runtime_name.clone());
        }

        Self {
            callables_by_name,
            callables_by_id,
            builtins_by_id,
        }
    }

    fn callable(&self, name: &str, symbol: Option<SymbolId>) -> Option<&TypedCallable> {
        symbol
            .and_then(|id| self.callables_by_id.get(&id))
            .or_else(|| self.callables_by_name.get(name))
    }

    fn builtin_name(&self, name: &str, symbol: Option<SymbolId>) -> String {
        symbol
            .and_then(|id| self.builtins_by_id.get(&id).cloned())
            .unwrap_or_else(|| name.to_string())
    }
}

struct ScalarKernelBuilder<'a> {
    names: &'a SymbolNames,
}

impl<'a> ScalarKernelBuilder<'a> {
    fn new(names: &'a SymbolNames) -> Self {
        Self { names }
    }

    fn lower_callable(&self, callable: &TypedCallable) -> Result<GpuExpr, String> {
        if callable.effect != Effect::Pure {
            return Err("GPU scalar kernels require pure callables".to_string());
        }
        let [input] = callable.inputs.as_slice() else {
            return Err("GPU scalar kernels require one input".to_string());
        };
        let [output] = callable.outputs.as_slice() else {
            return Err("GPU scalar kernels require one output".to_string());
        };
        match (&input.ty, &output.ty) {
            (Ty::I32, Ty::I32) | (Ty::F32, Ty::F32) | (Ty::F64, Ty::F64) => {}
            _ => {
                return Err(
                    "GPU scalar kernels require i32 -> i32, f32 -> f32, or f64 -> f64".to_string(),
                );
            }
        }

        let mut env = HashMap::from([(input.name.clone(), GpuExpr::Var("x".to_string()))]);
        for chain in &callable.chains {
            let mut value = self.lower_endpoint(&chain.source, &env)?;
            for stage in &chain.stages {
                match &stage.kind {
                    TypedStageKind::Call { name, symbol } => {
                        value = self.lower_call(name, *symbol, value)?;
                    }
                    TypedStageKind::Bind { target } => {
                        bind_expr(target, value.clone(), &mut env)?;
                    }
                    _ => return Err("GPU scalar kernels support only calls and binds".to_string()),
                }
            }
        }
        env.get(&output.name)
            .cloned()
            .ok_or_else(|| format!("GPU scalar kernel output `{}` was not bound", output.name))
    }

    fn lower_endpoint(
        &self,
        endpoint: &TypedEndpoint,
        env: &HashMap<String, GpuExpr>,
    ) -> Result<GpuExpr, String> {
        match &endpoint.kind {
            TypedEndpointKind::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("GPU scalar kernel value `{name}` is not bound")),
            TypedEndpointKind::Int(value) => Ok(GpuExpr::Int(*value)),
            TypedEndpointKind::Real(value) => Ok(GpuExpr::Real(*value)),
            TypedEndpointKind::Bool(value) => Ok(GpuExpr::Bool(*value)),
            TypedEndpointKind::Tuple(items) => Ok(GpuExpr::Tuple(
                items
                    .iter()
                    .map(|item| self.lower_endpoint(item, env))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            TypedEndpointKind::Eval { source, stages } => {
                let mut value = self.lower_endpoint(source, env)?;
                let mut local_env = env.clone();
                for stage in stages {
                    match &stage.kind {
                        TypedStageKind::Call { name, symbol } => {
                            value = self.lower_call(name, *symbol, value)?;
                        }
                        TypedStageKind::Bind { target } => {
                            bind_expr(target, value.clone(), &mut local_env)?;
                        }
                        _ => {
                            return Err(
                                "GPU scalar inline evaluations support only calls and binds"
                                    .to_string(),
                            );
                        }
                    }
                }
                Ok(value)
            }
            _ => Err(format!(
                "GPU scalar kernel does not support endpoint `{}`",
                endpoint.label
            )),
        }
    }

    fn lower_call(
        &self,
        name: &str,
        symbol: Option<SymbolId>,
        input: GpuExpr,
    ) -> Result<GpuExpr, String> {
        if let Some(callable) = self.names.callable(name, symbol) {
            let inlined = callable.clone();
            if inlined.inputs.len() == 1 {
                let input_name = inlined.inputs[0].name.clone();
                let output_name = inlined
                    .outputs
                    .first()
                    .map(|port| port.name.clone())
                    .ok_or_else(|| format!("GPU callable `{name}` has no output"))?;
                let mut env = HashMap::from([(input_name, input)]);
                for chain in &inlined.chains {
                    let mut value = self.lower_endpoint(&chain.source, &env)?;
                    for stage in &chain.stages {
                        match &stage.kind {
                            TypedStageKind::Call { name, symbol } => {
                                value = self.lower_call(name, *symbol, value)?;
                            }
                            TypedStageKind::Bind { target } => {
                                bind_expr(target, value.clone(), &mut env)?;
                            }
                            _ => {
                                return Err(
                                    "GPU scalar callable inlining supports only calls and binds"
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
                return env
                    .remove(&output_name)
                    .ok_or_else(|| format!("GPU callable `{name}` output was not bound"));
            }
        }

        self.lower_builtin(&self.names.builtin_name(name, symbol), input)
    }

    fn lower_builtin(&self, name: &str, input: GpuExpr) -> Result<GpuExpr, String> {
        match name {
            "add" => binary(input, GpuBinaryOp::Add),
            "sub" => binary(input, GpuBinaryOp::Sub),
            "mul" => binary(input, GpuBinaryOp::Mul),
            "div" => binary(input, GpuBinaryOp::Div),
            "eq" => binary(input, GpuBinaryOp::Eq),
            "lt" => binary(input, GpuBinaryOp::Lt),
            "gt" => binary(input, GpuBinaryOp::Gt),
            "le" => binary(input, GpuBinaryOp::Le),
            "ge" => binary(input, GpuBinaryOp::Ge),
            "and" => binary(input, GpuBinaryOp::And),
            "or" => binary(input, GpuBinaryOp::Or),
            "xor" => binary(input, GpuBinaryOp::Xor),
            "neg" => unary(input, GpuUnaryOp::Neg),
            "abs" => unary(input, GpuUnaryOp::Abs),
            "sqrt" => unary(input, GpuUnaryOp::Sqrt),
            "exp" => unary(input, GpuUnaryOp::Exp),
            "sin" => unary(input, GpuUnaryOp::Sin),
            "cos" => unary(input, GpuUnaryOp::Cos),
            "not" => unary(input, GpuUnaryOp::Not),
            "select" => {
                let GpuExpr::Tuple(mut items) = input else {
                    return Err("GPU select expected tuple input".to_string());
                };
                if items.len() != 3 {
                    return Err("GPU select expected three inputs".to_string());
                }
                let when_false = items.pop().expect("len checked");
                let when_true = items.pop().expect("len checked");
                let pred = items.pop().expect("len checked");
                Ok(GpuExpr::Select {
                    pred: Box::new(pred),
                    when_true: Box::new(when_true),
                    when_false: Box::new(when_false),
                })
            }
            _ => Err(format!(
                "GPU scalar kernel does not support builtin `{name}`"
            )),
        }
    }
}

fn bind_expr(
    target: &crate::ast::BindingTarget,
    value: GpuExpr,
    env: &mut HashMap<String, GpuExpr>,
) -> Result<(), String> {
    match target {
        crate::ast::BindingTarget::Discard => Ok(()),
        crate::ast::BindingTarget::Variable(name) => {
            env.insert(name.clone(), value);
            Ok(())
        }
        crate::ast::BindingTarget::Tuple(items) => {
            let GpuExpr::Tuple(values) = value else {
                return Err("GPU tuple bind expected tuple value".to_string());
            };
            if items.len() != values.len() {
                return Err("GPU tuple bind arity mismatch".to_string());
            }
            for (target, value) in items.iter().zip(values) {
                bind_expr(target, value, env)?;
            }
            Ok(())
        }
    }
}

fn unary(input: GpuExpr, op: GpuUnaryOp) -> Result<GpuExpr, String> {
    Ok(GpuExpr::Unary {
        op,
        value: Box::new(input),
    })
}

fn binary(input: GpuExpr, op: GpuBinaryOp) -> Result<GpuExpr, String> {
    let GpuExpr::Tuple(mut items) = input else {
        return Err("GPU binary op expected tuple input".to_string());
    };
    if items.len() != 2 {
        return Err("GPU binary op expected two inputs".to_string());
    }
    let right = items.pop().expect("len checked");
    let left = items.pop().expect("len checked");
    Ok(GpuExpr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    })
}

fn map_wgsl(kernel_id: &str, scalar: GpuScalarKind, expr: &GpuExpr) -> String {
    let element = match scalar {
        GpuScalarKind::I32 => "i32",
        GpuScalarKind::F32 => "f32",
        GpuScalarKind::F64 => "f64",
    };
    format!(
        "struct FaGpuMapParams {{ len: u32, _pad0: u32, _pad1: u32, _pad2: u32 }};\n\
@group(0) @binding(0) var<storage, read> fa_input: array<{element}>;\n\
@group(0) @binding(1) var<storage, read_write> fa_output: array<{element}>;\n\
@group(0) @binding(2) var<uniform> fa_params: FaGpuMapParams;\n\
\n\
@compute @workgroup_size(64)\n\
fn {kernel_id}(@builtin(global_invocation_id) fa_gid: vec3<u32>) {{\n\
  let fa_i = fa_gid.x;\n\
  if (fa_i >= fa_params.len) {{ return; }}\n\
  let x = fa_input[fa_i];\n\
  fa_output[fa_i] = {};\n\
}}\n",
        expr.wgsl(scalar)
    )
}

impl GpuExpr {
    fn wgsl(&self, scalar: GpuScalarKind) -> String {
        match self {
            GpuExpr::Var(name) => sanitize_gpu_ident(name),
            GpuExpr::Real(value) => wgsl_float_literal(*value, scalar),
            GpuExpr::Int(value) => match scalar {
                GpuScalarKind::I32 => format!("i32({value})"),
                GpuScalarKind::F32 => format!("f32({value})"),
                GpuScalarKind::F64 => format!("f64({value})"),
            },
            GpuExpr::Bool(value) => value.to_string(),
            GpuExpr::Tuple(_) => "/* unsupported tuple value */".to_string(),
            GpuExpr::Unary { op, value } => {
                let value = value.wgsl(scalar);
                match op {
                    GpuUnaryOp::Neg => format!("(-({value}))"),
                    GpuUnaryOp::Abs => format!("abs({value})"),
                    GpuUnaryOp::Sqrt => format!("sqrt({value})"),
                    GpuUnaryOp::Exp => format!("exp({value})"),
                    GpuUnaryOp::Sin => format!("sin({value})"),
                    GpuUnaryOp::Cos => format!("cos({value})"),
                    GpuUnaryOp::Not => format!("!({value})"),
                }
            }
            GpuExpr::Binary { op, left, right } => {
                let left = left.wgsl(scalar);
                let right = right.wgsl(scalar);
                match op {
                    GpuBinaryOp::Add => format!("(({left}) + ({right}))"),
                    GpuBinaryOp::Sub => format!("(({left}) - ({right}))"),
                    GpuBinaryOp::Mul => format!("(({left}) * ({right}))"),
                    GpuBinaryOp::Div => format!("(({left}) / ({right}))"),
                    GpuBinaryOp::Eq => format!("(({left}) == ({right}))"),
                    GpuBinaryOp::Lt => format!("(({left}) < ({right}))"),
                    GpuBinaryOp::Gt => format!("(({left}) > ({right}))"),
                    GpuBinaryOp::Le => format!("(({left}) <= ({right}))"),
                    GpuBinaryOp::Ge => format!("(({left}) >= ({right}))"),
                    GpuBinaryOp::And => format!("(({left}) && ({right}))"),
                    GpuBinaryOp::Or => format!("(({left}) || ({right}))"),
                    GpuBinaryOp::Xor => format!("(({left}) != ({right}))"),
                }
            }
            GpuExpr::Select {
                pred,
                when_true,
                when_false,
            } => format!(
                "select({}, {}, {})",
                when_false.wgsl(scalar),
                when_true.wgsl(scalar),
                pred.wgsl(scalar)
            ),
        }
    }
}

fn wgsl_float_literal(value: f64, scalar: GpuScalarKind) -> String {
    let constructor = match scalar {
        GpuScalarKind::I32 => "f32",
        GpuScalarKind::F32 => "f32",
        GpuScalarKind::F64 => "f64",
    };
    if value.is_finite() {
        let mut text = value.to_string();
        if !text.contains('.') && !text.contains('e') && !text.contains('E') {
            text.push_str(".0");
        }
        format!("{constructor}({text})")
    } else if value.is_nan() {
        format!("{constructor}(0.0) / {constructor}(0.0)")
    } else if value.is_sign_positive() {
        format!("{constructor}(1.0) / {constructor}(0.0)")
    } else {
        format!("-{constructor}(1.0) / {constructor}(0.0)")
    }
}

fn sanitize_gpu_ident(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

fn c_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n\"\n\""),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_ascii_graphic() || ch == ' ' => out.push(ch),
            ch => out.push_str(&format!("\\x{:02x}", ch as u32)),
        }
    }
    out.push('"');
    out
}
