use super::{DirectLlvm, LlvmValue, Ty};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue};

struct SeqCopyRange<'ctx, 'ty> {
    src: BasicValueEnum<'ctx>,
    src_ty: &'ty Ty,
    dst: BasicValueEnum<'ctx>,
    dst_ty: &'ty Ty,
    dst_start: IntValue<'ctx>,
    src_start: IntValue<'ctx>,
    len: IntValue<'ctx>,
}

impl<'ctx, 'a> DirectLlvm<'ctx, 'a> {
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
            "fa_calloc",
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
            .ok_or_else(|| "fa_calloc did not return a value".to_string())?;
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
            "fa_realloc",
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
            .ok_or_else(|| "fa_realloc did not return a value".to_string())?;
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
        self.copy_seq_range_to_offset(SeqCopyRange {
            src,
            src_ty,
            dst,
            dst_ty,
            dst_start: self.context.i64_type().const_zero(),
            src_start,
            len,
        })
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
        self.copy_seq_range_to_offset(SeqCopyRange {
            src,
            src_ty,
            dst,
            dst_ty,
            dst_start,
            src_start: self.context.i64_type().const_zero(),
            len,
        })
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

    fn copy_seq_range_to_offset(&mut self, range: SeqCopyRange<'ctx, '_>) -> Result<(), String> {
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
            .build_int_compare(IntPredicate::ULT, j, range.len, "copy.cond")
            .map_err(|error| format!("LLVM backend failed to compare copy index: {error}"))?;
        self.builder
            .build_conditional_branch(cond, body_block, after_block)
            .map_err(|error| format!("LLVM backend failed to branch in copy loop: {error}"))?;
        self.builder.position_at_end(body_block);
        let src_i = self
            .builder
            .build_int_add(range.src_start, j, "src_i")
            .map_err(|error| format!("LLVM backend failed to compute source index: {error}"))?;
        let dst_i = self
            .builder
            .build_int_add(range.dst_start, j, "dst_i")
            .map_err(|error| {
                format!("LLVM backend failed to compute destination index: {error}")
            })?;
        let item = self.load_seq_item(range.src, range.src_ty, src_i)?;
        self.store_seq_item(range.dst, range.dst_ty, dst_i, item)?;
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
