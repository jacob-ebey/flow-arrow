#include "runtime.h"

static FaSeq_Bytes fa_split_lines(FaBytes input) {
  size_t count = input.len == 0 ? 0 : 1;
  for (size_t i = 0; i < input.len; i++) {
    if (input.bytes[i] == '\n') count = fa_checked_size_add(count, 1, "split_lines: line count overflow");
  }
  if (input.len > 0 && input.bytes[input.len - 1] == '\n') count--;
  FaSeq_Bytes out = FaSeq_Bytes_new(count);
  size_t start = 0;
  size_t index = 0;
  for (size_t i = 0; i < input.len; i++) {
    if (input.bytes[i] == '\n') {
      size_t end = i;
      if (end > start && input.bytes[end - 1] == '\r') end--;
      out.items[index++] = fa_bytes_literal(input.bytes + start, end - start);
      start = fa_checked_size_add(i, 1, "split_lines: line index overflow");
    }
  }
  if (start < input.len) {
    size_t end = input.len;
    if (end > start && input.bytes[end - 1] == '\r') end--;
    out.items[index++] = fa_bytes_literal(input.bytes + start, end - start);
  }
  return out;
}

static FaBytes fa_trim(FaBytes input) {
  size_t start = 0;
  size_t end = input.len;
  while (start < end && isspace((unsigned char)input.bytes[start])) start++;
  while (end > start && isspace((unsigned char)input.bytes[end - 1])) end--;
  return fa_bytes_literal(input.bytes + start, end - start);
}

static int64_t fa_index_of(FaBytes input, FaBytes needle) {
  if (needle.len == 0) return 0;
  if (needle.len > input.len) return -1;
  size_t max_start = input.len - needle.len;
  for (size_t i = 0; i <= max_start; i++) {
    if (memcmp(input.bytes + i, needle.bytes, needle.len) == 0) return (int64_t)i;
  }
  return -1;
}

static int64_t fa_last_index_of(FaBytes input, FaBytes needle) {
  if (needle.len == 0) return fa_checked_size_to_i64(input.len, "last_index_of: input is too large");
  if (needle.len > input.len) return -1;
  size_t start = input.len - needle.len;
  for (;;) {
    if (memcmp(input.bytes + start, needle.bytes, needle.len) == 0) return (int64_t)start;
    if (start == 0) break;
    start--;
  }
  return -1;
}

static bool fa_bytes_contains(FaBytes input, FaBytes needle) {
  return fa_index_of(input, needle) >= 0;
}

static bool fa_bytes_starts_with(FaBytes input, FaBytes prefix) {
  return input.len >= prefix.len && memcmp(input.bytes, prefix.bytes, prefix.len) == 0;
}

static bool fa_bytes_ends_with(FaBytes input, FaBytes suffix) {
  return input.len >= suffix.len && memcmp(input.bytes + input.len - suffix.len, suffix.bytes, suffix.len) == 0;
}

static FaBytes fa_bytes_slice(FaBytes input, int64_t start, int64_t end) {
  if (start < 0 || end < start || (size_t)end > input.len) fa_die_usage("bytes.slice: index out of range");
  return fa_bytes_literal(input.bytes + start, (size_t)(end - start));
}

static FaBytes fa_bytes_take(FaBytes input, int64_t count) {
  if (count < 0) fa_die_usage("bytes.take: count must be non-negative");
  size_t len = (size_t)count > input.len ? input.len : (size_t)count;
  return fa_bytes_literal(input.bytes, len);
}

static FaBytes fa_bytes_drop(FaBytes input, int64_t count) {
  if (count < 0) fa_die_usage("bytes.drop: count must be non-negative");
  size_t offset = (size_t)count > input.len ? input.len : (size_t)count;
  return fa_bytes_literal(input.bytes + offset, input.len - offset);
}

static FaBytes fa_bytes_replace(FaBytes input, FaBytes needle, FaBytes replacement) {
  if (needle.len == 0) fa_die_usage("replace: needle cannot be empty");
  size_t matches = 0;
  if (needle.len <= input.len) {
    size_t max_start = input.len - needle.len;
    for (size_t i = 0; i <= max_start;) {
      if (memcmp(input.bytes + i, needle.bytes, needle.len) == 0) {
        matches = fa_checked_size_add(matches, 1, "replace: match count overflow");
        i += needle.len;
      } else {
        i++;
      }
    }
  }
  size_t removed = fa_checked_size_mul(matches, needle.len, "replace: byte length overflow");
  size_t added = fa_checked_size_mul(matches, replacement.len, "replace: byte length overflow");
  size_t total = fa_checked_size_add(input.len - removed, added, "replace: byte length overflow");
  char *bytes = (char *)fa_malloc(fa_checked_size_add(total, 1, "replace: byte length overflow"));
  size_t in = 0;
  size_t out = 0;
  size_t max_start = needle.len <= input.len ? input.len - needle.len : 0;
  while (in < input.len) {
    if (needle.len <= input.len && in <= max_start && memcmp(input.bytes + in, needle.bytes, needle.len) == 0) {
      memcpy(bytes + out, replacement.bytes, replacement.len);
      out += replacement.len;
      in += needle.len;
    } else {
      bytes[out++] = input.bytes[in++];
    }
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaBytes fa_bytes_repeat(FaBytes input, int64_t count) {
  if (count < 0) fa_die_usage("repeat: count must be non-negative");
  size_t total = fa_checked_size_mul(input.len, (size_t)count, "repeat: byte length overflow");
  char *bytes = (char *)fa_malloc(fa_checked_size_add(total, 1, "repeat: byte length overflow"));
  size_t offset = 0;
  for (int64_t i = 0; i < count; i++) {
    memcpy(bytes + offset, input.bytes, input.len);
    offset += input.len;
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaBytes fa_ascii_lower(FaBytes input) {
  char *bytes = (char *)fa_malloc(fa_checked_size_add(input.len, 1, "ascii_lower: byte length overflow"));
  for (size_t i = 0; i < input.len; i++) bytes[i] = (char)tolower((unsigned char)input.bytes[i]);
  bytes[input.len] = '\0';
  return fa_bytes_owned(bytes, input.len);
}

static FaBytes fa_ascii_upper(FaBytes input) {
  char *bytes = (char *)fa_malloc(fa_checked_size_add(input.len, 1, "ascii_upper: byte length overflow"));
  for (size_t i = 0; i < input.len; i++) bytes[i] = (char)toupper((unsigned char)input.bytes[i]);
  bytes[input.len] = '\0';
  return fa_bytes_owned(bytes, input.len);
}

static FaSeq_Bytes fa_split_on(FaBytes input, FaBytes delimiter) {
  if (delimiter.len == 0) fa_die_usage("split_on: delimiter cannot be empty");
  size_t count = 1;
  if (delimiter.len <= input.len) {
    size_t max_start = input.len - delimiter.len;
    for (size_t i = 0; i <= max_start;) {
      if (memcmp(input.bytes + i, delimiter.bytes, delimiter.len) == 0) {
        count = fa_checked_size_add(count, 1, "split_on: part count overflow");
        i += delimiter.len;
      } else {
        i++;
      }
    }
  }
  FaSeq_Bytes out = FaSeq_Bytes_new(count);
  size_t start = 0;
  size_t index = 0;
  if (delimiter.len <= input.len) {
    size_t max_start = input.len - delimiter.len;
    for (size_t i = 0; i <= max_start;) {
      if (memcmp(input.bytes + i, delimiter.bytes, delimiter.len) == 0) {
        out.items[index++] = fa_bytes_literal(input.bytes + start, i - start);
        i += delimiter.len;
        start = i;
      } else {
        i++;
      }
    }
  }
  out.items[index] = fa_bytes_literal(input.bytes + start, input.len - start);
  return out;
}

static FaFaultable_Bytes fa_strip_prefix(FaBytes input, FaBytes prefix) {
  if (input.len < prefix.len || memcmp(input.bytes, prefix.bytes, prefix.len) != 0) return FaFaultable_Bytes_fault(fa_fault_cstr("strip_prefix: prefix not present"));
  return FaFaultable_Bytes_ok(fa_bytes_literal(input.bytes + prefix.len, input.len - prefix.len));
}

static FaFaultable_Bytes fa_strip_suffix(FaBytes input, FaBytes suffix) {
  if (input.len < suffix.len || memcmp(input.bytes + input.len - suffix.len, suffix.bytes, suffix.len) != 0) return FaFaultable_Bytes_fault(fa_fault_cstr("strip_suffix: suffix not present"));
  return FaFaultable_Bytes_ok(fa_bytes_literal(input.bytes, input.len - suffix.len));
}

static FaSeq_Int fa_bytes_to_codes(FaBytes input) {
  FaSeq_Int out = FaSeq_Int_new(input.len);
  for (size_t i = 0; i < input.len; i++) out.items[i] = (unsigned char)input.bytes[i];
  return out;
}

static FaBytes fa_codes_to_bytes(FaSeq_Int codes) {
  char *bytes = (char *)fa_malloc(fa_checked_size_add(codes.count, 1, "codes_to_bytes: byte length overflow"));
  for (size_t i = 0; i < codes.count; i++) bytes[i] = (char)codes.items[i];
  bytes[codes.count] = '\0';
  return fa_bytes_owned(bytes, codes.count);
}

static FaBytes fa_join_bytes(FaSeq_Bytes values, FaBytes delimiter) {
  size_t total = 0;
  for (size_t i = 0; i < values.count; i++) {
    total = fa_checked_size_add(total, values.items[i].len, "join_bytes: byte length overflow");
  }
  if (values.count > 1) {
    total = fa_checked_size_add(
        total,
        fa_checked_size_mul(delimiter.len, values.count - 1, "join_bytes: byte length overflow"),
        "join_bytes: byte length overflow");
  }
  char *bytes = (char *)fa_malloc(fa_checked_size_add(total, 1, "join_bytes: byte length overflow"));
  size_t offset = 0;
  for (size_t i = 0; i < values.count; i++) {
    if (i > 0) {
      memcpy(bytes + offset, delimiter.bytes, delimiter.len);
      offset += delimiter.len;
    }
    memcpy(bytes + offset, values.items[i].bytes, values.items[i].len);
    offset += values.items[i].len;
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaBytes fa_reduce_concat_bytes(FaSeq_Bytes values, FaBytes identity) {
  size_t total = identity.len;
  for (size_t i = 0; i < values.count; i++) {
    total = fa_checked_size_add(total, values.items[i].len, "concat_bytes: byte length overflow");
  }
  char *bytes = (char *)fa_malloc(fa_checked_size_add(total, 1, "concat_bytes: byte length overflow"));
  size_t offset = 0;
  memcpy(bytes + offset, identity.bytes, identity.len);
  offset += identity.len;
  for (size_t i = 0; i < values.count; i++) {
    memcpy(bytes + offset, values.items[i].bytes, values.items[i].len);
    offset += values.items[i].len;
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaBytes fa_concat_bytes(FaSeq_Bytes values) {
  return fa_reduce_concat_bytes(values, fa_bytes_literal("", 0));
}
