#include "runtime.h"

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

#define FA_STREAM_BUFFER_SIZE (1024 * 1024)

static FaFaultable_Stream_Bytes fa_open_file(FaBytes path) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Stream_Bytes_fault(fa_fault_cstr("open_file: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  FILE *file = fopen(path_c, "rb");
  free(path_c);
  if (!file) return FaFaultable_Stream_Bytes_fault(fa_io_fault(path, "open_file"));

  FaStream stream;
  stream.file = file;
  stream.fd = fileno(file);
  stream.path = path;
  stream.state = NULL;
  stream.map_fn = NULL;
  stream.next = NULL;
  stream.close = NULL;
  stream.item_size = 0;
  stream.closed = false;
  return FaFaultable_Stream_Bytes_ok(stream);
}

static FaFaultable_Int fa_stream_size(FaStream stream) {
  if (!stream.file) return FaFaultable_Int_fault(fa_fault_cstr("size: stream is closed"));
  struct stat st;
  if (fstat(stream.fd, &st) != 0) return FaFaultable_Int_fault(fa_io_fault(stream.path, "size"));
  return FaFaultable_Int_ok((int64_t)st.st_size);
}

static FaFaultable_Bytes fa_stream_read_at(FaStream stream, int64_t offset, int64_t len) {
  if (!stream.file) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: stream is closed"));
  if (offset < 0) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: offset must be non-negative"));
  if (len < 0) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: length must be non-negative"));

  char *buffer = (char *)malloc((size_t)len + 1);
  if (!buffer) fa_die_alloc();

  size_t done = 0;
  while (done < (size_t)len) {
    ssize_t read = pread(stream.fd, buffer + done, (size_t)len - done, (off_t)offset + (off_t)done);
    if (read < 0) {
      FaFault fault = fa_io_fault(stream.path, "read_at");
      free(buffer);
      return FaFaultable_Bytes_fault(fault);
    }
    if (read == 0) {
      free(buffer);
      return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: requested range extends past end of stream"));
    }
    done += (size_t)read;
  }
  buffer[len] = '\0';
  return FaFaultable_Bytes_ok(fa_bytes_owned(buffer, (size_t)len));
}

static FaFaultable_Int fa_copy_stream_to_file(FaStream stream, FaBytes output_path) {
  if (!stream.file) return FaFaultable_Int_fault(fa_fault_cstr("copy_to_file: stream is closed"));
  if (memchr(output_path.bytes, '\0', output_path.len)) {
    return FaFaultable_Int_fault(fa_fault_cstr("copy_to_file: output path contains NUL byte"));
  }

  char *path_c = fa_copy_bytes(output_path.bytes, output_path.len);
  FILE *output = fopen(path_c, "wb");
  free(path_c);
  if (!output) return FaFaultable_Int_fault(fa_io_fault(output_path, "copy_to_file"));

  char *buffer = (char *)malloc(FA_STREAM_BUFFER_SIZE);
  if (!buffer) fa_die_alloc();
  for (;;) {
    size_t read = fread(buffer, 1, FA_STREAM_BUFFER_SIZE, stream.file);
    if (read > 0) {
      size_t written = fwrite(buffer, 1, read, output);
      if (written != read) {
        FaFault fault = fa_io_fault(output_path, "copy_to_file");
        free(buffer);
        fclose(output);
        return FaFaultable_Int_fault(fault);
      }
    }
    if (read < FA_STREAM_BUFFER_SIZE) {
      if (ferror(stream.file)) {
        FaFault fault = fa_io_fault(stream.path, "copy_to_file");
        free(buffer);
        fclose(output);
        return FaFaultable_Int_fault(fault);
      }
      break;
    }
  }
  free(buffer);
  if (fclose(output) != 0) {
    return FaFaultable_Int_fault(fa_io_fault(output_path, "copy_to_file"));
  }
  return FaFaultable_Int_ok(0);
}

static FaFaultable_Int fa_close_stream(FaStream stream) {
  FaFault fault;
  if (stream.close) {
    if (fa_stream_close(&stream, &fault) != 0) return FaFaultable_Int_fault(fault);
    return FaFaultable_Int_ok(0);
  }
  if (!stream.file) return FaFaultable_Int_fault(fa_fault_cstr("close: stream is already closed"));
  if (fclose(stream.file) != 0) {
    return FaFaultable_Int_fault(fa_io_fault(stream.path, "close"));
  }
  return FaFaultable_Int_ok(0);
}
