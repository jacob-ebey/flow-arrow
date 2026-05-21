#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct { int _unused; } FaUnit;
typedef struct { char *bytes; size_t len; } FaBytes;
typedef struct { FaBytes message; } FaFault;
typedef struct { int argc; char **argv; } FaArgs;
typedef struct { size_t count; FaBytes *items; } FaSeq_Bytes;
typedef struct { size_t count; int64_t *items; } FaSeq_Int;
typedef struct { bool is_fault; FaFault fault; int64_t value; } FaFaultable_Int;
typedef struct { bool is_fault; FaFault fault; double value; } FaFaultable_Real;
typedef struct { bool is_fault; FaFault fault; FaBytes value; } FaFaultable_Bytes;
typedef struct { size_t count; FaFault *items; } FaSeq_Fault;
typedef void (*FaParallelForFn)(void *ctx, size_t start, size_t end);

#define FA_PARALLEL_FOR_GRAIN 64
#define FA_PARALLEL_FOR_MAX_WORKERS 64

static void fa_die_usage(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(65);
}

static void fa_die_alloc(void) {
  fputs("flowarrow runtime: allocation failed\n", stderr);
  exit(70);
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
  char *copy = (char *)malloc(len + 1);
  if (!copy) fa_die_alloc();
  memcpy(copy, bytes, len);
  copy[len] = '\0';
  return copy;
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

static FaSeq_Bytes FaSeq_Bytes_new(size_t count) {
  FaSeq_Bytes seq;
  seq.count = count;
  seq.items = (FaBytes *)calloc(count ? count : 1, sizeof(FaBytes));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static FaSeq_Int FaSeq_Int_new(size_t count) {
  FaSeq_Int seq;
  seq.count = count;
  seq.items = (int64_t *)calloc(count ? count : 1, sizeof(int64_t));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static FaSeq_Fault FaSeq_Fault_new(size_t count) {
  FaSeq_Fault seq;
  seq.count = count;
  seq.items = (FaFault *)calloc(count ? count : 1, sizeof(FaFault));
  if (!seq.items) fa_die_alloc();
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

static FaBytes fa_concat_raw(FaBytes a, FaBytes b) {
  char *bytes = (char *)malloc(a.len + b.len + 1);
  if (!bytes) fa_die_alloc();
  memcpy(bytes, a.bytes, a.len);
  memcpy(bytes + a.len, b.bytes, b.len);
  bytes[a.len + b.len] = '\0';
  return fa_bytes_owned(bytes, a.len + b.len);
}
