use super::{DirectLlvm, LlvmValue, Ty, builtin_output_type_plain, unwrap_faultable_tuple};
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::types::{AnyType, ArrayType, BasicTypeEnum};
use inkwell::values::{ArrayValue, BasicValueEnum, IntValue};
use inkwell::{AddressSpace, IntPredicate};

impl<'ctx, 'a> DirectLlvm<'ctx, 'a> {
    pub(super) fn emit_stdlib_builtin_call(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
        output_ty: Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        if let Some(plain_input_ty) = unwrap_faultable_tuple(&input.ty)
            && let Ty::Faultable(output_inner) = &output_ty
            && let Ok(plain_output_ty) = builtin_output_type_plain(name, &plain_input_ty)
            && (&plain_output_ty == output_inner.as_ref() || plain_output_ty == output_ty)
        {
            let wrapped = self.coerce_faultable_tuple_to_faultable(input, &plain_input_ty)?;
            let value = self.emit_faultable_plain_builtin_call(
                name,
                wrapped,
                &plain_output_ty,
                &output_ty,
            )?;
            return Ok(LlvmValue {
                value,
                ty: output_ty,
            });
        }
        if let Ty::Faultable(input_inner) = input.ty.clone()
            && let Ty::Faultable(output_inner) = &output_ty
            && let Ok(plain_output_ty) = builtin_output_type_plain(name, input_inner.as_ref())
            && (&plain_output_ty == output_inner.as_ref() || plain_output_ty == output_ty)
        {
            let value =
                self.emit_faultable_plain_builtin_call(name, input, &plain_output_ty, &output_ty)?;
            return Ok(LlvmValue {
                value,
                ty: output_ty,
            });
        }
        let value = match name {
            "add" | "sub" | "mul" | "div" | "rem" if matches!(output_ty, Ty::Faultable(_)) => {
                let function_name = match (&output_ty, name) {
                    (Ty::Faultable(inner), "add") if inner.as_ref() == &Ty::I32 => {
                        "fa_faultable_i32_add"
                    }
                    (Ty::Faultable(inner), "add") if inner.as_ref() == &Ty::I64 => {
                        "fa_faultable_i64_add"
                    }
                    (Ty::Faultable(inner), "sub") if inner.as_ref() == &Ty::I32 => {
                        "fa_faultable_i32_sub"
                    }
                    (Ty::Faultable(inner), "sub") if inner.as_ref() == &Ty::I64 => {
                        "fa_faultable_i64_sub"
                    }
                    (Ty::Faultable(inner), "mul") if inner.as_ref() == &Ty::I32 => {
                        "fa_faultable_i32_mul"
                    }
                    (Ty::Faultable(inner), "mul") if inner.as_ref() == &Ty::I64 => {
                        "fa_faultable_i64_mul"
                    }
                    (Ty::Faultable(inner), "div") if inner.as_ref() == &Ty::I32 => {
                        "fa_faultable_i32_div"
                    }
                    (Ty::Faultable(inner), "div") if inner.as_ref() == &Ty::I64 => {
                        "fa_faultable_i64_div"
                    }
                    (Ty::Faultable(inner), "div") if inner.as_ref() == &Ty::F32 => {
                        "fa_faultable_f32_div"
                    }
                    (Ty::Faultable(inner), "div") if inner.as_ref() == &Ty::F64 => {
                        "fa_faultable_f64_div"
                    }
                    (Ty::Faultable(inner), "rem") if inner.as_ref() == &Ty::I32 => {
                        "fa_faultable_i32_rem"
                    }
                    (Ty::Faultable(inner), "rem") if inner.as_ref() == &Ty::I64 => {
                        "fa_faultable_i64_rem"
                    }
                    (Ty::Faultable(inner), "rem") if inner.as_ref() == &Ty::F32 => {
                        "fa_faultable_f32_rem"
                    }
                    (Ty::Faultable(inner), "rem") if inner.as_ref() == &Ty::F64 => {
                        "fa_faultable_f64_rem"
                    }
                    _ => {
                        return Err(format!(
                            "unsupported faultable numeric output `{output_ty}`"
                        ));
                    }
                };
                if matches!(&output_ty, Ty::Faultable(inner) if matches!(inner.as_ref(), Ty::F32 | Ty::F64))
                {
                    let Ty::Tuple(items) = input.ty.clone() else {
                        return Err(format!("`{name}` expected tuple input"));
                    };
                    let [left_ty, right_ty] = items.as_slice() else {
                        return Err(format!("`{name}` expected pair input"));
                    };
                    if left_ty != right_ty || !matches!(left_ty, Ty::F32 | Ty::F64) {
                        return Err(format!(
                            "`{name}` expected matching real operands, found `{left_ty}` and `{right_ty}`"
                        ));
                    }
                    let left = self.extract_tuple_field(&input, 0)?;
                    let right = self.extract_tuple_field(&input, 1)?;
                    let float_ty = self.types.basic_type(left_ty)?;
                    self.emit_runtime_sret_call(
                        function_name,
                        &output_ty,
                        &[float_ty, float_ty],
                        &[
                            left.into_float_value().into(),
                            right.into_float_value().into(),
                        ],
                    )?
                } else {
                    self.emit_runtime_binary(function_name, input, &output_ty)?
                }
            }
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                self.emit_numeric_binary(name, input)?
            }
            "from_int" => self.emit_from_int(input)?,
            "sqrt" if matches!(output_ty, Ty::Faultable(_)) => {
                let Ty::Faultable(inner) = &output_ty else {
                    unreachable!()
                };
                if input.ty != **inner || !matches!(inner.as_ref(), Ty::F32 | Ty::F64) {
                    return Err(format!(
                        "sqrt expected `{inner}` input, found `{}`",
                        input.ty
                    ));
                }
                let function_name = match inner.as_ref() {
                    Ty::F32 => "fa_faultable_sqrtf",
                    Ty::F64 => "fa_faultable_sqrt",
                    _ => unreachable!(),
                };
                let float_ty = self.types.basic_type(inner)?;
                self.emit_runtime_sret_call(
                    function_name,
                    &output_ty,
                    &[float_ty],
                    &[input.value.into_float_value().into()],
                )?
            }
            "neg" | "abs" if matches!(output_ty, Ty::Faultable(_)) => {
                let Ty::Faultable(inner) = &output_ty else {
                    unreachable!()
                };
                if input.ty != **inner || !matches!(inner.as_ref(), Ty::I32 | Ty::I64) {
                    return Err(format!(
                        "{name} expected `{inner}` input, found `{}`",
                        input.ty
                    ));
                }
                let function_name = match (name, inner.as_ref()) {
                    ("neg", Ty::I32) => "fa_faultable_i32_neg",
                    ("neg", Ty::I64) => "fa_faultable_i64_neg",
                    ("abs", Ty::I32) => "fa_faultable_i32_abs",
                    ("abs", Ty::I64) => "fa_faultable_i64_abs",
                    _ => unreachable!(),
                };
                let int_ty = self.types.basic_type(inner)?;
                self.emit_runtime_sret_call(
                    function_name,
                    &output_ty,
                    &[int_ty],
                    &[input.value.into_int_value().into()],
                )?
            }
            "neg" | "abs" | "sqrt" | "exp" | "sin" | "cos" => {
                self.emit_numeric_unary(name, input, &output_ty)?
            }
            "bit_and" | "bit_or" | "bit_xor" | "bit_shl" | "bit_shr" => {
                self.emit_int_binary(name, input)?
            }
            "eq" | "lt" | "gt" | "le" | "ge" => self.emit_compare(name, input)?,
            "and" | "or" | "xor" => self.emit_bool_binary(name, input)?,
            "not" => {
                let zero = self.context.i8_type().const_zero();
                let bit = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, input.value.into_int_value(), zero, "not")
                    .map_err(|error| format!("LLVM backend failed to build not: {error}"))?;
                self.builder
                    .build_int_z_extend(bit, self.context.i8_type(), "bool")
                    .map_err(|error| format!("LLVM backend failed to extend bool: {error}"))?
                    .into()
            }
            "select" => self.emit_select(input)?,
            "first" => self.emit_tuple_project(input, 0, &output_ty)?,
            "second" => self.emit_tuple_project(input, 1, &output_ty)?,
            "argv" => self.emit_runtime_unary("fa_argv", input, &output_ty)?,
            "flag_present" => self.emit_runtime_binary("fa_flag_present", input, &output_ty)?,
            "flag_value" => self.emit_runtime_binary("fa_flag_value", input, &output_ty)?,
            "parse_real" => self.emit_runtime_unary("fa_parse_real", input, &output_ty)?,
            "parse_int" => self.emit_runtime_unary("fa_parse_int", input, &output_ty)?,
            "format_real" => self.emit_maybe_faultable_runtime_unary(
                "fa_format_real",
                input,
                &output_ty,
                &Ty::Bytes,
            )?,
            "format_int" => self.emit_maybe_faultable_runtime_unary(
                "fa_format_int",
                input,
                &output_ty,
                &Ty::Bytes,
            )?,
            "concat_bytes" => self.emit_concat_bytes(input, &output_ty)?,
            "split_lines" => self.emit_maybe_faultable_runtime_unary(
                "fa_split_lines",
                input,
                &output_ty,
                &Ty::Seq(Box::new(Ty::Bytes)),
            )?,
            "trim" => {
                self.emit_maybe_faultable_runtime_unary("fa_trim", input, &output_ty, &Ty::Bytes)?
            }
            "ascii_lower" => self.emit_runtime_unary("fa_ascii_lower", input, &output_ty)?,
            "bytes_to_codes" => self.emit_runtime_unary("fa_bytes_to_codes", input, &output_ty)?,
            "codes_to_bytes" => self.emit_runtime_unary("fa_codes_to_bytes", input, &output_ty)?,
            "replace" => self.emit_runtime_ternary("fa_bytes_replace", input, &output_ty)?,
            "split_on" => self.emit_faultable_runtime_binary(
                "fa_split_on",
                input,
                &output_ty,
                &Ty::Seq(Box::new(Ty::Bytes)),
            )?,
            "join_bytes" => {
                self.emit_faultable_runtime_binary("fa_join_bytes", input, &output_ty, &Ty::Bytes)?
            }
            "format_faults" => self.emit_runtime_unary("fa_format_faults", input, &output_ty)?,
            "expect" => self.emit_expect(input, &output_ty)?,
            "index_of" => self.emit_runtime_binary("fa_index_of", input, &output_ty)?,
            "last_index_of" => self.emit_runtime_binary("fa_last_index_of", input, &output_ty)?,
            "slice" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64, Ty::I64])) => {
                self.emit_runtime_ternary("fa_bytes_slice", input, &output_ty)?
            }
            "take" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64])) => {
                self.emit_runtime_binary("fa_bytes_take", input, &output_ty)?
            }
            "drop" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::I64])) => {
                self.emit_runtime_binary("fa_bytes_drop", input, &output_ty)?
            }
            "repeat_bytes" => self.emit_runtime_binary("fa_bytes_repeat", input, &output_ty)?,
            "ascii_upper" => self.emit_runtime_unary("fa_ascii_upper", input, &output_ty)?,
            "strip_prefix" => {
                self.emit_faultable_runtime_binary_result("fa_strip_prefix", input, &output_ty)?
            }
            "strip_suffix" => {
                self.emit_faultable_runtime_binary_result("fa_strip_suffix", input, &output_ty)?
            }
            "contains" => self.emit_runtime_binary("fa_bytes_contains", input, &output_ty)?,
            "starts_with" => self.emit_runtime_binary("fa_bytes_starts_with", input, &output_ty)?,
            "ends_with" => self.emit_runtime_binary("fa_bytes_ends_with", input, &output_ty)?,
            "byte_length" => self.emit_byte_length(input, &output_ty)?,
            "length" => self.emit_length(input, &output_ty)?,
            "inner_length" => self.emit_inner_length(input, &output_ty)?,
            "has_faults" => self.emit_not_empty(input)?,
            "not_empty" => self.emit_not_empty(input)?,
            "is_empty" => self.emit_is_empty(input)?,
            "shift_right" => self.emit_shift_right(input, &output_ty)?,
            "zip" => self.emit_zip(input, &output_ty)?,
            "group_by_id" => self.emit_group_by_id(input, &output_ty)?,
            "head" => self.emit_head(input, &output_ty)?,
            "tail" => self.emit_tail(input, &output_ty)?,
            "reverse" => self.emit_reverse(input, &output_ty)?,
            "take" => self.emit_take(input, &output_ty)?,
            "drop" => self.emit_drop(input, &output_ty)?,
            "fill" => self.emit_fill(input, &output_ty)?,
            "slice" => self.emit_slice(input, &output_ty)?,
            "last" => self.emit_last(input, &output_ty)?,
            "at" => self.emit_at(input, &output_ty)?,
            "get" => self.emit_get(input)?,
            "get_or" => self.emit_get_or(input, &output_ty)?,
            "append" => self.emit_append(input, &output_ty)?,
            "set" => self.emit_set(input, &output_ty)?,
            "transpose" => self.emit_transpose(input, &output_ty)?,
            "flatten" => self.emit_flatten(input, &output_ty)?,
            "concat" => self.emit_seq_concat(input, &output_ty)?,
            "broadcast_left" => self.emit_broadcast_left(input, &output_ty)?,
            "broadcast_right" => self.emit_broadcast_right(input, &output_ty)?,
            "collect" => self.emit_collect(input, &output_ty)?,
            "to_seq" => self.emit_stream_to_seq(input, &output_ty)?,
            "any" => self.emit_all_any(input, false)?,
            "all" => self.emit_all_any(input, true)?,
            "exists" => self.emit_runtime_unary("fa_path_exists", input, &output_ty)?,
            "is_file" => self.emit_runtime_unary("fa_path_is_file", input, &output_ty)?,
            "is_dir" => self.emit_runtime_unary("fa_path_is_dir", input, &output_ty)?,
            "file_size" => {
                self.emit_faultable_runtime_unary_result("fa_path_file_size", input, &output_ty)?
            }
            "basename" => self.emit_runtime_unary("fa_basename", input, &output_ty)?,
            "dirname" => self.emit_runtime_unary("fa_dirname", input, &output_ty)?,
            "join_path" => self.emit_runtime_binary("fa_join_path", input, &output_ty)?,
            "read_file" => {
                self.emit_faultable_runtime_unary_result("fa_read_file", input, &output_ty)?
            }
            "write_file" => {
                self.emit_faultable_runtime_binary_result("fa_write_file", input, &output_ty)?
            }
            "walk_files" => {
                self.emit_faultable_runtime_unary_result("fa_walk_files", input, &output_ty)?
            }
            "list_dir" => {
                self.emit_faultable_runtime_unary_result("fa_list_dir", input, &output_ty)?
            }
            "read_files" => {
                self.emit_faultable_runtime_unary_result("fa_read_files", input, &output_ty)?
            }
            "open_file" => self.emit_runtime_unary("fa_open_file", input, &output_ty)?,
            "size" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_stream_size_ptr", input, &output_ty)?
            }
            "read_at" => self.emit_runtime_ternary_ptr_first_arg_sret(
                "fa_stream_read_at_ptr",
                input,
                &output_ty,
            )?,
            "copy_to_file" => self.emit_runtime_binary_ptr_first_arg_sret(
                "fa_copy_stream_to_file_ptr",
                input,
                &output_ty,
            )?,
            "close" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_close_stream_ptr", input, &output_ty)?
            }
            "write_stdout" => self.emit_maybe_faultable_runtime_unary(
                "fa_write_stdout",
                input,
                &output_ty,
                &Ty::I64,
            )?,
            "write_stderr" => self.emit_maybe_faultable_runtime_unary(
                "fa_write_stderr",
                input,
                &output_ty,
                &Ty::I64,
            )?,
            "read_stdin" => {
                let fn_value = self.runtime_function(
                    "fa_read_stdin",
                    Some(self.runtime_pair_type().into()),
                    &[],
                )?;
                let call = self
                    .builder
                    .build_call(fn_value, &[], "read_stdin")
                    .map_err(|error| format!("LLVM backend failed to call read_stdin: {error}"))?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "fa_read_stdin did not return a value".to_string())?;
                self.runtime_pair_to_value(call, &output_ty)?
            }
            "sqlite.open" => self.emit_runtime_unary("fa_sqlite_open", input, &output_ty)?,
            "sqlite.open_readonly" => {
                self.emit_runtime_unary("fa_sqlite_open_readonly", input, &output_ty)?
            }
            "sqlite.open_memory" => {
                self.emit_runtime_unary("fa_sqlite_open_memory", input, &output_ty)?
            }
            "sqlite.close" => self.emit_runtime_unary("fa_sqlite_close", input, &output_ty)?,
            "sqlite.busy_timeout" => {
                self.emit_runtime_binary("fa_sqlite_busy_timeout", input, &output_ty)?
            }
            "sqlite.foreign_keys" => {
                self.emit_runtime_binary("fa_sqlite_foreign_keys", input, &output_ty)?
            }
            "sqlite.begin" => self.emit_runtime_unary("fa_sqlite_begin", input, &output_ty)?,
            "sqlite.begin_immediate" => {
                self.emit_runtime_unary("fa_sqlite_begin_immediate", input, &output_ty)?
            }
            "sqlite.commit" => self.emit_runtime_unary("fa_sqlite_commit", input, &output_ty)?,
            "sqlite.rollback" => {
                self.emit_runtime_unary("fa_sqlite_rollback", input, &output_ty)?
            }
            "sqlite.null" => self.emit_runtime_unary_sret("fa_sqlite_null", input, &output_ty)?,
            "sqlite.int" => self.emit_runtime_unary_sret("fa_sqlite_int", input, &output_ty)?,
            "sqlite.real" => self.emit_runtime_unary_sret("fa_sqlite_real", input, &output_ty)?,
            "sqlite.text" => self.emit_runtime_unary_sret("fa_sqlite_text", input, &output_ty)?,
            "sqlite.blob" => self.emit_runtime_unary_sret("fa_sqlite_blob", input, &output_ty)?,
            "sqlite.exec" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_exec", input, &output_ty)?
            }
            "sqlite.query" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_query", input, &output_ty)?
            }
            "sqlite.query_all" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_query_all", input, &output_ty)?
            }
            "sqlite.column_count" => {
                self.emit_runtime_unary_ptr_arg("fa_sqlite_column_count", input, &output_ty)?
            }
            "sqlite.column_name" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_column_name", input, &output_ty)?
            }
            "sqlite.value_at" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_value_at", input, &output_ty)?
            }
            "sqlite.value_named" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_value_named", input, &output_ty)?
            }
            "sqlite.kind" => {
                self.emit_runtime_unary_ptr_arg("fa_sqlite_kind", input, &output_ty)?
            }
            "sqlite.is_null" => {
                self.emit_runtime_unary_ptr_arg("fa_sqlite_is_null", input, &output_ty)?
            }
            "sqlite.as_int" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_as_int", input, &output_ty)?
            }
            "sqlite.as_real" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_as_real", input, &output_ty)?
            }
            "sqlite.as_text" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_as_text", input, &output_ty)?
            }
            "sqlite.as_blob" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_sqlite_as_blob", input, &output_ty)?
            }
            "default_config" => {
                self.emit_runtime_nullary_sret("fa_http_default_config", &output_ty)?
            }
            "with_tcp_listener" => self.emit_runtime_ternary_ptr_first_arg_sret(
                "fa_http_with_tcp_listener",
                input,
                &output_ty,
            )?,
            "with_tls" => {
                self.emit_runtime_ternary_ptr_first_arg_sret("fa_http_with_tls", input, &output_ty)?
            }
            "with_http2" => self.emit_runtime_binary_ptr_first_arg_sret(
                "fa_http_with_http2",
                input,
                &output_ty,
            )?,
            "with_http3" => self.emit_runtime_binary_ptr_first_arg_sret(
                "fa_http_with_http3",
                input,
                &output_ty,
            )?,
            "listen" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_listen", input, &output_ty)?
            }
            "requests" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_requests", input, &output_ty)?
            }
            "serve" => self.emit_runtime_unary_ptr_arg_sret("fa_http_serve", input, &output_ty)?,
            "route" => self.emit_runtime_unary_ptr_arg_bool("fa_http_route", input)?,
            "body" => self.emit_runtime_unary_ptr_arg("fa_http_body", input, &output_ty)?,
            "response" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_response", input, &output_ty)?
            }
            "with_status" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_with_status", input, &output_ty)?
            }
            "with_header" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_with_header", input, &output_ty)?
            }
            "text" => self.emit_runtime_unary_ptr_arg_sret("fa_http_text", input, &output_ty)?,
            "json" => self.emit_runtime_unary_ptr_arg_sret("fa_http_json", input, &output_ty)?,
            "not_found" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_http_not_found", input, &output_ty)?
            }
            "decode" => self.emit_runtime_unary("fa_cv_decode", input, &output_ty)?,
            "decode_bmp" => self.emit_runtime_unary("fa_cv_decode_bmp", input, &output_ty)?,
            "decode_jpeg" => self.emit_runtime_unary("fa_cv_decode_jpeg", input, &output_ty)?,
            "decode_png" => self.emit_runtime_unary("fa_cv_decode_png", input, &output_ty)?,
            "decode_pnm" => self.emit_runtime_unary("fa_cv_decode_pnm", input, &output_ty)?,
            "encode_bmp" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_cv_encode_bmp", input, &output_ty)?
            }
            "encode_jpeg" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_cv_encode_jpeg", input, &output_ty)?
            }
            "encode_pgm" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_cv_encode_pgm", input, &output_ty)?
            }
            "encode_png" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_cv_encode_png", input, &output_ty)?
            }
            "encode_ppm" => {
                self.emit_runtime_unary_ptr_arg_sret("fa_cv_encode_ppm", input, &output_ty)?
            }
            "range_step" => self.emit_range_step(input, &output_ty)?,
            other => {
                return Err(format!(
                    "direct LLVM backend does not yet support builtin `{other}`"
                ));
            }
        };
        Ok(LlvmValue {
            value,
            ty: output_ty,
        })
    }

    pub(super) fn runtime_pair_type(&self) -> ArrayType<'ctx> {
        self.context.i64_type().array_type(2)
    }

    pub(super) fn runtime_abi_type(&mut self, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            Ty::Unit | Ty::SqliteConnection => Ok(self.context.i64_type().into()),
            Ty::Args | Ty::Bytes | Ty::Fault | Ty::Seq(_) => Ok(self.runtime_pair_type().into()),
            other => self.types.basic_type(other),
        }
    }

    pub(super) fn runtime_pair(
        &mut self,
        first: IntValue<'ctx>,
        second: IntValue<'ctx>,
        label: &str,
    ) -> Result<ArrayValue<'ctx>, String> {
        let mut pair = self.runtime_pair_type().const_zero();
        pair = self
            .builder
            .build_insert_value(pair, first, 0, label)
            .map_err(|error| format!("LLVM backend failed to build runtime pair: {error}"))?
            .into_array_value();
        pair = self
            .builder
            .build_insert_value(pair, second, 1, label)
            .map_err(|error| format!("LLVM backend failed to build runtime pair: {error}"))?
            .into_array_value();
        Ok(pair)
    }

    pub(super) fn emit_runtime_nullary_sret(
        &mut self,
        function_name: &str,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.emit_runtime_sret_call(function_name, output_ty, &[], &[])
    }

    pub(super) fn value_to_runtime_arg(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        match ty {
            Ty::Unit => Ok(i64_ty.const_zero().into()),
            Ty::Args => {
                let args = value.into_struct_value();
                let argc = self
                    .builder
                    .build_extract_value(args, 0, "argc")
                    .map_err(|error| format!("LLVM backend failed to extract argc: {error}"))?
                    .into_int_value();
                let argv = self
                    .builder
                    .build_extract_value(args, 1, "argv")
                    .map_err(|error| format!("LLVM backend failed to extract argv: {error}"))?
                    .into_pointer_value();
                let argc = self
                    .builder
                    .build_int_z_extend(argc, i64_ty, "argc.i64")
                    .map_err(|error| format!("LLVM backend failed to extend argc: {error}"))?;
                let argv = self
                    .builder
                    .build_ptr_to_int(argv, i64_ty, "argv.i64")
                    .map_err(|error| format!("LLVM backend failed to convert argv: {error}"))?;
                Ok(self.runtime_pair(argc, argv, "args.abi")?.into())
            }
            Ty::Bytes => self.bytes_to_runtime_pair(value),
            Ty::Fault => {
                let bytes = self
                    .builder
                    .build_extract_value(value.into_struct_value(), 0, "fault.bytes")
                    .map_err(|error| {
                        format!("LLVM backend failed to extract fault bytes: {error}")
                    })?;
                self.bytes_to_runtime_pair(bytes)
            }
            Ty::Seq(_) => {
                let seq = value.into_struct_value();
                let count = self
                    .builder
                    .build_extract_value(seq, 0, "seq.count")
                    .map_err(|error| {
                        format!("LLVM backend failed to extract sequence count: {error}")
                    })?
                    .into_int_value();
                let items = self
                    .builder
                    .build_extract_value(seq, 1, "seq.items")
                    .map_err(|error| {
                        format!("LLVM backend failed to extract sequence items: {error}")
                    })?
                    .into_pointer_value();
                let items = self
                    .builder
                    .build_ptr_to_int(items, i64_ty, "seq.items.i64")
                    .map_err(|error| {
                        format!("LLVM backend failed to convert sequence items: {error}")
                    })?;
                Ok(self.runtime_pair(count, items, "seq.abi")?.into())
            }
            Ty::SqliteConnection => {
                let connection = value.into_struct_value();
                let state = self
                    .builder
                    .build_extract_value(connection, 0, "sqlite.state")
                    .map_err(|error| {
                        format!("LLVM backend failed to extract sqlite connection: {error}")
                    })?
                    .into_pointer_value();
                Ok(self
                    .builder
                    .build_ptr_to_int(state, i64_ty, "sqlite.state.i64")
                    .map_err(|error| {
                        format!("LLVM backend failed to convert sqlite connection: {error}")
                    })?
                    .into())
            }
            _ => Ok(value),
        }
    }

    fn bytes_to_runtime_pair(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        let bytes = value.into_struct_value();
        let ptr = self
            .builder
            .build_extract_value(bytes, 0, "bytes.ptr")
            .map_err(|error| format!("LLVM backend failed to extract bytes pointer: {error}"))?
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(bytes, 1, "bytes.len")
            .map_err(|error| format!("LLVM backend failed to extract bytes length: {error}"))?
            .into_int_value();
        let ptr = self
            .builder
            .build_ptr_to_int(ptr, i64_ty, "bytes.ptr.i64")
            .map_err(|error| format!("LLVM backend failed to convert bytes pointer: {error}"))?;
        Ok(self.runtime_pair(ptr, len, "bytes.abi")?.into())
    }

    pub(super) fn runtime_pair_to_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let pair = value.into_array_value();
        let first = self
            .builder
            .build_extract_value(pair, 0, "abi.first")
            .map_err(|error| format!("LLVM backend failed to extract runtime pair: {error}"))?
            .into_int_value();
        let second = self
            .builder
            .build_extract_value(pair, 1, "abi.second")
            .map_err(|error| format!("LLVM backend failed to extract runtime pair: {error}"))?
            .into_int_value();
        match ty {
            Ty::Bytes => {
                let ptr = self
                    .builder
                    .build_int_to_ptr(
                        first,
                        self.context.ptr_type(AddressSpace::default()),
                        "bytes.ptr",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to convert bytes pointer: {error}")
                    })?;
                let mut bytes = self
                    .types
                    .basic_type(&Ty::Bytes)?
                    .into_struct_type()
                    .const_zero();
                bytes = self
                    .builder
                    .build_insert_value(bytes, ptr, 0, "bytes")
                    .map_err(|error| format!("LLVM backend failed to build bytes value: {error}"))?
                    .into_struct_value();
                bytes = self
                    .builder
                    .build_insert_value(bytes, second, 1, "bytes")
                    .map_err(|error| format!("LLVM backend failed to build bytes value: {error}"))?
                    .into_struct_value();
                Ok(bytes.into())
            }
            Ty::Seq(_) => {
                let ptr = self
                    .builder
                    .build_int_to_ptr(
                        second,
                        self.context.ptr_type(AddressSpace::default()),
                        "seq.items",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to convert sequence items: {error}")
                    })?;
                let mut seq = self.types.basic_type(ty)?.into_struct_type().const_zero();
                seq = self
                    .builder
                    .build_insert_value(seq, first, 0, "seq")
                    .map_err(|error| {
                        format!("LLVM backend failed to build sequence value: {error}")
                    })?
                    .into_struct_value();
                seq = self
                    .builder
                    .build_insert_value(seq, ptr, 1, "seq")
                    .map_err(|error| {
                        format!("LLVM backend failed to build sequence value: {error}")
                    })?
                    .into_struct_value();
                Ok(seq.into())
            }
            Ty::Fault => {
                let bytes = self.runtime_pair_to_value(value, &Ty::Bytes)?;
                let mut fault = self
                    .types
                    .basic_type(&Ty::Fault)?
                    .into_struct_type()
                    .const_zero();
                fault = self
                    .builder
                    .build_insert_value(fault, bytes, 0, "fault")
                    .map_err(|error| format!("LLVM backend failed to build fault value: {error}"))?
                    .into_struct_value();
                Ok(fault.into())
            }
            other => Err(format!("runtime pair cannot be converted to `{other}`")),
        }
    }

    pub(super) fn emit_runtime_unary(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let input_value = self.value_to_runtime_arg(input.value, &input.ty)?;
        let input_llvm_ty = self.runtime_abi_type(&input.ty)?;
        if matches!(output_ty, Ty::Faultable(_)) {
            let sret = self.context.create_type_attribute(
                Attribute::get_named_enum_kind_id("sret"),
                output_llvm_ty.as_any_type_enum(),
            );
            let fn_value = self.runtime_function(
                function_name,
                None,
                &[
                    self.context.ptr_type(AddressSpace::default()).into(),
                    input_llvm_ty,
                ],
            )?;
            fn_value.add_attribute(AttributeLoc::Param(0), sret);
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, function_name)
                .map_err(|error| {
                    format!("LLVM backend failed to allocate `{function_name}` result: {error}")
                })?;
            let call = self
                .builder
                .build_call(
                    fn_value,
                    &[out_ptr.into(), input_value.into()],
                    function_name,
                )
                .map_err(|error| {
                    format!("LLVM backend failed to call `{function_name}`: {error}")
                })?;
            call.add_attribute(AttributeLoc::Param(0), sret);
            call.set_alignment_attribute(AttributeLoc::Param(0), 8);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, function_name)
                .map_err(|error| {
                    format!("LLVM backend failed to load `{function_name}` result: {error}")
                });
        }

        let return_ty = self.runtime_abi_type(output_ty)?;
        let fn_value = self.runtime_function(function_name, Some(return_ty), &[input_llvm_ty])?;
        let call = self
            .builder
            .build_call(fn_value, &[input_value.into()], function_name)
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))?;
        match output_ty {
            Ty::Bytes | Ty::Seq(_) => self.runtime_pair_to_value(call, output_ty),
            _ => Ok(call),
        }
    }

    pub(super) fn emit_runtime_unary_sret(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let input_value = self.value_to_runtime_arg(input.value, &input.ty)?;
        let input_llvm_ty = self.runtime_abi_type(&input.ty)?;
        self.emit_runtime_sret_call(function_name, output_ty, &[input_llvm_ty], &[input_value])
    }

    pub(super) fn emit_runtime_unary_ptr_arg(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let input_ptr = self.store_runtime_pointer_arg(function_name, input)?;
        let return_ty = self.runtime_abi_type(output_ty)?;
        let fn_value = self.runtime_function(
            function_name,
            Some(return_ty),
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        let call = self
            .builder
            .build_call(fn_value, &[input_ptr.into()], function_name)
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))?;
        match output_ty {
            Ty::Bytes | Ty::Seq(_) | Ty::Fault => self.runtime_pair_to_value(call, output_ty),
            _ => Ok(call),
        }
    }

    pub(super) fn emit_runtime_unary_ptr_arg_bool(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let input_ptr = self.store_runtime_pointer_arg(function_name, input)?;
        let fn_value = self.runtime_function(
            function_name,
            Some(self.context.bool_type().into()),
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        let call = self
            .builder
            .build_call(fn_value, &[input_ptr.into()], function_name)
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))?
            .into_int_value();
        Ok(self
            .builder
            .build_int_z_extend(call, self.context.i8_type(), function_name)
            .map_err(|error| {
                format!("LLVM backend failed to extend `{function_name}` bool: {error}")
            })?
            .into())
    }

    pub(super) fn emit_runtime_unary_ptr_arg_sret(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let input_ptr = self.store_runtime_pointer_arg(function_name, input)?;
        self.emit_runtime_sret_call(
            function_name,
            output_ty,
            &[self.context.ptr_type(AddressSpace::default()).into()],
            &[input_ptr.into()],
        )
    }

    pub(super) fn emit_runtime_binary_ptr_first_arg_sret(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err(format!("`{function_name}` expected tuple input"));
        };
        let [first_ty, second_ty] = items.as_slice() else {
            return Err(format!("`{function_name}` expected pair input"));
        };
        let first = LlvmValue {
            value: self.extract_tuple_field(&input, 0)?,
            ty: first_ty.clone(),
        };
        let first_ptr = self.store_runtime_pointer_arg(function_name, first)?;
        let second = self.extract_tuple_field(&input, 1)?;
        let second_arg = self.value_to_runtime_arg(second, second_ty)?;
        let second_abi_ty = self.runtime_abi_type(second_ty)?;
        self.emit_runtime_sret_call(
            function_name,
            output_ty,
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                second_abi_ty,
            ],
            &[first_ptr.into(), second_arg],
        )
    }

    pub(super) fn emit_runtime_ternary_ptr_first_arg_sret(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err(format!("`{function_name}` expected tuple input"));
        };
        let [first_ty, second_ty, third_ty] = items.as_slice() else {
            return Err(format!("`{function_name}` expected triple input"));
        };
        let first = LlvmValue {
            value: self.extract_tuple_field(&input, 0)?,
            ty: first_ty.clone(),
        };
        let first_ptr = self.store_runtime_pointer_arg(function_name, first)?;
        let second = self.extract_tuple_field(&input, 1)?;
        let third = self.extract_tuple_field(&input, 2)?;
        let second_arg = self.value_to_runtime_arg(second, second_ty)?;
        let third_arg = self.value_to_runtime_arg(third, third_ty)?;
        let second_abi_ty = self.runtime_abi_type(second_ty)?;
        let third_abi_ty = self.runtime_abi_type(third_ty)?;
        self.emit_runtime_sret_call(
            function_name,
            output_ty,
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                second_abi_ty,
                third_abi_ty,
            ],
            &[first_ptr.into(), second_arg, third_arg],
        )
    }

    pub(super) fn emit_faultable_runtime_unary_result(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return self.emit_runtime_unary(function_name, input, output_ty);
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable `{function_name}` expected faultable output, found `{output_ty}`"
            ));
        };
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate faultable `{function_name}`: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, "runtime_unary.fault");
        let ok_block = self
            .context
            .append_basic_block(function, "runtime_unary.ok");
        let after_block = self
            .context
            .append_basic_block(function, "runtime_unary.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on faultable `{function_name}`: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable `{function_name}` fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` fault: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let runtime_result = self.emit_runtime_unary(
            function_name,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
            output_ty,
        )?;
        self.builder
            .build_store(out_ptr, runtime_result)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable `{function_name}` value: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` ok: {error}")
            })?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to load faultable `{function_name}`: {error}")
            })
    }

    pub(super) fn emit_runtime_sret_call(
        &mut self,
        function_name: &str,
        output_ty: &Ty,
        input_tys: &[BasicTypeEnum<'ctx>],
        input_values: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let sret = self.context.create_type_attribute(
            Attribute::get_named_enum_kind_id("sret"),
            output_llvm_ty.as_any_type_enum(),
        );
        let mut params = Vec::with_capacity(input_tys.len() + 1);
        params.push(self.context.ptr_type(AddressSpace::default()).into());
        params.extend(input_tys.iter().copied());
        let fn_value = self.runtime_function(function_name, None, &params)?;
        fn_value.add_attribute(AttributeLoc::Param(0), sret);
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate `{function_name}` result: {error}")
            })?;
        let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(input_values.len() + 1);
        args.push(out_ptr.into());
        for value in input_values {
            args.push(inkwell::values::BasicMetadataValueEnum::from(*value));
        }
        let call = self
            .builder
            .build_call(fn_value, &args, function_name)
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?;
        call.add_attribute(AttributeLoc::Param(0), sret);
        call.set_alignment_attribute(AttributeLoc::Param(0), 8);
        self.builder
            .build_load(output_llvm_ty, out_ptr, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to load `{function_name}` result: {error}")
            })
    }

    fn store_runtime_pointer_arg(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let input_llvm_ty = self.types.basic_type(&input.ty)?;
        let input_ptr = self
            .builder
            .build_alloca(input_llvm_ty, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate `{function_name}` input: {error}")
            })?;
        self.builder
            .build_store(input_ptr, input.value)
            .map_err(|error| {
                format!("LLVM backend failed to store `{function_name}` input: {error}")
            })?;
        Ok(input_ptr)
    }

    pub(super) fn emit_runtime_binary(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err(format!("`{function_name}` expected tuple input"));
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err(format!("`{function_name}` expected pair input"));
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let left_arg = self.value_to_runtime_arg(left, left_ty)?;
        let right_arg = self.value_to_runtime_arg(right, right_ty)?;
        let left_abi_ty = self.runtime_abi_type(left_ty)?;
        let right_abi_ty = self.runtime_abi_type(right_ty)?;
        let output_llvm_ty = self.types.basic_type(output_ty)?;

        if matches!(output_ty, Ty::Faultable(_)) {
            let sret = self.context.create_type_attribute(
                Attribute::get_named_enum_kind_id("sret"),
                output_llvm_ty.as_any_type_enum(),
            );
            let fn_value = self.runtime_function(
                function_name,
                None,
                &[
                    self.context.ptr_type(AddressSpace::default()).into(),
                    left_abi_ty,
                    right_abi_ty,
                ],
            )?;
            fn_value.add_attribute(AttributeLoc::Param(0), sret);
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, function_name)
                .map_err(|error| {
                    format!("LLVM backend failed to allocate `{function_name}` result: {error}")
                })?;
            let call = self
                .builder
                .build_call(
                    fn_value,
                    &[out_ptr.into(), left_arg.into(), right_arg.into()],
                    function_name,
                )
                .map_err(|error| {
                    format!("LLVM backend failed to call `{function_name}`: {error}")
                })?;
            call.add_attribute(AttributeLoc::Param(0), sret);
            call.set_alignment_attribute(AttributeLoc::Param(0), 8);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, function_name)
                .map_err(|error| {
                    format!("LLVM backend failed to load `{function_name}` result: {error}")
                });
        }

        let return_ty = self.runtime_abi_type(output_ty)?;
        let fn_value =
            self.runtime_function(function_name, Some(return_ty), &[left_abi_ty, right_abi_ty])?;
        let call = self
            .builder
            .build_call(
                fn_value,
                &[left_arg.into(), right_arg.into()],
                function_name,
            )
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))?;
        match output_ty {
            Ty::Bytes | Ty::Seq(_) => self.runtime_pair_to_value(call, output_ty),
            Ty::Bool => Ok(self
                .builder
                .build_int_z_extend(call.into_int_value(), self.context.i8_type(), "bool")
                .map_err(|error| format!("LLVM backend failed to extend runtime bool: {error}"))?
                .into()),
            _ => Ok(call),
        }
    }

    pub(super) fn emit_faultable_runtime_binary(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
        plain_output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(plain_input_ty) = super::unwrap_faultable_tuple(&input.ty) {
            let wrapped = self.coerce_faultable_tuple_to_faultable(input, &plain_input_ty)?;
            return self.emit_faultable_runtime_binary(
                function_name,
                wrapped,
                output_ty,
                plain_output_ty,
            );
        }
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return self.emit_runtime_binary(function_name, input, output_ty);
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable `{function_name}` expected faultable output, found `{output_ty}`"
            ));
        };
        if output_inner.as_ref() != plain_output_ty {
            return Err(format!("faultable output mismatch for `{function_name}`"));
        }
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate faultable `{function_name}`: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, "runtime_binary.fault");
        let ok_block = self
            .context
            .append_basic_block(function, "runtime_binary.ok");
        let after_block = self
            .context
            .append_basic_block(function, "runtime_binary.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on faultable `{function_name}`: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable `{function_name}` fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` fault: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let plain = self.emit_runtime_binary(
            function_name,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
            plain_output_ty,
        )?;
        let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
        self.builder.build_store(out_ptr, ok).map_err(|error| {
            format!("LLVM backend failed to store faultable `{function_name}` value: {error}")
        })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` ok: {error}")
            })?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to load faultable `{function_name}`: {error}")
            })
    }

    pub(super) fn emit_faultable_runtime_binary_result(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(plain_input_ty) = super::unwrap_faultable_tuple(&input.ty) {
            let wrapped = self.coerce_faultable_tuple_to_faultable(input, &plain_input_ty)?;
            return self.emit_faultable_runtime_binary_result(function_name, wrapped, output_ty);
        }
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return self.emit_runtime_binary(function_name, input, output_ty);
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable `{function_name}` expected faultable output, found `{output_ty}`"
            ));
        };
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate faultable `{function_name}`: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, "runtime_result.fault");
        let ok_block = self
            .context
            .append_basic_block(function, "runtime_result.ok");
        let after_block = self
            .context
            .append_basic_block(function, "runtime_result.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on faultable `{function_name}`: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable `{function_name}` fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` fault: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let runtime_result = self.emit_runtime_binary(
            function_name,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
            output_ty,
        )?;
        self.builder
            .build_store(out_ptr, runtime_result)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable `{function_name}` value: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable `{function_name}` ok: {error}")
            })?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, function_name)
            .map_err(|error| {
                format!("LLVM backend failed to load faultable `{function_name}`: {error}")
            })
    }

    pub(super) fn emit_runtime_ternary(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err(format!("`{function_name}` expected tuple input"));
        };
        let [first_ty, second_ty, third_ty] = items.as_slice() else {
            return Err(format!("`{function_name}` expected triple input"));
        };
        let first = self.extract_tuple_field(&input, 0)?;
        let second = self.extract_tuple_field(&input, 1)?;
        let third = self.extract_tuple_field(&input, 2)?;
        let first_arg = self.value_to_runtime_arg(first, first_ty)?;
        let second_arg = self.value_to_runtime_arg(second, second_ty)?;
        let third_arg = self.value_to_runtime_arg(third, third_ty)?;
        let first_abi_ty = self.runtime_abi_type(first_ty)?;
        let second_abi_ty = self.runtime_abi_type(second_ty)?;
        let third_abi_ty = self.runtime_abi_type(third_ty)?;
        let return_ty = self.runtime_abi_type(output_ty)?;
        let fn_value = self.runtime_function(
            function_name,
            Some(return_ty),
            &[first_abi_ty, second_abi_ty, third_abi_ty],
        )?;
        let call = self
            .builder
            .build_call(
                fn_value,
                &[first_arg.into(), second_arg.into(), third_arg.into()],
                function_name,
            )
            .map_err(|error| format!("LLVM backend failed to call `{function_name}`: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("runtime function `{function_name}` did not return a value"))?;
        match output_ty {
            Ty::Bytes | Ty::Seq(_) => self.runtime_pair_to_value(call, output_ty),
            _ => Ok(call),
        }
    }

    pub(super) fn emit_byte_length(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            if input_inner.as_ref() != &Ty::Bytes {
                return Err(format!("byte_length expected Bytes, found `{}`", input.ty));
            }
            return self.emit_faultable_count(
                input,
                output_ty,
                "byte_length",
                |this, plain_input| this.emit_byte_length(plain_input, &Ty::I64),
            );
        }
        if input.ty != Ty::Bytes {
            return Err(format!("byte_length expected Bytes, found `{}`", input.ty));
        }
        self.builder
            .build_extract_value(input.value.into_struct_value(), 1, "byte_length")
            .map_err(|error| format!("LLVM backend failed to extract byte length: {error}"))
    }

    pub(super) fn emit_length(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            if !matches!(input_inner.as_ref(), Ty::Seq(_)) {
                return Err(format!("length expected Seq, found `{}`", input.ty));
            }
            return self.emit_faultable_count(input, output_ty, "length", |this, plain_input| {
                this.emit_length(plain_input, &Ty::I64)
            });
        }
        match input.ty {
            Ty::Seq(_) => Ok(self.seq_count(input.value)?.into()),
            other => Err(format!("length expected Seq, found `{other}`")),
        }
    }

    pub(super) fn emit_inner_length(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            if !matches!(input_inner.as_ref(), Ty::Seq(row) if matches!(row.as_ref(), Ty::Seq(_))) {
                return Err(format!(
                    "inner_length expected Seq[Seq[V]], found `{}`",
                    input.ty
                ));
            }
            return self.emit_faultable_count(
                input,
                output_ty,
                "inner_length",
                |this, plain_input| this.emit_inner_length(plain_input, &Ty::I64),
            );
        }
        let Ty::Seq(row_ty) = input.ty.clone() else {
            return Err(format!(
                "inner_length expected Seq[Seq[V]], found `{}`",
                input.ty
            ));
        };
        if !matches!(row_ty.as_ref(), Ty::Seq(_)) {
            return Err(format!(
                "inner_length expected Seq[Seq[V]], found `{}`",
                input.ty
            ));
        }
        let function = self.current_function()?;
        let empty_block = self
            .context
            .append_basic_block(function, "inner_length.empty");
        let row_block = self
            .context
            .append_basic_block(function, "inner_length.row");
        let after_block = self
            .context
            .append_basic_block(function, "inner_length.after");
        let out_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "inner_length")
            .map_err(|error| format!("LLVM backend failed to allocate inner_length: {error}"))?;
        let outer_count = self.seq_count(input.value)?;
        let is_empty = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                outer_count,
                self.context.i64_type().const_zero(),
                "inner_length.empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare inner_length: {error}"))?;
        self.builder
            .build_conditional_branch(is_empty, empty_block, row_block)
            .map_err(|error| format!("LLVM backend failed to branch in inner_length: {error}"))?;
        self.builder.position_at_end(empty_block);
        self.builder
            .build_store(out_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to store empty inner_length: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave empty inner_length: {error}"))?;
        self.builder.position_at_end(row_block);
        let row =
            self.load_seq_item(input.value, &input.ty, self.context.i64_type().const_zero())?;
        let count = self.seq_count(row)?;
        self.builder
            .build_store(out_ptr, count)
            .map_err(|error| format!("LLVM backend failed to store row inner_length: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave row inner_length: {error}"))?;
        self.builder.position_at_end(after_block);
        self.builder
            .build_load(self.context.i64_type(), out_ptr, "inner_length")
            .map_err(|error| format!("LLVM backend failed to load inner_length: {error}"))
    }

    fn emit_faultable_count(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
        name: &str,
        emit_plain: impl FnOnce(&mut Self, LlvmValue<'ctx>) -> Result<BasicValueEnum<'ctx>, String>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return emit_plain(self, input);
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable {name} expected faultable output, found `{output_ty}`"
            ));
        };
        if output_inner.as_ref() != &Ty::I64 {
            return Err(format!("faultable {name} expected Faultable[i64] output"));
        }
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, name)
            .map_err(|error| {
                format!("LLVM backend failed to allocate faultable {name}: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, &format!("{name}.fault"));
        let ok_block = self
            .context
            .append_basic_block(function, &format!("{name}.ok"));
        let after_block = self
            .context
            .append_basic_block(function, &format!("{name}.done"));
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on faultable {name}: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store faultable {name} fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable {name} fault: {error}")
            })?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let plain = emit_plain(
            self,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
        )?;
        let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
        self.builder.build_store(out_ptr, ok).map_err(|error| {
            format!("LLVM backend failed to store faultable {name} value: {error}")
        })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave faultable {name} ok: {error}")
            })?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, name)
            .map_err(|error| format!("LLVM backend failed to load faultable {name}: {error}"))
    }

    pub(super) fn emit_not_empty(
        &mut self,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let size = match input.ty {
            Ty::Bytes => self
                .builder
                .build_extract_value(input.value.into_struct_value(), 1, "len")
                .map_err(|error| format!("LLVM backend failed to extract bytes length: {error}"))?
                .into_int_value(),
            Ty::Seq(_) => self.seq_count(input.value)?,
            other => return Err(format!("not_empty expected Bytes or Seq, found `{other}`")),
        };
        let bit = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                size,
                self.context.i64_type().const_zero(),
                "not_empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare length: {error}"))?;
        Ok(self
            .builder
            .build_int_z_extend(bit, self.context.i8_type(), "bool")
            .map_err(|error| format!("LLVM backend failed to extend bool: {error}"))?
            .into())
    }

    pub(super) fn emit_is_empty(
        &mut self,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let not_empty = self.emit_not_empty(input)?.into_int_value();
        let bit = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                not_empty,
                self.context.i8_type().const_zero(),
                "is_empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare empty: {error}"))?;
        Ok(self
            .builder
            .build_int_z_extend(bit, self.context.i8_type(), "bool")
            .map_err(|error| format!("LLVM backend failed to extend bool: {error}"))?
            .into())
    }

    pub(super) fn emit_maybe_faultable_runtime_unary(
        &mut self,
        function_name: &str,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
        plain_output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(input_inner) = input.ty.clone() else {
            return self.emit_runtime_unary(function_name, input, output_ty);
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "faultable input to `{function_name}` expected faultable output"
            ));
        };
        if output_inner.as_ref() != plain_output_ty {
            return Err(format!("faultable output mismatch for `{function_name}`"));
        }

        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "faultable.call")
            .map_err(|error| format!("LLVM backend failed to allocate faultable call: {error}"))?;
        let function = self.current_function()?;
        let fault_block = self.context.append_basic_block(function, "faultable.fault");
        let ok_block = self.context.append_basic_block(function, "faultable.ok");
        let after_block = self.context.append_basic_block(function, "faultable.after");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch on faultable call: {error}"))?;

        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let faulted = self.faultable_value(plain_output_ty, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store faultable fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave fault block: {error}"))?;

        self.builder.position_at_end(ok_block);
        let plain_input = self.extract_faultable_value(input.value)?;
        let plain = self.emit_runtime_unary(
            function_name,
            LlvmValue {
                value: plain_input,
                ty: input_inner.as_ref().clone(),
            },
            plain_output_ty,
        )?;
        let ok = self.faultable_value(plain_output_ty, false, None, Some(plain))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store faultable ok: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave ok block: {error}"))?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "faultable.result")
            .map_err(|error| format!("LLVM backend failed to load faultable result: {error}"))
    }

    pub(super) fn emit_concat_bytes(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(inner) = input.ty.clone() {
            let Ty::Seq(item) = inner.as_ref() else {
                return Err(format!(
                    "concat_bytes expected Seq[Bytes], found `{}`",
                    input.ty
                ));
            };
            if item.as_ref() != &Ty::Bytes {
                return Err(format!(
                    "concat_bytes expected Seq[Bytes], found `{}`",
                    input.ty
                ));
            }
            return self.emit_maybe_faultable_runtime_unary(
                "fa_concat_bytes",
                input,
                output_ty,
                &Ty::Bytes,
            );
        }
        match &input.ty {
            Ty::Seq(item) if item.as_ref() == &Ty::Bytes => {
                self.emit_runtime_unary("fa_concat_bytes", input, output_ty)
            }
            Ty::Seq(item) if matches!(item.as_ref(), Ty::Faultable(inner) if inner.as_ref() == &Ty::Bytes) => {
                self.emit_faultable_concat_bytes(input, output_ty)
            }
            _ => Err(format!(
                "concat_bytes expected Seq[Bytes], found `{}`",
                input.ty
            )),
        }
    }

    fn emit_faultable_concat_bytes(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "concat.faultable")
            .map_err(|error| format!("LLVM backend failed to allocate concat result: {error}"))?;
        let count = self.seq_count(input.value)?;
        let ok_seq = self.emit_seq_new(&Ty::Seq(Box::new(Ty::Bytes)), count)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "concat.loop");
        let body_block = self.context.append_basic_block(function, "concat.body");
        let after_block = self.context.append_basic_block(function, "concat.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate concat index: {error}"))?;
        let fault_ptr = self
            .builder
            .build_alloca(self.context.i8_type(), "fault")
            .map_err(|error| {
                format!("LLVM backend failed to allocate concat fault flag: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize concat index: {error}"))?;
        self.builder
            .build_store(fault_ptr, self.context.i8_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize concat fault flag: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to branch to concat loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load concat index: {error}"))?
            .into_int_value();
        let fault_flag = self
            .builder
            .build_load(self.context.i8_type(), fault_ptr, "fault")
            .map_err(|error| format!("LLVM backend failed to load concat fault flag: {error}"))?
            .into_int_value();
        let not_faulted = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                fault_flag,
                self.context.i8_type().const_zero(),
                "not_faulted",
            )
            .map_err(|error| {
                format!("LLVM backend failed to compare concat fault flag: {error}")
            })?;
        let in_range = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "concat.cond")
            .map_err(|error| format!("LLVM backend failed to compare concat index: {error}"))?;
        let cond = self
            .builder
            .build_and(not_faulted, in_range, "concat.keep")
            .map_err(|error| {
                format!("LLVM backend failed to build concat loop condition: {error}")
            })?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in concat loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let item_fault = self.extract_faultable_is_fault(item)?;
        let item_fault_block = self
            .context
            .append_basic_block(function, "concat.item_fault");
        let item_ok_block = self.context.append_basic_block(function, "concat.item_ok");
        let item_after_block = self
            .context
            .append_basic_block(function, "concat.item_after");
        self.builder
            .build_conditional_branch(item_fault, item_fault_block, item_ok_block)
            .map_err(|error| format!("LLVM backend failed to branch on concat item: {error}"))?;
        self.builder.position_at_end(item_fault_block);
        let fault = self.extract_faultable_fault(item)?;
        let faulted = self.faultable_value(&Ty::Bytes, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store concat fault: {error}"))?;
        self.builder
            .build_store(fault_ptr, self.context.i8_type().const_int(1, false))
            .map_err(|error| format!("LLVM backend failed to set concat fault flag: {error}"))?;
        self.builder
            .build_unconditional_branch(item_after_block)
            .map_err(|error| format!("LLVM backend failed to leave concat fault item: {error}"))?;
        self.builder.position_at_end(item_ok_block);
        let bytes = self.extract_faultable_value(item)?;
        self.store_seq_item(ok_seq.value, &ok_seq.ty, i, bytes)?;
        self.builder
            .build_unconditional_branch(item_after_block)
            .map_err(|error| format!("LLVM backend failed to leave concat ok item: {error}"))?;
        self.builder.position_at_end(item_after_block);
        let next = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment concat index: {error}"))?;
        self.builder
            .build_store(i_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store concat index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue concat loop: {error}"))?;

        self.builder.position_at_end(after_block);
        let was_faulted = self
            .builder
            .build_load(self.context.i8_type(), fault_ptr, "fault")
            .map_err(|error| {
                format!("LLVM backend failed to load concat final fault flag: {error}")
            })?
            .into_int_value();
        let faulted = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                was_faulted,
                self.context.i8_type().const_zero(),
                "faulted",
            )
            .map_err(|error| {
                format!("LLVM backend failed to compare concat final fault: {error}")
            })?;
        let ok_block = self.context.append_basic_block(function, "concat.final_ok");
        let final_block = self.context.append_basic_block(function, "concat.final");
        self.builder
            .build_conditional_branch(faulted, final_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch on concat final: {error}"))?;
        self.builder.position_at_end(ok_block);
        let bytes = self.emit_runtime_unary("fa_concat_bytes", ok_seq, &Ty::Bytes)?;
        let ok = self.faultable_value(&Ty::Bytes, false, None, Some(bytes))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store concat ok: {error}"))?;
        self.builder
            .build_unconditional_branch(final_block)
            .map_err(|error| format!("LLVM backend failed to leave concat final ok: {error}"))?;
        self.builder.position_at_end(final_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "concat.result")
            .map_err(|error| format!("LLVM backend failed to load concat result: {error}"))
    }
}
