#include "runtime.h"
#include <sqlite3.h>

typedef struct FaSqliteConnectionState FaSqliteConnectionState;

typedef struct {
  FaSqliteConnectionState *state;
} FaSqliteConnection;

typedef enum {
  FA_SQLITE_NULL = 0,
  FA_SQLITE_INT = 1,
  FA_SQLITE_REAL = 2,
  FA_SQLITE_TEXT = 3,
  FA_SQLITE_BLOB = 4
} FaSqliteValueKind;

typedef struct {
  int kind;
  int64_t int_value;
  double real_value;
  FaBytes bytes_value;
} FaSqliteValue;

typedef struct {
  size_t count;
  FaBytes *names;
  FaSqliteValue *values;
} FaSqliteRow;

typedef struct {
  size_t count;
  FaSqliteValue *items;
} FaSeq_SqliteValue;

typedef struct {
  size_t count;
  FaSqliteRow *items;
} FaSeq_SqliteRow;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaSqliteConnection value;
} FaFaultable_SqliteConnection;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaSqliteValue value;
} FaFaultable_SqliteValue;

typedef struct {
  FaSqliteConnection f0;
  bool f1;
} FaTuple_SqliteConnection_Bool;

typedef struct {
  FaSqliteConnection f0;
  int64_t f1;
} FaTuple_SqliteConnection_Int;

typedef struct {
  FaSqliteConnection f0;
  FaBytes f1;
  FaSeq_SqliteValue f2;
} FaTuple_SqliteConnection_Bytes_Seq_SqliteValue;

typedef struct {
  FaSqliteConnection f0;
  FaStream f1;
} FaTuple_SqliteConnection_Stream_SqliteRow;

typedef struct {
  FaSqliteConnection f0;
  FaSeq_SqliteRow f1;
} FaTuple_SqliteConnection_Seq_SqliteRow;

typedef struct {
  FaSqliteRow f0;
  int64_t f1;
} FaTuple_SqliteRow_Int;

typedef struct {
  FaSqliteRow f0;
  FaBytes f1;
} FaTuple_SqliteRow_Bytes;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaTuple_SqliteConnection_Int value;
} FaFaultable_Tuple_SqliteConnection_Int;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaTuple_SqliteConnection_Stream_SqliteRow value;
} FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaTuple_SqliteConnection_Seq_SqliteRow value;
} FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow;

static FaSeq_SqliteValue FaSeq_SqliteValue_new(size_t count) {
  FaSeq_SqliteValue seq;
  seq.count = count;
  seq.items = (FaSqliteValue *)fa_calloc(count ? count : 1, sizeof(FaSqliteValue));
  return seq;
}

static FaSeq_SqliteRow FaSeq_SqliteRow_new(size_t count) {
  FaSeq_SqliteRow seq;
  seq.count = count;
  seq.items = (FaSqliteRow *)fa_calloc(count ? count : 1, sizeof(FaSqliteRow));
  return seq;
}

static FaFaultable_SqliteConnection FaFaultable_SqliteConnection_ok(FaSqliteConnection value) {
  FaFaultable_SqliteConnection out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_SqliteConnection FaFaultable_SqliteConnection_fault(FaFault fault) {
  FaFaultable_SqliteConnection out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_SqliteValue FaFaultable_SqliteValue_ok(FaSqliteValue value) {
  FaFaultable_SqliteValue out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_SqliteValue FaFaultable_SqliteValue_fault(FaFault fault) {
  FaFaultable_SqliteValue out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Int FaFaultable_Tuple_SqliteConnection_Int_ok(FaTuple_SqliteConnection_Int value) {
  FaFaultable_Tuple_SqliteConnection_Int out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Int FaFaultable_Tuple_SqliteConnection_Int_fault(FaFault fault) {
  FaFaultable_Tuple_SqliteConnection_Int out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_ok(FaTuple_SqliteConnection_Stream_SqliteRow value) {
  FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_fault(FaFault fault) {
  FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_ok(FaTuple_SqliteConnection_Seq_SqliteRow value) {
  FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_fault(FaFault fault) {
  FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}
