#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef enum {
  FA_UNIT,
  FA_INT,
  FA_REAL,
  FA_BOOL,
  FA_BYTES,
  FA_SEQ,
  FA_FAULT
} FaKind;

typedef struct FaValue FaValue;

typedef struct {
  FaValue *items;
  size_t count;
} FaSeq;

struct FaValue {
  FaKind kind;
  int64_t i;
  double real;
  bool b;
  char *bytes;
  size_t len;
  FaSeq seq;
};

typedef struct {
  FaValue ok;
  FaValue faults;
} FaFaultMapResult;

typedef enum {
  FA_REDUCE_ADD,
  FA_REDUCE_CONCAT_BYTES
} FaReduceOp;

typedef FaValue (*FaFunction)(FaValue);

static void fa_die_usage(const char *message) {
  fputs(message, stderr);
  fputc('\n', stderr);
  exit(65);
}

static void fa_die_alloc(void) {
  fputs("flowarrow runtime: allocation failed\n", stderr);
  exit(70);
}

static char *fa_copy_bytes(const char *bytes, size_t len) {
  char *copy = (char *)malloc(len + 1);
  if (!copy) fa_die_alloc();
  memcpy(copy, bytes, len);
  copy[len] = '\0';
  return copy;
}

static FaValue fa_unit(void) {
  FaValue value;
  memset(&value, 0, sizeof(value));
  value.kind = FA_UNIT;
  return value;
}

static FaValue fa_int(int64_t i) {
  FaValue value = fa_unit();
  value.kind = FA_INT;
  value.i = i;
  return value;
}

static FaValue fa_real(double real) {
  FaValue value = fa_unit();
  value.kind = FA_REAL;
  value.real = real;
  return value;
}

static FaValue fa_bool(bool b) {
  FaValue value = fa_unit();
  value.kind = FA_BOOL;
  value.b = b;
  return value;
}

static FaValue fa_bytes_owned(char *bytes, size_t len) {
  FaValue value = fa_unit();
  value.kind = FA_BYTES;
  value.bytes = bytes;
  value.len = len;
  return value;
}

static FaValue fa_bytes_from_slice(const char *bytes, size_t len) {
  return fa_bytes_owned(fa_copy_bytes(bytes, len), len);
}

static FaValue fa_bytes_literal(const char *bytes, size_t len) {
  return fa_bytes_from_slice(bytes, len);
}

static FaValue fa_fault_from_slice(const char *bytes, size_t len) {
  FaValue value = fa_bytes_from_slice(bytes, len);
  value.kind = FA_FAULT;
  return value;
}

static FaValue fa_fault_from_cstr(const char *message) {
  return fa_fault_from_slice(message, strlen(message));
}

static FaValue fa_seq_new(size_t count) {
  FaValue value = fa_unit();
  value.kind = FA_SEQ;
  value.seq.count = count;
  value.seq.items = (FaValue *)calloc(count ? count : 1, sizeof(FaValue));
  if (!value.seq.items) fa_die_alloc();
  return value;
}

static void fa_seq_set(FaValue *seq, size_t index, FaValue item) {
  if (!seq || seq->kind != FA_SEQ || index >= seq->seq.count) {
    fa_die_usage("flowarrow runtime: invalid sequence write");
  }
  seq->seq.items[index] = item;
}

static FaValue fa_seq_get(FaValue seq, size_t index) {
  if (seq.kind != FA_SEQ || index >= seq.seq.count) {
    fa_die_usage("flowarrow runtime: invalid sequence read");
  }
  return seq.seq.items[index];
}

static FaValue fa_expect_seq(FaValue value, const char *op) {
  if (value.kind == FA_FAULT) return value;
  if (value.kind != FA_SEQ) {
    fprintf(stderr, "flowarrow runtime: %s expected Seq input\n", op);
    exit(65);
  }
  return value;
}

static int64_t fa_expect_int(FaValue value, const char *op) {
  if (value.kind != FA_INT) {
    fprintf(stderr, "flowarrow runtime: %s expected Int input\n", op);
    exit(65);
  }
  return value.i;
}

static double fa_expect_real(FaValue value, const char *op) {
  if (value.kind != FA_REAL) {
    fprintf(stderr, "flowarrow runtime: %s expected Real input\n", op);
    exit(65);
  }
  return value.real;
}

static double fa_expect_number(FaValue value, const char *op) {
  if (value.kind == FA_INT) return (double)value.i;
  if (value.kind == FA_REAL) return value.real;
  fprintf(stderr, "flowarrow runtime: %s expected numeric input\n", op);
  exit(65);
}

static FaValue fa_expect_bytes(FaValue value, const char *op) {
  if (value.kind == FA_FAULT) return value;
  if (value.kind != FA_BYTES) {
    fprintf(stderr, "flowarrow runtime: %s expected Bytes input\n", op);
    exit(65);
  }
  return value;
}

static FaValue fa_propagate_fault_from_seq(FaValue value) {
  if (value.kind == FA_FAULT) return value;
  if (value.kind == FA_SEQ) {
    for (size_t i = 0; i < value.seq.count; i++) {
      FaValue item = value.seq.items[i];
      if (item.kind == FA_FAULT) return item;
    }
  }
  return fa_unit();
}

static int64_t fa_checked_integer_add(int64_t left, int64_t right, const char *op) {
  int64_t result = 0;
  if (__builtin_add_overflow(left, right, &result)) {
    fprintf(stderr, "flowarrow runtime: %s overflow\n", op);
    exit(65);
  }
  return result;
}

static int64_t fa_checked_integer_sub(int64_t left, int64_t right, const char *op) {
  int64_t result = 0;
  if (__builtin_sub_overflow(left, right, &result)) {
    fprintf(stderr, "flowarrow runtime: %s overflow\n", op);
    exit(65);
  }
  return result;
}

