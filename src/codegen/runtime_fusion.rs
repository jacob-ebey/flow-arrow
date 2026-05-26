use super::*;

impl<'a> TypedCodegen<'a> {
    pub(super) fn fusion_for_name(&self, name: &str) -> Option<Fusion> {
        let callable = self.callables.get(name)?;
        self.fusion_for_callable(callable, &mut HashSet::new(), false)
    }

    pub(super) fn gpu_fusion_for_name(&self, name: &str) -> Option<Fusion> {
        let callable = self.callables.get(name)?;
        self.fusion_for_callable(callable, &mut HashSet::new(), true)
    }

    fn fusion_for_callable(
        &self,
        callable: &TypedCallable,
        visiting: &mut HashSet<String>,
        allow_zero_initializers: bool,
    ) -> Option<Fusion> {
        if !visiting.insert(callable.name.clone()) {
            return None;
        }
        let fusion = self.fusion_for_callable_inner(callable, visiting, allow_zero_initializers);
        visiting.remove(&callable.name);
        fusion
    }

    fn fusion_for_callable_inner(
        &self,
        callable: &TypedCallable,
        visiting: &mut HashSet<String>,
        allow_zero_initializers: bool,
    ) -> Option<Fusion> {
        if let Some(fusion) = self.mean_fusion(callable, visiting) {
            return Some(fusion);
        }
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let zero_bindings = if allow_zero_initializers {
            self.zero_initializer_bindings(callable)
        } else {
            HashSet::new()
        };
        let effective_chains = if allow_zero_initializers {
            callable
                .chains
                .iter()
                .filter(|chain| {
                    final_variable(chain)
                        .map(|name| !zero_bindings.contains(name))
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>()
        } else {
            callable.chains.iter().collect::<Vec<_>>()
        };
        let [chain] = effective_chains.as_slice() else {
            return None;
        };
        let stages = stages_binding_output(chain, &output.name)?;
        match stages {
            [stage] => match &stage.kind {
                TypedStageKind::Reduce { op, identity, .. }
                    if self.is_add(op) && is_zero_or_binding(identity, &zero_bindings) =>
                {
                    Some(Fusion::Sum)
                }
                TypedStageKind::Map { name, .. } => {
                    self.unary_op_for_node(name).map(Fusion::MapUnary)
                }
                TypedStageKind::Call { name, .. } if self.is_sqrt(name) => {
                    Some(Fusion::Sqrt(Box::new(Fusion::Sum)))
                }
                _ => None,
            },
            [first, second] => match (&first.kind, &second.kind) {
                (
                    TypedStageKind::Map { name: node, .. },
                    TypedStageKind::Call { name: next, .. },
                ) => {
                    if self.called_fusion(node, visiting, allow_zero_initializers)
                        == Some(Fusion::Sum)
                        && self.called_fusion(next, visiting, allow_zero_initializers)
                            == Some(Fusion::Sum)
                    {
                        Some(Fusion::NestedSum)
                    } else if self.called_fusion(next, visiting, allow_zero_initializers)
                        == Some(Fusion::Sum)
                    {
                        self.map_reduce_op_for_node(node).map(Fusion::MapReduceAdd)
                    } else {
                        None
                    }
                }
                (
                    TypedStageKind::Map { name: node, .. },
                    TypedStageKind::Reduce { op, identity, .. },
                ) if self.is_add(op) && is_zero_or_binding(identity, &zero_bindings) => {
                    self.map_reduce_op_for_node(node).map(Fusion::MapReduceAdd)
                }
                (
                    TypedStageKind::Call { name: zip, .. },
                    TypedStageKind::Map { name: node, .. },
                ) if self.is_zip(zip) => {
                    if self.binary_eq_for_node(node) {
                        Some(Fusion::ZipAllEqual)
                    } else {
                        self.binary_op_for_node(node).map(Fusion::ZipMap)
                    }
                }
                (
                    TypedStageKind::Call { name: first, .. },
                    TypedStageKind::Call { name: second, .. },
                ) => {
                    let first_fusion = self.called_fusion(first, visiting, allow_zero_initializers);
                    let second_fusion =
                        self.called_fusion(second, visiting, allow_zero_initializers);
                    if first_fusion == Some(Fusion::ZipMap(BinaryOp::Sub))
                        && second_fusion == Some(Fusion::MapReduceAdd(MapOp::Square))
                    {
                        return Some(Fusion::ZipDifferenceSquareSum);
                    }
                    if self.is_sqrt(second) {
                        return first_fusion.map(|fusion| Fusion::Sqrt(Box::new(fusion)));
                    }
                    None
                }
                _ => None,
            },
            [first, second, third] => match (&first.kind, &second.kind, &third.kind) {
                (
                    TypedStageKind::Call { name: zip, .. },
                    TypedStageKind::Map { name: node, .. },
                    TypedStageKind::Reduce { op, identity, .. },
                ) if self.is_zip(zip)
                    && self.is_add(op)
                    && is_zero_or_binding(identity, &zero_bindings) =>
                {
                    self.binary_op_for_node(node).map(Fusion::ZipMapReduceAdd)
                }
                (
                    TypedStageKind::Call { name: zip, .. },
                    TypedStageKind::Map { name: node, .. },
                    TypedStageKind::Call { name: all, .. },
                ) if self.is_zip(zip) && self.is_all(all) && self.binary_eq_for_node(node) => {
                    Some(Fusion::ZipAllEqual)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn mean_fusion(
        &self,
        callable: &TypedCallable,
        visiting: &mut HashSet<String>,
    ) -> Option<Fusion> {
        let [input] = callable.inputs.as_slice() else {
            return None;
        };
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let [sum_chain, length_chain, div_chain] = callable.chains.as_slice() else {
            return None;
        };
        let sum_binding = final_variable(sum_chain)?;
        let length_binding = final_variable(length_chain)?;
        if !matches!(&sum_chain.source.kind, TypedEndpointKind::Variable(name) if name == &input.name)
        {
            return None;
        }
        if !matches!(&length_chain.source.kind, TypedEndpointKind::Variable(name) if name == &input.name)
        {
            return None;
        }
        let sum_stages = stages_binding_output(sum_chain, sum_binding)?;
        let length_stages = stages_binding_output(length_chain, length_binding)?;
        if !matches!(sum_stages, [stage] if matches!(&stage.kind, TypedStageKind::Call { name, .. } if self.called_fusion(name, visiting, false) == Some(Fusion::Sum)))
        {
            return None;
        }
        if !matches!(length_stages, [stage] if matches!(&stage.kind, TypedStageKind::Call { name, .. } if self.is_length(name)))
        {
            return None;
        }
        let div_stages = stages_binding_output(div_chain, &output.name)?;
        if !matches!(div_stages, [stage] if matches!(&stage.kind, TypedStageKind::Call { name, .. } if self.is_div(name)))
        {
            return None;
        }
        if !matches!(
            &div_chain.source.kind,
            TypedEndpointKind::Tuple(items)
                if items.len() == 2
                    && matches!(&items[0].kind, TypedEndpointKind::Variable(name) if name == sum_binding)
                    && matches!(&items[1].kind, TypedEndpointKind::Variable(name) if name == length_binding)
        ) {
            return None;
        }
        Some(Fusion::Mean)
    }

    pub(super) fn emit_fusion_assign(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        fusion: &Fusion,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        match fusion {
            Fusion::Sum => self.emit_fused_sum(out, target, input, input_ty),
            Fusion::NestedSum => self.emit_fused_nested_sum(out, target, input, input_ty),
            Fusion::Mean => self.emit_fused_mean(out, target, input),
            Fusion::MapUnary(op) => self.emit_fused_map_unary(out, target, output_ty, *op, input),
            Fusion::ZipMap(op) => self.emit_fused_zip_map(out, target, output_ty, *op, input),
            Fusion::ZipMapReduceAdd(op) => self.emit_fused_zip_map_reduce(out, target, *op, input),
            Fusion::MapReduceAdd(op) => self.emit_fused_map_reduce(out, target, *op, input),
            Fusion::ZipAllEqual => self.emit_fused_zip_all_equal(out, target, input),
            Fusion::ZipDifferenceSquareSum => {
                self.emit_fused_zip_difference_square_sum(out, target, input)
            }
            Fusion::Sqrt(inner) => {
                let tmp = self.next_temp();
                out.push_str(&format!("  double {tmp};\n"));
                self.emit_fusion_assign(out, &tmp, &Ty::F64, inner, input, input_ty)?;
                out.push_str(&format!("  {target} = sqrt({tmp});\n"));
                Ok(())
            }
        }
    }

    fn emit_fused_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(item_ty) = input_ty else {
            return Err("sum fusion expected sequence input".to_string());
        };
        let i = self.next_temp();
        out.push_str(&format!("  {target} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target} = {};\n",
            add_expr(target, &format!("{input}.items[{i}]"), item_ty)
        ));
        out.push_str("  }\n");
        Ok(())
    }

    pub(super) fn emit_fused_nested_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Seq(row_ty) = input_ty else {
            return Err("nested sum fusion expected sequence input".to_string());
        };
        let Ty::Seq(item_ty) = row_ty.as_ref() else {
            return Err("nested sum fusion expected nested sequence input".to_string());
        };
        let r = self.next_temp();
        let c = self.next_temp();
        out.push_str(&format!("  {target} = 0;\n"));
        out.push_str(&format!(
            "  for (size_t {r} = 0; {r} < {input}.count; {r}++) {{\n"
        ));
        out.push_str(&format!(
            "    for (size_t {c} = 0; {c} < {input}.items[{r}].count; {c}++) {{\n"
        ));
        out.push_str(&format!(
            "      {target} = {};\n",
            add_expr(target, &format!("{input}.items[{r}].items[{c}]"), item_ty)
        ));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    pub(super) fn emit_fused_matvec_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("matvec-sum fusion expected tuple input".to_string());
        };
        let [Ty::Seq(row_ty), Ty::Seq(_)] = items.as_slice() else {
            return Err("matvec-sum fusion expected (matrix, vector) input".to_string());
        };
        let Ty::Seq(item_ty) = row_ty.as_ref() else {
            return Err("matvec-sum fusion expected matrix input".to_string());
        };
        if item_ty.as_ref() != &Ty::F64 {
            return Err("matvec-sum fusion expected real matrix input".to_string());
        }

        let row = self.next_temp();
        let col = self.next_temp();
        let dot = self.next_temp();
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {row} = 0; {row} < {input}.f0.count; {row}++) {{\n"
        ));
        out.push_str(&format!(
            "    if ({input}.f0.items[{row}].count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"
        ));
        out.push_str(&format!("    double {dot} = 0.0;\n"));
        out.push_str(&format!(
            "    for (size_t {col} = 0; {col} < {input}.f1.count; {col}++) {{\n"
        ));
        out.push_str(&format!(
            "      {dot} += {input}.f0.items[{row}].items[{col}] * {input}.f1.items[{col}];\n"
        ));
        out.push_str("    }\n");
        out.push_str(&format!("    {target} += {dot};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    pub(super) fn emit_fused_matmul_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
        input_ty: &Ty,
    ) -> Result<(), String> {
        let Ty::Tuple(items) = input_ty else {
            return Err("matmul-sum fusion expected tuple input".to_string());
        };
        let [Ty::Seq(left_row_ty), Ty::Seq(right_row_ty)] = items.as_slice() else {
            return Err("matmul-sum fusion expected matrix pair input".to_string());
        };
        if !matches!(left_row_ty.as_ref(), Ty::Seq(item_ty) if item_ty.as_ref() == &Ty::F64)
            || !matches!(right_row_ty.as_ref(), Ty::Seq(item_ty) if item_ty.as_ref() == &Ty::F64)
        {
            return Err("matmul-sum fusion expected real matrix inputs".to_string());
        }

        let inner = self.next_temp();
        let cols = self.next_temp();
        let check = self.next_temp();
        let k = self.next_temp();
        let row = self.next_temp();
        let col = self.next_temp();
        let left_sum = self.next_temp();
        let right_sum = self.next_temp();

        out.push_str(&format!("  size_t {inner} = {input}.f1.count;\n"));
        out.push_str(&format!(
            "  size_t {cols} = {inner} == 0 ? 0 : {input}.f1.items[0].count;\n"
        ));
        out.push_str(&format!(
            "  for (size_t {check} = 0; {check} < {inner}; {check}++) {{\n"
        ));
        out.push_str(&format!(
            "    if ({input}.f1.items[{check}].count != {cols}) fa_die_usage(\"transpose: rows must have the same length\");\n"
        ));
        out.push_str("  }\n");
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!("  if ({cols} > 0) {{\n"));
        out.push_str(&format!(
            "    for (size_t {k} = 0; {k} < {inner}; {k}++) {{\n"
        ));
        out.push_str(&format!("      double {left_sum} = 0.0;\n"));
        out.push_str(&format!(
            "      for (size_t {row} = 0; {row} < {input}.f0.count; {row}++) {{\n"
        ));
        out.push_str(&format!(
            "        if ({input}.f0.items[{row}].count != {inner}) fa_die_usage(\"zip: sequences must have the same length\");\n"
        ));
        out.push_str(&format!(
            "        {left_sum} += {input}.f0.items[{row}].items[{k}];\n"
        ));
        out.push_str("      }\n");
        out.push_str(&format!("      double {right_sum} = 0.0;\n"));
        out.push_str(&format!(
            "      for (size_t {col} = 0; {col} < {cols}; {col}++) {{\n"
        ));
        out.push_str(&format!(
            "        {right_sum} += {input}.f1.items[{k}].items[{col}];\n"
        ));
        out.push_str("      }\n");
        out.push_str(&format!("      {target} += {left_sum} * {right_sum};\n"));
        out.push_str("    }\n");
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_mean(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let total = self.next_temp();
        out.push_str(&format!("  double {total} = 0.0;\n"));
        self.emit_fused_sum(out, &total, input, &Ty::Seq(Box::new(Ty::F64)))?;
        out.push_str(&format!("  {target} = {total} / (double){input}.count;\n"));
        Ok(())
    }

    fn emit_fused_map_unary(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        op: UnaryOp,
        input: &str,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  {target} = {new_fn}({input}.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        let expr = match op {
            UnaryOp::Neg => format!("-({input}.items[{i}])"),
            UnaryOp::Abs => format!("fabs({input}.items[{i}])"),
        };
        out.push_str(&format!("    {target}.items[{i}] = {expr};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_map(
        &mut self,
        out: &mut String,
        target: &str,
        output_ty: &Ty,
        op: BinaryOp,
        input: &str,
    ) -> Result<(), String> {
        let new_fn = self.types.seq_new_name(output_ty)?;
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = {new_fn}({input}.f0.count);\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target}.items[{i}] = {};\n",
            binary_op_expr(
                op,
                &format!("{input}.f0.items[{i}]"),
                &format!("{input}.f1.items[{i}]")
            )
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_map_reduce(
        &mut self,
        out: &mut String,
        target: &str,
        op: BinaryOp,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    {target} += {};\n",
            binary_op_expr(
                op,
                &format!("{input}.f0.items[{i}]"),
                &format!("{input}.f1.items[{i}]")
            )
        ));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_map_reduce(
        &mut self,
        out: &mut String,
        target: &str,
        op: MapOp,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.count; {i}++) {{\n"
        ));
        let value = format!("{input}.items[{i}]");
        let expr = match op {
            MapOp::Square => format!("({value} * {value})"),
            MapOp::Abs => format!("fabs({value})"),
        };
        out.push_str(&format!("    {target} += {expr};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_all_equal(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = true;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!("    if ({input}.f0.items[{i}] != {input}.f1.items[{i}]) {{ {target} = false; break; }}\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn emit_fused_zip_difference_square_sum(
        &mut self,
        out: &mut String,
        target: &str,
        input: &str,
    ) -> Result<(), String> {
        let i = self.next_temp();
        let delta = self.next_temp();
        out.push_str(&format!("  if ({input}.f0.count != {input}.f1.count) fa_die_usage(\"zip: sequences must have the same length\");\n"));
        out.push_str(&format!("  {target} = 0.0;\n"));
        out.push_str(&format!(
            "  for (size_t {i} = 0; {i} < {input}.f0.count; {i}++) {{\n"
        ));
        out.push_str(&format!(
            "    double {delta} = {input}.f0.items[{i}] - {input}.f1.items[{i}];\n"
        ));
        out.push_str(&format!("    {target} += {delta} * {delta};\n"));
        out.push_str("  }\n");
        Ok(())
    }

    fn zero_initializer_bindings(&self, callable: &TypedCallable) -> HashSet<String> {
        callable
            .chains
            .iter()
            .filter_map(|chain| self.zero_initializer_binding(chain))
            .collect()
    }

    fn zero_initializer_binding(&self, chain: &TypedChain) -> Option<String> {
        if !is_zero(&chain.source) {
            return None;
        }
        let binding = final_variable(chain)?;
        let stages = stages_binding_output(chain, binding)?;
        match stages {
            [] => Some(binding.to_string()),
            [stage] if matches!(&stage.kind, TypedStageKind::Call { name, .. } if self.is_from_int(name)) => {
                Some(binding.to_string())
            }
            _ => None,
        }
    }

    fn called_fusion(
        &self,
        name: &str,
        visiting: &mut HashSet<String>,
        allow_zero_initializers: bool,
    ) -> Option<Fusion> {
        self.callables.get(name).and_then(|callable| {
            self.fusion_for_callable(callable, visiting, allow_zero_initializers)
        })
    }

    fn unary_op_for_node(&self, name: &str) -> Option<UnaryOp> {
        let op = self.direct_single_builtin(name)?;
        match op.as_str() {
            "neg" => Some(UnaryOp::Neg),
            "abs" => Some(UnaryOp::Abs),
            _ => None,
        }
    }

    fn map_reduce_op_for_node(&self, name: &str) -> Option<MapOp> {
        if self.is_square_node(name) {
            return Some(MapOp::Square);
        }
        if self.unary_op_for_node(name) == Some(UnaryOp::Abs) {
            return Some(MapOp::Abs);
        }
        None
    }

    fn binary_op_for_node(&self, name: &str) -> Option<BinaryOp> {
        let op = self.direct_single_builtin(name)?;
        match op.as_str() {
            "add" => Some(BinaryOp::Add),
            "sub" => Some(BinaryOp::Sub),
            "mul" => Some(BinaryOp::Mul),
            "div" => Some(BinaryOp::Div),
            _ => None,
        }
    }

    fn binary_eq_for_node(&self, name: &str) -> bool {
        self.direct_single_builtin(name)
            .map(|op| op == "eq")
            .unwrap_or(false)
    }

    pub(super) fn is_map_sum_callable(&self, name: &str) -> bool {
        let Some(callable) = self.callables.get(name) else {
            return false;
        };
        let [input] = callable.inputs.as_slice() else {
            return false;
        };
        let [output] = callable.outputs.as_slice() else {
            return false;
        };
        let [chain] = callable.chains.as_slice() else {
            return false;
        };
        if !matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &input.name) {
            return false;
        }
        let Some([stage]) = stages_binding_output(chain, &output.name) else {
            return false;
        };
        let TypedStageKind::Map { name: node, .. } = &stage.kind else {
            return false;
        };
        self.fusion_for_name(node) == Some(Fusion::Sum)
    }

    pub(super) fn is_matmul_name(&self, name: &str) -> bool {
        name == "__flow_std_matrix_matmul"
    }

    pub(super) fn is_matvec_name(&self, name: &str) -> bool {
        name == "__flow_std_matrix_matvec"
    }

    fn direct_single_builtin(&self, name: &str) -> Option<String> {
        let callable = self.callables.get(name)?;
        let [input] = callable.inputs.as_slice() else {
            return None;
        };
        let [output] = callable.outputs.as_slice() else {
            return None;
        };
        let [chain] = callable.chains.as_slice() else {
            return None;
        };
        if !matches!(&chain.source.kind, TypedEndpointKind::Variable(name) if name == &input.name) {
            return None;
        }
        let [stage] = stages_binding_output(chain, &output.name)? else {
            return None;
        };
        let TypedStageKind::Call { name: op, .. } = &stage.kind else {
            return None;
        };
        Some(self.canonical_name(op))
    }

    fn is_square_node(&self, name: &str) -> bool {
        let Some(callable) = self.callables.get(name) else {
            return false;
        };
        let [input] = callable.inputs.as_slice() else {
            return false;
        };
        let [output] = callable.outputs.as_slice() else {
            return false;
        };
        let [chain] = callable.chains.as_slice() else {
            return false;
        };
        if !matches!(
            &chain.source.kind,
            TypedEndpointKind::Tuple(items)
                if items.len() == 2
                    && matches!(&items[0].kind, TypedEndpointKind::Variable(name) if name == &input.name)
                    && matches!(&items[1].kind, TypedEndpointKind::Variable(name) if name == &input.name)
        ) {
            return false;
        }
        matches!(
            stages_binding_output(chain, &output.name),
            Some([stage]) if matches!(&stage.kind, TypedStageKind::Call { name: op, .. } if self.is_mul(op))
        )
    }

    pub(super) fn is_add(&self, name: &str) -> bool {
        self.canonical_name(name) == "add"
    }

    fn is_mul(&self, name: &str) -> bool {
        self.canonical_name(name) == "mul"
    }

    fn is_div(&self, name: &str) -> bool {
        self.canonical_name(name) == "div"
    }

    fn is_sqrt(&self, name: &str) -> bool {
        self.canonical_name(name) == "sqrt"
    }

    fn is_from_int(&self, name: &str) -> bool {
        matches!(
            self.canonical_name(name).as_str(),
            "from_int" | "from_int_f32"
        )
    }

    fn is_zip(&self, name: &str) -> bool {
        self.canonical_name(name) == "zip"
    }

    fn is_all(&self, name: &str) -> bool {
        self.canonical_name(name) == "all"
    }

    fn is_length(&self, name: &str) -> bool {
        self.canonical_name(name) == "length"
    }
}

fn is_zero_or_binding(endpoint: &TypedEndpoint, zero_bindings: &HashSet<String>) -> bool {
    is_zero(endpoint)
        || matches!(&endpoint.kind, TypedEndpointKind::Variable(name) if zero_bindings.contains(name))
}
