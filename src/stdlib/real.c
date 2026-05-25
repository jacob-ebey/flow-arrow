#include "runtime.h"

static FaBytes fa_format_real(double value) {
  char buf[128];
  int len = snprintf(buf, sizeof(buf), "%.15g", value);
  return fa_bytes_literal(buf, (size_t)len);
}

static FaFaultable_Real fa_parse_real(FaBytes bytes) {
  char *copy = fa_copy_bytes(bytes.bytes, bytes.len);
  char *start = copy;
  while (isspace((unsigned char)*start)) start++;
  char *end = NULL;
  errno = 0;
  double value = strtod(start, &end);
  while (end && isspace((unsigned char)*end)) end++;
  if (errno == ERANGE || end == start || !end || *end != '\0') {
    char message[512];
    snprintf(message, sizeof(message), "expected Real, got \"%.*s\"", fa_preview_len(bytes.len), bytes.bytes);
    fa_free(copy);
    return FaFaultable_Real_fault(fa_fault_cstr(message));
  }
  fa_free(copy);
  return FaFaultable_Real_ok(value);
}
