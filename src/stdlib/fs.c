#include "runtime.h"
#include <glob.h>

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
  fa_free(path_c);
  if (!file) return FaFaultable_Bytes_fault(fa_io_fault(path, "read_file"));

  size_t cap = 4096;
  size_t len = 0;
  char *buf = (char *)malloc(cap + 1);
  if (!buf) fa_die_alloc();
  for (;;) {
    if (len == cap) {
      cap = fa_checked_size_mul(cap, 2, "read_file: file too large");
      char *next = (char *)realloc(buf, fa_checked_size_add(cap, 1, "read_file: file too large"));
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
  fa_free(path_c);
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

static bool fa_path_stat(FaBytes path, struct stat *st) {
  if (memchr(path.bytes, '\0', path.len)) return false;
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  int result = stat(path_c, st);
  fa_free(path_c);
  return result == 0;
}

static bool fa_path_exists(FaBytes path) {
  struct stat st;
  return fa_path_stat(path, &st);
}

static bool fa_path_is_file(FaBytes path) {
  struct stat st;
  return fa_path_stat(path, &st) && S_ISREG(st.st_mode);
}

static bool fa_path_is_dir(FaBytes path) {
  struct stat st;
  return fa_path_stat(path, &st) && S_ISDIR(st.st_mode);
}

static FaFaultable_Int fa_path_file_size(FaBytes path) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Int_fault(fa_fault_cstr("file_size: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  struct stat st;
  int result = stat(path_c, &st);
  fa_free(path_c);
  if (result != 0) return FaFaultable_Int_fault(fa_io_fault(path, "file_size"));
  if (!S_ISREG(st.st_mode)) return FaFaultable_Int_fault(fa_fault_cstr("file_size: path is not a regular file"));
  return FaFaultable_Int_ok((int64_t)st.st_size);
}

static int fa_compare_bytes_items(const void *left, const void *right) {
  const FaBytes *a = (const FaBytes *)left;
  const FaBytes *b = (const FaBytes *)right;
  size_t common = a->len < b->len ? a->len : b->len;
  int cmp = common == 0 ? 0 : memcmp(a->bytes, b->bytes, common);
  if (cmp != 0) return cmp;
  if (a->len < b->len) return -1;
  if (a->len > b->len) return 1;
  return 0;
}

static FaBytes fa_join_path(FaBytes base, FaBytes child) {
  if (base.len == 0) return fa_bytes_literal(child.bytes, child.len);
  if (child.len == 0) return fa_bytes_literal(base.bytes, base.len);
  bool needs_sep = base.bytes[base.len - 1] != '/';
  size_t total = fa_checked_size_add(base.len, needs_sep ? 1 : 0, "join_path: path length overflow");
  total = fa_checked_size_add(total, child.len, "join_path: path length overflow");
  char *bytes = (char *)malloc(fa_checked_size_add(total, 1, "join_path: path length overflow"));
  if (!bytes) fa_die_alloc();
  size_t offset = 0;
  memcpy(bytes + offset, base.bytes, base.len);
  offset += base.len;
  if (needs_sep) bytes[offset++] = '/';
  memcpy(bytes + offset, child.bytes, child.len);
  bytes[total] = '\0';
  return fa_bytes_owned(bytes, total);
}

static FaBytes fa_basename(FaBytes path) {
  size_t end = path.len;
  while (end > 1 && path.bytes[end - 1] == '/') end--;
  size_t start = end;
  while (start > 0 && path.bytes[start - 1] != '/') start--;
  return fa_bytes_literal(path.bytes + start, end - start);
}

static FaBytes fa_dirname(FaBytes path) {
  size_t end = path.len;
  while (end > 1 && path.bytes[end - 1] == '/') end--;
  while (end > 0 && path.bytes[end - 1] != '/') end--;
  if (end == 0) return fa_bytes_literal(".", 1);
  while (end > 1 && path.bytes[end - 1] == '/') end--;
  return fa_bytes_literal(path.bytes, end);
}

typedef struct {
  FaBytes *items;
  size_t count;
  size_t cap;
} FaBytesVec;

static void fa_bytes_vec_push(FaBytesVec *vec, FaBytes value) {
  if (vec->count == vec->cap) {
    vec->cap = vec->cap == 0 ? 16 : fa_checked_size_mul(vec->cap, 2, "path list size overflow");
    size_t bytes = fa_checked_size_mul(vec->cap, sizeof(FaBytes), "path list size overflow");
    FaBytes *items = (FaBytes *)realloc(vec->items, bytes);
    if (!items) fa_die_alloc();
    vec->items = items;
  }
  vec->items[vec->count++] = value;
}

static FaSeq_Bytes fa_bytes_vec_finish(FaBytesVec *vec) {
  qsort(vec->items, vec->count, sizeof(FaBytes), fa_compare_bytes_items);
  FaSeq_Bytes out = FaSeq_Bytes_new(vec->count);
  for (size_t i = 0; i < vec->count; i++) out.items[i] = vec->items[i];
  free(vec->items);
  vec->items = NULL;
  vec->count = 0;
  vec->cap = 0;
  return out;
}

static FaFaultable_Seq_Bytes fa_list_dir(FaBytes path) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Seq_Bytes_fault(fa_fault_cstr("list_dir: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  DIR *dir = opendir(path_c);
  fa_free(path_c);
  if (!dir) return FaFaultable_Seq_Bytes_fault(fa_io_fault(path, "list_dir"));

  FaBytesVec entries = {0};
  errno = 0;
  for (;;) {
    struct dirent *entry = readdir(dir);
    if (!entry) break;
    if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
    fa_bytes_vec_push(&entries, fa_bytes_literal(entry->d_name, strlen(entry->d_name)));
  }
  if (errno != 0) {
    FaFault fault = fa_io_fault(path, "list_dir");
    closedir(dir);
    free(entries.items);
    return FaFaultable_Seq_Bytes_fault(fault);
  }
  if (closedir(dir) != 0) {
    FaFault fault = fa_io_fault(path, "list_dir");
    free(entries.items);
    return FaFaultable_Seq_Bytes_fault(fault);
  }
  return FaFaultable_Seq_Bytes_ok(fa_bytes_vec_finish(&entries));
}

static bool fa_is_dot_dir_name(const char *name) {
  return strcmp(name, ".") == 0 || strcmp(name, "..") == 0;
}

static int fa_walk_files_into(FaBytes path, FaBytesVec *files, FaFault *fault) {
  if (memchr(path.bytes, '\0', path.len)) {
    *fault = fa_fault_cstr("walk_files: path contains NUL byte");
    return -1;
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  struct stat st;
  if (lstat(path_c, &st) != 0) {
    fa_free(path_c);
    *fault = fa_io_fault(path, "walk_files");
    return -1;
  }
  if (S_ISREG(st.st_mode)) {
    fa_free(path_c);
    fa_bytes_vec_push(files, fa_bytes_literal(path.bytes, path.len));
    return 0;
  }
  if (!S_ISDIR(st.st_mode)) {
    fa_free(path_c);
    return 0;
  }
  DIR *dir = opendir(path_c);
  fa_free(path_c);
  if (!dir) {
    *fault = fa_io_fault(path, "walk_files");
    return -1;
  }
  errno = 0;
  for (;;) {
    struct dirent *entry = readdir(dir);
    if (!entry) break;
    if (fa_is_dot_dir_name(entry->d_name)) continue;
    FaBytes child = fa_join_path(path, fa_bytes_literal(entry->d_name, strlen(entry->d_name)));
    if (fa_walk_files_into(child, files, fault) != 0) {
      closedir(dir);
      return -1;
    }
  }
  if (errno != 0) {
    *fault = fa_io_fault(path, "walk_files");
    closedir(dir);
    return -1;
  }
  if (closedir(dir) != 0) {
    *fault = fa_io_fault(path, "walk_files");
    return -1;
  }
  return 0;
}

static bool fa_path_has_glob(FaBytes path) {
  for (size_t i = 0; i < path.len; i++) {
    if (path.bytes[i] == '*' || path.bytes[i] == '?' || path.bytes[i] == '[') return true;
  }
  return false;
}

static FaFaultable_Seq_Bytes fa_walk_files(FaBytes path) {
  FaBytesVec files = {0};
  FaFault fault;
  if (fa_path_has_glob(path)) {
    if (memchr(path.bytes, '\0', path.len)) {
      return FaFaultable_Seq_Bytes_fault(fa_fault_cstr("walk_files: path contains NUL byte"));
    }
    char *pattern = fa_copy_bytes(path.bytes, path.len);
    glob_t matches;
    memset(&matches, 0, sizeof(matches));
    int result = glob(pattern, 0, NULL, &matches);
    fa_free(pattern);
    if (result != 0) {
      globfree(&matches);
      if (result == GLOB_NOMATCH) {
        return FaFaultable_Seq_Bytes_fault(fa_fault_cstr("walk_files: glob pattern matched no paths"));
      }
      return FaFaultable_Seq_Bytes_fault(fa_fault_cstr("walk_files: glob expansion failed"));
    }
    for (size_t i = 0; i < matches.gl_pathc; i++) {
      FaBytes match = fa_bytes_literal(matches.gl_pathv[i], strlen(matches.gl_pathv[i]));
      if (fa_walk_files_into(match, &files, &fault) != 0) {
        globfree(&matches);
        free(files.items);
        return FaFaultable_Seq_Bytes_fault(fault);
      }
    }
    globfree(&matches);
  } else {
    if (fa_walk_files_into(path, &files, &fault) != 0) {
      free(files.items);
      return FaFaultable_Seq_Bytes_fault(fault);
    }
  }
  return FaFaultable_Seq_Bytes_ok(fa_bytes_vec_finish(&files));
}

static FaFaultable_Seq_Tuple_Bytes_Bytes fa_read_files(FaSeq_Bytes paths) {
  FaSeq_Tuple_Bytes_Bytes out = FaSeq_Tuple_Bytes_Bytes_new(paths.count);
  for (size_t i = 0; i < paths.count; i++) {
    FaFaultable_Bytes contents = fa_read_file(paths.items[i]);
    if (contents.is_fault) return FaFaultable_Seq_Tuple_Bytes_Bytes_fault(contents.fault);
    out.items[i].f0 = paths.items[i];
    out.items[i].f1 = contents.value;
  }
  return FaFaultable_Seq_Tuple_Bytes_Bytes_ok(out);
}

#define FA_STREAM_BUFFER_SIZE (1024 * 1024)

typedef struct {
  FILE *file;
  int fd;
  FaBytes path;
  bool closed;
} FaFileStreamState;

static int fa_file_stream_next(void *state_ptr, void *out, FaFault *fault) {
  FaFileStreamState *state = (FaFileStreamState *)state_ptr;
  if (!state || state->closed || !state->file) return 0;

  char *buffer = (char *)malloc(FA_STREAM_BUFFER_SIZE + 1);
  if (!buffer) fa_die_alloc();
  size_t read = fread(buffer, 1, FA_STREAM_BUFFER_SIZE, state->file);
  if (read == 0) {
    free(buffer);
    if (ferror(state->file)) {
      *fault = fa_io_fault(state->path, "read_file");
      return -1;
    }
    return 0;
  }
  buffer[read] = '\0';
  *(FaBytes *)out = fa_bytes_owned(buffer, read);
  return 1;
}

static int fa_file_stream_close(void *state_ptr, FaFault *fault) {
  FaFileStreamState *state = (FaFileStreamState *)state_ptr;
  if (!state || state->closed) return 0;
  state->closed = true;
  int status = 0;
  if (state->file && fclose(state->file) != 0) {
    *fault = fa_io_fault(state->path, "close");
    status = -1;
  }
  state->file = NULL;
  free(state);
  return status;
}

static FaFaultable_Stream_Bytes fa_open_file(FaBytes path) {
  if (memchr(path.bytes, '\0', path.len)) {
    return FaFaultable_Stream_Bytes_fault(fa_fault_cstr("open_file: path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  FILE *file = fopen(path_c, "rb");
  fa_free(path_c);
  if (!file) return FaFaultable_Stream_Bytes_fault(fa_io_fault(path, "open_file"));

  FaFileStreamState *state = (FaFileStreamState *)calloc(1, sizeof(FaFileStreamState));
  if (!state) fa_die_alloc();
  state->file = file;
  state->fd = fileno(file);
  state->path = path;
  state->closed = false;

  FaStream stream;
  stream.file = file;
  stream.fd = state->fd;
  stream.path = path;
  stream.state = state;
  stream.map_fn = NULL;
  stream.next = fa_file_stream_next;
  stream.close = fa_file_stream_close;
  stream.item_size = sizeof(FaBytes);
  stream.closed = false;
  return FaFaultable_Stream_Bytes_ok(stream);
}

static FaFileStreamState *fa_file_stream_state(FaStream *stream) {
  return stream && stream->state ? (FaFileStreamState *)stream->state : NULL;
}

static FILE *fa_stream_file_handle(FaStream *stream) {
  if (stream->file) return stream->file;
  FaFileStreamState *state = fa_file_stream_state(stream);
  return state && !state->closed ? state->file : NULL;
}

static int fa_stream_fd_value(FaStream *stream) {
  if (stream->fd >= 0) return stream->fd;
  FaFileStreamState *state = fa_file_stream_state(stream);
  return state ? state->fd : -1;
}

static FaBytes fa_stream_path_value(FaStream *stream) {
  if (stream->path.bytes) return stream->path;
  FaFileStreamState *state = fa_file_stream_state(stream);
  return state ? state->path : fa_bytes_literal("", 0);
}

static FaFaultable_Int fa_stream_size(FaStream stream) {
  if (!fa_stream_file_handle(&stream)) return FaFaultable_Int_fault(fa_fault_cstr("size: stream is closed"));
  struct stat st;
  if (fstat(fa_stream_fd_value(&stream), &st) != 0) return FaFaultable_Int_fault(fa_io_fault(fa_stream_path_value(&stream), "size"));
  return FaFaultable_Int_ok((int64_t)st.st_size);
}

static FaFaultable_Bytes fa_stream_read_at(FaStream stream, int64_t offset, int64_t len) {
  if (!fa_stream_file_handle(&stream)) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: stream is closed"));
  if (offset < 0) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: offset must be non-negative"));
  if (len < 0) return FaFaultable_Bytes_fault(fa_fault_cstr("read_at: length must be non-negative"));

  char *buffer = (char *)malloc(fa_checked_size_add((size_t)len, 1, "read_at: length overflow"));
  if (!buffer) fa_die_alloc();

  size_t done = 0;
  while (done < (size_t)len) {
    ssize_t read = pread(fa_stream_fd_value(&stream), buffer + done, (size_t)len - done, (off_t)offset + (off_t)done);
    if (read < 0) {
      FaFault fault = fa_io_fault(fa_stream_path_value(&stream), "read_at");
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
  FILE *input = fa_stream_file_handle(&stream);
  if (!input) return FaFaultable_Int_fault(fa_fault_cstr("copy_to_file: stream is closed"));
  if (memchr(output_path.bytes, '\0', output_path.len)) {
    return FaFaultable_Int_fault(fa_fault_cstr("copy_to_file: output path contains NUL byte"));
  }

  char *path_c = fa_copy_bytes(output_path.bytes, output_path.len);
  FILE *output = fopen(path_c, "wb");
  fa_free(path_c);
  if (!output) return FaFaultable_Int_fault(fa_io_fault(output_path, "copy_to_file"));

  char *buffer = (char *)malloc(FA_STREAM_BUFFER_SIZE);
  if (!buffer) fa_die_alloc();
  for (;;) {
    size_t read = fread(buffer, 1, FA_STREAM_BUFFER_SIZE, input);
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
      if (ferror(input)) {
        FaFault fault = fa_io_fault(fa_stream_path_value(&stream), "copy_to_file");
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

static FaFaultable_Int fa_stream_size_ptr(FaStream *stream) {
  return fa_stream_size(*stream);
}

static FaFaultable_Bytes fa_stream_read_at_ptr(FaStream *stream, int64_t offset, int64_t len) {
  return fa_stream_read_at(*stream, offset, len);
}

static FaFaultable_Int fa_copy_stream_to_file_ptr(FaStream *stream, FaBytes output_path) {
  return fa_copy_stream_to_file(*stream, output_path);
}

static FaFaultable_Int fa_close_stream_ptr(FaStream *stream) {
  return fa_close_stream(*stream);
}
