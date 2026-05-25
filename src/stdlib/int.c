#include "runtime.h"

static FaBytes fa_format_int(int64_t value) {
  char buf[64];
  int len = snprintf(buf, sizeof(buf), "%lld", (long long)value);
  return fa_bytes_literal(buf, (size_t)len);
}

static FaFaultable_Int fa_parse_int(FaBytes bytes) {
  char *copy = fa_copy_bytes(bytes.bytes, bytes.len);
  char *start = copy;
  while (isspace((unsigned char)*start)) start++;
  char *end = NULL;
  errno = 0;
  long long value = strtoll(start, &end, 10);
  while (end && isspace((unsigned char)*end)) end++;
  if (errno == ERANGE || end == start || !end || *end != '\0') {
    char message[512];
    snprintf(message, sizeof(message), "expected Int, got \"%.*s\"", fa_preview_len(bytes.len), bytes.bytes);
    fa_free(copy);
    return FaFaultable_Int_fault(fa_fault_cstr(message));
  }
  fa_free(copy);
  return FaFaultable_Int_ok((int64_t)value);
}
