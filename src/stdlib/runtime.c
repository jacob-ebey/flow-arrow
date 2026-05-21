#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

static void fa_die_usage(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(65);
}

static void fa_die_alloc(void) {
  fputs("flowarrow runtime: allocation failed\n", stderr);
  exit(70);
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
