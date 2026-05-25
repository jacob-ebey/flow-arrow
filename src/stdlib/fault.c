#include "runtime.h"

static FaFault fa_fault_with_line(size_t line, FaFault fault) {
  char prefix[64];
  int prefix_len = snprintf(prefix, sizeof(prefix), "line %zu: ", line);
  FaBytes prefix_bytes = fa_bytes_literal(prefix, (size_t)prefix_len);
  return fa_fault_bytes(fa_concat_raw(prefix_bytes, fault.message));
}

static FaBytes fa_format_faults(FaSeq_Fault faults) {
  size_t total = 0;
  for (size_t i = 0; i < faults.count; i++) {
    total = fa_checked_size_add(total, faults.items[i].message.len, "format_faults: byte length overflow");
    total = fa_checked_size_add(total, 1, "format_faults: byte length overflow");
  }
  char *bytes = (char *)fa_malloc(fa_checked_size_add(total, 1, "format_faults: byte length overflow"));
  size_t offset = 0;
  for (size_t i = 0; i < faults.count; i++) {
    memcpy(bytes + offset, faults.items[i].message.bytes, faults.items[i].message.len);
    offset += faults.items[i].message.len;
    bytes[offset++] = '\n';
  }
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}
