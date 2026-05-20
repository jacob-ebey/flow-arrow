static FaValue fa_map(FaValue input, FaFunction fn) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "map");
  FaValue out = fa_seq_new(seq.seq.count);
  for (size_t i = 0; i < seq.seq.count; i++) {
    fa_seq_set(&out, i, fn(seq.seq.items[i]));
  }
  return out;
}

static FaFaultMapResult fa_fault_map(FaValue input, FaFunction fn) {
  if (input.kind == FA_FAULT) {
    FaFaultMapResult pair;
    pair.ok = fa_seq_new(0);
    pair.faults = fa_seq_new(1);
    fa_seq_set(&pair.faults, 0, input);
    return pair;
  }
  FaValue seq = fa_expect_seq(input, "fault map");
  FaValue ok = fa_seq_new(seq.seq.count);
  FaValue faults = fa_seq_new(seq.seq.count);
  size_t ok_count = 0;
  size_t fault_count = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue result = fn(seq.seq.items[i]);
    if (result.kind == FA_FAULT) {
      if (fn == fa_builtin_parse_real || fn == fa_builtin_parse_int) {
        FaValue bytes = fa_expect_bytes(seq.seq.items[i], fn == fa_builtin_parse_real ? "parse_real" : "parse_int");
        char message[512];
        snprintf(message, sizeof(message), "line %zu: expected %s, got \"%.*s\"", i + 1, fn == fa_builtin_parse_real ? "Real" : "Int", (int)bytes.len, bytes.bytes);
        result = fa_fault_from_cstr(message);
      }
      fa_seq_set(&faults, fault_count++, result);
    } else {
      fa_seq_set(&ok, ok_count++, result);
    }
  }
  FaFaultMapResult pair;
  pair.ok = fa_seq_new(ok_count);
  pair.faults = fa_seq_new(fault_count);
  for (size_t i = 0; i < ok_count; i++) fa_seq_set(&pair.ok, i, ok.seq.items[i]);
  for (size_t i = 0; i < fault_count; i++) fa_seq_set(&pair.faults, i, faults.seq.items[i]);
  return pair;
}

static FaValue fa_filter(FaValue input, FaFunction pred) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "filter");
  FaValue out = fa_seq_new(seq.seq.count);
  size_t count = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue keep = pred(seq.seq.items[i]);
    if (keep.kind != FA_BOOL) fa_die_usage("flowarrow runtime: filter predicate must return Bool");
    if (keep.b) fa_seq_set(&out, count++, seq.seq.items[i]);
  }
  FaValue trimmed = fa_seq_new(count);
  for (size_t i = 0; i < count; i++) fa_seq_set(&trimmed, i, out.seq.items[i]);
  return trimmed;
}

static FaValue fa_repeat(FaValue initial, FaValue count_value, FaFunction step) {
  int64_t count = fa_expect_int(count_value, "repeat count");
  if (count < 0) fa_die_usage("flowarrow runtime: repeat count cannot be negative");
  FaValue state = initial;
  for (int64_t i = 0; i < count; i++) state = step(state);
  return state;
}

static FaValue fa_reduce(FaValue input, FaReduceOp op, FaValue identity) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "reduce");
  if (op == FA_REDUCE_CONCAT_BYTES) return seq.seq.count == 0 ? identity : fa_builtin_concat_bytes(seq);
  if (op == FA_REDUCE_ADD) {
    if (identity.kind == FA_INT) {
      int64_t total = identity.i;
      for (size_t i = 0; i < seq.seq.count; i++) {
        FaValue item = seq.seq.items[i];
        if (item.kind == FA_FAULT) return item;
        total = fa_checked_integer_add(total, fa_expect_int(item, "reduce add"), "reduce add");
      }
      return fa_int(total);
    }
    double total = fa_expect_number(identity, "reduce add identity");
    for (size_t i = 0; i < seq.seq.count; i++) {
      FaValue item = seq.seq.items[i];
      if (item.kind == FA_FAULT) return item;
      total += fa_expect_number(item, "reduce add");
    }
    return fa_real(total);
  }
  fa_die_usage("flowarrow runtime: unsupported reduce op");
  return fa_unit();
}

static FaValue fa_scan(FaValue input, FaFunction op, FaValue identity) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "scan");
  FaValue out = fa_seq_new(seq.seq.count);
  FaValue state = identity;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue pair = fa_seq_new(2);
    fa_seq_set(&pair, 0, state);
    fa_seq_set(&pair, 1, seq.seq.items[i]);
    state = op(pair);
    fa_seq_set(&out, i, state);
  }
  return out;
}

static int fa_value_to_exit_code(FaValue value) {
  if (value.kind == FA_FAULT) {
    fprintf(stderr, "%.*s\n", (int)value.len, value.bytes);
    exit(65);
  }
  return (int)fa_expect_int(value, "program result");
}

