use super::{DirectLlvm, LlvmValue, Ty, builtin_output_type_plain, unwrap_faultable_tuple};
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::types::{AnyType, ArrayType, BasicType, BasicTypeEnum};
use inkwell::values::{ArrayValue, BasicValueEnum, IntValue};
use inkwell::{AddressSpace, IntPredicate};

impl<'ctx, 'a> DirectLlvm<'ctx, 'a> {
    pub(super) fn emit_stdlib_builtin_call(
        &mut self,
        name: &str,
        input: LlvmValue<'ctx>,
        output_ty: Ty,
    ) -> Result<LlvmValue<'ctx>, String> {
        if let Some(plain_input_ty) = unwrap_faultable_tuple(&input.ty) {
            if let Ty::Faultable(output_inner) = &output_ty {
                if let Ok(plain_output_ty) = builtin_output_type_plain(name, &plain_input_ty) {
                    if &plain_output_ty != output_inner.as_ref() && plain_output_ty != output_ty {
                        // Fall through to the builtin-specific lowering.
                    } else {
                        let wrapped =
                            self.coerce_faultable_tuple_to_faultable(input, &plain_input_ty)?;
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
                }
            }
        }
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            if let Ty::Faultable(output_inner) = &output_ty {
                if let Ok(plain_output_ty) = builtin_output_type_plain(name, input_inner.as_ref()) {
                    if &plain_output_ty == output_inner.as_ref() || plain_output_ty == output_ty {
                        let value = self.emit_faultable_plain_builtin_call(
                            name,
                            input,
                            &plain_output_ty,
                            &output_ty,
                        )?;
                        return Ok(LlvmValue {
                            value,
                            ty: output_ty,
                        });
                    }
                }
            }
        }
        let value = match name {
            "add" | "sub" | "mul" | "div" | "rem" | "min" | "max" => {
                self.emit_numeric_binary(name, input)?
            }
            "from_int" => self.emit_from_int(input)?,
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
            "slice" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int, Ty::Int])) => {
                self.emit_runtime_ternary("fa_bytes_slice", input, &output_ty)?
            }
            "take" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) => {
                self.emit_runtime_binary("fa_bytes_take", input, &output_ty)?
            }
            "drop" if matches!(input.ty, Ty::Tuple(ref items) if matches!(items.as_slice(), [Ty::Bytes, Ty::Int])) => {
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
                &Ty::Int,
            )?,
            "write_stderr" => self.emit_maybe_faultable_runtime_unary(
                "fa_write_stderr",
                input,
                &output_ty,
                &Ty::Int,
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

    fn emit_runtime_sret_call(
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
                |this, plain_input| this.emit_byte_length(plain_input, &Ty::Int),
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
                this.emit_length(plain_input, &Ty::Int)
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
                |this, plain_input| this.emit_inner_length(plain_input, &Ty::Int),
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
        if output_inner.as_ref() != &Ty::Int {
            return Err(format!("faultable {name} expected Faultable[Int] output"));
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

    pub(super) fn emit_range_step(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("range_step expected tuple input".to_string());
        };
        if items.as_slice() != [Ty::Int, Ty::Int, Ty::Int] {
            return Err(format!(
                "range_step expected (Int, Int, Int), found `{}`",
                input.ty
            ));
        }
        let start = self.extract_tuple_field(&input, 0)?;
        let end = self.extract_tuple_field(&input, 1)?;
        let step = self.extract_tuple_field(&input, 2)?;
        let fn_value = self.runtime_function(
            "fa_range_step",
            Some(self.runtime_pair_type().into()),
            &[
                self.context.i64_type().into(),
                self.context.i64_type().into(),
                self.context.i64_type().into(),
            ],
        )?;
        let call = self
            .builder
            .build_call(
                fn_value,
                &[start.into(), end.into(), step.into()],
                "range_step",
            )
            .map_err(|error| format!("LLVM backend failed to call fa_range_step: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "fa_range_step did not return a value".to_string())?;
        self.runtime_pair_to_value(call, output_ty)
    }

    pub(super) fn emit_reduce_concat_bytes(
        &mut self,
        input: LlvmValue<'ctx>,
        identity: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if input.ty != Ty::Seq(Box::new(Ty::Bytes)) {
            return Err(format!(
                "reduce concat_bytes expected Seq[Bytes], found `{}`",
                input.ty
            ));
        }
        if identity.ty != Ty::Bytes {
            return Err(format!(
                "reduce concat_bytes identity expected Bytes, found `{}`",
                identity.ty
            ));
        }
        let seq_arg = self.value_to_runtime_arg(input.value, &input.ty)?;
        let identity_arg = self.value_to_runtime_arg(identity.value, &identity.ty)?;
        let fn_value = self.runtime_function(
            "fa_reduce_concat_bytes",
            Some(self.runtime_pair_type().into()),
            &[
                self.runtime_pair_type().into(),
                self.runtime_pair_type().into(),
            ],
        )?;
        let call = self
            .builder
            .build_call(
                fn_value,
                &[seq_arg.into(), identity_arg.into()],
                "reduce_concat_bytes",
            )
            .map_err(|error| {
                format!("LLVM backend failed to call fa_reduce_concat_bytes: {error}")
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "fa_reduce_concat_bytes did not return a value".to_string())?;
        self.runtime_pair_to_value(call, &Ty::Bytes)
    }

    pub(super) fn emit_shift_right(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("shift_right expected tuple input".to_string());
        };
        let [seq_ty, fill_ty] = items.as_slice() else {
            return Err("shift_right expected pair input".to_string());
        };
        let Ty::Seq(item_ty) = seq_ty else {
            return Err("shift_right expected sequence input".to_string());
        };
        if item_ty.as_ref() != fill_ty {
            return Err(format!(
                "shift_right fill expected `{item_ty}`, found `{fill_ty}`"
            ));
        }
        let seq = self.extract_tuple_field(&input, 0)?;
        let fill = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(seq)?;
        let output = self.emit_seq_new(output_ty, count)?;
        let function = self.current_function()?;
        let check_block = self.context.append_basic_block(function, "shift.check");
        let loop_block = self.context.append_basic_block(function, "shift.loop");
        let body_block = self.context.append_basic_block(function, "shift.body");
        let after_block = self.context.append_basic_block(function, "shift.after");
        let non_empty = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                count,
                self.context.i64_type().const_zero(),
                "shift.non_empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare shift count: {error}"))?;
        self.builder
            .build_conditional_branch(non_empty, check_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in shift_right: {error}"))?;
        self.builder.position_at_end(check_block);
        self.store_seq_item(
            output.value,
            output_ty,
            self.context.i64_type().const_zero(),
            fill,
        )?;
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate shift index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_int(1, false))
            .map_err(|error| format!("LLVM backend failed to initialize shift index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter shift loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load shift index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "shift.cond")
            .map_err(|error| format!("LLVM backend failed to compare shift index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in shift loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let prev_i = self
            .builder
            .build_int_sub(i, self.context.i64_type().const_int(1, false), "prev")
            .map_err(|error| format!("LLVM backend failed to decrement shift index: {error}"))?;
        let value = self.load_seq_item(seq, seq_ty, prev_i)?;
        self.store_seq_item(output.value, output_ty, i, value)?;
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment shift index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store shift index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue shift loop: {error}"))?;

        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_zip(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("zip expected tuple input".to_string());
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err("zip expected pair input".to_string());
        };
        let (Ty::Seq(left_item_ty), Ty::Seq(right_item_ty)) = (left_ty, right_ty) else {
            return Err("zip expected sequence inputs".to_string());
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(left)?;
        let right_count = self.seq_count(right)?;
        let same_len = self
            .builder
            .build_int_compare(IntPredicate::EQ, count, right_count, "zip.same_len")
            .map_err(|error| format!("LLVM backend failed to compare zip lengths: {error}"))?;
        let length_mismatch = self
            .builder
            .build_not(same_len, "zip.length_mismatch")
            .map_err(|error| format!("LLVM backend failed to invert zip length check: {error}"))?;
        self.branch_die_if(
            length_mismatch,
            "zip: sequences must have the same length",
            "zip",
        )?;
        let output = self.emit_seq_new(output_ty, count)?;
        let out_item_ty = Ty::Tuple(vec![
            left_item_ty.as_ref().clone(),
            right_item_ty.as_ref().clone(),
        ]);
        let out_item_llvm_ty = self.types.basic_type(&out_item_ty)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "zip.loop");
        let body_block = self.context.append_basic_block(function, "zip.body");
        let after_block = self.context.append_basic_block(function, "zip.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate zip index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize zip index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter zip loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load zip index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "zip.cond")
            .map_err(|error| format!("LLVM backend failed to compare zip index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in zip loop: {error}"))?;

        self.builder.position_at_end(body_block);
        let left_item = self.load_seq_item(left, left_ty, i)?;
        let right_item = self.load_seq_item(right, right_ty, i)?;
        let mut pair = out_item_llvm_ty.into_struct_type().const_zero();
        pair = self
            .builder
            .build_insert_value(pair, left_item, 0, "zip.item")
            .map_err(|error| format!("LLVM backend failed to build zip item: {error}"))?
            .into_struct_value();
        pair = self
            .builder
            .build_insert_value(pair, right_item, 1, "zip.item")
            .map_err(|error| format!("LLVM backend failed to build zip item: {error}"))?
            .into_struct_value();
        self.store_seq_item(output.value, output_ty, i, pair.into())?;
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment zip index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store zip index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue zip loop: {error}"))?;

        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_group_by_id(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("group_by_id expected tuple input".to_string());
        };
        let [values_ty, ids_ty] = items.as_slice() else {
            return Err("group_by_id expected pair input".to_string());
        };
        let (Ty::Seq(value_item_ty), Ty::Seq(id_item_ty)) = (values_ty, ids_ty) else {
            return Err("group_by_id expected sequence inputs".to_string());
        };
        if id_item_ty.as_ref() != &Ty::Int {
            return Err("group_by_id expected Seq[Int] ids".to_string());
        }
        let group_ty = Ty::Seq(value_item_ty.clone());
        let values = self.extract_tuple_field(&input, 0)?;
        let ids = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(values)?;

        let function = self.current_function()?;
        let groups_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "groups")
            .map_err(|error| format!("LLVM backend failed to allocate group count: {error}"))?;
        let zero = self.context.i64_type().const_zero();
        let one = self.context.i64_type().const_int(1, false);
        let non_empty = self
            .builder
            .build_int_compare(IntPredicate::NE, count, zero, "groups.non_empty")
            .map_err(|error| format!("LLVM backend failed to compare group count: {error}"))?;
        let initial_groups = self
            .builder
            .build_select(non_empty, one, zero, "groups.initial")
            .map_err(|error| format!("LLVM backend failed to select group count: {error}"))?
            .into_int_value();
        self.builder
            .build_store(groups_ptr, initial_groups)
            .map_err(|error| format!("LLVM backend failed to initialize group count: {error}"))?;

        let count_loop = self
            .context
            .append_basic_block(function, "group.count.loop");
        let count_body = self
            .context
            .append_basic_block(function, "group.count.body");
        let build_block = self.context.append_basic_block(function, "group.build");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate group count index: {error}")
            })?;
        self.builder.build_store(i_ptr, one).map_err(|error| {
            format!("LLVM backend failed to initialize group count index: {error}")
        })?;
        self.builder
            .build_unconditional_branch(count_loop)
            .map_err(|error| format!("LLVM backend failed to enter group count loop: {error}"))?;

        self.builder.position_at_end(count_loop);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load group count index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "group.count.cond")
            .map_err(|error| {
                format!("LLVM backend failed to compare group count index: {error}")
            })?;
        self.builder
            .build_conditional_branch(cond, count_body, build_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in group count loop: {error}")
            })?;

        self.builder.position_at_end(count_body);
        let prev_i = self
            .builder
            .build_int_sub(i, one, "prev")
            .map_err(|error| format!("LLVM backend failed to decrement group index: {error}"))?;
        let id = self.load_seq_item(ids, ids_ty, i)?.into_int_value();
        let prev_id = self.load_seq_item(ids, ids_ty, prev_i)?.into_int_value();
        let changed = self
            .builder
            .build_int_compare(IntPredicate::NE, id, prev_id, "group.changed")
            .map_err(|error| format!("LLVM backend failed to compare group ids: {error}"))?;
        let groups = self
            .builder
            .build_load(self.context.i64_type(), groups_ptr, "groups")
            .map_err(|error| format!("LLVM backend failed to load group count: {error}"))?
            .into_int_value();
        let inc_groups = self
            .builder
            .build_int_add(groups, one, "groups.inc")
            .map_err(|error| format!("LLVM backend failed to increment group count: {error}"))?;
        let next_groups = self
            .builder
            .build_select(changed, inc_groups, groups, "groups.next")
            .map_err(|error| format!("LLVM backend failed to select group count: {error}"))?;
        self.builder
            .build_store(groups_ptr, next_groups)
            .map_err(|error| format!("LLVM backend failed to store group count: {error}"))?;
        let next_i = self
            .builder
            .build_int_add(i, one, "next")
            .map_err(|error| {
                format!("LLVM backend failed to increment group count index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store group count index: {error}"))?;
        self.builder
            .build_unconditional_branch(count_loop)
            .map_err(|error| {
                format!("LLVM backend failed to continue group count loop: {error}")
            })?;

        self.builder.position_at_end(build_block);
        let groups = self
            .builder
            .build_load(self.context.i64_type(), groups_ptr, "groups.final")
            .map_err(|error| format!("LLVM backend failed to load final group count: {error}"))?
            .into_int_value();
        let output = self.emit_seq_new(output_ty, groups)?;

        let outer_loop = self
            .context
            .append_basic_block(function, "group.outer.loop");
        let check_change = self.context.append_basic_block(function, "group.check");
        let advance_block = self.context.append_basic_block(function, "group.advance");
        let make_group = self.context.append_basic_block(function, "group.make");
        let after_block = self.context.append_basic_block(function, "group.after");
        let run_start_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "run_start")
            .map_err(|error| format!("LLVM backend failed to allocate run start: {error}"))?;
        let group_i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "group_i")
            .map_err(|error| format!("LLVM backend failed to allocate group index: {error}"))?;
        self.builder
            .build_store(run_start_ptr, zero)
            .map_err(|error| format!("LLVM backend failed to initialize run start: {error}"))?;
        self.builder
            .build_store(group_i_ptr, zero)
            .map_err(|error| format!("LLVM backend failed to initialize group index: {error}"))?;
        self.builder.build_store(i_ptr, one).map_err(|error| {
            format!("LLVM backend failed to initialize group scan index: {error}")
        })?;
        self.builder
            .build_unconditional_branch(outer_loop)
            .map_err(|error| format!("LLVM backend failed to enter group build loop: {error}"))?;

        self.builder.position_at_end(outer_loop);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load group scan index: {error}"))?
            .into_int_value();
        let at_end = self
            .builder
            .build_int_compare(IntPredicate::EQ, i, count, "group.at_end")
            .map_err(|error| format!("LLVM backend failed to compare group end: {error}"))?;
        self.builder
            .build_conditional_branch(at_end, make_group, check_change)
            .map_err(|error| format!("LLVM backend failed to branch on group end: {error}"))?;

        self.builder.position_at_end(check_change);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load group scan index: {error}"))?
            .into_int_value();
        let run_start = self
            .builder
            .build_load(self.context.i64_type(), run_start_ptr, "run_start")
            .map_err(|error| format!("LLVM backend failed to load run start: {error}"))?
            .into_int_value();
        let id = self.load_seq_item(ids, ids_ty, i)?;
        let run_id = self.load_seq_item(ids, ids_ty, run_start)?;
        let changed = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                id.into_int_value(),
                run_id.into_int_value(),
                "group.changed",
            )
            .map_err(|error| format!("LLVM backend failed to compare group ids: {error}"))?;
        self.builder
            .build_conditional_branch(changed, make_group, advance_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in group build loop: {error}")
            })?;

        self.builder.position_at_end(advance_block);
        let next_i = self
            .builder
            .build_int_add(i, one, "next")
            .map_err(|error| {
                format!("LLVM backend failed to increment group scan index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store group scan index: {error}"))?;
        self.builder
            .build_unconditional_branch(outer_loop)
            .map_err(|error| {
                format!("LLVM backend failed to continue group build loop: {error}")
            })?;

        self.builder.position_at_end(make_group);
        let len = self
            .builder
            .build_int_sub(i, run_start, "group.len")
            .map_err(|error| format!("LLVM backend failed to compute group length: {error}"))?;
        let group = self.emit_seq_new(&group_ty, len)?;
        let copy_loop = self.context.append_basic_block(function, "group.copy.loop");
        let copy_body = self.context.append_basic_block(function, "group.copy.body");
        let copy_after = self
            .context
            .append_basic_block(function, "group.copy.after");
        let j_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "j")
            .map_err(|error| {
                format!("LLVM backend failed to allocate group copy index: {error}")
            })?;
        self.builder.build_store(j_ptr, zero).map_err(|error| {
            format!("LLVM backend failed to initialize group copy index: {error}")
        })?;
        self.builder
            .build_unconditional_branch(copy_loop)
            .map_err(|error| format!("LLVM backend failed to enter group copy loop: {error}"))?;

        self.builder.position_at_end(copy_loop);
        let j = self
            .builder
            .build_load(self.context.i64_type(), j_ptr, "j")
            .map_err(|error| format!("LLVM backend failed to load group copy index: {error}"))?
            .into_int_value();
        let copy_cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, j, len, "group.copy.cond")
            .map_err(|error| format!("LLVM backend failed to compare group copy index: {error}"))?;
        self.builder
            .build_conditional_branch(copy_cond, copy_body, copy_after)
            .map_err(|error| {
                format!("LLVM backend failed to branch in group copy loop: {error}")
            })?;

        self.builder.position_at_end(copy_body);
        let source_i = self
            .builder
            .build_int_add(run_start, j, "source_i")
            .map_err(|error| {
                format!("LLVM backend failed to compute group source index: {error}")
            })?;
        let item = self.load_seq_item(values, values_ty, source_i)?;
        self.store_seq_item(group.value, &group_ty, j, item)?;
        let next_j = self
            .builder
            .build_int_add(j, one, "next_j")
            .map_err(|error| {
                format!("LLVM backend failed to increment group copy index: {error}")
            })?;
        self.builder
            .build_store(j_ptr, next_j)
            .map_err(|error| format!("LLVM backend failed to store group copy index: {error}"))?;
        self.builder
            .build_unconditional_branch(copy_loop)
            .map_err(|error| format!("LLVM backend failed to continue group copy loop: {error}"))?;

        self.builder.position_at_end(copy_after);
        let group_i = self
            .builder
            .build_load(self.context.i64_type(), group_i_ptr, "group_i")
            .map_err(|error| format!("LLVM backend failed to load group index: {error}"))?
            .into_int_value();
        self.store_seq_item(output.value, output_ty, group_i, group.value)?;
        let next_group_i = self
            .builder
            .build_int_add(group_i, one, "next_group")
            .map_err(|error| format!("LLVM backend failed to increment group index: {error}"))?;
        self.builder
            .build_store(group_i_ptr, next_group_i)
            .map_err(|error| format!("LLVM backend failed to store group index: {error}"))?;
        self.builder
            .build_store(run_start_ptr, i)
            .map_err(|error| format!("LLVM backend failed to store run start: {error}"))?;
        let next_i = self
            .builder
            .build_int_add(i, one, "next")
            .map_err(|error| {
                format!("LLVM backend failed to increment group scan index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store group scan index: {error}"))?;
        let done = self
            .builder
            .build_int_compare(IntPredicate::EQ, i, count, "group.done")
            .map_err(|error| format!("LLVM backend failed to compare group done: {error}"))?;
        self.builder
            .build_conditional_branch(done, after_block, outer_loop)
            .map_err(|error| {
                format!("LLVM backend failed to continue group build loop: {error}")
            })?;

        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_expect(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(inner) = input.ty.clone() else {
            return Ok(input.value);
        };
        if inner.as_ref() != output_ty {
            return Err(format!(
                "expect output expected `{inner}`, found `{output_ty}`"
            ));
        }
        let function = self.current_function()?;
        let fault_block = self.context.append_basic_block(function, "expect.fault");
        let ok_block = self.context.append_basic_block(function, "expect.ok");
        let is_fault = self.extract_faultable_is_fault(input.value)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in expect: {error}"))?;
        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(input.value)?;
        let fault = self.value_to_runtime_arg(fault, &Ty::Fault)?;
        let exit_fault =
            self.runtime_function("fa_exit_fault", None, &[self.runtime_pair_type().into()])?;
        self.builder
            .build_call(exit_fault, &[fault.into()], "expect_fault")
            .map_err(|error| format!("LLVM backend failed to call fa_exit_fault: {error}"))?;
        self.builder
            .build_unreachable()
            .map_err(|error| format!("LLVM backend failed to build expect unreachable: {error}"))?;
        self.builder.position_at_end(ok_block);
        self.extract_faultable_value(input.value)
    }

    pub(super) fn emit_head(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            let Ty::Seq(_) = input_inner.as_ref() else {
                return Err(format!("head expected Seq input, found `{}`", input.ty));
            };
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable head expected faultable output, found `{output_ty}`"
                ));
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "head.faultable")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable head: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self
                .context
                .append_basic_block(function, "head.outer_fault");
            let ok_block = self.context.append_basic_block(function, "head.outer_ok");
            let after_block = self
                .context
                .append_basic_block(function, "head.outer_after");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable head: {error}")
                })?;
            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable head fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable head fault: {error}")
                })?;
            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let headed = self.emit_head(
                LlvmValue {
                    value: plain_input,
                    ty: input_inner.as_ref().clone(),
                },
                output_ty,
            )?;
            self.builder.build_store(out_ptr, headed).map_err(|error| {
                format!("LLVM backend failed to store faultable head value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable head ok: {error}")
                })?;
            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "head.result")
                .map_err(|error| format!("LLVM backend failed to load faultable head: {error}"));
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err(format!("head expected Seq input, found `{}`", input.ty));
        };
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "head expected faultable output, found `{output_ty}`"
            ));
        };
        if output_inner.as_ref() != item_ty.as_ref() {
            return Err(format!(
                "head output expected `{item_ty}`, found `{output_ty}`"
            ));
        }
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "head")
            .map_err(|error| format!("LLVM backend failed to allocate head result: {error}"))?;
        let function = self.current_function()?;
        let fault_block = self.context.append_basic_block(function, "head.fault");
        let ok_block = self.context.append_basic_block(function, "head.ok");
        let after_block = self.context.append_basic_block(function, "head.after");
        let count = self.seq_count(input.value)?;
        let empty = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                count,
                self.context.i64_type().const_zero(),
                "head.empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare head count: {error}"))?;
        self.builder
            .build_conditional_branch(empty, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in head: {error}"))?;
        self.builder.position_at_end(fault_block);
        let message = self
            .builder
            .build_global_string_ptr("head: empty sequence", "head_fault")
            .map_err(|error| format!("LLVM backend failed to build head fault string: {error}"))?;
        let fault_fn = self.runtime_function(
            "fa_fault_cstr",
            Some(self.runtime_pair_type().into()),
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        let fault = self
            .builder
            .build_call(fault_fn, &[message.as_pointer_value().into()], "fault")
            .map_err(|error| format!("LLVM backend failed to call fa_fault_cstr: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "fa_fault_cstr did not return a value".to_string())?;
        let fault = self.runtime_pair_to_value(fault, &Ty::Fault)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store head fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave head fault: {error}"))?;
        self.builder.position_at_end(ok_block);
        let item =
            self.load_seq_item(input.value, &input.ty, self.context.i64_type().const_zero())?;
        let ok = self.faultable_value(output_inner, false, None, Some(item))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store head value: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave head ok: {error}"))?;
        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "head.result")
            .map_err(|error| format!("LLVM backend failed to load head result: {error}"))
    }

    pub(super) fn emit_tail(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Seq(_) = input.ty.clone() else {
            return Err(format!("tail expected Seq input, found `{}`", input.ty));
        };
        let count = self.seq_count(input.value)?;
        let tail_count = self
            .builder
            .build_select(
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        count,
                        self.context.i64_type().const_zero(),
                        "tail.empty",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to compare tail count: {error}")
                    })?,
                self.context.i64_type().const_zero(),
                self.builder
                    .build_int_sub(
                        count,
                        self.context.i64_type().const_int(1, false),
                        "tail.count",
                    )
                    .map_err(|error| {
                        format!("LLVM backend failed to compute tail count: {error}")
                    })?,
                "tail.count",
            )
            .map_err(|error| format!("LLVM backend failed to select tail count: {error}"))?
            .into_int_value();
        let output = self.emit_seq_new(output_ty, tail_count)?;
        self.copy_seq_range(
            input.value,
            &input.ty,
            output.value,
            output_ty,
            self.context.i64_type().const_int(1, false),
            tail_count,
        )?;
        Ok(output.value)
    }

    pub(super) fn emit_reverse(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Seq(_) = input.ty.clone() else {
            return Err("reverse expected sequence input".to_string());
        };
        let count = self.seq_count(input.value)?;
        let output = self.emit_seq_new(output_ty, count)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "reverse.loop");
        let body_block = self.context.append_basic_block(function, "reverse.body");
        let after_block = self.context.append_basic_block(function, "reverse.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate reverse index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize reverse index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter reverse loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load reverse index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "reverse.cond")
            .map_err(|error| format!("LLVM backend failed to compare reverse index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in reverse loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let one = self.context.i64_type().const_int(1, false);
        let last = self
            .builder
            .build_int_sub(count, one, "reverse.last")
            .map_err(|error| {
                format!("LLVM backend failed to compute reverse last index: {error}")
            })?;
        let src_i = self
            .builder
            .build_int_sub(last, i, "reverse.src")
            .map_err(|error| {
                format!("LLVM backend failed to compute reverse source index: {error}")
            })?;
        let item = self.load_seq_item(input.value, &input.ty, src_i)?;
        self.store_seq_item(output.value, output_ty, i, item)?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_take(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (seq, seq_ty, count_value) = self.seq_count_tuple_input(&input, "take")?;
        self.branch_die_if_negative(count_value, "take: count must be non-negative", "take")?;
        let seq_count = self.seq_count(seq)?;
        let take_all = self
            .builder
            .build_int_compare(IntPredicate::UGT, count_value, seq_count, "take.all")
            .map_err(|error| format!("LLVM backend failed to compare take count: {error}"))?;
        let len = self
            .builder
            .build_select(take_all, seq_count, count_value, "take.len")
            .map_err(|error| format!("LLVM backend failed to select take length: {error}"))?
            .into_int_value();
        let output = self.emit_seq_new(output_ty, len)?;
        self.copy_seq_range(
            seq,
            &seq_ty,
            output.value,
            output_ty,
            self.context.i64_type().const_zero(),
            len,
        )?;
        Ok(output.value)
    }

    pub(super) fn emit_drop(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (seq, seq_ty, count_value) = self.seq_count_tuple_input(&input, "drop")?;
        self.branch_die_if_negative(count_value, "drop: count must be non-negative", "drop")?;
        let seq_count = self.seq_count(seq)?;
        let drop_all = self
            .builder
            .build_int_compare(IntPredicate::UGT, count_value, seq_count, "drop.all")
            .map_err(|error| format!("LLVM backend failed to compare drop count: {error}"))?;
        let offset = self
            .builder
            .build_select(drop_all, seq_count, count_value, "drop.offset")
            .map_err(|error| format!("LLVM backend failed to select drop offset: {error}"))?
            .into_int_value();
        let len = self
            .builder
            .build_int_sub(seq_count, offset, "drop.len")
            .map_err(|error| format!("LLVM backend failed to compute drop length: {error}"))?;
        let output = self.emit_seq_new(output_ty, len)?;
        self.copy_seq_range(seq, &seq_ty, output.value, output_ty, offset, len)?;
        Ok(output.value)
    }

    pub(super) fn emit_fill(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("fill expected tuple input".to_string());
        };
        let [item_ty, Ty::Int] = items.as_slice() else {
            return Err("fill expected (V,Int) input".to_string());
        };
        let item = self.extract_tuple_field(&input, 0)?;
        let count = self.extract_tuple_field(&input, 1)?.into_int_value();
        self.branch_die_if_negative(count, "fill: count must be non-negative", "fill")?;
        let output = self.emit_seq_new(output_ty, count)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "fill.loop");
        let body_block = self.context.append_basic_block(function, "fill.body");
        let after_block = self.context.append_basic_block(function, "fill.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate fill index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize fill index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter fill loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load fill index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "fill.cond")
            .map_err(|error| format!("LLVM backend failed to compare fill index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in fill loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let item = self.coerce_value_to_ty(
            LlvmValue {
                value: item,
                ty: item_ty.clone(),
            },
            sequence_item_ty(output_ty)?,
        )?;
        self.store_seq_item(output.value, output_ty, i, item.value)?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_slice(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("slice expected tuple input".to_string());
        };
        let [seq_ty @ Ty::Seq(_), Ty::Int, Ty::Int] = items.as_slice() else {
            return Err("slice expected (Seq[V],Int,Int) input".to_string());
        };
        let seq = self.extract_tuple_field(&input, 0)?;
        let start = self.extract_tuple_field(&input, 1)?.into_int_value();
        let stop = self.extract_tuple_field(&input, 2)?.into_int_value();
        let seq_count = self.seq_count(seq)?;
        let zero = self.context.i64_type().const_zero();
        let start_neg = self
            .builder
            .build_int_compare(IntPredicate::SLT, start, zero, "slice.start_neg")
            .map_err(|error| format!("LLVM backend failed to compare slice start: {error}"))?;
        let stop_before = self
            .builder
            .build_int_compare(IntPredicate::SLT, stop, start, "slice.stop_before")
            .map_err(|error| format!("LLVM backend failed to compare slice stop: {error}"))?;
        let stop_after = self
            .builder
            .build_int_compare(IntPredicate::UGT, stop, seq_count, "slice.stop_after")
            .map_err(|error| format!("LLVM backend failed to compare slice stop: {error}"))?;
        let invalid = self
            .builder
            .build_or(start_neg, stop_before, "slice.invalid0")
            .map_err(|error| format!("LLVM backend failed to combine slice checks: {error}"))?;
        let invalid = self
            .builder
            .build_or(invalid, stop_after, "slice.invalid")
            .map_err(|error| format!("LLVM backend failed to combine slice checks: {error}"))?;
        self.branch_die_if(invalid, "slice: index out of range", "slice")?;
        let len = self
            .builder
            .build_int_sub(stop, start, "slice.len")
            .map_err(|error| format!("LLVM backend failed to compute slice length: {error}"))?;
        let output = self.emit_seq_new(output_ty, len)?;
        self.copy_seq_range(seq, seq_ty, output.value, output_ty, start, len)?;
        Ok(output.value)
    }

    pub(super) fn emit_last(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let count = self.seq_count(input.value)?;
        let one = self.context.i64_type().const_int(1, false);
        let index = self
            .builder
            .build_int_sub(count, one, "last.index")
            .map_err(|error| format!("LLVM backend failed to compute last index: {error}"))?;
        self.emit_faultable_seq_index(
            input.value,
            &input.ty,
            index,
            output_ty,
            "last",
            "last: empty sequence",
        )
    }

    pub(super) fn emit_at(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (seq, seq_ty, index) = self.seq_count_tuple_input(&input, "at")?;
        self.emit_faultable_seq_index(
            seq,
            &seq_ty,
            index,
            output_ty,
            "at",
            "at: index out of range",
        )
    }

    pub(super) fn emit_get(
        &mut self,
        input: LlvmValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (seq, seq_ty, index) = self.seq_count_tuple_input(&input, "get")?;
        let invalid = self.seq_index_invalid(seq, index)?;
        self.branch_die_if(invalid, "get: index out of range", "get")?;
        self.load_seq_item(seq, &seq_ty, index)
    }

    pub(super) fn emit_get_or(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("get_or expected tuple input".to_string());
        };
        let [seq_ty @ Ty::Seq(_), Ty::Int, fallback_ty] = items.as_slice() else {
            return Err("get_or expected (Seq[V],Int,V) input".to_string());
        };
        let seq = self.extract_tuple_field(&input, 0)?;
        let index = self.extract_tuple_field(&input, 1)?.into_int_value();
        let fallback = self.extract_tuple_field(&input, 2)?;
        let invalid = self.seq_index_invalid(seq, index)?;
        let function = self.current_function()?;
        let fallback_block = self.context.append_basic_block(function, "get_or.fallback");
        let ok_block = self.context.append_basic_block(function, "get_or.ok");
        let after_block = self.context.append_basic_block(function, "get_or.after");
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "get_or.result")
            .map_err(|error| format!("LLVM backend failed to allocate get_or result: {error}"))?;
        self.builder
            .build_conditional_branch(invalid, fallback_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in get_or: {error}"))?;
        self.builder.position_at_end(fallback_block);
        let fallback = self.coerce_value_to_ty(
            LlvmValue {
                value: fallback,
                ty: fallback_ty.clone(),
            },
            output_ty,
        )?;
        self.builder
            .build_store(out_ptr, fallback.value)
            .map_err(|error| format!("LLVM backend failed to store get_or fallback: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave get_or fallback: {error}"))?;
        self.builder.position_at_end(ok_block);
        let item = self.load_seq_item(seq, seq_ty, index)?;
        self.builder
            .build_store(out_ptr, item)
            .map_err(|error| format!("LLVM backend failed to store get_or item: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave get_or ok: {error}"))?;
        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "get_or.result")
            .map_err(|error| format!("LLVM backend failed to load get_or result: {error}"))
    }

    pub(super) fn emit_append(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("append expected tuple input".to_string());
        };
        let [seq_ty @ Ty::Seq(_), item_ty] = items.as_slice() else {
            return Err("append expected (Seq[V],V) input".to_string());
        };
        let seq = self.extract_tuple_field(&input, 0)?;
        let item = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(seq)?;
        let one = self.context.i64_type().const_int(1, false);
        let out_count = self
            .builder
            .build_int_add(count, one, "append.count")
            .map_err(|error| format!("LLVM backend failed to compute append count: {error}"))?;
        let output = self.emit_seq_new(output_ty, out_count)?;
        self.copy_seq_range(
            seq,
            seq_ty,
            output.value,
            output_ty,
            self.context.i64_type().const_zero(),
            count,
        )?;
        let item = self.coerce_value_to_ty(
            LlvmValue {
                value: item,
                ty: item_ty.clone(),
            },
            sequence_item_ty(output_ty)?,
        )?;
        self.store_seq_item(output.value, output_ty, count, item.value)?;
        Ok(output.value)
    }

    pub(super) fn emit_set(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("set expected tuple input".to_string());
        };
        let [seq_ty @ Ty::Seq(_), Ty::Int, item_ty] = items.as_slice() else {
            return Err("set expected (Seq[V],Int,V) input".to_string());
        };
        let seq = self.extract_tuple_field(&input, 0)?;
        let index = self.extract_tuple_field(&input, 1)?.into_int_value();
        let item = self.extract_tuple_field(&input, 2)?;
        let invalid = self.seq_index_invalid(seq, index)?;
        self.branch_die_if(invalid, "set: index out of range", "set")?;
        let count = self.seq_count(seq)?;
        let output = self.emit_seq_new(output_ty, count)?;
        self.copy_seq_range(
            seq,
            seq_ty,
            output.value,
            output_ty,
            self.context.i64_type().const_zero(),
            count,
        )?;
        let item = self.coerce_value_to_ty(
            LlvmValue {
                value: item,
                ty: item_ty.clone(),
            },
            sequence_item_ty(output_ty)?,
        )?;
        self.store_seq_item(output.value, output_ty, index, item.value)?;
        Ok(output.value)
    }

    pub(super) fn emit_flatten(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable flatten expected faultable output, found `{output_ty}`"
                ));
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "flatten.faultable")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable flatten result: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self.context.append_basic_block(function, "flatten.fault");
            let ok_block = self.context.append_basic_block(function, "flatten.ok");
            let after_block = self.context.append_basic_block(function, "flatten.done");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable flatten: {error}")
                })?;

            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable flatten fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable flatten fault: {error}")
                })?;

            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let plain = self.emit_flatten(
                LlvmValue {
                    value: plain_input,
                    ty: input_inner.as_ref().clone(),
                },
                output_inner,
            )?;
            let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable flatten value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable flatten ok: {error}")
                })?;

            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "flatten.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable flatten result: {error}")
                });
        }
        let Ty::Seq(inner_seq_ty) = input.ty.clone() else {
            return Err(format!("flatten expected Seq input, found `{}`", input.ty));
        };
        let Ty::Seq(_) = inner_seq_ty.as_ref() else {
            return Err("flatten expected nested sequence input".to_string());
        };
        let function = self.current_function()?;
        let total_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "flatten.total")
            .map_err(|error| format!("LLVM backend failed to allocate flatten total: {error}"))?;
        self.builder
            .build_store(total_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize flatten total: {error}"))?;
        let outer_count = self.seq_count(input.value)?;
        let count_loop = self
            .context
            .append_basic_block(function, "flatten.count.loop");
        let count_body = self
            .context
            .append_basic_block(function, "flatten.count.body");
        let build_block = self.context.append_basic_block(function, "flatten.build");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate flatten index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize flatten index: {error}"))?;
        self.builder
            .build_unconditional_branch(count_loop)
            .map_err(|error| format!("LLVM backend failed to enter flatten count loop: {error}"))?;
        self.builder.position_at_end(count_loop);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load flatten index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, outer_count, "flatten.cond")
            .map_err(|error| format!("LLVM backend failed to compare flatten index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, count_body, build_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in flatten count loop: {error}")
            })?;
        self.builder.position_at_end(count_body);
        let inner = self.load_seq_item(input.value, &input.ty, i)?;
        let inner_count = self.seq_count(inner)?;
        let total = self
            .builder
            .build_load(self.context.i64_type(), total_ptr, "total")
            .map_err(|error| format!("LLVM backend failed to load flatten total: {error}"))?
            .into_int_value();
        let next_total = self
            .builder
            .build_int_add(total, inner_count, "total.next")
            .map_err(|error| format!("LLVM backend failed to add flatten total: {error}"))?;
        self.builder
            .build_store(total_ptr, next_total)
            .map_err(|error| format!("LLVM backend failed to store flatten total: {error}"))?;
        self.increment_and_continue(i_ptr, count_loop)?;

        self.builder.position_at_end(build_block);
        let total = self
            .builder
            .build_load(self.context.i64_type(), total_ptr, "total.final")
            .map_err(|error| format!("LLVM backend failed to load final flatten total: {error}"))?
            .into_int_value();
        let output = self.emit_seq_new(output_ty, total)?;
        let out_i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "out_i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate flatten output index: {error}")
            })?;
        self.builder
            .build_store(out_i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize flatten output: {error}")
            })?;
        // Reuse the already-proven sequence copy primitive for each nested item.
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to reset flatten index: {error}"))?;
        let loop_block = self.context.append_basic_block(function, "flatten.loop");
        let body_block = self.context.append_basic_block(function, "flatten.body");
        let after_block = self.context.append_basic_block(function, "flatten.after");
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter flatten loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load flatten index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, outer_count, "flatten.cond")
            .map_err(|error| format!("LLVM backend failed to compare flatten index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in flatten loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let inner = self.load_seq_item(input.value, &input.ty, i)?;
        let inner_count = self.seq_count(inner)?;
        let base = self
            .builder
            .build_load(self.context.i64_type(), out_i_ptr, "out_i")
            .map_err(|error| format!("LLVM backend failed to load flatten output index: {error}"))?
            .into_int_value();
        self.copy_seq_range_to_offset_from_zero(
            inner,
            inner_seq_ty.as_ref(),
            output.value,
            output_ty,
            base,
            inner_count,
        )?;
        let next_base = self
            .builder
            .build_int_add(base, inner_count, "out_i.next")
            .map_err(|error| format!("LLVM backend failed to increment flatten output: {error}"))?;
        self.builder
            .build_store(out_i_ptr, next_base)
            .map_err(|error| format!("LLVM backend failed to store flatten output: {error}"))?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_transpose(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Seq(row_ty) = input.ty.clone() else {
            return Err(format!(
                "transpose expected Seq input, found `{}`",
                input.ty
            ));
        };
        let Ty::Seq(item_ty) = row_ty.as_ref() else {
            return Err("transpose expected nested sequence input".to_string());
        };
        let rows = self.seq_count(input.value)?;
        let function = self.current_function()?;
        let cols_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "transpose.cols")
            .map_err(|error| {
                format!("LLVM backend failed to allocate transpose columns: {error}")
            })?;
        let empty_block = self.context.append_basic_block(function, "transpose.empty");
        let first_row_block = self
            .context
            .append_basic_block(function, "transpose.first_row");
        let check_block = self.context.append_basic_block(function, "transpose.check");
        let build_block = self.context.append_basic_block(function, "transpose.build");
        let is_empty = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                rows,
                self.context.i64_type().const_zero(),
                "transpose.empty",
            )
            .map_err(|error| format!("LLVM backend failed to compare transpose rows: {error}"))?;
        self.builder
            .build_conditional_branch(is_empty, empty_block, first_row_block)
            .map_err(|error| format!("LLVM backend failed to branch in transpose: {error}"))?;

        self.builder.position_at_end(empty_block);
        self.builder
            .build_store(cols_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to store empty transpose cols: {error}")
            })?;
        self.builder
            .build_unconditional_branch(build_block)
            .map_err(|error| format!("LLVM backend failed to leave empty transpose: {error}"))?;

        self.builder.position_at_end(first_row_block);
        let first_row =
            self.load_seq_item(input.value, &input.ty, self.context.i64_type().const_zero())?;
        let first_cols = self.seq_count(first_row)?;
        self.builder
            .build_store(cols_ptr, first_cols)
            .map_err(|error| format!("LLVM backend failed to store transpose cols: {error}"))?;
        self.builder
            .build_unconditional_branch(check_block)
            .map_err(|error| format!("LLVM backend failed to enter transpose check: {error}"))?;

        self.builder.position_at_end(check_block);
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate transpose check index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize transpose check index: {error}")
            })?;
        let check_loop = self
            .context
            .append_basic_block(function, "transpose.check.loop");
        let check_body = self
            .context
            .append_basic_block(function, "transpose.check.body");
        self.builder
            .build_unconditional_branch(check_loop)
            .map_err(|error| {
                format!("LLVM backend failed to enter transpose check loop: {error}")
            })?;
        self.builder.position_at_end(check_loop);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load transpose check index: {error}"))?
            .into_int_value();
        let in_range = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, rows, "transpose.check.cond")
            .map_err(|error| {
                format!("LLVM backend failed to compare transpose check index: {error}")
            })?;
        self.builder
            .build_conditional_branch(in_range, check_body, build_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in transpose check: {error}")
            })?;
        self.builder.position_at_end(check_body);
        let row = self.load_seq_item(input.value, &input.ty, i)?;
        let row_cols = self.seq_count(row)?;
        let expected_cols = self
            .builder
            .build_load(self.context.i64_type(), cols_ptr, "cols")
            .map_err(|error| format!("LLVM backend failed to load transpose cols: {error}"))?
            .into_int_value();
        let same = self
            .builder
            .build_int_compare(IntPredicate::EQ, row_cols, expected_cols, "transpose.same")
            .map_err(|error| {
                format!("LLVM backend failed to compare transpose row length: {error}")
            })?;
        let die_block = self.context.append_basic_block(function, "transpose.die");
        let continue_block = self
            .context
            .append_basic_block(function, "transpose.continue");
        self.builder
            .build_conditional_branch(same, continue_block, die_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on transpose row length: {error}")
            })?;
        self.builder.position_at_end(die_block);
        let message = self
            .builder
            .build_global_string_ptr(
                "transpose: rows must have the same length",
                "transpose.error",
            )
            .map_err(|error| format!("LLVM backend failed to build transpose error: {error}"))?;
        let die = self.runtime_function(
            "fa_die_usage",
            None,
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        self.builder
            .build_call(die, &[message.as_pointer_value().into()], "transpose.die")
            .map_err(|error| format!("LLVM backend failed to call fa_die_usage: {error}"))?;
        self.builder.build_unreachable().map_err(|error| {
            format!("LLVM backend failed to build transpose unreachable: {error}")
        })?;
        self.builder.position_at_end(continue_block);
        self.increment_and_continue(i_ptr, check_loop)?;

        self.builder.position_at_end(build_block);
        let cols = self
            .builder
            .build_load(self.context.i64_type(), cols_ptr, "cols")
            .map_err(|error| format!("LLVM backend failed to load final transpose cols: {error}"))?
            .into_int_value();
        let output = self.emit_seq_new(output_ty, cols)?;
        let c_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "c")
            .map_err(|error| {
                format!("LLVM backend failed to allocate transpose column: {error}")
            })?;
        self.builder
            .build_store(c_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize transpose column: {error}")
            })?;
        let col_loop = self
            .context
            .append_basic_block(function, "transpose.col.loop");
        let col_body = self
            .context
            .append_basic_block(function, "transpose.col.body");
        let after_block = self.context.append_basic_block(function, "transpose.after");
        self.builder
            .build_unconditional_branch(col_loop)
            .map_err(|error| {
                format!("LLVM backend failed to enter transpose column loop: {error}")
            })?;
        self.builder.position_at_end(col_loop);
        let c = self
            .builder
            .build_load(self.context.i64_type(), c_ptr, "c")
            .map_err(|error| format!("LLVM backend failed to load transpose column: {error}"))?
            .into_int_value();
        let has_col = self
            .builder
            .build_int_compare(IntPredicate::ULT, c, cols, "transpose.col.cond")
            .map_err(|error| format!("LLVM backend failed to compare transpose column: {error}"))?;
        self.builder
            .build_conditional_branch(has_col, col_body, after_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in transpose column loop: {error}")
            })?;
        self.builder.position_at_end(col_body);
        let out_row = self.emit_seq_new(row_ty.as_ref(), rows)?;
        let r_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "r")
            .map_err(|error| format!("LLVM backend failed to allocate transpose row: {error}"))?;
        self.builder
            .build_store(r_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize transpose row: {error}"))?;
        let row_loop = self
            .context
            .append_basic_block(function, "transpose.row.loop");
        let row_body = self
            .context
            .append_basic_block(function, "transpose.row.body");
        let row_after = self
            .context
            .append_basic_block(function, "transpose.row.after");
        self.builder
            .build_unconditional_branch(row_loop)
            .map_err(|error| format!("LLVM backend failed to enter transpose row loop: {error}"))?;
        self.builder.position_at_end(row_loop);
        let r = self
            .builder
            .build_load(self.context.i64_type(), r_ptr, "r")
            .map_err(|error| format!("LLVM backend failed to load transpose row: {error}"))?
            .into_int_value();
        let has_row = self
            .builder
            .build_int_compare(IntPredicate::ULT, r, rows, "transpose.row.cond")
            .map_err(|error| format!("LLVM backend failed to compare transpose row: {error}"))?;
        self.builder
            .build_conditional_branch(has_row, row_body, row_after)
            .map_err(|error| {
                format!("LLVM backend failed to branch in transpose row loop: {error}")
            })?;
        self.builder.position_at_end(row_body);
        let in_row = self.load_seq_item(input.value, &input.ty, r)?;
        let item = self.load_seq_item(in_row, row_ty.as_ref(), c)?;
        self.store_seq_item(out_row.value, row_ty.as_ref(), r, item)?;
        self.increment_and_continue(r_ptr, row_loop)?;
        self.builder.position_at_end(row_after);
        self.store_seq_item(output.value, output_ty, c, out_row.value)?;
        self.increment_and_continue(c_ptr, col_loop)?;
        self.builder.position_at_end(after_block);
        let _ = item_ty;
        Ok(output.value)
    }

    pub(super) fn emit_seq_concat(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("concat expected tuple input".to_string());
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err("concat expected pair input".to_string());
        };
        if super::contains_tuple_faultable_ty(left_ty)
            || super::contains_tuple_faultable_ty(right_ty)
        {
            let Ty::Faultable(output_inner) = output_ty else {
                return Err("faultable concat expected faultable output".to_string());
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "concat.faultable_seq")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate concat result: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self
                .context
                .append_basic_block(function, "concat_seq.fault");
            let ok_block = self.context.append_basic_block(function, "concat_seq.ok");
            let after_block = self
                .context
                .append_basic_block(function, "concat_seq.after");
            let left = self.extract_tuple_field(&input, 0)?;
            let right = self.extract_tuple_field(&input, 1)?;
            let mut fault_cond = self.context.bool_type().const_zero();
            let mut selected_fault = None;
            self.collect_nested_fault_state(left, left_ty, &mut fault_cond, &mut selected_fault)?;
            self.collect_nested_fault_state(right, right_ty, &mut fault_cond, &mut selected_fault)?;
            self.builder
                .build_conditional_branch(fault_cond, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on concat fault: {error}")
                })?;
            self.builder.position_at_end(fault_block);
            let fault = selected_fault
                .ok_or_else(|| "faultable concat had no faultable fields".to_string())?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| format!("LLVM backend failed to store concat fault: {error}"))?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| format!("LLVM backend failed to leave concat fault: {error}"))?;
            self.builder.position_at_end(ok_block);
            let left_value = self.strip_nested_faultable_value(left, left_ty)?;
            let right_value = self.strip_nested_faultable_value(right, right_ty)?;
            let pair_ty = Ty::Tuple(vec![left_value.ty.clone(), right_value.ty.clone()]);
            let mut pair = self
                .types
                .basic_type(&pair_ty)?
                .into_struct_type()
                .const_zero();
            pair = self
                .builder
                .build_insert_value(pair, left_value.value, 0, "concat.pair")
                .map_err(|error| format!("LLVM backend failed to build concat pair: {error}"))?
                .into_struct_value();
            pair = self
                .builder
                .build_insert_value(pair, right_value.value, 1, "concat.pair")
                .map_err(|error| format!("LLVM backend failed to build concat pair: {error}"))?
                .into_struct_value();
            let plain = self.emit_seq_concat(
                LlvmValue {
                    value: pair.into(),
                    ty: pair_ty,
                },
                output_inner,
            )?;
            let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
            self.builder
                .build_store(out_ptr, ok)
                .map_err(|error| format!("LLVM backend failed to store concat ok: {error}"))?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| format!("LLVM backend failed to leave concat ok: {error}"))?;
            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "concat.result")
                .map_err(|error| format!("LLVM backend failed to load concat result: {error}"));
        }
        let (Ty::Seq(_), Ty::Seq(_)) = (left_ty, right_ty) else {
            return Err("concat expected sequence inputs".to_string());
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let left_count = self.seq_count(left)?;
        let right_count = self.seq_count(right)?;
        let total = self
            .builder
            .build_int_add(left_count, right_count, "concat.count")
            .map_err(|error| format!("LLVM backend failed to compute concat count: {error}"))?;
        let output = self.emit_seq_new(output_ty, total)?;
        self.copy_seq_range(
            left,
            left_ty,
            output.value,
            output_ty,
            self.context.i64_type().const_zero(),
            left_count,
        )?;
        self.copy_seq_range_to_offset_from_zero(
            right,
            right_ty,
            output.value,
            output_ty,
            left_count,
            right_count,
        )?;
        Ok(output.value)
    }

    pub(super) fn emit_broadcast_right(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("broadcast_right expected tuple input".to_string());
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err("broadcast_right expected pair input".to_string());
        };
        if super::contains_tuple_faultable_ty(left_ty)
            || super::contains_tuple_faultable_ty(right_ty)
        {
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable broadcast_right expected faultable output, found `{output_ty}`"
                ));
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "broadcast.faultable")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable broadcast result: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self.context.append_basic_block(function, "broadcast.fault");
            let ok_block = self.context.append_basic_block(function, "broadcast.ok");
            let after_block = self.context.append_basic_block(function, "broadcast.done");
            let left = self.extract_tuple_field(&input, 0)?;
            let right = self.extract_tuple_field(&input, 1)?;

            let mut fault_cond = self.context.bool_type().const_zero();
            let mut selected_fault = None;
            self.collect_nested_fault_state(left, left_ty, &mut fault_cond, &mut selected_fault)?;
            self.collect_nested_fault_state(right, right_ty, &mut fault_cond, &mut selected_fault)?;

            self.builder
                .build_conditional_branch(fault_cond, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable broadcast: {error}")
                })?;

            self.builder.position_at_end(fault_block);
            let fault = selected_fault
                .ok_or_else(|| "faultable broadcast had no faultable fields".to_string())?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable broadcast fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable broadcast fault: {error}")
                })?;

            self.builder.position_at_end(ok_block);
            let left_value = self.strip_nested_faultable_value(left, left_ty)?;
            let right_value = self.strip_nested_faultable_value(right, right_ty)?;
            let pair_ty = Ty::Tuple(vec![left_value.ty.clone(), right_value.ty.clone()]);
            let mut pair = self
                .types
                .basic_type(&pair_ty)?
                .into_struct_type()
                .const_zero();
            pair = self
                .builder
                .build_insert_value(pair, left_value.value, 0, "broadcast.pair")
                .map_err(|error| format!("LLVM backend failed to build broadcast pair: {error}"))?
                .into_struct_value();
            pair = self
                .builder
                .build_insert_value(pair, right_value.value, 1, "broadcast.pair")
                .map_err(|error| format!("LLVM backend failed to build broadcast pair: {error}"))?
                .into_struct_value();
            let plain = self.emit_broadcast_right(
                LlvmValue {
                    value: pair.into(),
                    ty: pair_ty,
                },
                output_inner,
            )?;
            let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable broadcast value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable broadcast ok: {error}")
                })?;

            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "broadcast.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable broadcast result: {error}")
                });
        }
        let Ty::Seq(left_item_ty) = left_ty else {
            return Err("broadcast_right expected (Seq[A], B) input".to_string());
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(left)?;
        let output = self.emit_seq_new(output_ty, count)?;
        let out_item_ty = Ty::Tuple(vec![left_item_ty.as_ref().clone(), right_ty.clone()]);
        let out_item_llvm_ty = self.types.basic_type(&out_item_ty)?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "broadcast.loop");
        let body_block = self.context.append_basic_block(function, "broadcast.body");
        let after_block = self.context.append_basic_block(function, "broadcast.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate broadcast index: {error}"))?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize broadcast index: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter broadcast loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load broadcast index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "broadcast.cond")
            .map_err(|error| format!("LLVM backend failed to compare broadcast index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in broadcast loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let left_item = self.load_seq_item(left, left_ty, i)?;
        let mut pair = out_item_llvm_ty.into_struct_type().const_zero();
        pair = self
            .builder
            .build_insert_value(pair, left_item, 0, "broadcast.item")
            .map_err(|error| format!("LLVM backend failed to build broadcast item: {error}"))?
            .into_struct_value();
        pair = self
            .builder
            .build_insert_value(pair, right, 1, "broadcast.item")
            .map_err(|error| format!("LLVM backend failed to build broadcast item: {error}"))?
            .into_struct_value();
        self.store_seq_item(output.value, output_ty, i, pair.into())?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_broadcast_left(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err("broadcast_left expected tuple input".to_string());
        };
        let [left_ty, right_ty] = items.as_slice() else {
            return Err("broadcast_left expected pair input".to_string());
        };
        if super::contains_tuple_faultable_ty(left_ty)
            || super::contains_tuple_faultable_ty(right_ty)
        {
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable broadcast_left expected faultable output, found `{output_ty}`"
                ));
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "broadcast_left.faultable")
                .map_err(|error| {
                    format!(
                        "LLVM backend failed to allocate faultable broadcast_left result: {error}"
                    )
                })?;
            let function = self.current_function()?;
            let fault_block = self
                .context
                .append_basic_block(function, "broadcast_left.fault");
            let ok_block = self
                .context
                .append_basic_block(function, "broadcast_left.ok");
            let after_block = self
                .context
                .append_basic_block(function, "broadcast_left.done");
            let left = self.extract_tuple_field(&input, 0)?;
            let right = self.extract_tuple_field(&input, 1)?;

            let mut fault_cond = self.context.bool_type().const_zero();
            let mut selected_fault = None;
            self.collect_nested_fault_state(left, left_ty, &mut fault_cond, &mut selected_fault)?;
            self.collect_nested_fault_state(right, right_ty, &mut fault_cond, &mut selected_fault)?;

            self.builder
                .build_conditional_branch(fault_cond, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable broadcast_left: {error}")
                })?;

            self.builder.position_at_end(fault_block);
            let fault = selected_fault
                .ok_or_else(|| "faultable broadcast_left had no faultable fields".to_string())?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable broadcast_left fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable broadcast_left fault: {error}")
                })?;

            self.builder.position_at_end(ok_block);
            let left_value = self.strip_nested_faultable_value(left, left_ty)?;
            let right_value = self.strip_nested_faultable_value(right, right_ty)?;
            let pair_ty = Ty::Tuple(vec![left_value.ty.clone(), right_value.ty.clone()]);
            let mut pair = self
                .types
                .basic_type(&pair_ty)?
                .into_struct_type()
                .const_zero();
            pair = self
                .builder
                .build_insert_value(pair, left_value.value, 0, "broadcast_left.pair")
                .map_err(|error| {
                    format!("LLVM backend failed to build broadcast_left pair: {error}")
                })?
                .into_struct_value();
            pair = self
                .builder
                .build_insert_value(pair, right_value.value, 1, "broadcast_left.pair")
                .map_err(|error| {
                    format!("LLVM backend failed to build broadcast_left pair: {error}")
                })?
                .into_struct_value();
            let plain = self.emit_broadcast_left(
                LlvmValue {
                    value: pair.into(),
                    ty: pair_ty,
                },
                output_inner,
            )?;
            let ok = self.faultable_value(output_inner, false, None, Some(plain))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable broadcast_left value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable broadcast_left ok: {error}")
                })?;

            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "broadcast_left.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable broadcast_left result: {error}")
                });
        }
        let Ty::Seq(right_item_ty) = right_ty else {
            return Err("broadcast_left expected (A, Seq[B]) input".to_string());
        };
        let left = self.extract_tuple_field(&input, 0)?;
        let right = self.extract_tuple_field(&input, 1)?;
        let count = self.seq_count(right)?;
        let output = self.emit_seq_new(output_ty, count)?;
        let out_item_ty = Ty::Tuple(vec![left_ty.clone(), right_item_ty.as_ref().clone()]);
        let out_item_llvm_ty = self.types.basic_type(&out_item_ty)?;
        let function = self.current_function()?;
        let loop_block = self
            .context
            .append_basic_block(function, "broadcast_left.loop");
        let body_block = self
            .context
            .append_basic_block(function, "broadcast_left.body");
        let after_block = self
            .context
            .append_basic_block(function, "broadcast_left.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate broadcast_left index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize broadcast_left index: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| {
                format!("LLVM backend failed to enter broadcast_left loop: {error}")
            })?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load broadcast_left index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "broadcast_left.cond")
            .map_err(|error| {
                format!("LLVM backend failed to compare broadcast_left index: {error}")
            })?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in broadcast_left loop: {error}")
            })?;
        self.builder.position_at_end(body_block);
        let right_item = self.load_seq_item(right, right_ty, i)?;
        let mut pair = out_item_llvm_ty.into_struct_type().const_zero();
        pair = self
            .builder
            .build_insert_value(pair, left, 0, "broadcast_left.item")
            .map_err(|error| format!("LLVM backend failed to build broadcast_left item: {error}"))?
            .into_struct_value();
        pair = self
            .builder
            .build_insert_value(pair, right_item, 1, "broadcast_left.item")
            .map_err(|error| format!("LLVM backend failed to build broadcast_left item: {error}"))?
            .into_struct_value();
        self.store_seq_item(output.value, output_ty, i, pair.into())?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(output.value)
    }

    pub(super) fn emit_all_any(
        &mut self,
        input: LlvmValue<'ctx>,
        all: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            if input_inner.as_ref() != &Ty::Seq(Box::new(Ty::Bool)) {
                return Err(format!(
                    "{} expected Seq[Bool], found `{}`",
                    if all { "all" } else { "any" },
                    input.ty
                ));
            }
            let output_ty = Ty::Faultable(Box::new(Ty::Bool));
            let output_llvm_ty = self.types.basic_type(&output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "bool.reduce.faultable")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable bool reduce: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self
                .context
                .append_basic_block(function, "bool_reduce.fault");
            let ok_block = self.context.append_basic_block(function, "bool_reduce.ok");
            let after_block = self
                .context
                .append_basic_block(function, "bool_reduce.after");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable bool reduce: {error}")
                })?;
            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(&Ty::Bool, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable bool reduce fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable bool reduce fault: {error}")
                })?;
            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let plain = self.emit_all_any(
                LlvmValue {
                    value: plain_input,
                    ty: input_inner.as_ref().clone(),
                },
                all,
            )?;
            let ok = self.faultable_value(&Ty::Bool, false, None, Some(plain))?;
            self.builder.build_store(out_ptr, ok).map_err(|error| {
                format!("LLVM backend failed to store faultable bool reduce value: {error}")
            })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable bool reduce ok: {error}")
                })?;
            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "bool.reduce.result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable bool reduce: {error}")
                });
        }
        if input.ty != Ty::Seq(Box::new(Ty::Bool)) {
            return Err(format!(
                "{} expected Seq[Bool], found `{}`",
                if all { "all" } else { "any" },
                input.ty
            ));
        }
        let count = self.seq_count(input.value)?;
        let result_ptr = self
            .builder
            .build_alloca(self.context.i8_type(), "bool.reduce")
            .map_err(|error| format!("LLVM backend failed to allocate bool reduce: {error}"))?;
        self.builder
            .build_store(
                result_ptr,
                self.context
                    .i8_type()
                    .const_int(if all { 1 } else { 0 }, false),
            )
            .map_err(|error| format!("LLVM backend failed to initialize bool reduce: {error}"))?;
        let function = self.current_function()?;
        let loop_block = self
            .context
            .append_basic_block(function, "bool.reduce.loop");
        let body_block = self
            .context
            .append_basic_block(function, "bool.reduce.body");
        let after_block = self
            .context
            .append_basic_block(function, "bool.reduce.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| {
                format!("LLVM backend failed to allocate bool reduce index: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize bool reduce index: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter bool reduce loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load bool reduce index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "bool.reduce.cond")
            .map_err(|error| {
                format!("LLVM backend failed to compare bool reduce index: {error}")
            })?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch in bool reduce loop: {error}")
            })?;
        self.builder.position_at_end(body_block);
        let current = self
            .builder
            .build_load(self.context.i8_type(), result_ptr, "current")
            .map_err(|error| format!("LLVM backend failed to load bool reduce value: {error}"))?
            .into_int_value();
        let item = self
            .load_seq_item(input.value, &input.ty, i)?
            .into_int_value();
        let next = if all {
            self.builder.build_and(current, item, "all")
        } else {
            self.builder.build_or(current, item, "any")
        }
        .map_err(|error| format!("LLVM backend failed to combine bool reduce: {error}"))?;
        self.builder
            .build_store(result_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store bool reduce value: {error}"))?;
        self.increment_and_continue(i_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        self.builder
            .build_load(self.context.i8_type(), result_ptr, "bool.reduce.result")
            .map_err(|error| format!("LLVM backend failed to load bool reduce result: {error}"))
    }

    pub(super) fn emit_collect(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Ty::Faultable(input_inner) = input.ty.clone() {
            let Ty::Faultable(output_inner) = output_ty else {
                return Err(format!(
                    "faultable collect expected faultable output, found `{output_ty}`"
                ));
            };
            let output_llvm_ty = self.types.basic_type(output_ty)?;
            let out_ptr = self
                .builder
                .build_alloca(output_llvm_ty, "collect.faultable")
                .map_err(|error| {
                    format!("LLVM backend failed to allocate faultable collect result: {error}")
                })?;
            let function = self.current_function()?;
            let fault_block = self
                .context
                .append_basic_block(function, "collect.outer_fault");
            let ok_block = self
                .context
                .append_basic_block(function, "collect.outer_ok");
            let after_block = self
                .context
                .append_basic_block(function, "collect.outer_done");
            let is_fault = self.extract_faultable_is_fault(input.value)?;
            self.builder
                .build_conditional_branch(is_fault, fault_block, ok_block)
                .map_err(|error| {
                    format!("LLVM backend failed to branch on faultable collect: {error}")
                })?;

            self.builder.position_at_end(fault_block);
            let fault = self.extract_faultable_fault(input.value)?;
            let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
            self.builder
                .build_store(out_ptr, faulted)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable collect fault: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable collect fault: {error}")
                })?;

            self.builder.position_at_end(ok_block);
            let plain_input = self.extract_faultable_value(input.value)?;
            let collected = self.emit_collect(
                LlvmValue {
                    value: plain_input,
                    ty: input_inner.as_ref().clone(),
                },
                output_ty,
            )?;
            self.builder
                .build_store(out_ptr, collected)
                .map_err(|error| {
                    format!("LLVM backend failed to store faultable collect value: {error}")
                })?;
            self.builder
                .build_unconditional_branch(after_block)
                .map_err(|error| {
                    format!("LLVM backend failed to leave faultable collect ok: {error}")
                })?;

            self.builder.position_at_end(after_block);
            return self
                .builder
                .build_load(output_llvm_ty, out_ptr, "collect.outer_result")
                .map_err(|error| {
                    format!("LLVM backend failed to load faultable collect result: {error}")
                });
        }
        let Ty::Seq(item_ty) = input.ty.clone() else {
            return Err("collect expected sequence input".to_string());
        };
        let Ty::Faultable(ok_item_ty) = item_ty.as_ref() else {
            return Err("collect expected Seq[Faultable[V]] input".to_string());
        };
        let Ty::Faultable(seq_ty) = output_ty else {
            return Err("collect expected faultable sequence output".to_string());
        };
        let plain_seq_ty = Ty::Seq(ok_item_ty.clone());
        if seq_ty.as_ref() != &plain_seq_ty {
            return Err(format!(
                "collect output expected `{plain_seq_ty}`, found `{output_ty}`"
            ));
        }
        let count = self.seq_count(input.value)?;
        let ok_seq = self.emit_seq_new(&plain_seq_ty, count)?;
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "collect")
            .map_err(|error| format!("LLVM backend failed to allocate collect result: {error}"))?;
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "collect.loop");
        let body_block = self.context.append_basic_block(function, "collect.body");
        let ok_block = self.context.append_basic_block(function, "collect.ok");
        let fault_block = self.context.append_basic_block(function, "collect.fault");
        let after_item_block = self
            .context
            .append_basic_block(function, "collect.after_item");
        let after_block = self.context.append_basic_block(function, "collect.after");
        let i_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "i")
            .map_err(|error| format!("LLVM backend failed to allocate collect index: {error}"))?;
        let faulted_ptr = self
            .builder
            .build_alloca(self.context.i8_type(), "faulted")
            .map_err(|error| {
                format!("LLVM backend failed to allocate collect fault flag: {error}")
            })?;
        self.builder
            .build_store(i_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize collect index: {error}"))?;
        self.builder
            .build_store(faulted_ptr, self.context.i8_type().const_zero())
            .map_err(|error| {
                format!("LLVM backend failed to initialize collect fault flag: {error}")
            })?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter collect loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let i = self
            .builder
            .build_load(self.context.i64_type(), i_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load collect index: {error}"))?
            .into_int_value();
        let faulted = self
            .builder
            .build_load(self.context.i8_type(), faulted_ptr, "faulted")
            .map_err(|error| format!("LLVM backend failed to load collect fault flag: {error}"))?
            .into_int_value();
        let in_range = self
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "collect.cond")
            .map_err(|error| format!("LLVM backend failed to compare collect index: {error}"))?;
        let not_faulted = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                faulted,
                self.context.i8_type().const_zero(),
                "collect.not_faulted",
            )
            .map_err(|error| {
                format!("LLVM backend failed to compare collect fault flag: {error}")
            })?;
        let keep = self
            .builder
            .build_and(in_range, not_faulted, "collect.keep")
            .map_err(|error| format!("LLVM backend failed to build collect condition: {error}"))?;
        self.builder
            .build_conditional_branch(keep, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in collect loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let item = self.load_seq_item(input.value, &input.ty, i)?;
        let is_fault = self.extract_faultable_is_fault(item)?;
        self.builder
            .build_conditional_branch(is_fault, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch on collect item: {error}"))?;
        self.builder.position_at_end(fault_block);
        let fault = self.extract_faultable_fault(item)?;
        let faulted = self.faultable_value(seq_ty, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store collect fault: {error}"))?;
        self.builder
            .build_store(faulted_ptr, self.context.i8_type().const_int(1, false))
            .map_err(|error| format!("LLVM backend failed to set collect fault flag: {error}"))?;
        self.builder
            .build_unconditional_branch(after_item_block)
            .map_err(|error| format!("LLVM backend failed to leave collect fault: {error}"))?;
        self.builder.position_at_end(ok_block);
        let value = self.extract_faultable_value(item)?;
        self.store_seq_item(ok_seq.value, &plain_seq_ty, i, value)?;
        self.builder
            .build_unconditional_branch(after_item_block)
            .map_err(|error| format!("LLVM backend failed to leave collect ok: {error}"))?;
        self.builder.position_at_end(after_item_block);
        let next_i = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment collect index: {error}"))?;
        self.builder
            .build_store(i_ptr, next_i)
            .map_err(|error| format!("LLVM backend failed to store collect index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue collect loop: {error}"))?;
        self.builder.position_at_end(after_block);
        let faulted = self
            .builder
            .build_load(self.context.i8_type(), faulted_ptr, "faulted")
            .map_err(|error| {
                format!("LLVM backend failed to load collect final fault flag: {error}")
            })?
            .into_int_value();
        let is_faulted = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                faulted,
                self.context.i8_type().const_zero(),
                "collect.faulted",
            )
            .map_err(|error| {
                format!("LLVM backend failed to compare collect final fault: {error}")
            })?;
        let ok_final = self
            .context
            .append_basic_block(function, "collect.final_ok");
        let final_block = self.context.append_basic_block(function, "collect.final");
        self.builder
            .build_conditional_branch(is_faulted, final_block, ok_final)
            .map_err(|error| format!("LLVM backend failed to branch on collect final: {error}"))?;
        self.builder.position_at_end(ok_final);
        let ok = self.faultable_value(seq_ty, false, None, Some(ok_seq.value))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store collect ok: {error}"))?;
        self.builder
            .build_unconditional_branch(final_block)
            .map_err(|error| format!("LLVM backend failed to leave collect ok final: {error}"))?;
        self.builder.position_at_end(final_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "collect.result")
            .map_err(|error| format!("LLVM backend failed to load collect result: {error}"))
    }

    pub(super) fn emit_stream_to_seq(
        &mut self,
        input: LlvmValue<'ctx>,
        output_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Stream(item_ty) = input.ty.clone() else {
            return Err("to_seq expected stream input".to_string());
        };
        let Ty::Faultable(seq_ty) = output_ty else {
            return Err("to_seq expected faultable sequence output".to_string());
        };
        let seq_ty = seq_ty.as_ref();
        let Ty::Seq(seq_item_ty) = seq_ty else {
            return Err("to_seq expected sequence output".to_string());
        };
        if seq_item_ty.as_ref() != item_ty.as_ref() {
            return Err("to_seq stream item/output item mismatch".to_string());
        }

        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, "stream.to_seq")
            .map_err(|error| format!("LLVM backend failed to allocate stream.to_seq: {error}"))?;
        let stream = input.value.into_struct_value();
        let next = self
            .builder
            .build_extract_value(stream, 5, "stream.next")
            .map_err(|error| format!("LLVM backend failed to extract stream next: {error}"))?
            .into_pointer_value();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let has_next = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                next,
                ptr_ty.const_null(),
                "stream.has_next",
            )
            .map_err(|error| format!("LLVM backend failed to test stream next: {error}"))?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, "stream.to_seq_fault");
        let setup_block = self
            .context
            .append_basic_block(function, "stream.to_seq_setup");
        let loop_block = self
            .context
            .append_basic_block(function, "stream.to_seq_loop");
        let grow_block = self
            .context
            .append_basic_block(function, "stream.to_seq_grow");
        let read_block = self
            .context
            .append_basic_block(function, "stream.to_seq_read");
        let done_block = self
            .context
            .append_basic_block(function, "stream.to_seq_done");
        let ok_block = self
            .context
            .append_basic_block(function, "stream.to_seq_ok");
        let item_fault_block = self
            .context
            .append_basic_block(function, "stream.to_seq_item_fault");
        let after_block = self
            .context
            .append_basic_block(function, "stream.to_seq_after");
        self.builder
            .build_conditional_branch(has_next, setup_block, fault_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on stream.to_seq next: {error}")
            })?;

        self.builder.position_at_end(fault_block);
        let fault = self.fault_from_cstr(
            "stream.to_seq: stream is not pull-readable",
            "stream.to_seq",
        )?;
        let faulted = self.faultable_value(seq_ty, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| {
                format!("LLVM backend failed to store stream.to_seq fault: {error}")
            })?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| {
                format!("LLVM backend failed to leave stream.to_seq fault: {error}")
            })?;

        self.builder.position_at_end(setup_block);
        let i64_ty = self.context.i64_type();
        let i32_ty = self.context.i32_type();
        let item_llvm_ty = self.types.basic_type(&item_ty)?;
        let item_size = item_llvm_ty
            .size_of()
            .ok_or_else(|| format!("cannot compute size of `{item_ty}`"))?;
        let cap_ptr = self
            .builder
            .build_alloca(i64_ty, "stream.cap")
            .map_err(|error| format!("LLVM backend failed to allocate stream cap: {error}"))?;
        let count_ptr = self
            .builder
            .build_alloca(i64_ty, "stream.count")
            .map_err(|error| format!("LLVM backend failed to allocate stream count: {error}"))?;
        let items_ptr = self
            .builder
            .build_alloca(ptr_ty, "stream.items")
            .map_err(|error| {
                format!("LLVM backend failed to allocate stream items pointer: {error}")
            })?;
        let fault_ty = self.types.basic_type(&Ty::Fault)?;
        let fault_ptr = self
            .builder
            .build_alloca(fault_ty, "stream.fault")
            .map_err(|error| format!("LLVM backend failed to allocate stream fault: {error}"))?;
        let status_ptr = self
            .builder
            .build_alloca(i32_ty, "stream.status")
            .map_err(|error| format!("LLVM backend failed to allocate stream status: {error}"))?;
        let calloc = self.runtime_function(
            "calloc",
            Some(ptr_ty.into()),
            &[i64_ty.into(), i64_ty.into()],
        )?;
        let cap = i64_ty.const_int(8, false);
        let items = self
            .builder
            .build_call(calloc, &[cap.into(), item_size.into()], "stream.items")
            .map_err(|error| format!("LLVM backend failed to allocate stream items: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "calloc did not return a value".to_string())?;
        self.builder
            .build_store(cap_ptr, cap)
            .map_err(|error| format!("LLVM backend failed to store stream cap: {error}"))?;
        self.builder
            .build_store(count_ptr, i64_ty.const_zero())
            .map_err(|error| format!("LLVM backend failed to store stream count: {error}"))?;
        self.builder
            .build_store(items_ptr, items)
            .map_err(|error| format!("LLVM backend failed to store stream items: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter stream loop: {error}"))?;

        self.builder.position_at_end(loop_block);
        let count = self
            .builder
            .build_load(i64_ty, count_ptr, "count")
            .map_err(|error| format!("LLVM backend failed to load stream count: {error}"))?
            .into_int_value();
        let cap = self
            .builder
            .build_load(i64_ty, cap_ptr, "cap")
            .map_err(|error| format!("LLVM backend failed to load stream cap: {error}"))?
            .into_int_value();
        let full = self
            .builder
            .build_int_compare(IntPredicate::EQ, count, cap, "stream.full")
            .map_err(|error| format!("LLVM backend failed to compare stream cap: {error}"))?;
        self.builder
            .build_conditional_branch(full, grow_block, read_block)
            .map_err(|error| format!("LLVM backend failed to branch on stream grow: {error}"))?;

        self.builder.position_at_end(grow_block);
        let new_cap = self
            .builder
            .build_int_mul(cap, i64_ty.const_int(2, false), "new_cap")
            .map_err(|error| format!("LLVM backend failed to grow stream cap: {error}"))?;
        let byte_len = self
            .builder
            .build_int_mul(new_cap, item_size, "byte_len")
            .map_err(|error| {
                format!("LLVM backend failed to compute stream byte length: {error}")
            })?;
        let realloc = self.runtime_function(
            "realloc",
            Some(ptr_ty.into()),
            &[ptr_ty.into(), i64_ty.into()],
        )?;
        let old_items = self
            .builder
            .build_load(ptr_ty, items_ptr, "old_items")
            .map_err(|error| format!("LLVM backend failed to load old stream items: {error}"))?;
        let new_items = self
            .builder
            .build_call(realloc, &[old_items.into(), byte_len.into()], "new_items")
            .map_err(|error| format!("LLVM backend failed to reallocate stream items: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "realloc did not return a value".to_string())?;
        self.builder
            .build_store(cap_ptr, new_cap)
            .map_err(|error| format!("LLVM backend failed to store new stream cap: {error}"))?;
        self.builder
            .build_store(items_ptr, new_items)
            .map_err(|error| format!("LLVM backend failed to store new stream items: {error}"))?;
        self.builder
            .build_unconditional_branch(read_block)
            .map_err(|error| format!("LLVM backend failed to leave stream grow: {error}"))?;

        self.builder.position_at_end(read_block);
        let items = self
            .builder
            .build_load(ptr_ty, items_ptr, "items")
            .map_err(|error| format!("LLVM backend failed to load stream items: {error}"))?
            .into_pointer_value();
        let count = self
            .builder
            .build_load(i64_ty, count_ptr, "count")
            .map_err(|error| format!("LLVM backend failed to reload stream count: {error}"))?
            .into_int_value();
        let item_ptr = unsafe {
            self.builder
                .build_gep(item_llvm_ty, items, &[count], "item_ptr")
                .map_err(|error| {
                    format!("LLVM backend failed to compute stream item ptr: {error}")
                })?
        };
        let state = self
            .builder
            .build_extract_value(stream, 3, "stream.state")
            .map_err(|error| format!("LLVM backend failed to extract stream state: {error}"))?
            .into_pointer_value();
        let next_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
        let status = self
            .builder
            .build_indirect_call(
                next_ty,
                next,
                &[state.into(), item_ptr.into(), fault_ptr.into()],
                "status",
            )
            .map_err(|error| format!("LLVM backend failed to call stream next: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "stream next did not return a value".to_string())?
            .into_int_value();
        self.builder
            .build_store(status_ptr, status)
            .map_err(|error| format!("LLVM backend failed to store stream status: {error}"))?;
        let has_item = self
            .builder
            .build_int_compare(IntPredicate::SGT, status, i32_ty.const_zero(), "has_item")
            .map_err(|error| format!("LLVM backend failed to test stream item status: {error}"))?;
        let inc_block = self
            .context
            .append_basic_block(function, "stream.to_seq_inc");
        self.builder
            .build_conditional_branch(has_item, inc_block, done_block)
            .map_err(|error| format!("LLVM backend failed to branch on stream item: {error}"))?;

        self.builder.position_at_end(inc_block);
        let count = self
            .builder
            .build_load(i64_ty, count_ptr, "count")
            .map_err(|error| format!("LLVM backend failed to load stream inc count: {error}"))?
            .into_int_value();
        let next_count = self
            .builder
            .build_int_add(count, i64_ty.const_int(1, false), "next_count")
            .map_err(|error| format!("LLVM backend failed to increment stream count: {error}"))?;
        self.builder
            .build_store(count_ptr, next_count)
            .map_err(|error| format!("LLVM backend failed to store stream inc count: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue stream loop: {error}"))?;

        self.builder.position_at_end(done_block);
        let status = self
            .builder
            .build_load(i32_ty, status_ptr, "status")
            .map_err(|error| format!("LLVM backend failed to load final stream status: {error}"))?
            .into_int_value();
        let no_more = self
            .builder
            .build_int_compare(IntPredicate::EQ, status, i32_ty.const_zero(), "no_more")
            .map_err(|error| format!("LLVM backend failed to test stream done: {error}"))?;
        self.builder
            .build_conditional_branch(no_more, ok_block, item_fault_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on stream final status: {error}")
            })?;

        self.builder.position_at_end(item_fault_block);
        let fault = self
            .builder
            .build_load(fault_ty, fault_ptr, "fault")
            .map_err(|error| format!("LLVM backend failed to load stream fault: {error}"))?;
        let faulted = self.faultable_value(seq_ty, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store stream item fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave stream item fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let count = self
            .builder
            .build_load(i64_ty, count_ptr, "count")
            .map_err(|error| format!("LLVM backend failed to load final stream count: {error}"))?
            .into_int_value();
        let items = self
            .builder
            .build_load(ptr_ty, items_ptr, "items")
            .map_err(|error| format!("LLVM backend failed to load final stream items: {error}"))?;
        let mut seq = self
            .types
            .basic_type(seq_ty)?
            .into_struct_type()
            .const_zero();
        seq = self
            .builder
            .build_insert_value(seq, count, 0, "seq")
            .map_err(|error| format!("LLVM backend failed to build stream sequence: {error}"))?
            .into_struct_value();
        seq = self
            .builder
            .build_insert_value(seq, items, 1, "seq")
            .map_err(|error| format!("LLVM backend failed to build stream sequence: {error}"))?
            .into_struct_value();
        let ok = self.faultable_value(seq_ty, false, None, Some(seq.into()))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store stream ok: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave stream ok: {error}"))?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, "stream.to_seq")
            .map_err(|error| format!("LLVM backend failed to load stream.to_seq: {error}"))
    }

    fn fault_from_cstr(
        &mut self,
        message: &str,
        label: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let message = self
            .builder
            .build_global_string_ptr(message, &format!("{label}.fault_msg"))
            .map_err(|error| {
                format!("LLVM backend failed to build `{label}` fault string: {error}")
            })?;
        let fault_fn = self.runtime_function(
            "fa_fault_cstr",
            Some(self.runtime_pair_type().into()),
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        let fault = self
            .builder
            .build_call(fault_fn, &[message.as_pointer_value().into()], "fault")
            .map_err(|error| format!("LLVM backend failed to call fa_fault_cstr: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "fa_fault_cstr did not return a value".to_string())?;
        self.runtime_pair_to_value(fault, &Ty::Fault)
    }

    fn increment_and_continue(
        &mut self,
        index_ptr: inkwell::values::PointerValue<'ctx>,
        loop_block: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(), String> {
        let i = self
            .builder
            .build_load(self.context.i64_type(), index_ptr, "i")
            .map_err(|error| format!("LLVM backend failed to load loop index: {error}"))?
            .into_int_value();
        let next = self
            .builder
            .build_int_add(i, self.context.i64_type().const_int(1, false), "next")
            .map_err(|error| format!("LLVM backend failed to increment loop index: {error}"))?;
        self.builder
            .build_store(index_ptr, next)
            .map_err(|error| format!("LLVM backend failed to store loop index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to continue loop: {error}"))?;
        Ok(())
    }

    fn copy_seq_range(
        &mut self,
        src: BasicValueEnum<'ctx>,
        src_ty: &Ty,
        dst: BasicValueEnum<'ctx>,
        dst_ty: &Ty,
        src_start: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), String> {
        self.copy_seq_range_to_offset(
            src,
            src_ty,
            dst,
            dst_ty,
            self.context.i64_type().const_zero(),
            src_start,
            len,
        )
    }

    fn copy_seq_range_to_offset_from_zero(
        &mut self,
        src: BasicValueEnum<'ctx>,
        src_ty: &Ty,
        dst: BasicValueEnum<'ctx>,
        dst_ty: &Ty,
        dst_start: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), String> {
        self.copy_seq_range_to_offset(
            src,
            src_ty,
            dst,
            dst_ty,
            dst_start,
            self.context.i64_type().const_zero(),
            len,
        )
    }

    fn seq_count_tuple_input(
        &mut self,
        input: &LlvmValue<'ctx>,
        name: &str,
    ) -> Result<(BasicValueEnum<'ctx>, Ty, IntValue<'ctx>), String> {
        let Ty::Tuple(items) = input.ty.clone() else {
            return Err(format!("{name} expected tuple input"));
        };
        let [seq_ty @ Ty::Seq(_), Ty::Int] = items.as_slice() else {
            return Err(format!("{name} expected (Seq[V],Int) input"));
        };
        let seq = self.extract_tuple_field(input, 0)?;
        let count = self.extract_tuple_field(input, 1)?.into_int_value();
        Ok((seq, seq_ty.clone(), count))
    }

    fn seq_index_invalid(
        &mut self,
        seq: BasicValueEnum<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let zero = self.context.i64_type().const_zero();
        let negative = self
            .builder
            .build_int_compare(IntPredicate::SLT, index, zero, "seq.index_negative")
            .map_err(|error| format!("LLVM backend failed to compare sequence index: {error}"))?;
        let count = self.seq_count(seq)?;
        let out_of_range = self
            .builder
            .build_int_compare(IntPredicate::UGE, index, count, "seq.index_range")
            .map_err(|error| format!("LLVM backend failed to compare sequence index: {error}"))?;
        self.builder
            .build_or(negative, out_of_range, "seq.index_invalid")
            .map_err(|error| {
                format!("LLVM backend failed to combine sequence index checks: {error}")
            })
    }

    fn branch_die_if_negative(
        &mut self,
        value: IntValue<'ctx>,
        message: &str,
        label: &str,
    ) -> Result<(), String> {
        let negative = self
            .builder
            .build_int_compare(
                IntPredicate::SLT,
                value,
                self.context.i64_type().const_zero(),
                &format!("{label}.negative"),
            )
            .map_err(|error| format!("LLVM backend failed to compare `{label}` value: {error}"))?;
        self.branch_die_if(negative, message, label)
    }

    fn branch_die_if(
        &mut self,
        condition: IntValue<'ctx>,
        message: &str,
        label: &str,
    ) -> Result<(), String> {
        let function = self.current_function()?;
        let die_block = self
            .context
            .append_basic_block(function, &format!("{label}.die"));
        let continue_block = self
            .context
            .append_basic_block(function, &format!("{label}.continue"));
        self.builder
            .build_conditional_branch(condition, die_block, continue_block)
            .map_err(|error| {
                format!("LLVM backend failed to branch on `{label}` usage: {error}")
            })?;
        self.builder.position_at_end(die_block);
        self.die_usage(message, label)?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn die_usage(&mut self, message: &str, label: &str) -> Result<(), String> {
        let message = self
            .builder
            .build_global_string_ptr(message, &format!("{label}.error"))
            .map_err(|error| format!("LLVM backend failed to build `{label}` error: {error}"))?;
        let die = self.runtime_function(
            "fa_die_usage",
            None,
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        self.builder
            .build_call(
                die,
                &[message.as_pointer_value().into()],
                &format!("{label}.die"),
            )
            .map_err(|error| format!("LLVM backend failed to call fa_die_usage: {error}"))?;
        self.builder.build_unreachable().map_err(|error| {
            format!("LLVM backend failed to build `{label}` unreachable: {error}")
        })?;
        Ok(())
    }

    fn emit_faultable_seq_index(
        &mut self,
        seq: BasicValueEnum<'ctx>,
        seq_ty: &Ty,
        index: IntValue<'ctx>,
        output_ty: &Ty,
        label: &str,
        message: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let Ty::Faultable(output_inner) = output_ty else {
            return Err(format!(
                "{label} expected faultable output, found `{output_ty}`"
            ));
        };
        let output_llvm_ty = self.types.basic_type(output_ty)?;
        let out_ptr = self
            .builder
            .build_alloca(output_llvm_ty, &format!("{label}.result"))
            .map_err(|error| {
                format!("LLVM backend failed to allocate `{label}` result: {error}")
            })?;
        let function = self.current_function()?;
        let fault_block = self
            .context
            .append_basic_block(function, &format!("{label}.fault"));
        let ok_block = self
            .context
            .append_basic_block(function, &format!("{label}.ok"));
        let after_block = self
            .context
            .append_basic_block(function, &format!("{label}.after"));
        let invalid = self.seq_index_invalid(seq, index)?;
        self.builder
            .build_conditional_branch(invalid, fault_block, ok_block)
            .map_err(|error| format!("LLVM backend failed to branch in `{label}`: {error}"))?;

        self.builder.position_at_end(fault_block);
        let message_value = self
            .builder
            .build_global_string_ptr(message, &format!("{label}.fault_msg"))
            .map_err(|error| {
                format!("LLVM backend failed to build `{label}` fault string: {error}")
            })?;
        let fault_fn = self.runtime_function(
            "fa_fault_cstr",
            Some(self.runtime_pair_type().into()),
            &[self.context.ptr_type(AddressSpace::default()).into()],
        )?;
        let fault = self
            .builder
            .build_call(
                fault_fn,
                &[message_value.as_pointer_value().into()],
                "fault",
            )
            .map_err(|error| format!("LLVM backend failed to call fa_fault_cstr: {error}"))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "fa_fault_cstr did not return a value".to_string())?;
        let fault = self.runtime_pair_to_value(fault, &Ty::Fault)?;
        let faulted = self.faultable_value(output_inner, true, Some(fault), None)?;
        self.builder
            .build_store(out_ptr, faulted)
            .map_err(|error| format!("LLVM backend failed to store `{label}` fault: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave `{label}` fault: {error}"))?;

        self.builder.position_at_end(ok_block);
        let item = self.load_seq_item(seq, seq_ty, index)?;
        let ok = self.faultable_value(output_inner, false, None, Some(item))?;
        self.builder
            .build_store(out_ptr, ok)
            .map_err(|error| format!("LLVM backend failed to store `{label}` value: {error}"))?;
        self.builder
            .build_unconditional_branch(after_block)
            .map_err(|error| format!("LLVM backend failed to leave `{label}` ok: {error}"))?;

        self.builder.position_at_end(after_block);
        self.builder
            .build_load(output_llvm_ty, out_ptr, &format!("{label}.result"))
            .map_err(|error| format!("LLVM backend failed to load `{label}` result: {error}"))
    }

    fn copy_seq_range_to_offset(
        &mut self,
        src: BasicValueEnum<'ctx>,
        src_ty: &Ty,
        dst: BasicValueEnum<'ctx>,
        dst_ty: &Ty,
        dst_start: IntValue<'ctx>,
        src_start: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), String> {
        let function = self.current_function()?;
        let loop_block = self.context.append_basic_block(function, "copy.loop");
        let body_block = self.context.append_basic_block(function, "copy.body");
        let after_block = self.context.append_basic_block(function, "copy.after");
        let j_ptr = self
            .builder
            .build_alloca(self.context.i64_type(), "j")
            .map_err(|error| format!("LLVM backend failed to allocate copy index: {error}"))?;
        self.builder
            .build_store(j_ptr, self.context.i64_type().const_zero())
            .map_err(|error| format!("LLVM backend failed to initialize copy index: {error}"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| format!("LLVM backend failed to enter copy loop: {error}"))?;
        self.builder.position_at_end(loop_block);
        let j = self
            .builder
            .build_load(self.context.i64_type(), j_ptr, "j")
            .map_err(|error| format!("LLVM backend failed to load copy index: {error}"))?
            .into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::ULT, j, len, "copy.cond")
            .map_err(|error| format!("LLVM backend failed to compare copy index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in copy loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let src_i = self
            .builder
            .build_int_add(src_start, j, "src_i")
            .map_err(|error| format!("LLVM backend failed to compute source index: {error}"))?;
        let dst_i = self
            .builder
            .build_int_add(dst_start, j, "dst_i")
            .map_err(|error| {
                format!("LLVM backend failed to compute destination index: {error}")
            })?;
        let item = self.load_seq_item(src, src_ty, src_i)?;
        self.store_seq_item(dst, dst_ty, dst_i, item)?;
        self.increment_and_continue(j_ptr, loop_block)?;
        self.builder.position_at_end(after_block);
        Ok(())
    }
}

fn sequence_item_ty(seq_ty: &Ty) -> Result<&Ty, String> {
    let Ty::Seq(item) = seq_ty else {
        return Err(format!("expected sequence type, found `{seq_ty}`"));
    };
    Ok(item.as_ref())
}
