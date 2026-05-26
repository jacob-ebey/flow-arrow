#ifndef FLOWARROW_STDLIB_CV_H
#define FLOWARROW_STDLIB_CV_H

#include "runtime.h"

typedef struct { double f0; double f1; } FaTuple_f64_f64;
typedef struct { double f0; FaTuple_f64_f64 f1; } FaTuple_f64_Tuple_f64_f64;
typedef struct {
  size_t count;
  FaTuple_f64_Tuple_f64_f64 *items;
} FaSeq_Tuple_f64_Tuple_f64_f64;
typedef struct {
  size_t count;
  FaSeq_Tuple_f64_Tuple_f64_f64 *items;
} FaSeq_Seq_Tuple_f64_Tuple_f64_f64;
typedef struct { int64_t f0; int64_t f1; } FaTuple_i64_i64;
typedef struct {
  FaTuple_i64_i64 f0;
  FaSeq_Seq_Tuple_f64_Tuple_f64_f64 f1;
} FaTuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64;
typedef struct {
  bool is_fault;
  FaFault fault;
  FaTuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 value;
} FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64;

typedef FaTuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 FaCvImage;
typedef FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 FaCvImageResult;
typedef FaTuple_f64_Tuple_f64_f64 FaCvPixel;

static inline FaSeq_Tuple_f64_Tuple_f64_f64 FaSeq_Tuple_f64_Tuple_f64_Real_new(size_t count) {
  FaSeq_Tuple_f64_Tuple_f64_f64 seq;
  seq.count = count;
  seq.items = (FaTuple_f64_Tuple_f64_f64 *)calloc(count ? count : 1, sizeof(FaTuple_f64_Tuple_f64_f64));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static inline FaSeq_Seq_Tuple_f64_Tuple_f64_f64 FaSeq_Seq_Tuple_f64_Tuple_f64_Real_new(size_t count) {
  FaSeq_Seq_Tuple_f64_Tuple_f64_f64 seq;
  seq.count = count;
  seq.items = (FaSeq_Tuple_f64_Tuple_f64_f64 *)calloc(count ? count : 1, sizeof(FaSeq_Tuple_f64_Tuple_f64_f64));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static inline FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_Real_ok(
    FaTuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 value
) {
  FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static inline FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_Real_fault(
    FaFault fault
) {
  FaFaultable_Tuple_Tuple_i64_Int_Seq_Seq_Tuple_f64_Tuple_f64_f64 out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

#endif
