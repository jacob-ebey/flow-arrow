static FaValue fa_builtin_length(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "length");
  return fa_int((int64_t)seq.seq.count);
}

static FaValue fa_builtin_group_by_id(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "group_by_id");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: group_by_id expected (Seq[T], Seq[Int])");
  FaValue values = fa_expect_seq(pair.seq.items[0], "group_by_id values");
  FaValue ids = fa_expect_seq(pair.seq.items[1], "group_by_id ids");
  if (values.seq.count != ids.seq.count) {
    return fa_fault_from_cstr("group_by_id: values and ids must have the same length");
  }
  if (values.seq.count == 0) return fa_seq_new(0);
  /* Count distinct group ids (must be non-decreasing). */
  size_t groups = 1;
  int64_t prev_id = fa_expect_int(ids.seq.items[0], "group_by_id ids");
  for (size_t i = 1; i < ids.seq.count; i++) {
    int64_t id = fa_expect_int(ids.seq.items[i], "group_by_id ids");
    if (id < prev_id) {
      return fa_fault_from_cstr("group_by_id: ids must be non-decreasing");
    }
    if (id != prev_id) groups++;
    prev_id = id;
  }
  FaValue out = fa_seq_new(groups);
  size_t group_index = 0;
  size_t run_start = 0;
  prev_id = fa_expect_int(ids.seq.items[0], "group_by_id ids");
  for (size_t i = 1; i <= ids.seq.count; i++) {
    int64_t id = i < ids.seq.count ? fa_expect_int(ids.seq.items[i], "group_by_id ids") : prev_id + 1;
    if (id != prev_id) {
      size_t len = i - run_start;
      FaValue group = fa_seq_new(len);
      for (size_t j = 0; j < len; j++) {
        fa_seq_set(&group, j, values.seq.items[run_start + j]);
      }
      fa_seq_set(&out, group_index++, group);
      run_start = i;
      prev_id = id;
    }
  }
  return out;
}

static FaValue fa_builtin_zip(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "zip");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: zip expected (Seq[A], Seq[B])");
  FaValue a = fa_expect_seq(pair.seq.items[0], "zip");
  FaValue b = fa_expect_seq(pair.seq.items[1], "zip");
  if (a.seq.count != b.seq.count) {
    return fa_fault_from_cstr("zip: sequences must have the same length");
  }
  FaValue out = fa_seq_new(a.seq.count);
  for (size_t i = 0; i < a.seq.count; i++) {
    FaValue tup = fa_seq_new(2);
    fa_seq_set(&tup, 0, a.seq.items[i]);
    fa_seq_set(&tup, 1, b.seq.items[i]);
    fa_seq_set(&out, i, tup);
  }
  return out;
}

static FaValue fa_builtin_shift_right(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "shift_right");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: shift_right expected (Seq[V], V)");
  FaValue seq = fa_expect_seq(pair.seq.items[0], "shift_right");
  FaValue fill = pair.seq.items[1];
  FaValue out = fa_seq_new(seq.seq.count);
  if (seq.seq.count > 0) {
    fa_seq_set(&out, 0, fill);
    for (size_t i = 1; i < seq.seq.count; i++) {
      fa_seq_set(&out, i, seq.seq.items[i - 1]);
    }
  }
  return out;
}

static FaValue fa_builtin_head(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "head");
  if (seq.seq.count == 0) return fa_fault_from_cstr("head: empty sequence");
  return seq.seq.items[0];
}
