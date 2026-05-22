use crate::ast::Module;
use inkwell::AddressSpace;
use inkwell::context::Context;

#[allow(dead_code)]
pub fn emit_module(module: &Module) -> Result<String, String> {
    let _ = module;

    let context = Context::create();
    let llvm_module = context.create_module("flowarrow");
    let builder = context.create_builder();

    let i32_ty = context.i32_type();
    let ptr_ty = context.ptr_type(AddressSpace::default());
    let main_ty = i32_ty.fn_type(&[i32_ty.into(), ptr_ty.into()], false);
    let runtime_main = llvm_module.add_function("flow_unboxed_main", main_ty, None);
    let main = llvm_module.add_function("main", main_ty, None);
    let block = context.append_basic_block(main, "entry");

    builder.position_at_end(block);
    let argc = main
        .get_nth_param(0)
        .ok_or_else(|| "LLVM backend failed to create `argc` parameter".to_string())?
        .into_int_value();
    let argv = main
        .get_nth_param(1)
        .ok_or_else(|| "LLVM backend failed to create `argv` parameter".to_string())?
        .into_pointer_value();
    let exit = builder
        .build_call(runtime_main, &[argc.into(), argv.into()], "exit")
        .map_err(|error| format!("LLVM backend failed to build runtime entry call: {error}"))?
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| "LLVM backend runtime entry call did not return a value".to_string())?
        .into_int_value();
    builder
        .build_return(Some(&exit))
        .map_err(|error| format!("LLVM backend failed to build return: {error}"))?;

    Ok(llvm_module.print_to_string().to_string())
}
