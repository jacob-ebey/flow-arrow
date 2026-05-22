use crate::{
    TypeScriptCompileMode, TypeScriptCompileOptions,
    compile_javascript_artifacts_source_with_options, compile_llvm_ir_source_with_options,
    compile_typescript_source_with_options,
};
use std::cell::{Cell, RefCell};
use std::mem;

thread_local! {
    static LAST_OK: Cell<bool> = const { Cell::new(false) };
    static LAST_RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn flowarrow_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    mem::forget(buffer);
    ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flowarrow_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flowarrow_compile_typescript(
    source_ptr: *const u8,
    source_len: usize,
    mode: u32,
) -> u32 {
    let source = match unsafe { wasm_input(source_ptr, source_len) } {
        Ok(source) => source,
        Err(error) => {
            store_result(false, error);
            return 0;
        }
    };
    let options = match typescript_options(mode) {
        Ok(options) => options,
        Err(error) => {
            store_result(false, error);
            return 0;
        }
    };
    match compile_typescript_source_with_options(source, options) {
        Ok(output) => {
            store_result(true, output);
            1
        }
        Err(error) => {
            store_result(false, error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flowarrow_compile_javascript_artifacts(
    source_ptr: *const u8,
    source_len: usize,
    mode: u32,
) -> u32 {
    let source = match unsafe { wasm_input(source_ptr, source_len) } {
        Ok(source) => source,
        Err(error) => {
            store_result(false, error);
            return 0;
        }
    };
    let options = match typescript_options(mode) {
        Ok(options) => options,
        Err(error) => {
            store_result(false, error);
            return 0;
        }
    };
    match compile_javascript_artifacts_source_with_options(source, options) {
        Ok((declarations, javascript)) => {
            store_result(true, format!("{declarations}\0{javascript}"));
            1
        }
        Err(error) => {
            store_result(false, error);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flowarrow_compile_llvm_ir(
    source_ptr: *const u8,
    source_len: usize,
    mode: u32,
) -> u32 {
    let source = match unsafe { wasm_input(source_ptr, source_len) } {
        Ok(source) => source,
        Err(error) => {
            store_result(false, error);
            return 0;
        }
    };
    let mode = match mode {
        0 => TypeScriptCompileMode::Program,
        1 => TypeScriptCompileMode::Library,
        other => {
            store_result(false, format!("unknown LLVM IR compile mode `{other}`"));
            return 0;
        }
    };
    match compile_llvm_ir_source_with_options(
        source,
        TypeScriptCompileOptions {
            mode,
            ..TypeScriptCompileOptions::default()
        },
    ) {
        Ok(output) => {
            store_result(true, output);
            1
        }
        Err(error) => {
            store_result(false, error);
            0
        }
    }
}

fn typescript_options(flags: u32) -> Result<TypeScriptCompileOptions, String> {
    let mode = match flags & 1 {
        0 => TypeScriptCompileMode::Program,
        1 => TypeScriptCompileMode::Library,
        _ => unreachable!(),
    };
    if flags & !0b11 != 0 {
        return Err(format!("unknown TypeScript compile option flags `{flags}`"));
    }
    Ok(TypeScriptCompileOptions {
        mode,
        worker_concurrency: flags & 0b10 != 0,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn flowarrow_result_ok() -> u32 {
    LAST_OK.with(|ok| u32::from(ok.get()))
}

#[unsafe(no_mangle)]
pub extern "C" fn flowarrow_result_ptr() -> *const u8 {
    LAST_RESULT.with(|result| result.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn flowarrow_result_len() -> usize {
    LAST_RESULT.with(|result| result.borrow().len())
}

#[unsafe(no_mangle)]
pub extern "C" fn flowarrow_result_clear() {
    LAST_OK.with(|ok| ok.set(false));
    LAST_RESULT.with(|result| result.borrow_mut().clear());
}

unsafe fn wasm_input<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    if ptr.is_null() && len != 0 {
        return Err("source pointer is null".to_string());
    }
    let bytes = if len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    std::str::from_utf8(bytes).map_err(|error| format!("source is not valid UTF-8: {error}"))
}

fn store_result(ok: bool, result: impl Into<String>) {
    LAST_OK.with(|last_ok| last_ok.set(ok));
    LAST_RESULT.with(|last_result| {
        *last_result.borrow_mut() = result.into().into_bytes();
    });
}
