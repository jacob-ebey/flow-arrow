static FaValue fa_binary_numeric(FaValue input, const char *op) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, op);
  FaValue fault = fa_propagate_fault_from_seq(seq);
  if (fault.kind == FA_FAULT) return fault;
  if (seq.seq.count != 2) {
    fprintf(stderr, "flowarrow runtime: %s expected two inputs\n", op);
    exit(65);
  }
  FaValue left = seq.seq.items[0];
  FaValue right = seq.seq.items[1];
  if (strcmp(op, "add") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    return fa_int(fa_checked_integer_add(left.i, right.i, "add"));
  }
  if (strcmp(op, "sub") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    return fa_int(fa_checked_integer_sub(left.i, right.i, "sub"));
  }
  if (strcmp(op, "mul") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    int64_t result = 0;
    if (__builtin_mul_overflow(left.i, right.i, &result)) {
      fputs("flowarrow runtime: mul overflow\n", stderr);
      exit(65);
    }
    return fa_int(result);
  }
  if (strcmp(op, "max") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    return fa_int(left.i > right.i ? left.i : right.i);
  }
  if (strcmp(op, "min") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    return fa_int(left.i < right.i ? left.i : right.i);
  }
  if (strcmp(op, "div") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    if (right.i == 0) fa_die_usage("flowarrow runtime: div by zero");
    return fa_int(left.i / right.i);
  }
  if (strcmp(op, "rem") == 0 && left.kind == FA_INT && right.kind == FA_INT) {
    if (right.i == 0) fa_die_usage("flowarrow runtime: rem by zero");
    return fa_int(left.i % right.i);
  }
  double l = fa_expect_number(left, op);
  double r = fa_expect_number(right, op);
  if (strcmp(op, "add") == 0) return fa_real(l + r);
  if (strcmp(op, "sub") == 0) return fa_real(l - r);
  if (strcmp(op, "mul") == 0) return fa_real(l * r);
  if (strcmp(op, "div") == 0) {
    if (r == 0.0) fa_die_usage("flowarrow runtime: div by zero");
    return fa_real(l / r);
  }
  if (strcmp(op, "rem") == 0) {
    if (r == 0.0) fa_die_usage("flowarrow runtime: rem by zero");
    return fa_real(fmod(l, r));
  }
  if (strcmp(op, "min") == 0) return fa_real(l < r ? l : r);
  if (strcmp(op, "max") == 0) return fa_real(l > r ? l : r);
  fa_die_usage("flowarrow runtime: unknown numeric op");
  return fa_unit();
}

static FaValue fa_builtin_add(FaValue input) { return fa_binary_numeric(input, "add"); }
static FaValue fa_builtin_sub(FaValue input) { return fa_binary_numeric(input, "sub"); }
static FaValue fa_builtin_mul(FaValue input) { return fa_binary_numeric(input, "mul"); }
static FaValue fa_builtin_div(FaValue input) { return fa_binary_numeric(input, "div"); }
static FaValue fa_builtin_rem(FaValue input) { return fa_binary_numeric(input, "rem"); }
static FaValue fa_builtin_min(FaValue input) { return fa_binary_numeric(input, "min"); }
static FaValue fa_builtin_max(FaValue input) { return fa_binary_numeric(input, "max"); }

static FaValue fa_unary_numeric(FaValue input, const char *op) {
  if (input.kind == FA_FAULT) return input;
  if (strcmp(op, "neg") == 0 && input.kind == FA_INT) {
    if (input.i == INT64_MIN) fa_die_usage("flowarrow runtime: neg overflow");
    return fa_int(-input.i);
  }
  if (strcmp(op, "abs") == 0 && input.kind == FA_INT) {
    if (input.i == INT64_MIN) fa_die_usage("flowarrow runtime: abs overflow");
    return fa_int(input.i < 0 ? -input.i : input.i);
  }
  double value = fa_expect_number(input, op);
  if (strcmp(op, "neg") == 0) return fa_real(-value);
  if (strcmp(op, "abs") == 0) return fa_real(fabs(value));
  if (strcmp(op, "sqrt") == 0) {
    if (value < 0.0) fa_die_usage("flowarrow runtime: sqrt of negative number");
    return fa_real(sqrt(value));
  }
  fa_die_usage("flowarrow runtime: unknown unary numeric op");
  return fa_unit();
}

static FaValue fa_builtin_neg(FaValue input) { return fa_unary_numeric(input, "neg"); }
static FaValue fa_builtin_abs(FaValue input) { return fa_unary_numeric(input, "abs"); }
static FaValue fa_builtin_sqrt(FaValue input) { return fa_unary_numeric(input, "sqrt"); }

static FaValue fa_compare_numeric(FaValue input, const char *op) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, op);
  FaValue fault = fa_propagate_fault_from_seq(seq);
  if (fault.kind == FA_FAULT) return fault;
  if (seq.seq.count != 2) {
    fprintf(stderr, "flowarrow runtime: %s expected two inputs\n", op);
    exit(65);
  }
  FaValue left = seq.seq.items[0];
  FaValue right = seq.seq.items[1];
  double l = fa_expect_number(left, op);
  double r = fa_expect_number(right, op);
  if (strcmp(op, "eq") == 0) return fa_bool(l == r);
  if (strcmp(op, "lt") == 0) return fa_bool(l < r);
  if (strcmp(op, "gt") == 0) return fa_bool(l > r);
  if (strcmp(op, "le") == 0) return fa_bool(l <= r);
  if (strcmp(op, "ge") == 0) return fa_bool(l >= r);
  fa_die_usage("flowarrow runtime: unknown comparison op");
  return fa_unit();
}

static FaValue fa_builtin_eq(FaValue input) { return fa_compare_numeric(input, "eq"); }
static FaValue fa_builtin_lt(FaValue input) { return fa_compare_numeric(input, "lt"); }
static FaValue fa_builtin_gt(FaValue input) { return fa_compare_numeric(input, "gt"); }
static FaValue fa_builtin_le(FaValue input) { return fa_compare_numeric(input, "le"); }
static FaValue fa_builtin_ge(FaValue input) { return fa_compare_numeric(input, "ge"); }

static FaValue fa_builtin_select(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "select");
  if (seq.seq.count != 3 || seq.seq.items[0].kind != FA_BOOL) {
    fa_die_usage("flowarrow runtime: select expected (Bool, T, T)");
  }
  return seq.seq.items[0].b ? seq.seq.items[1] : seq.seq.items[2];
}

static FaValue fa_builtin_not_empty(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  return fa_bool(fa_expect_bytes(input, "not_empty").len > 0);
}

static FaValue fa_builtin_is_empty(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  return fa_bool(fa_expect_bytes(input, "is_empty").len == 0);
}

static FaValue fa_bool_pair(FaValue input, const char *op) {
  FaValue seq = fa_expect_seq(input, op);
  if (seq.seq.count != 2 || seq.seq.items[0].kind != FA_BOOL || seq.seq.items[1].kind != FA_BOOL) {
    fprintf(stderr, "flowarrow runtime: %s expected (Bool, Bool)\n", op);
    exit(65);
  }
  if (strcmp(op, "and") == 0) return fa_bool(seq.seq.items[0].b && seq.seq.items[1].b);
  if (strcmp(op, "or") == 0) return fa_bool(seq.seq.items[0].b || seq.seq.items[1].b);
  return fa_bool(seq.seq.items[0].b != seq.seq.items[1].b);
}

static FaValue fa_builtin_and(FaValue input) { return fa_bool_pair(input, "and"); }
static FaValue fa_builtin_or(FaValue input) { return fa_bool_pair(input, "or"); }
static FaValue fa_builtin_xor(FaValue input) { return fa_bool_pair(input, "xor"); }

static FaValue fa_builtin_not(FaValue input) {
  if (input.kind != FA_BOOL) fa_die_usage("flowarrow runtime: not expected Bool");
  return fa_bool(!input.b);
}

static FaValue fa_builtin_all(FaValue input) {
  FaValue seq = fa_expect_seq(input, "all");
  for (size_t i = 0; i < seq.seq.count; i++) {
    if (seq.seq.items[i].kind != FA_BOOL) fa_die_usage("flowarrow runtime: all expected Seq[Bool]");
    if (!seq.seq.items[i].b) return fa_bool(false);
  }
  return fa_bool(true);
}

static FaValue fa_builtin_any(FaValue input) {
  FaValue seq = fa_expect_seq(input, "any");
  for (size_t i = 0; i < seq.seq.count; i++) {
    if (seq.seq.items[i].kind != FA_BOOL) fa_die_usage("flowarrow runtime: any expected Seq[Bool]");
    if (seq.seq.items[i].b) return fa_bool(true);
  }
  return fa_bool(false);
}

static FaValue fa_builtin_has_faults(FaValue input) {
  FaValue seq = fa_expect_seq(input, "has_faults");
  return fa_bool(seq.kind == FA_SEQ && seq.seq.count > 0);
}

static FaValue fa_builtin_format_faults(FaValue input) {
  FaValue seq = fa_expect_seq(input, "format_faults");
  size_t total = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    if (seq.seq.items[i].kind != FA_FAULT) fa_die_usage("flowarrow runtime: format_faults expected Fault items");
    total += seq.seq.items[i].len + 1;
  }
  char *bytes = (char *)malloc(total + 1);
  if (!bytes) fa_die_alloc();
  size_t offset = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue fault = seq.seq.items[i];
    memcpy(bytes + offset, fault.bytes, fault.len);
    offset += fault.len;
    bytes[offset++] = '\n';
  }
  bytes[offset] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaValue fa_builtin_range_step(FaValue input) {
  FaValue seq = fa_expect_seq(input, "range_step");
  if (seq.seq.count != 3) fa_die_usage("flowarrow runtime: range_step expected three Int inputs");
  int64_t start = fa_expect_int(seq.seq.items[0], "range_step");
  int64_t stop = fa_expect_int(seq.seq.items[1], "range_step");
  int64_t step = fa_expect_int(seq.seq.items[2], "range_step");
  if (step == 0) fa_die_usage("flowarrow runtime: range_step step cannot be zero");
  size_t count = 0;
  for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) count++;
  FaValue out = fa_seq_new(count);
  size_t index = 0;
  for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) {
    fa_seq_set(&out, index++, fa_int(i));
  }
  return out;
}
