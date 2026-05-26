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

static size_t fa_checked_size_add(size_t left, size_t right, const char *message) {
  if (left > SIZE_MAX - right) fa_die_usage(message);
  return left + right;
}

static size_t fa_checked_size_mul(size_t left, size_t right, const char *message) {
  if (left != 0 && right > SIZE_MAX / left) fa_die_usage(message);
  return left * right;
}

static int64_t fa_checked_size_to_i64(size_t value, const char *message) {
  if (value > (size_t)INT64_MAX) fa_die_usage(message);
  return (int64_t)value;
}

static int64_t fa_checked_i64_add(int64_t left, int64_t right) {
  int64_t out;
  if (__builtin_add_overflow(left, right, &out)) fa_die_usage("add: integer overflow");
  return out;
}

static int64_t fa_checked_i64_sub(int64_t left, int64_t right) {
  int64_t out;
  if (__builtin_sub_overflow(left, right, &out)) fa_die_usage("sub: integer overflow");
  return out;
}

static int64_t fa_checked_i64_mul(int64_t left, int64_t right) {
  int64_t out;
  if (__builtin_mul_overflow(left, right, &out)) fa_die_usage("mul: integer overflow");
  return out;
}

static int64_t fa_checked_i64_div(int64_t left, int64_t right) {
  if (right == 0) fa_die_usage("div: division by zero");
  if (left == INT64_MIN && right == -1) fa_die_usage("div: integer overflow");
  return left / right;
}

static int64_t fa_checked_i64_rem(int64_t left, int64_t right) {
  if (right == 0) fa_die_usage("rem: remainder by zero");
  if (left == INT64_MIN && right == -1) fa_die_usage("rem: integer overflow");
  return left % right;
}

static int64_t fa_checked_i64_neg(int64_t value) {
  if (value == INT64_MIN) fa_die_usage("neg: integer overflow");
  return -value;
}

static int64_t fa_checked_i64_abs(int64_t value) {
  if (value == INT64_MIN) fa_die_usage("abs: integer overflow");
  return value < 0 ? -value : value;
}

static double fa_checked_f64_div(double left, double right) {
  if (right == 0.0) fa_die_usage("div: division by zero");
  return left / right;
}

static double fa_checked_f64_rem(double left, double right) {
  if (right == 0.0) fa_die_usage("rem: remainder by zero");
  return fmod(left, right);
}

static double fa_checked_sqrt(double value) {
  if (value < 0.0) fa_die_usage("sqrt: negative input");
  return sqrt(value);
}

static FaFaultable_i64 fa_faultable_i64_div(int64_t left, int64_t right) {
  if (right == 0) return FaFaultable_i64_fault(fa_fault_cstr("div: division by zero"));
  if (left == INT64_MIN && right == -1) return FaFaultable_i64_fault(fa_fault_cstr("div: integer overflow"));
  return FaFaultable_i64_ok(left / right);
}

static FaFaultable_i64 fa_faultable_i64_rem(int64_t left, int64_t right) {
  if (right == 0) return FaFaultable_i64_fault(fa_fault_cstr("rem: remainder by zero"));
  if (left == INT64_MIN && right == -1) return FaFaultable_i64_fault(fa_fault_cstr("rem: integer overflow"));
  return FaFaultable_i64_ok(left % right);
}

static FaFaultable_f64 fa_faultable_f64_div(double left, double right) {
  if (right == 0.0) return FaFaultable_f64_fault(fa_fault_cstr("div: division by zero"));
  return FaFaultable_f64_ok(left / right);
}

static FaFaultable_f64 fa_faultable_f64_rem(double left, double right) {
  if (right == 0.0) return FaFaultable_f64_fault(fa_fault_cstr("rem: remainder by zero"));
  return FaFaultable_f64_ok(fmod(left, right));
}

static FaFaultable_f64 fa_faultable_sqrt(double value) {
  if (value < 0.0) return FaFaultable_f64_fault(fa_fault_cstr("sqrt: negative input"));
  return FaFaultable_f64_ok(sqrt(value));
}

static int fa_preview_len(size_t len) {
  return len > 240 ? 240 : (int)len;
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
  size_t total = fa_checked_size_add(sizeof(FaAllocHeader), size, "allocation size overflow");
  FaAllocHeader *header = fa_current_allocator.alloc
      ? (FaAllocHeader *)fa_current_allocator.alloc(fa_current_allocator.ctx, total)
      : (FaAllocHeader *)malloc(total);
  if (!header) fa_die_alloc();
  header->size = size;
  header->scoped = fa_current_allocator.alloc != NULL;
  return (void *)(header + 1);
}

static void *fa_calloc(size_t count, size_t size) {
  size_t total = fa_checked_size_mul(count ? count : 1, size, "allocation size overflow");
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
  size_t total = fa_checked_size_add(sizeof(FaAllocHeader), size, "allocation size overflow");
  FaAllocHeader *next = (FaAllocHeader *)realloc(header, total);
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
  size_t worker_limit;
  pthread_mutex_t lock;
} FaParallelForState;

typedef struct {
  FaParallelTaskFn *fns;
  void **ctxs;
  size_t next;
  size_t count;
  size_t worker_limit;
  pthread_mutex_t lock;
} FaParallelTaskState;

static _Thread_local int fa_parallel_depth = 0;
static _Thread_local size_t fa_parallel_worker_limit = 0;

static size_t fa_parallel_available_worker_count(void) {
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

static size_t fa_parallel_worker_count(void) {
  size_t workers = fa_parallel_available_worker_count();
  if (fa_parallel_worker_limit > 0 && workers > fa_parallel_worker_limit) {
    workers = fa_parallel_worker_limit;
  }
  return workers;
}

static void *fa_parallel_for_worker(void *arg) {
  FaParallelForState *state = (FaParallelForState *)arg;
  size_t previous_worker_limit = fa_parallel_worker_limit;
  if (state->worker_limit > 0) fa_parallel_worker_limit = state->worker_limit;
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
  fa_parallel_worker_limit = previous_worker_limit;
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
  state.worker_limit = fa_parallel_worker_limit;
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

static void *fa_parallel_tasks_worker(void *arg) {
  FaParallelTaskState *state = (FaParallelTaskState *)arg;
  size_t previous_worker_limit = fa_parallel_worker_limit;
  fa_parallel_worker_limit = state->worker_limit;
  for (;;) {
    size_t index;
    pthread_mutex_lock(&state->lock);
    index = state->next;
    if (index >= state->count) {
      pthread_mutex_unlock(&state->lock);
      break;
    }
    state->next++;
    pthread_mutex_unlock(&state->lock);
    state->fns[index](state->ctxs[index]);
  }
  fa_parallel_worker_limit = previous_worker_limit;
  return NULL;
}

static void fa_parallel_tasks(size_t count, FaParallelTaskFn *fns, void **ctxs) {
  if (count == 0) return;
  size_t workers = fa_parallel_worker_count();
  if (fa_parallel_depth > 0 || workers <= 1 || count <= 1) {
    for (size_t i = 0; i < count; i++) fns[i](ctxs[i]);
    return;
  }
  if (workers > count) workers = count;
  size_t per_task_workers = fa_parallel_worker_count() / workers;
  if (per_task_workers < 1) per_task_workers = 1;

  FaParallelTaskState state;
  state.fns = fns;
  state.ctxs = ctxs;
  state.next = 0;
  state.count = count;
  state.worker_limit = per_task_workers;
  if (pthread_mutex_init(&state.lock, NULL) != 0) {
    for (size_t i = 0; i < count; i++) fns[i](ctxs[i]);
    return;
  }

  pthread_t threads[FA_PARALLEL_FOR_MAX_WORKERS];
  size_t spawned = 0;
  for (; spawned + 1 < workers; spawned++) {
    if (pthread_create(&threads[spawned], NULL, fa_parallel_tasks_worker, &state) != 0) break;
  }
  fa_parallel_tasks_worker(&state);
  for (size_t i = 0; i < spawned; i++) pthread_join(threads[i], NULL);
  pthread_mutex_destroy(&state.lock);
}

static FaUnit fa_unit(void) {
  FaUnit unit;
  unit._unused = 0;
  return unit;
}

static char *fa_copy_bytes(const char *bytes, size_t len) {
  char *copy = (char *)fa_malloc(fa_checked_size_add(len, 1, "byte copy length overflow"));
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
  if (fault.message.len > 0) fwrite(fault.message.bytes, 1, fault.message.len, stderr);
  fputc('\n', stderr);
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

static FaSeq_i64 FaSeq_i64_new(size_t count) {
  FaSeq_i64 seq;
  seq.count = count;
  seq.items = (int64_t *)fa_calloc(count ? count : 1, sizeof(int64_t));
  return seq;
}

static FaSeq_f64 FaSeq_f64_new(size_t count) {
  FaSeq_f64 seq;
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

static FaFaultable_i64 FaFaultable_i64_ok(int64_t value) {
  FaFaultable_i64 out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_i64 FaFaultable_i64_fault(FaFault fault) {
  FaFaultable_i64 out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_f64 FaFaultable_f64_ok(double value) {
  FaFaultable_f64 out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_f64 FaFaultable_f64_fault(FaFault fault) {
  FaFaultable_f64 out;
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

static FaFaultable_Seq_f64 FaFaultable_Seq_f64_ok(FaSeq_f64 value) {
  FaFaultable_Seq_f64 out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Seq_f64 FaFaultable_Seq_f64_fault(FaFault fault) {
  FaFaultable_Seq_f64 out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaBytes fa_concat_raw(FaBytes a, FaBytes b) {
  size_t len = fa_checked_size_add(a.len, b.len, "concat: byte length overflow");
  size_t alloc = fa_checked_size_add(len, 1, "concat: byte length overflow");
  char *bytes = (char *)fa_malloc(alloc);
  memcpy(bytes, a.bytes, a.len);
  memcpy(bytes + a.len, b.bytes, b.len);
  bytes[len] = '\0';
  return fa_bytes_owned(bytes, len);
}
