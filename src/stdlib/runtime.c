#include "runtime.h"

static void fa_die_usage(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(65);
}

static void fa_die_alloc(void) {
  fputs("flowarrow runtime: allocation failed\n", stderr);
  exit(70);
}

static _Thread_local FaScopedAllocator fa_current_allocator = { NULL, NULL };

typedef struct {
  size_t size;
  bool scoped;
} FaAllocHeader;

static FaScopedAllocator fa_scoped_allocator_enter(FaScopedAllocFn alloc, void *ctx) {
  FaScopedAllocator previous = fa_current_allocator;
  fa_current_allocator.alloc = alloc;
  fa_current_allocator.ctx = ctx;
  return previous;
}

static void fa_scoped_allocator_restore(FaScopedAllocator previous) {
  fa_current_allocator = previous;
}

static void *fa_malloc(size_t size) {
  if (size == 0) size = 1;
  size_t total = sizeof(FaAllocHeader) + size;
  FaAllocHeader *header = fa_current_allocator.alloc
      ? (FaAllocHeader *)fa_current_allocator.alloc(fa_current_allocator.ctx, total)
      : (FaAllocHeader *)malloc(total);
  if (!header) fa_die_alloc();
  header->size = size;
  header->scoped = fa_current_allocator.alloc != NULL;
  return (void *)(header + 1);
}

static void *fa_calloc(size_t count, size_t size) {
  size_t total = (count ? count : 1) * size;
  void *ptr = fa_malloc(total);
  memset(ptr, 0, total);
  return ptr;
}

static void *fa_realloc(void *ptr, size_t size) {
  if (!ptr) return fa_malloc(size);
  if (size == 0) size = 1;
  FaAllocHeader *header = ((FaAllocHeader *)ptr) - 1;
  if (header->scoped || fa_current_allocator.alloc) {
    void *next = fa_malloc(size);
    memcpy(next, ptr, header->size < size ? header->size : size);
    return next;
  }
  FaAllocHeader *next = (FaAllocHeader *)realloc(header, sizeof(FaAllocHeader) + size);
  if (!next) fa_die_alloc();
  next->size = size;
  next->scoped = false;
  return (void *)(next + 1);
}

static void fa_free(void *ptr) {
  if (!ptr) return;
  FaAllocHeader *header = ((FaAllocHeader *)ptr) - 1;
  if (!header->scoped) free(header);
}

typedef struct {
  FaParallelForFn fn;
  void *ctx;
  size_t next;
  size_t end;
  size_t grain;
  pthread_mutex_t lock;
} FaParallelForState;

static _Thread_local int fa_parallel_depth = 0;

static size_t fa_parallel_worker_count(void) {
  const char *env = getenv("FLOWARROW_THREADS");
  if (env && *env) {
    char *end = NULL;
    errno = 0;
    long value = strtol(env, &end, 10);
    if (errno == 0 && end != env && value > 0) {
      return value > FA_PARALLEL_FOR_MAX_WORKERS ? FA_PARALLEL_FOR_MAX_WORKERS : (size_t)value;
    }
  }
  long cpus = sysconf(_SC_NPROCESSORS_ONLN);
  if (cpus < 1) cpus = 1;
  return cpus > FA_PARALLEL_FOR_MAX_WORKERS ? FA_PARALLEL_FOR_MAX_WORKERS : (size_t)cpus;
}

static void *fa_parallel_for_worker(void *arg) {
  FaParallelForState *state = (FaParallelForState *)arg;
  fa_parallel_depth++;
  for (;;) {
    size_t start;
    size_t end;
    pthread_mutex_lock(&state->lock);
    start = state->next;
    if (start >= state->end) {
      pthread_mutex_unlock(&state->lock);
      break;
    }
    end = start + state->grain;
    if (end > state->end) end = state->end;
    state->next = end;
    pthread_mutex_unlock(&state->lock);
    state->fn(state->ctx, start, end);
  }
  fa_parallel_depth--;
  return NULL;
}

static void fa_parallel_for(size_t start, size_t end, size_t grain, FaParallelForFn fn, void *ctx) {
  if (end <= start) return;
  if (grain == 0) grain = FA_PARALLEL_FOR_GRAIN;
  size_t count = end - start;
  size_t workers = fa_parallel_worker_count();
  size_t chunks = (count + grain - 1) / grain;
  if (fa_parallel_depth > 0 || workers <= 1 || chunks <= 1) {
    fn(ctx, start, end);
    return;
  }
  if (workers > chunks) workers = chunks;

  FaParallelForState state;
  state.fn = fn;
  state.ctx = ctx;
  state.next = start;
  state.end = end;
  state.grain = grain;
  if (pthread_mutex_init(&state.lock, NULL) != 0) {
    fn(ctx, start, end);
    return;
  }

  pthread_t threads[FA_PARALLEL_FOR_MAX_WORKERS];
  size_t spawned = 0;
  for (; spawned + 1 < workers; spawned++) {
    if (pthread_create(&threads[spawned], NULL, fa_parallel_for_worker, &state) != 0) break;
  }
  fa_parallel_for_worker(&state);
  for (size_t i = 0; i < spawned; i++) pthread_join(threads[i], NULL);
  pthread_mutex_destroy(&state.lock);
}

static FaUnit fa_unit(void) {
  FaUnit unit;
  unit._unused = 0;
  return unit;
}

static char *fa_copy_bytes(const char *bytes, size_t len) {
  char *copy = (char *)fa_malloc(len + 1);
  memcpy(copy, bytes, len);
  copy[len] = '\0';
  return copy;
}

static FaBytes fa_bytes_borrowed(const char *bytes, size_t len) {
  FaBytes out;
  out.bytes = (char *)bytes;
  out.len = len;
  return out;
}

static FaBytes fa_bytes_owned(char *bytes, size_t len) {
  FaBytes out;
  out.bytes = bytes;
  out.len = len;
  return out;
}

static FaBytes fa_bytes_literal(const char *bytes, size_t len) {
  return fa_bytes_owned(fa_copy_bytes(bytes, len), len);
}

static FaFault fa_fault_bytes(FaBytes message) {
  FaFault fault;
  fault.message = message;
  return fault;
}

static FaFault fa_fault_cstr(const char *message) {
  return fa_fault_bytes(fa_bytes_literal(message, strlen(message)));
}

static void fa_exit_fault(FaFault fault) {
  fprintf(stderr, "%.*s\n", (int)fault.message.len, fault.message.bytes);
  exit(65);
}

static int fa_stream_close(FaStream *stream, FaFault *fault) {
  if (stream->closed) return 0;
  stream->closed = true;
  if (stream->close) return stream->close(stream->state, fault);
  return 0;
}

static FaSeq_Bytes FaSeq_Bytes_new(size_t count) {
  FaSeq_Bytes seq;
  seq.count = count;
  seq.items = (FaBytes *)fa_calloc(count ? count : 1, sizeof(FaBytes));
  return seq;
}

static FaSeq_Tuple_Bytes_Bytes FaSeq_Tuple_Bytes_Bytes_new(size_t count) {
  FaSeq_Tuple_Bytes_Bytes seq;
  seq.count = count;
  seq.items = (FaTuple_Bytes_Bytes *)fa_calloc(count ? count : 1, sizeof(FaTuple_Bytes_Bytes));
  return seq;
}

static FaSeq_Int FaSeq_Int_new(size_t count) {
  FaSeq_Int seq;
  seq.count = count;
  seq.items = (int64_t *)fa_calloc(count ? count : 1, sizeof(int64_t));
  return seq;
}

static FaSeq_Real FaSeq_Real_new(size_t count) {
  FaSeq_Real seq;
  seq.count = count;
  seq.items = (double *)fa_calloc(count ? count : 1, sizeof(double));
  return seq;
}

static FaSeq_Fault FaSeq_Fault_new(size_t count) {
  FaSeq_Fault seq;
  seq.count = count;
  seq.items = (FaFault *)fa_calloc(count ? count : 1, sizeof(FaFault));
  return seq;
}

static FaFaultable_Int FaFaultable_Int_ok(int64_t value) {
  FaFaultable_Int out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Int FaFaultable_Int_fault(FaFault fault) {
  FaFaultable_Int out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Real FaFaultable_Real_ok(double value) {
  FaFaultable_Real out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Real FaFaultable_Real_fault(FaFault fault) {
  FaFaultable_Real out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Bytes FaFaultable_Bytes_ok(FaBytes value) {
  FaFaultable_Bytes out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Bytes FaFaultable_Bytes_fault(FaFault fault) {
  FaFaultable_Bytes out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Seq_Bytes FaFaultable_Seq_Bytes_ok(FaSeq_Bytes value) {
  FaFaultable_Seq_Bytes out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Seq_Bytes FaFaultable_Seq_Bytes_fault(FaFault fault) {
  FaFaultable_Seq_Bytes out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Seq_Tuple_Bytes_Bytes FaFaultable_Seq_Tuple_Bytes_Bytes_ok(FaSeq_Tuple_Bytes_Bytes value) {
  FaFaultable_Seq_Tuple_Bytes_Bytes out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Seq_Tuple_Bytes_Bytes FaFaultable_Seq_Tuple_Bytes_Bytes_fault(FaFault fault) {
  FaFaultable_Seq_Tuple_Bytes_Bytes out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_ok(FaStream value) {
  FaFaultable_Stream_Bytes out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_fault(FaFault fault) {
  FaFaultable_Stream_Bytes out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Seq_Real FaFaultable_Seq_Real_ok(FaSeq_Real value) {
  FaFaultable_Seq_Real out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Seq_Real FaFaultable_Seq_Real_fault(FaFault fault) {
  FaFaultable_Seq_Real out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaBytes fa_concat_raw(FaBytes a, FaBytes b) {
  char *bytes = (char *)fa_malloc(a.len + b.len + 1);
  memcpy(bytes, a.bytes, a.len);
  memcpy(bytes + a.len, b.bytes, b.len);
  bytes[a.len + b.len] = '\0';
  return fa_bytes_owned(bytes, a.len + b.len);
}
