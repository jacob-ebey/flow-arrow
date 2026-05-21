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
