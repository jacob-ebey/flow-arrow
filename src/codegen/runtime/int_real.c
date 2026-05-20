static FaValue fa_builtin_format_int(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  char buf[64];
  snprintf(buf, sizeof(buf), "%lld", (long long)fa_expect_int(input, "format_int"));
  return fa_bytes_from_slice(buf, strlen(buf));
}

static bool fa_try_parse_int(FaValue input, int64_t *out) {
  FaValue bytes = fa_expect_bytes(input, "parse_int");
  char *copy = fa_copy_bytes(bytes.bytes, bytes.len);
  char *start = copy;
  while (isspace((unsigned char)*start)) start++;
  errno = 0;
  char *end = NULL;
  long long value = strtoll(start, &end, 10);
  while (end && isspace((unsigned char)*end)) end++;
  bool ok = !(start == end || errno == ERANGE || !end || *end != '\0');
  free(copy);
  if (ok) *out = (int64_t)value;
  return ok;
}

static FaValue fa_builtin_parse_int(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  int64_t value = 0;
  if (!fa_try_parse_int(input, &value)) {
    FaValue bytes = fa_expect_bytes(input, "parse_int");
    char message[512];
    snprintf(message, sizeof(message), "expected Int, got \"%.*s\"", (int)bytes.len, bytes.bytes);
    return fa_fault_from_cstr(message);
  }
  return fa_int(value);
}

static bool fa_try_parse_real(FaValue input, double *out) {
  FaValue bytes = fa_expect_bytes(input, "parse_real");
  char *copy = fa_copy_bytes(bytes.bytes, bytes.len);
  char *start = copy;
  while (isspace((unsigned char)*start)) start++;
  errno = 0;
  char *end = NULL;
  double value = strtod(start, &end);
  while (end && isspace((unsigned char)*end)) end++;
  bool ok = !(start == end || errno == ERANGE || !end || *end != '\0');
  free(copy);
  if (ok) *out = value;
  return ok;
}

static FaValue fa_builtin_parse_real(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  double value = 0.0;
  if (!fa_try_parse_real(input, &value)) {
    FaValue bytes = fa_expect_bytes(input, "parse_real");
    char message[512];
    snprintf(message, sizeof(message), "expected Real, got \"%.*s\"", (int)bytes.len, bytes.bytes);
    return fa_fault_from_cstr(message);
  }
  return fa_real(value);
}

static FaValue fa_builtin_format_real(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  char buf[64];
  snprintf(buf, sizeof(buf), "%.15g", fa_expect_real(input, "format_real"));
  return fa_bytes_from_slice(buf, strlen(buf));
}

