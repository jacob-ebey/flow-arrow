#include "runtime.h"

static FaSeq_Bytes fa_split_lines(FaBytes input) {
  size_t count = input.len == 0 ? 0 : 1;
  for (size_t i = 0; i < input.len; i++) if (input.bytes[i] == '\n') count++;
  if (input.len > 0 && input.bytes[input.len - 1] == '\n') count--;
  FaSeq_Bytes out = FaSeq_Bytes_new(count);
  size_t start = 0;
  size_t index = 0;
  for (size_t i = 0; i <= input.len; i++) {
    if (i == input.len || input.bytes[i] == '\n') {
      size_t end = i;
      if (end > start && input.bytes[end - 1] == '\r') end--;
      if (i > start || i < input.len) out.items[index++] = fa_bytes_literal(input.bytes + start, end - start);
      start = i + 1;
    }
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

static FaSeq_Bytes fa_split_on(FaBytes input, FaBytes delimiter) {
  if (delimiter.len == 0) fa_die_usage("split_on: delimiter cannot be empty");
  size_t count = 1;
  for (size_t i = 0; i + delimiter.len <= input.len;) {
    if (memcmp(input.bytes + i, delimiter.bytes, delimiter.len) == 0) {
      count++;
      i += delimiter.len;
    } else {
      i++;
    }
  }
  FaSeq_Bytes out = FaSeq_Bytes_new(count);
  size_t start = 0;
  size_t index = 0;
  for (size_t i = 0; i + delimiter.len <= input.len;) {
    if (memcmp(input.bytes + i, delimiter.bytes, delimiter.len) == 0) {
      out.items[index++] = fa_bytes_literal(input.bytes + start, i - start);
      i += delimiter.len;
      start = i;
    } else {
      i++;
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
  char *bytes = (char *)malloc(codes.count + 1);
  if (!bytes) fa_die_alloc();
  for (size_t i = 0; i < codes.count; i++) bytes[i] = (char)codes.items[i];
  bytes[codes.count] = '\0';
  return fa_bytes_owned(bytes, codes.count);
}

static FaBytes fa_join_bytes(FaSeq_Bytes values, FaBytes delimiter) {
  size_t total = 0;
  for (size_t i = 0; i < values.count; i++) total += values.items[i].len;
  if (values.count > 1) total += delimiter.len * (values.count - 1);
  char *bytes = (char *)malloc(total + 1);
  if (!bytes) fa_die_alloc();
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
  for (size_t i = 0; i < values.count; i++) total += values.items[i].len;
  char *bytes = (char *)malloc(total + 1);
  if (!bytes) fa_die_alloc();
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
