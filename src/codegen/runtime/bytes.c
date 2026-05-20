static FaValue fa_builtin_split_lines(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "split_lines");
  size_t count = 0;
  size_t start = 0;
  for (size_t i = 0; i < bytes.len; i++) {
    if (bytes.bytes[i] == '\n') {
      count++;
      start = i + 1;
    }
  }
  if (start < bytes.len) count++;
  FaValue out = fa_seq_new(count);
  size_t index = 0;
  start = 0;
  for (size_t i = 0; i < bytes.len; i++) {
    if (bytes.bytes[i] == '\n') {
      size_t end = i;
      if (end > start && bytes.bytes[end - 1] == '\r') end--;
      fa_seq_set(&out, index++, fa_bytes_from_slice(bytes.bytes + start, end - start));
      start = i + 1;
    }
  }
  if (start < bytes.len) {
    size_t end = bytes.len;
    if (end > start && bytes.bytes[end - 1] == '\r') end--;
    fa_seq_set(&out, index++, fa_bytes_from_slice(bytes.bytes + start, end - start));
  }
  return out;
}

static FaValue fa_builtin_concat_bytes(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "concat_bytes");
  FaValue fault = fa_propagate_fault_from_seq(seq);
  if (fault.kind == FA_FAULT) return fault;
  size_t total = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    total += fa_expect_bytes(seq.seq.items[i], "concat_bytes").len;
  }
  char *bytes = (char *)malloc(total + 1);
  if (!bytes) fa_die_alloc();
  size_t offset = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue part = seq.seq.items[i];
    memcpy(bytes + offset, part.bytes, part.len);
    offset += part.len;
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaValue fa_builtin_join_bytes(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "join_bytes");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: join_bytes expected (Seq[Bytes], Bytes)");
  FaValue seq = fa_expect_seq(pair.seq.items[0], "join_bytes");
  FaValue sep = fa_expect_bytes(pair.seq.items[1], "join_bytes");
  if (seq.seq.count == 0) return fa_bytes_from_slice("", 0);
  size_t total = sep.len * (seq.seq.count - 1);
  for (size_t i = 0; i < seq.seq.count; i++) {
    total += fa_expect_bytes(seq.seq.items[i], "join_bytes").len;
  }
  char *bytes = (char *)malloc(total + 1);
  if (!bytes) fa_die_alloc();
  size_t offset = 0;
  for (size_t i = 0; i < seq.seq.count; i++) {
    FaValue part = seq.seq.items[i];
    memcpy(bytes + offset, part.bytes, part.len);
    offset += part.len;
    if (i + 1 < seq.seq.count) {
      memcpy(bytes + offset, sep.bytes, sep.len);
      offset += sep.len;
    }
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

