static FaValue fa_builtin_read_stdin(FaValue input) {
  (void)input;
  size_t cap = 4096;
  size_t len = 0;
  char *buf = (char *)malloc(cap);
  if (!buf) fa_die_alloc();
  for (;;) {
    if (len == cap) {
      cap *= 2;
      char *next = (char *)realloc(buf, cap);
      if (!next) fa_die_alloc();
      buf = next;
    }
    size_t n = fread(buf + len, 1, cap - len, stdin);
    len += n;
    if (n == 0) {
      if (ferror(stdin)) {
        fputs("flowarrow runtime: failed to read stdin\n", stderr);
        exit(74);
      }
      break;
    }
  }
  char *shrunk = (char *)realloc(buf, len + 1);
  if (!shrunk) {
    free(buf);
    fa_die_alloc();
  }
  shrunk[len] = '\0';
  return fa_bytes_owned(shrunk, len);
}

static FaValue fa_builtin_write_stdout(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "write_stdout");
  size_t written = fwrite(bytes.bytes, 1, bytes.len, stdout);
  return fa_int(written == bytes.len ? 0 : 1);
}

static FaValue fa_builtin_write_stderr(FaValue input) {
  if (input.kind == FA_FAULT) return input;
  FaValue bytes = fa_expect_bytes(input, "write_stderr");
  size_t written = fwrite(bytes.bytes, 1, bytes.len, stderr);
  return fa_int(written == bytes.len ? 0 : 1);
}

