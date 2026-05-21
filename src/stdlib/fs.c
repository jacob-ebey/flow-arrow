static FaFault fa_io_fault(FaBytes path, const char *operation) {
  FaBytes prefix = fa_bytes_literal(operation, strlen(operation));
  FaBytes middle = fa_bytes_literal(": ", 2);
  FaBytes reason = fa_bytes_literal(strerror(errno), strlen(strerror(errno)));
  return fa_fault_bytes(fa_concat_raw(fa_concat_raw(fa_concat_raw(prefix, path), middle), reason));
}

static FaFaultable_Bytes fa_read_file(FaBytes path) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Bytes_fault(fa_fault_cstr("read_file: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  FILE *file = fopen(path_c, "rb");
  free(path_c);
  if (!file) return FaFaultable_Bytes_fault(fa_io_fault(path, "read_file"));

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
    size_t n = fread(buf + len, 1, cap - len, file);
    len += n;
    if (n == 0) break;
  }
  if (ferror(file)) {
    FaFault fault = fa_io_fault(path, "read_file");
    fclose(file);
    free(buf);
    return FaFaultable_Bytes_fault(fault);
  }
  if (fclose(file) != 0) {
    FaFault fault = fa_io_fault(path, "read_file");
    free(buf);
    return FaFaultable_Bytes_fault(fault);
  }
  buf[len] = '\0';
  return FaFaultable_Bytes_ok(fa_bytes_owned(buf, len));
}

static FaFaultable_Int fa_write_file(FaBytes path, FaBytes contents) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Int_fault(fa_fault_cstr("write_file: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  FILE *file = fopen(path_c, "wb");
  free(path_c);
  if (!file) return FaFaultable_Int_fault(fa_io_fault(path, "write_file"));

  if (contents.len > 0) fwrite(contents.bytes, 1, contents.len, file);
  if (ferror(file)) {
    FaFault fault = fa_io_fault(path, "write_file");
    fclose(file);
    return FaFaultable_Int_fault(fault);
  }
  if (fclose(file) != 0) return FaFaultable_Int_fault(fa_io_fault(path, "write_file"));
  return FaFaultable_Int_ok(0);
}
