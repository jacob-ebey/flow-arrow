#ifndef FLOWARROW_STDLIB_RUNTIME_H
#define FLOWARROW_STDLIB_RUNTIME_H

/*
 * Shared runtime declarations for generated runtime.c and for standalone
 * editor analysis of stdlib C fragments.
 */

#include <ctype.h>
#include <errno.h>
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
typedef struct { size_t count; int64_t *items; } FaSeq_Int;
typedef struct { size_t count; double *items; } FaSeq_Real;
typedef struct { bool is_fault; FaFault fault; int64_t value; } FaFaultable_Int;
typedef struct { bool is_fault; FaFault fault; double value; } FaFaultable_Real;
typedef struct { bool is_fault; FaFault fault; FaBytes value; } FaFaultable_Bytes;
typedef struct { bool is_fault; FaFault fault; FaStream value; } FaFaultable_Stream_Bytes;
typedef struct { bool is_fault; FaFault fault; FaSeq_Real value; } FaFaultable_Seq_Real;
typedef struct { size_t count; FaFault *items; } FaSeq_Fault;
typedef void (*FaParallelForFn)(void *ctx, size_t start, size_t end);

#define FA_PARALLEL_FOR_GRAIN 64
#define FA_PARALLEL_FOR_MAX_WORKERS 64

#ifdef __clang__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wundefined-internal"
#endif

static void fa_die_usage(const char *message);
static void fa_die_alloc(void);
static void fa_parallel_for(size_t start, size_t end, size_t grain, FaParallelForFn fn, void *ctx);
static FaUnit fa_unit(void);
static char *fa_copy_bytes(const char *bytes, size_t len);
static FaBytes fa_bytes_owned(char *bytes, size_t len);
static FaBytes fa_bytes_literal(const char *bytes, size_t len);
static FaFault fa_fault_bytes(FaBytes message);
static FaFault fa_fault_cstr(const char *message);
static void fa_exit_fault(FaFault fault);
static int fa_stream_close(FaStream *stream, FaFault *fault);
static FaSeq_Bytes FaSeq_Bytes_new(size_t count);
static FaSeq_Int FaSeq_Int_new(size_t count);
static FaSeq_Real FaSeq_Real_new(size_t count);
static FaSeq_Fault FaSeq_Fault_new(size_t count);
static FaFaultable_Int FaFaultable_Int_ok(int64_t value);
static FaFaultable_Int FaFaultable_Int_fault(FaFault fault);
static FaFaultable_Real FaFaultable_Real_ok(double value);
static FaFaultable_Real FaFaultable_Real_fault(FaFault fault);
static FaFaultable_Bytes FaFaultable_Bytes_ok(FaBytes value);
static FaFaultable_Bytes FaFaultable_Bytes_fault(FaFault fault);
static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_ok(FaStream value);
static FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_fault(FaFault fault);
static FaFaultable_Seq_Real FaFaultable_Seq_Real_ok(FaSeq_Real value);
static FaFaultable_Seq_Real FaFaultable_Seq_Real_fault(FaFault fault);
static FaBytes fa_concat_raw(FaBytes a, FaBytes b);

#ifdef __clang__
#pragma clang diagnostic pop
#endif

#endif
