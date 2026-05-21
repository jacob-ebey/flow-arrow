#ifndef FLOWARROW_STDLIB_RUNTIME_H
#define FLOWARROW_STDLIB_RUNTIME_H

/*
 * Editor-only declarations for stdlib C fragments. Local includes are
 * stripped before the compiler writes the generated runtime.c.
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
typedef struct { FILE *file; int fd; FaBytes path; } FaStream;
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

typedef struct { double f0; double f1; } FaTuple_Real_Real;
typedef struct { double f0; FaTuple_Real_Real f1; } FaTuple_Real_Tuple_Real_Real;
typedef struct {
  size_t count;
  FaTuple_Real_Tuple_Real_Real *items;
} FaSeq_Tuple_Real_Tuple_Real_Real;
typedef struct {
  size_t count;
  FaSeq_Tuple_Real_Tuple_Real_Real *items;
} FaSeq_Seq_Tuple_Real_Tuple_Real_Real;
typedef struct { int64_t f0; int64_t f1; } FaTuple_Int_Int;
typedef struct {
  FaTuple_Int_Int f0;
  FaSeq_Seq_Tuple_Real_Tuple_Real_Real f1;
} FaTuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real;
typedef struct {
  bool is_fault;
  FaFault fault;
  FaTuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real value;
} FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real;

void fa_die_usage(const char *message);
void fa_die_alloc(void);
void fa_parallel_for(size_t start, size_t end, size_t grain, FaParallelForFn fn, void *ctx);
FaUnit fa_unit(void);
char *fa_copy_bytes(const char *bytes, size_t len);
FaBytes fa_bytes_owned(char *bytes, size_t len);
FaBytes fa_bytes_literal(const char *bytes, size_t len);
FaFault fa_fault_bytes(FaBytes message);
FaFault fa_fault_cstr(const char *message);
void fa_exit_fault(FaFault fault);
FaSeq_Bytes FaSeq_Bytes_new(size_t count);
FaSeq_Int FaSeq_Int_new(size_t count);
FaSeq_Real FaSeq_Real_new(size_t count);
FaSeq_Fault FaSeq_Fault_new(size_t count);
FaSeq_Tuple_Real_Tuple_Real_Real FaSeq_Tuple_Real_Tuple_Real_Real_new(size_t count);
FaSeq_Seq_Tuple_Real_Tuple_Real_Real FaSeq_Seq_Tuple_Real_Tuple_Real_Real_new(size_t count);
FaFaultable_Int FaFaultable_Int_ok(int64_t value);
FaFaultable_Int FaFaultable_Int_fault(FaFault fault);
FaFaultable_Real FaFaultable_Real_ok(double value);
FaFaultable_Real FaFaultable_Real_fault(FaFault fault);
FaFaultable_Bytes FaFaultable_Bytes_ok(FaBytes value);
FaFaultable_Bytes FaFaultable_Bytes_fault(FaFault fault);
FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_ok(FaStream value);
FaFaultable_Stream_Bytes FaFaultable_Stream_Bytes_fault(FaFault fault);
FaFaultable_Seq_Real FaFaultable_Seq_Real_ok(FaSeq_Real value);
FaFaultable_Seq_Real FaFaultable_Seq_Real_fault(FaFault fault);
FaBytes fa_concat_raw(FaBytes a, FaBytes b);

#endif
