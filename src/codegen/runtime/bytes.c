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

static bool fa_is_ascii_whitespace(unsigned char c) {
  return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f';
}

static FaValue fa_builtin_trim(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "trim");
  size_t start = 0;
  size_t end = bytes.len;
  while (start < end && fa_is_ascii_whitespace((unsigned char)bytes.bytes[start])) start++;
  while (end > start && fa_is_ascii_whitespace((unsigned char)bytes.bytes[end - 1])) end--;
  return fa_bytes_from_slice(bytes.bytes + start, end - start);
}

static FaValue fa_builtin_split_on(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "split_on");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: split_on expected (Bytes, Bytes)");
  FaValue source = fa_expect_bytes(pair.seq.items[0], "split_on");
  if (source.kind == FA_FAULT) return source;
  FaValue sep = fa_expect_bytes(pair.seq.items[1], "split_on");
  if (sep.kind == FA_FAULT) return sep;
  if (sep.len == 0) fa_die_usage("flowarrow runtime: split_on separator must be non-empty");
  size_t count = 1;
  if (source.len >= sep.len) {
    for (size_t i = 0; i + sep.len <= source.len; ) {
      if (memcmp(source.bytes + i, sep.bytes, sep.len) == 0) {
        count++;
        i += sep.len;
      } else {
        i++;
      }
    }
  }
  FaValue out = fa_seq_new(count);
  size_t index = 0;
  size_t start = 0;
  if (source.len >= sep.len) {
    for (size_t i = 0; i + sep.len <= source.len; ) {
      if (memcmp(source.bytes + i, sep.bytes, sep.len) == 0) {
        fa_seq_set(&out, index++, fa_bytes_from_slice(source.bytes + start, i - start));
        i += sep.len;
        start = i;
      } else {
        i++;
      }
    }
  }
  fa_seq_set(&out, index++, fa_bytes_from_slice(source.bytes + start, source.len - start));
  return out;
}

static FaValue fa_builtin_strip_prefix(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "strip_prefix");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: strip_prefix expected (Bytes, Bytes)");
  FaValue source = fa_expect_bytes(pair.seq.items[0], "strip_prefix");
  if (source.kind == FA_FAULT) return source;
  FaValue prefix = fa_expect_bytes(pair.seq.items[1], "strip_prefix");
  if (prefix.kind == FA_FAULT) return prefix;
  if (source.len < prefix.len ||
      memcmp(source.bytes, prefix.bytes, prefix.len) != 0) {
    return fa_fault_from_cstr("strip_prefix: input does not start with the expected prefix");
  }
  return fa_bytes_from_slice(source.bytes + prefix.len, source.len - prefix.len);
}

static FaValue fa_builtin_strip_suffix(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue pair = fa_expect_seq(input, "strip_suffix");
  if (pair.seq.count != 2) fa_die_usage("flowarrow runtime: strip_suffix expected (Bytes, Bytes)");
  FaValue source = fa_expect_bytes(pair.seq.items[0], "strip_suffix");
  if (source.kind == FA_FAULT) return source;
  FaValue suffix = fa_expect_bytes(pair.seq.items[1], "strip_suffix");
  if (suffix.kind == FA_FAULT) return suffix;
  if (source.len < suffix.len ||
      memcmp(source.bytes + source.len - suffix.len, suffix.bytes, suffix.len) != 0) {
    return fa_fault_from_cstr("strip_suffix: input does not end with the expected suffix");
  }
  return fa_bytes_from_slice(source.bytes, source.len - suffix.len);
}



static FaValue fa_builtin_bytes_to_codes(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "bytes_to_codes");
  FaValue out = fa_seq_new(bytes.len);
  for (size_t i = 0; i < bytes.len; i++) {
    fa_seq_set(&out, i, fa_int((int64_t)(unsigned char)bytes.bytes[i]));
  }
  return out;
}

static FaValue fa_builtin_codes_to_bytes(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue seq = fa_expect_seq(input, "codes_to_bytes");
  FaValue fault = fa_propagate_fault_from_seq(seq);
  if (fault.kind == FA_FAULT) return fault;
  char *buf = (char *)malloc(seq.seq.count + 1);
  if (!buf) fa_die_alloc();
  for (size_t i = 0; i < seq.seq.count; i++) {
    int64_t code = fa_expect_int(seq.seq.items[i], "codes_to_bytes");
    if (code < 0 || code > 255) {
      free(buf);
      char message[128];
      snprintf(message, sizeof(message), "codes_to_bytes: byte code %lld out of range 0..255", (long long)code);
      return fa_fault_from_cstr(message);
    }
    buf[i] = (char)(unsigned char)code;
  }
  buf[seq.seq.count] = '\0';
  return fa_bytes_owned(buf, seq.seq.count);
}

static FaValue fa_builtin_byte_length(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "byte_length");
  return fa_int((int64_t)bytes.len);
}
