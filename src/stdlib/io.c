#include "runtime.h"

static FaBytes fa_read_stdin(void) {
  size_t cap = 4096;
  size_t len = 0;
  char *buf = (char *)malloc(cap + 1);
  if (!buf) fa_die_alloc();
  for (;;) {
    if (len == cap) {
      cap *= 2;
      char *next = (char *)realloc(buf, cap + 1);
      if (!next) fa_die_alloc();
      buf = next;
    }
    size_t n = fread(buf + len, 1, cap - len, stdin);
    len += n;
    if (n == 0) break;
  }
  buf[len] = '\0';
  return fa_bytes_owned(buf, len);
}

static int64_t fa_write_bytes(FILE *file, FaBytes bytes) {
  if (bytes.len > 0) fwrite(bytes.bytes, 1, bytes.len, file);
  return ferror(file) ? 1 : 0;
}
