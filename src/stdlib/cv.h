#ifndef FLOWARROW_STDLIB_CV_H
#define FLOWARROW_STDLIB_CV_H

#include "runtime.h"

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

typedef FaTuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real FaCvImage;
typedef FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real FaCvImageResult;
typedef FaTuple_Real_Tuple_Real_Real FaCvPixel;

static inline FaSeq_Tuple_Real_Tuple_Real_Real FaSeq_Tuple_Real_Tuple_Real_Real_new(size_t count) {
  FaSeq_Tuple_Real_Tuple_Real_Real seq;
  seq.count = count;
  seq.items = (FaTuple_Real_Tuple_Real_Real *)calloc(count ? count : 1, sizeof(FaTuple_Real_Tuple_Real_Real));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static inline FaSeq_Seq_Tuple_Real_Tuple_Real_Real FaSeq_Seq_Tuple_Real_Tuple_Real_Real_new(size_t count) {
  FaSeq_Seq_Tuple_Real_Tuple_Real_Real seq;
  seq.count = count;
  seq.items = (FaSeq_Tuple_Real_Tuple_Real_Real *)calloc(count ? count : 1, sizeof(FaSeq_Tuple_Real_Tuple_Real_Real));
  if (!seq.items) fa_die_alloc();
  return seq;
}

static inline FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real_ok(
    FaTuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real value
) {
  FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static inline FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real_fault(
    FaFault fault
) {
  FaFaultable_Tuple_Tuple_Int_Int_Seq_Seq_Tuple_Real_Tuple_Real_Real out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

#endif
