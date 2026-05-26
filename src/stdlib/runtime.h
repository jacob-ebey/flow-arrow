#ifndef FLOWARROW_STDLIB_RUNTIME_H
#define FLOWARROW_STDLIB_RUNTIME_H

/*
 * Shared runtime declarations for generated runtime.c and for standalone
 * editor analysis of stdlib C fragments.
 */

#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <limits.h>
#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

typedef struct { int _unused; } FaUnit;
typedef struct { char *bytes; size_t len; } FaBytes;
typedef struct { FaBytes message; } FaFault;
typedef struct { int argc; char **argv; } FaArgs;
typedef int (*FaStreamNextFn)(void *state, void *out, FaFault *fault);
typedef int (*FaStreamCloseFn)(void *state, FaFault *fault);
typedef struct {
  FILE *file;
  int fd;
  FaBytes path;
  void *state;
  void *map_fn;
  FaStreamNextFn next;
  FaStreamCloseFn close;
  size_t item_size;
  bool closed;
} FaStream;
typedef struct { size_t count; FaBytes *items; } FaSeq_Bytes;
typedef struct { FaBytes f0; FaBytes f1; } FaTuple_Bytes_Bytes;
typedef struct { size_t count; FaTuple_Bytes_Bytes *items; } FaSeq_Tuple_Bytes_Bytes;
typedef struct { size_t count; int32_t *items; } FaSeq_i32;
typedef struct { size_t count; int64_t *items; } FaSeq_i64;
typedef struct { size_t count; float *items; } FaSeq_f32;
typedef struct { size_t count; double *items; } FaSeq_f64;
typedef struct { bool is_fault; FaFault fault; int32_t value; } FaFaultable_i32;
typedef struct { bool is_fault; FaFault fault; int64_t value; } FaFaultable_i64;
typedef struct { bool is_fault; FaFault fault; float value; } FaFaultable_f32;
typedef struct { bool is_fault; FaFault fault; double value; } FaFaultable_f64;
typedef struct { bool is_fault; FaFault fault; FaBytes value; } FaFaultable_Bytes;
typedef struct { bool is_fault; FaFault fault; FaSeq_Bytes value; } FaFaultable_Seq_Bytes;
typedef struct { bool is_fault; FaFault fault; FaSeq_Tuple_Bytes_Bytes value; } FaFaultable_Seq_Tuple_Bytes_Bytes;
typedef struct { bool is_fault; FaFault fault; FaStream value; } FaFaultable_Stream_Bytes;
typedef struct { bool is_fault; FaFault fault; FaSeq_f64 value; } FaFaultable_Seq_f64;
typedef struct { size_t count; FaFault *items; } FaSeq_Fault;
typedef void (*FaParallelForFn)(void *ctx, size_t start, size_t end);
typedef void (*FaParallelTaskFn)(void *ctx);
typedef void *(*FaScopedAllocFn)(void *ctx, size_t size);
typedef struct { FaScopedAllocFn alloc; void *ctx; } FaScopedAllocator;

#define FA_PARALLEL_FOR_GRAIN 64
#define FA_PARALLEL_FOR_MAX_WORKERS 64

#ifdef __clang__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wundefined-internal"
#endif

static void fa_die_usage(const char *message);
static void fa_die_alloc(void);
static size_t fa_checked_size_add(size_t left, size_t right, const char *message);
static size_t fa_checked_size_mul(size_t left, size_t right, const char *message);
static int64_t fa_checked_size_to_i64(size_t value, const char *message);
static int32_t fa_checked_i32_add(int32_t left, int32_t right);
static int32_t fa_checked_i32_sub(int32_t left, int32_t right);
static int32_t fa_checked_i32_mul(int32_t left, int32_t right);
static int32_t fa_checked_i32_div(int32_t left, int32_t right);
static int32_t fa_checked_i32_rem(int32_t left, int32_t right);
static int32_t fa_checked_i32_neg(int32_t value);
static int32_t fa_checked_i32_abs(int32_t value);
static int64_t fa_checked_i64_add(int64_t left, int64_t right);
static int64_t fa_checked_i64_sub(int64_t left, int64_t right);
static int64_t fa_checked_i64_mul(int64_t left, int64_t right);
static int64_t fa_checked_i64_div(int64_t left, int64_t right);
static int64_t fa_checked_i64_rem(int64_t left, int64_t right);
static int64_t fa_checked_i64_neg(int64_t value);
static int64_t fa_checked_i64_abs(int64_t value);
static FaFaultable_i64 fa_faultable_i64_add(int64_t left, int64_t right);
static FaFaultable_i64 fa_faultable_i64_sub(int64_t left, int64_t right);
static FaFaultable_i64 fa_faultable_i64_mul(int64_t left, int64_t right);
static FaFaultable_i64 fa_faultable_i64_neg(int64_t value);
static FaFaultable_i64 fa_faultable_i64_abs(int64_t value);
static FaFaultable_i32 fa_faultable_i32_add(int32_t left, int32_t right);
static FaFaultable_i32 fa_faultable_i32_sub(int32_t left, int32_t right);
static FaFaultable_i32 fa_faultable_i32_mul(int32_t left, int32_t right);
static FaFaultable_i32 fa_faultable_i32_neg(int32_t value);
static FaFaultable_i32 fa_faultable_i32_abs(int32_t value);
static float fa_checked_f32_div(float left, float right);
static float fa_checked_f32_rem(float left, float right);
static float fa_checked_sqrtf(float value);
static double fa_checked_f64_div(double left, double right);
static double fa_checked_f64_rem(double left, double right);
static double fa_checked_sqrt(double value);
static FaFaultable_i32 fa_faultable_i32_div(int32_t left, int32_t right);
static FaFaultable_i32 fa_faultable_i32_rem(int32_t left, int32_t right);
static FaFaultable_i64 fa_faultable_i64_div(int64_t left, int64_t right);
static FaFaultable_i64 fa_faultable_i64_rem(int64_t left, int64_t right);
static FaFaultable_f32 fa_faultable_f32_div(float left, float right);
static FaFaultable_f32 fa_faultable_f32_rem(float left, float right);
static FaFaultable_f32 fa_faultable_sqrtf(float value);
static FaFaultable_f64 fa_faultable_f64_div(double left, double right);
static FaFaultable_f64 fa_faultable_f64_rem(double left, double right);
static FaFaultable_f64 fa_faultable_sqrt(double value);
static int fa_preview_len(size_t len);
static FaScopedAllocator fa_scoped_allocator_enter(FaScopedAllocFn alloc, void *ctx);
static void fa_scoped_allocator_restore(FaScopedAllocator previous);
static void *fa_malloc(size_t size);
static void *fa_calloc(size_t count, size_t size);
static void *fa_realloc(void *ptr, size_t size);
static void fa_free(void *ptr);
static void fa_parallel_for(size_t start, size_t end, size_t grain, FaParallelForFn fn, void *ctx);
static void fa_parallel_tasks(size_t count, FaParallelTaskFn *fns, void **ctxs);
static FaUnit fa_unit(void);
static char *fa_copy_bytes(const char *bytes, size_t len);
static FaBytes fa_bytes_borrowed(const char *bytes, size_t len);
static FaBytes fa_bytes_owned(char *bytes, size_t len);
static FaBytes fa_bytes_literal(const char *bytes, size_t len);
static FaFault fa_fault_bytes(FaBytes message);
static FaFault fa_fault_cstr(const char *message);
static void fa_exit_fault(FaFault fault);
static int fa_stream_close(FaStream *stream, FaFault *fault);
static FaSeq_Bytes FaSeq_Bytes_new(size_t count);
static FaSeq_Tuple_Bytes_Bytes FaSeq_Tuple_Bytes_Bytes_new(size_t count);
static FaSeq_i32 FaSeq_i32_new(size_t count);
static FaSeq_i64 FaSeq_i64_new(size_t count);
static FaSeq_f32 FaSeq_f32_new(size_t count);
static FaSeq_f64 FaSeq_f64_new(size_t count);
static FaSeq_Fault FaSeq_Fault_new(size_t count);
static FaFaultable_i32 FaFaultable_i32_ok(int32_t value);
static FaFaultable_i32 FaFaultable_i32_fault(FaFault fault);
static FaFaultable_i64 FaFaultable_i64_ok(int64_t value);
static FaFaultable_i64 FaFaultable_i64_fault(FaFault fault);
static FaFaultable_f32 FaFaultable_f32_ok(float value);
static FaFaultable_f32 FaFaultable_f32_fault(FaFault fault);
static FaFaultable_f64 FaFaultable_f64_ok(double value);
static FaFaultable_f64 FaFaultable_f64_fault(FaFault fault);
static FaFaultable_Bytes FaFaultable_Bytes_ok(FaBytes value);
static FaFaultable_Bytes FaFaultable_Bytes_fault(FaFault fault);
static FaFaultable_Seq_Bytes FaFaultable_Seq_Bytes_ok(FaSeq_Bytes value);
static FaFaultable_Seq_Bytes FaFaultable_Seq_Bytes_fault(FaFault fault);
static FaFaultable_Seq_Tuple_Bytes_Bytes FaFaultable_Seq_Tuple_Bytes_Bytes_ok(FaSeq_Tuple_Bytes_Bytes value);
static FaFaultable_Seq_Tuple_Bytes_Bytes FaFaultable_Seq_Tuple_Bytes_Bytes_fault(FaFault fault);
static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_ok(FaStream value);
static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_fault(FaFault fault);
static FaFaultable_Seq_f64 FaFaultable_Seq_f64_ok(FaSeq_f64 value);
static FaFaultable_Seq_f64 FaFaultable_Seq_f64_fault(FaFault fault);
static FaBytes fa_concat_raw(FaBytes a, FaBytes b);

#ifdef __clang__
#pragma clang diagnostic pop
#endif

#endif
