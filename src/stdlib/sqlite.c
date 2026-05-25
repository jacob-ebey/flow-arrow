#include "sqlite.h"

struct FaSqliteConnectionState {
  sqlite3 *db;
  bool closed;
  bool close_requested;
  size_t refs;
};

typedef struct {
  sqlite3_stmt *stmt;
  FaSqliteConnectionState *connection;
  bool finalized;
} FaSqliteRowStreamState;

static FaFaultable_Tuple_SqliteConnection_Int fa_sqlite_exec(FaTuple_SqliteConnection_Bytes_Seq_SqliteValue input);

static FaFault fa_sqlite_fault_message(const char *operation, int rc, const char *message) {
  size_t op_len = strlen(operation);
  size_t msg_len = strlen(message);
  char rc_buf[32];
  snprintf(rc_buf, sizeof(rc_buf), "%d", rc);
  size_t rc_len = strlen(rc_buf);
  const char *prefix = "std.sqlite: ";
  size_t prefix_len = strlen(prefix);
  size_t len = fa_checked_size_add(prefix_len, op_len, "std.sqlite: fault message length overflow");
  len = fa_checked_size_add(len, 10, "std.sqlite: fault message length overflow");
  len = fa_checked_size_add(len, rc_len, "std.sqlite: fault message length overflow");
  len = fa_checked_size_add(len, 3, "std.sqlite: fault message length overflow");
  len = fa_checked_size_add(len, msg_len, "std.sqlite: fault message length overflow");
  size_t out_len = fa_checked_size_add(len, 1, "std.sqlite: fault message length overflow");
  char *out = (char *)malloc(out_len);
  if (!out) fa_die_alloc();
  snprintf(out, out_len, "%s%s failed (%s): %s", prefix, operation, rc_buf, message);
  return fa_fault_bytes(fa_bytes_owned(out, strlen(out)));
}

static FaFault fa_sqlite_fault(sqlite3 *db, const char *operation, int rc) {
  const char *message = db ? sqlite3_errmsg(db) : sqlite3_errstr(rc);
  return fa_sqlite_fault_message(operation, rc, message);
}

static FaFault fa_sqlite_fault_cstr(const char *message) {
  return fa_fault_cstr(message);
}

static bool fa_sqlite_bytes_has_nul(FaBytes bytes) {
  return memchr(bytes.bytes, '\0', bytes.len) != NULL;
}

static void fa_sqlite_retain(FaSqliteConnectionState *state) {
  if (state) state->refs++;
}

static void fa_sqlite_release(FaSqliteConnectionState *state) {
  if (!state || state->refs == 0) return;
  state->refs--;
  if (state->refs == 0 && state->db) {
    sqlite3_close_v2(state->db);
    state->db = NULL;
  }
}

static sqlite3 *fa_sqlite_db(FaSqliteConnection connection, FaFault *fault, const char *operation) {
  if (!connection.state || connection.state->closed || !connection.state->db) {
    *fault = fa_sqlite_fault_cstr("std.sqlite: connection is closed");
    return NULL;
  }
  (void)operation;
  return connection.state->db;
}

static int fa_sqlite_exec_pragma(sqlite3 *db, const char *sql) {
  char *errmsg = NULL;
  int rc = sqlite3_exec(db, sql, NULL, NULL, &errmsg);
  if (errmsg) sqlite3_free(errmsg);
  return rc;
}

static FaFaultable_SqliteConnection fa_sqlite_open_flags(FaBytes path, int flags, const char *operation) {
  if (fa_sqlite_bytes_has_nul(path)) {
    return FaFaultable_SqliteConnection_fault(fa_sqlite_fault_cstr("std.sqlite: database path contains NUL byte"));
  }
  char *path_c = fa_copy_bytes(path.bytes, path.len);
  sqlite3 *db = NULL;
  int rc = sqlite3_open_v2(path_c, &db, flags, NULL);
  fa_free(path_c);
  if (rc != SQLITE_OK) {
    FaFault fault = fa_sqlite_fault(db, operation, rc);
    if (db) sqlite3_close_v2(db);
    return FaFaultable_SqliteConnection_fault(fault);
  }
  sqlite3_busy_timeout(db, 5000);
  rc = fa_sqlite_exec_pragma(db, "PRAGMA foreign_keys = ON");
  if (rc != SQLITE_OK) {
    FaFault fault = fa_sqlite_fault(db, "foreign_keys", rc);
    sqlite3_close_v2(db);
    return FaFaultable_SqliteConnection_fault(fault);
  }
  FaSqliteConnectionState *state = (FaSqliteConnectionState *)calloc(1, sizeof(FaSqliteConnectionState));
  if (!state) fa_die_alloc();
  state->db = db;
  state->closed = false;
  state->close_requested = false;
  state->refs = 1;
  FaSqliteConnection connection;
  connection.state = state;
  return FaFaultable_SqliteConnection_ok(connection);
}

static FaFaultable_SqliteConnection fa_sqlite_open(FaBytes path) {
  return fa_sqlite_open_flags(path, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, "open");
}

static FaFaultable_SqliteConnection fa_sqlite_open_readonly(FaBytes path) {
  return fa_sqlite_open_flags(path, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, "open_readonly");
}

static FaFaultable_SqliteConnection fa_sqlite_open_memory(FaUnit unit) {
  (void)unit;
  return fa_sqlite_open_flags(fa_bytes_literal(":memory:", 8), SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, "open_memory");
}

static FaFaultable_Int fa_sqlite_close(FaSqliteConnection connection) {
  if (!connection.state || connection.state->closed) {
    return FaFaultable_Int_fault(fa_sqlite_fault_cstr("std.sqlite: connection is closed"));
  }
  connection.state->closed = true;
  connection.state->close_requested = true;
  fa_sqlite_release(connection.state);
  return FaFaultable_Int_ok(0);
}

static FaFaultable_SqliteConnection fa_sqlite_busy_timeout(FaTuple_SqliteConnection_Int input) {
  FaFault fault;
  sqlite3 *db = fa_sqlite_db(input.f0, &fault, "busy_timeout");
  if (!db) return FaFaultable_SqliteConnection_fault(fault);
  if (input.f1 < 0 || input.f1 > INT32_MAX) {
    return FaFaultable_SqliteConnection_fault(fa_sqlite_fault_cstr("std.sqlite: busy_timeout expects milliseconds in range 0..2147483647"));
  }
  int rc = sqlite3_busy_timeout(db, (int)input.f1);
  if (rc != SQLITE_OK) return FaFaultable_SqliteConnection_fault(fa_sqlite_fault(db, "busy_timeout", rc));
  return FaFaultable_SqliteConnection_ok(input.f0);
}

static FaFaultable_SqliteConnection fa_sqlite_foreign_keys(FaTuple_SqliteConnection_Bool input) {
  FaFault fault;
  sqlite3 *db = fa_sqlite_db(input.f0, &fault, "foreign_keys");
  if (!db) return FaFaultable_SqliteConnection_fault(fault);
  int rc = fa_sqlite_exec_pragma(db, input.f1 ? "PRAGMA foreign_keys = ON" : "PRAGMA foreign_keys = OFF");
  if (rc != SQLITE_OK) return FaFaultable_SqliteConnection_fault(fa_sqlite_fault(db, "foreign_keys", rc));
  return FaFaultable_SqliteConnection_ok(input.f0);
}

static FaFaultable_SqliteConnection fa_sqlite_exec_no_params(FaSqliteConnection connection, const char *sql, const char *operation) {
  FaSeq_SqliteValue params = FaSeq_SqliteValue_new(0);
  FaTuple_SqliteConnection_Bytes_Seq_SqliteValue input;
  input.f0 = connection;
  input.f1 = fa_bytes_literal(sql, strlen(sql));
  input.f2 = params;
  FaFaultable_Tuple_SqliteConnection_Int result = fa_sqlite_exec(input);
  if (result.is_fault) return FaFaultable_SqliteConnection_fault(result.fault);
  return FaFaultable_SqliteConnection_ok(result.value.f0);
}

static FaFaultable_SqliteConnection fa_sqlite_begin(FaSqliteConnection connection) {
  return fa_sqlite_exec_no_params(connection, "BEGIN", "begin");
}

static FaFaultable_SqliteConnection fa_sqlite_begin_immediate(FaSqliteConnection connection) {
  return fa_sqlite_exec_no_params(connection, "BEGIN IMMEDIATE", "begin_immediate");
}

static FaFaultable_SqliteConnection fa_sqlite_commit(FaSqliteConnection connection) {
  return fa_sqlite_exec_no_params(connection, "COMMIT", "commit");
}

static FaFaultable_SqliteConnection fa_sqlite_rollback(FaSqliteConnection connection) {
  return fa_sqlite_exec_no_params(connection, "ROLLBACK", "rollback");
}

static FaSqliteValue fa_sqlite_null(FaUnit unit) {
  (void)unit;
  FaSqliteValue value;
  value.kind = FA_SQLITE_NULL;
  value.int_value = 0;
  value.real_value = 0.0;
  value.bytes_value = fa_bytes_literal("", 0);
  return value;
}

static FaSqliteValue fa_sqlite_int(int64_t input) {
  FaSqliteValue value = fa_sqlite_null(fa_unit());
  value.kind = FA_SQLITE_INT;
  value.int_value = input;
  return value;
}

static FaSqliteValue fa_sqlite_real(double input) {
  FaSqliteValue value = fa_sqlite_null(fa_unit());
  value.kind = FA_SQLITE_REAL;
  value.real_value = input;
  return value;
}

static FaSqliteValue fa_sqlite_text(FaBytes input) {
  FaSqliteValue value = fa_sqlite_null(fa_unit());
  value.kind = FA_SQLITE_TEXT;
  value.bytes_value = input;
  return value;
}

static FaSqliteValue fa_sqlite_blob(FaBytes input) {
  FaSqliteValue value = fa_sqlite_null(fa_unit());
  value.kind = FA_SQLITE_BLOB;
  value.bytes_value = input;
  return value;
}

static FaFaultable_SqliteValue fa_sqlite_value_fault(const char *message) {
  return FaFaultable_SqliteValue_fault(fa_sqlite_fault_cstr(message));
}

static FaFaultable_Bytes fa_sqlite_bytes_fault(const char *message) {
  return FaFaultable_Bytes_fault(fa_sqlite_fault_cstr(message));
}

static int fa_sqlite_prepare(sqlite3 *db, FaBytes sql, const char *operation, sqlite3_stmt **stmt, FaFault *fault) {
  if (fa_sqlite_bytes_has_nul(sql)) {
    *fault = fa_sqlite_fault_cstr("std.sqlite: SQL contains NUL byte");
    return -1;
  }
  char *sql_c = fa_copy_bytes(sql.bytes, sql.len);
  const char *tail = NULL;
  int rc = sqlite3_prepare_v2(db, sql_c, -1, stmt, &tail);
  if (rc != SQLITE_OK) {
    *fault = fa_sqlite_fault(db, operation, rc);
    fa_free(sql_c);
    return -1;
  }
  if (!*stmt) {
    *fault = fa_sqlite_fault_cstr("std.sqlite: SQL did not contain a statement");
    fa_free(sql_c);
    return -1;
  }
  while (tail && *tail && isspace((unsigned char)*tail)) tail++;
  if (tail && *tail) {
    sqlite3_finalize(*stmt);
    *stmt = NULL;
    *fault = fa_sqlite_fault_cstr("std.sqlite: SQL must contain exactly one statement");
    fa_free(sql_c);
    return -1;
  }
  fa_free(sql_c);
  return 0;
}

static int fa_sqlite_bind_value(sqlite3_stmt *stmt, int index, FaSqliteValue value) {
  switch (value.kind) {
    case FA_SQLITE_NULL:
      return sqlite3_bind_null(stmt, index);
    case FA_SQLITE_INT:
      return sqlite3_bind_int64(stmt, index, (sqlite3_int64)value.int_value);
    case FA_SQLITE_REAL:
      return sqlite3_bind_double(stmt, index, value.real_value);
    case FA_SQLITE_TEXT:
      return sqlite3_bind_text64(stmt, index, value.bytes_value.bytes, (sqlite3_uint64)value.bytes_value.len, SQLITE_TRANSIENT, SQLITE_UTF8);
    case FA_SQLITE_BLOB:
      return sqlite3_bind_blob64(stmt, index, value.bytes_value.bytes, (sqlite3_uint64)value.bytes_value.len, SQLITE_TRANSIENT);
    default:
      return SQLITE_MISMATCH;
  }
}

static int fa_sqlite_bind_params(sqlite3 *db, sqlite3_stmt *stmt, FaSeq_SqliteValue params, FaFault *fault) {
  int expected = sqlite3_bind_parameter_count(stmt);
  if (expected != (int)params.count) {
    *fault = fa_sqlite_fault_cstr("std.sqlite: SQL parameter count does not match provided params");
    return -1;
  }
  for (size_t i = 0; i < params.count; i++) {
    int rc = fa_sqlite_bind_value(stmt, (int)i + 1, params.items[i]);
    if (rc != SQLITE_OK) {
      *fault = fa_sqlite_fault(db, "bind", rc);
      return -1;
    }
  }
  return 0;
}

static FaSqliteValue fa_sqlite_column_value(sqlite3_stmt *stmt, int column) {
  int kind = sqlite3_column_type(stmt, column);
  switch (kind) {
    case SQLITE_NULL:
      return fa_sqlite_null(fa_unit());
    case SQLITE_INTEGER:
      return fa_sqlite_int((int64_t)sqlite3_column_int64(stmt, column));
    case SQLITE_FLOAT:
      return fa_sqlite_real(sqlite3_column_double(stmt, column));
    case SQLITE_TEXT: {
      const unsigned char *text = sqlite3_column_text(stmt, column);
      int len = sqlite3_column_bytes(stmt, column);
      return fa_sqlite_text(fa_bytes_owned(fa_copy_bytes((const char *)(text ? text : (const unsigned char *)""), (size_t)len), (size_t)len));
    }
    case SQLITE_BLOB: {
      const void *blob = sqlite3_column_blob(stmt, column);
      int len = sqlite3_column_bytes(stmt, column);
      return fa_sqlite_blob(fa_bytes_owned(fa_copy_bytes((const char *)(blob ? blob : ""), (size_t)len), (size_t)len));
    }
    default:
      return fa_sqlite_null(fa_unit());
  }
}

static FaSqliteRow fa_sqlite_copy_row(sqlite3_stmt *stmt) {
  int count = sqlite3_column_count(stmt);
  FaSqliteRow row;
  row.count = (size_t)count;
  row.names = (FaBytes *)calloc(row.count ? row.count : 1, sizeof(FaBytes));
  row.values = (FaSqliteValue *)calloc(row.count ? row.count : 1, sizeof(FaSqliteValue));
  if (!row.names || !row.values) fa_die_alloc();
  for (int i = 0; i < count; i++) {
    const char *name = sqlite3_column_name(stmt, i);
    if (!name) name = "";
    row.names[i] = fa_bytes_literal(name, strlen(name));
    row.values[i] = fa_sqlite_column_value(stmt, i);
  }
  return row;
}

static FaFaultable_Tuple_SqliteConnection_Int fa_sqlite_exec(FaTuple_SqliteConnection_Bytes_Seq_SqliteValue input) {
  FaFault fault;
  sqlite3 *db = fa_sqlite_db(input.f0, &fault, "exec");
  if (!db) return FaFaultable_Tuple_SqliteConnection_Int_fault(fault);
  sqlite3_stmt *stmt = NULL;
  if (fa_sqlite_prepare(db, input.f1, "exec", &stmt, &fault) != 0) {
    return FaFaultable_Tuple_SqliteConnection_Int_fault(fault);
  }
  if (fa_sqlite_bind_params(db, stmt, input.f2, &fault) != 0) {
    sqlite3_finalize(stmt);
    return FaFaultable_Tuple_SqliteConnection_Int_fault(fault);
  }
  int rc = sqlite3_step(stmt);
  if (rc == SQLITE_ROW) {
    sqlite3_finalize(stmt);
    return FaFaultable_Tuple_SqliteConnection_Int_fault(fa_sqlite_fault_cstr("std.sqlite: exec returned rows; use query or query_all"));
  }
  if (rc != SQLITE_DONE) {
    fault = fa_sqlite_fault(db, "exec", rc);
    sqlite3_finalize(stmt);
    return FaFaultable_Tuple_SqliteConnection_Int_fault(fault);
  }
  rc = sqlite3_finalize(stmt);
  if (rc != SQLITE_OK) {
    return FaFaultable_Tuple_SqliteConnection_Int_fault(fa_sqlite_fault(db, "exec", rc));
  }
  FaTuple_SqliteConnection_Int value;
  value.f0 = input.f0;
  value.f1 = (int64_t)sqlite3_changes64(db);
  return FaFaultable_Tuple_SqliteConnection_Int_ok(value);
}

static int fa_sqlite_row_stream_close(void *state_ptr, FaFault *fault) {
  FaSqliteRowStreamState *state = (FaSqliteRowStreamState *)state_ptr;
  if (!state || state->finalized) return 0;
  state->finalized = true;
  int rc = SQLITE_OK;
  sqlite3 *db = state->connection ? state->connection->db : NULL;
  if (state->stmt) {
    rc = sqlite3_finalize(state->stmt);
    state->stmt = NULL;
  }
  fa_sqlite_release(state->connection);
  if (rc != SQLITE_OK) {
    *fault = fa_sqlite_fault(db, "query finalize", rc);
    return -1;
  }
  return 0;
}

static int fa_sqlite_row_stream_next(void *state_ptr, void *out, FaFault *fault) {
  FaSqliteRowStreamState *state = (FaSqliteRowStreamState *)state_ptr;
  if (!state || state->finalized || !state->stmt) return 0;
  sqlite3 *db = state->connection ? state->connection->db : NULL;
  int rc = sqlite3_step(state->stmt);
  if (rc == SQLITE_ROW) {
    *(FaSqliteRow *)out = fa_sqlite_copy_row(state->stmt);
    return 1;
  }
  if (rc == SQLITE_DONE) {
    return fa_sqlite_row_stream_close(state, fault) == 0 ? 0 : -1;
  }
  *fault = fa_sqlite_fault(db, "query step", rc);
  fa_sqlite_row_stream_close(state, fault);
  return -1;
}

static FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow fa_sqlite_query(FaTuple_SqliteConnection_Bytes_Seq_SqliteValue input) {
  FaFault fault;
  sqlite3 *db = fa_sqlite_db(input.f0, &fault, "query");
  if (!db) return FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_fault(fault);
  sqlite3_stmt *stmt = NULL;
  if (fa_sqlite_prepare(db, input.f1, "query", &stmt, &fault) != 0) {
    return FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_fault(fault);
  }
  if (fa_sqlite_bind_params(db, stmt, input.f2, &fault) != 0) {
    sqlite3_finalize(stmt);
    return FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_fault(fault);
  }
  FaSqliteRowStreamState *state = (FaSqliteRowStreamState *)calloc(1, sizeof(FaSqliteRowStreamState));
  if (!state) fa_die_alloc();
  state->stmt = stmt;
  state->connection = input.f0.state;
  state->finalized = false;
  fa_sqlite_retain(state->connection);

  FaStream stream;
  stream.file = NULL;
  stream.fd = -1;
  stream.path = fa_bytes_literal("", 0);
  stream.state = state;
  stream.map_fn = NULL;
  stream.next = fa_sqlite_row_stream_next;
  stream.close = fa_sqlite_row_stream_close;
  stream.item_size = sizeof(FaSqliteRow);
  stream.closed = false;

  FaTuple_SqliteConnection_Stream_SqliteRow value;
  value.f0 = input.f0;
  value.f1 = stream;
  return FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow_ok(value);
}

static FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow fa_sqlite_query_all(FaTuple_SqliteConnection_Bytes_Seq_SqliteValue input) {
  FaFaultable_Tuple_SqliteConnection_Stream_SqliteRow query = fa_sqlite_query(input);
  if (query.is_fault) return FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_fault(query.fault);
  FaStream stream = query.value.f1;
  size_t count = 0;
  size_t cap = 8;
  FaSqliteRow *items = (FaSqliteRow *)calloc(cap, sizeof(FaSqliteRow));
  if (!items) fa_die_alloc();
  for (;;) {
    if (count == cap) {
      cap = fa_checked_size_mul(cap, 2, "std.sqlite: row list size overflow");
      size_t bytes = fa_checked_size_mul(cap, sizeof(FaSqliteRow), "std.sqlite: row list size overflow");
      FaSqliteRow *next = (FaSqliteRow *)realloc(items, bytes);
      if (!next) fa_die_alloc();
      items = next;
    }
    FaSqliteRow row;
    FaFault fault;
    int status = stream.next(stream.state, &row, &fault);
    if (status < 0) {
      fa_stream_close(&stream, &fault);
      free(items);
      return FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_fault(fault);
    }
    if (status == 0) break;
    items[count++] = row;
  }
  FaFault close_fault;
  if (fa_stream_close(&stream, &close_fault) != 0) {
    free(items);
    return FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_fault(close_fault);
  }
  FaSeq_SqliteRow rows = FaSeq_SqliteRow_new(count);
  for (size_t i = 0; i < count; i++) rows.items[i] = items[i];
  free(items);
  FaTuple_SqliteConnection_Seq_SqliteRow value;
  value.f0 = query.value.f0;
  value.f1 = rows;
  return FaFaultable_Tuple_SqliteConnection_Seq_SqliteRow_ok(value);
}

static int64_t fa_sqlite_column_count(FaSqliteRow row) {
  return fa_checked_size_to_i64(row.count, "std.sqlite: column count exceeds Int range");
}

static FaFaultable_Bytes fa_sqlite_column_name(FaTuple_SqliteRow_Int input) {
  if (input.f1 < 0 || (size_t)input.f1 >= input.f0.count) {
    return fa_sqlite_bytes_fault("std.sqlite: column index out of range");
  }
  return FaFaultable_Bytes_ok(input.f0.names[input.f1]);
}

static FaFaultable_SqliteValue fa_sqlite_value_at(FaTuple_SqliteRow_Int input) {
  if (input.f1 < 0 || (size_t)input.f1 >= input.f0.count) {
    return fa_sqlite_value_fault("std.sqlite: column index out of range");
  }
  return FaFaultable_SqliteValue_ok(input.f0.values[input.f1]);
}

static FaFaultable_SqliteValue fa_sqlite_value_named(FaTuple_SqliteRow_Bytes input) {
  for (size_t i = 0; i < input.f0.count; i++) {
    FaBytes name = input.f0.names[i];
    if (name.len == input.f1.len && memcmp(name.bytes, input.f1.bytes, name.len) == 0) {
      return FaFaultable_SqliteValue_ok(input.f0.values[i]);
    }
  }
  return fa_sqlite_value_fault("std.sqlite: column name not found");
}

static FaBytes fa_sqlite_kind(FaSqliteValue value) {
  switch (value.kind) {
    case FA_SQLITE_NULL: return fa_bytes_literal("null", 4);
    case FA_SQLITE_INT: return fa_bytes_literal("int", 3);
    case FA_SQLITE_REAL: return fa_bytes_literal("real", 4);
    case FA_SQLITE_TEXT: return fa_bytes_literal("text", 4);
    case FA_SQLITE_BLOB: return fa_bytes_literal("blob", 4);
    default: return fa_bytes_literal("unknown", 7);
  }
}

static bool fa_sqlite_is_null(FaSqliteValue value) {
  return value.kind == FA_SQLITE_NULL;
}

static FaFaultable_Int fa_sqlite_as_int(FaSqliteValue value) {
  if (value.kind != FA_SQLITE_INT) return FaFaultable_Int_fault(fa_sqlite_fault_cstr("std.sqlite: value is not an integer"));
  return FaFaultable_Int_ok(value.int_value);
}

static FaFaultable_Real fa_sqlite_as_real(FaSqliteValue value) {
  if (value.kind != FA_SQLITE_REAL) return FaFaultable_Real_fault(fa_sqlite_fault_cstr("std.sqlite: value is not a real"));
  return FaFaultable_Real_ok(value.real_value);
}

static FaFaultable_Bytes fa_sqlite_as_text(FaSqliteValue value) {
  if (value.kind != FA_SQLITE_TEXT) return FaFaultable_Bytes_fault(fa_sqlite_fault_cstr("std.sqlite: value is not text"));
  return FaFaultable_Bytes_ok(value.bytes_value);
}

static FaFaultable_Bytes fa_sqlite_as_blob(FaSqliteValue value) {
  if (value.kind != FA_SQLITE_BLOB) return FaFaultable_Bytes_fault(fa_sqlite_fault_cstr("std.sqlite: value is not a blob"));
  return FaFaultable_Bytes_ok(value.bytes_value);
}
