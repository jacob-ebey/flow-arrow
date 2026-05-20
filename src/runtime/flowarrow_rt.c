#include <stdbool.h>
#include <ctype.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef enum {
    FA_UNIT = 0,
    FA_INT = 1,
    FA_REAL = 2,
    FA_BOOL = 3,
    FA_BYTES = 4,
    FA_SEQ = 5
} FaKind;

typedef struct FaValue {
    FaKind kind;
    int64_t i;
    double real;
    bool b;
    char *bytes;
    size_t len;
    struct FaValue **items;
    size_t count;
} FaValue;

static FaValue *fa_alloc(FaKind kind) {
    FaValue *value = (FaValue *)calloc(1, sizeof(FaValue));
    if (!value) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    value->kind = kind;
    return value;
}

FaValue *fa_unit(void) {
    static FaValue unit = { FA_UNIT, 0, 0.0, false, NULL, 0, NULL, 0 };
    return &unit;
}

FaValue *fa_int(int64_t i) {
    FaValue *value = fa_alloc(FA_INT);
    value->i = i;
    return value;
}

FaValue *fa_real(double real) {
    FaValue *value = fa_alloc(FA_REAL);
    value->real = real;
    return value;
}

FaValue *fa_bool(bool b) {
    FaValue *value = fa_alloc(FA_BOOL);
    value->b = b;
    return value;
}

FaValue *fa_cstr(const char *text) {
    FaValue *value = fa_alloc(FA_BYTES);
    value->len = strlen(text);
    value->bytes = (char *)malloc(value->len + 1);
    if (!value->bytes) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    memcpy(value->bytes, text, value->len + 1);
    return value;
}

FaValue *fa_seq_new(int64_t count) {
    if (count < 0) {
        fputs("flowarrow runtime: negative sequence size\n", stderr);
        exit(65);
    }
    FaValue *value = fa_alloc(FA_SEQ);
    value->count = (size_t)count;
    value->items = (FaValue **)calloc(value->count ? value->count : 1, sizeof(FaValue *));
    if (!value->items) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    return value;
}

void fa_seq_set(FaValue *seq, int64_t index, FaValue *item) {
    if (!seq || seq->kind != FA_SEQ || index < 0 || (size_t)index >= seq->count) {
        fputs("flowarrow runtime: invalid sequence write\n", stderr);
        exit(65);
    }
    seq->items[index] = item;
}

FaValue *fa_seq_get(FaValue *seq, int64_t index) {
    if (!seq || seq->kind != FA_SEQ || index < 0 || (size_t)index >= seq->count) {
        fputs("flowarrow runtime: invalid sequence read\n", stderr);
        exit(65);
    }
    return seq->items[index];
}

static FaValue *fa_expect_seq(FaValue *value, const char *op) {
    if (!value || value->kind != FA_SEQ) {
        fprintf(stderr, "flowarrow runtime: %s expected Seq input\n", op);
        exit(65);
    }
    return value;
}

static int64_t fa_expect_int(FaValue *value, const char *op) {
    if (!value || value->kind != FA_INT) {
        fprintf(stderr, "flowarrow runtime: %s expected Int input\n", op);
        exit(65);
    }
    return value->i;
}

static double fa_expect_real(FaValue *value, const char *op) {
    if (!value || value->kind != FA_REAL) {
        fprintf(stderr, "flowarrow runtime: %s expected Real input\n", op);
        exit(65);
    }
    return value->real;
}

static FaValue *fa_expect_bytes(FaValue *value, const char *op) {
    if (!value || value->kind != FA_BYTES) {
        fprintf(stderr, "flowarrow runtime: %s expected Bytes input\n", op);
        exit(65);
    }
    return value;
}

static char *fa_copy_bytes(const char *bytes, size_t len) {
    char *copy = (char *)malloc(len + 1);
    if (!copy) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    memcpy(copy, bytes, len);
    copy[len] = '\0';
    return copy;
}

static FaValue *fa_bytes_from_slice(const char *bytes, size_t len) {
    FaValue *value = fa_alloc(FA_BYTES);
    value->len = len;
    value->bytes = fa_copy_bytes(bytes, len);
    return value;
}

static FaValue *fa_read_stdin(FaValue *input) {
    (void)input;
    size_t cap = 4096;
    size_t len = 0;
    char *buf = (char *)malloc(cap);
    if (!buf) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    for (;;) {
        if (len == cap) {
            cap *= 2;
            char *next = (char *)realloc(buf, cap);
            if (!next) {
                fputs("flowarrow runtime: allocation failed\n", stderr);
                exit(70);
            }
            buf = next;
        }
        size_t n = fread(buf + len, 1, cap - len, stdin);
        len += n;
        if (n == 0) {
            if (ferror(stdin)) {
                fputs("flowarrow runtime: failed to read stdin\n", stderr);
                exit(74);
            }
            break;
        }
    }
    FaValue *value = fa_alloc(FA_BYTES);
    value->len = len;
    value->bytes = (char *)realloc(buf, len + 1);
    if (!value->bytes) {
        free(buf);
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    value->bytes[len] = '\0';
    return value;
}

static FaValue *fa_split_lines(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "split_lines");
    size_t count = 0;
    size_t start = 0;
    for (size_t i = 0; i < bytes->len; i++) {
        if (bytes->bytes[i] == '\n') {
            count++;
            start = i + 1;
        }
    }
    if (start < bytes->len) {
        count++;
    }

    FaValue *out = fa_seq_new((int64_t)count);
    size_t index = 0;
    start = 0;
    for (size_t i = 0; i < bytes->len; i++) {
        if (bytes->bytes[i] == '\n') {
            size_t end = i;
            if (end > start && bytes->bytes[end - 1] == '\r') {
                end--;
            }
            fa_seq_set(out, (int64_t)index++, fa_bytes_from_slice(bytes->bytes + start, end - start));
            start = i + 1;
        }
    }
    if (start < bytes->len) {
        size_t end = bytes->len;
        if (end > start && bytes->bytes[end - 1] == '\r') {
            end--;
        }
        fa_seq_set(out, (int64_t)index++, fa_bytes_from_slice(bytes->bytes + start, end - start));
    }
    return out;
}

static FaValue *fa_format_int(FaValue *input) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%lld", (long long)fa_expect_int(input, "format_int"));
    return fa_cstr(buf);
}

FaValue *fa_parse_int(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "parse_int");
    char *copy = fa_copy_bytes(bytes->bytes, bytes->len);
    char *start = copy;
    while (isspace((unsigned char)*start)) {
        start++;
    }
    errno = 0;
    char *end = NULL;
    long long value = strtoll(start, &end, 10);
    while (end && isspace((unsigned char)*end)) {
        end++;
    }
    if (start == end || errno == ERANGE || !end || *end != '\0') {
        fprintf(stderr, "flowarrow runtime: invalid Int input `%.*s`\n", (int)bytes->len, bytes->bytes);
        exit(65);
    }
    free(copy);
    return fa_int((int64_t)value);
}

FaValue *fa_parse_real(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "parse_real");
    char *copy = fa_copy_bytes(bytes->bytes, bytes->len);
    char *start = copy;
    while (isspace((unsigned char)*start)) {
        start++;
    }
    errno = 0;
    char *end = NULL;
    double value = strtod(start, &end);
    while (end && isspace((unsigned char)*end)) {
        end++;
    }
    if (start == end || errno == ERANGE || !end || *end != '\0') {
        fprintf(stderr, "flowarrow runtime: invalid Real input `%.*s`\n", (int)bytes->len, bytes->bytes);
        exit(65);
    }
    free(copy);
    return fa_real(value);
}

static FaValue *fa_format_real(FaValue *input) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%.15g", fa_expect_real(input, "format_real"));
    return fa_cstr(buf);
}

FaValue *fa_not_empty(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "not_empty");
    return fa_bool(bytes->len > 0);
}

static int64_t fa_checked_add_int(int64_t left, int64_t right, const char *op) {
    int64_t result = 0;
    if (__builtin_add_overflow(left, right, &result)) {
        fprintf(stderr, "flowarrow runtime: %s overflow\n", op);
        exit(65);
    }
    return result;
}

static FaValue *fa_concat_bytes(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "concat_bytes");
    size_t total = 0;
    for (size_t i = 0; i < seq->count; i++) {
        total += fa_expect_bytes(seq->items[i], "concat_bytes")->len;
    }
    FaValue *out = fa_alloc(FA_BYTES);
    out->len = total;
    out->bytes = (char *)malloc(total + 1);
    if (!out->bytes) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    size_t offset = 0;
    for (size_t i = 0; i < seq->count; i++) {
        FaValue *part = seq->items[i];
        memcpy(out->bytes + offset, part->bytes, part->len);
        offset += part->len;
    }
    out->bytes[total] = '\0';
    return out;
}

static FaValue *fa_add_int(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "add_int");
    if (seq->count != 2) {
        fputs("flowarrow runtime: add_int expected two inputs\n", stderr);
        exit(65);
    }
    int64_t left = fa_expect_int(seq->items[0], "add_int");
    int64_t right = fa_expect_int(seq->items[1], "add_int");
    return fa_int(fa_checked_add_int(left, right, "add_int"));
}

static FaValue *fa_sub_int(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "sub_int");
    if (seq->count != 2) {
        fputs("flowarrow runtime: sub_int expected two inputs\n", stderr);
        exit(65);
    }
    return fa_int(fa_expect_int(seq->items[0], "sub_int") - fa_expect_int(seq->items[1], "sub_int"));
}

static FaValue *fa_eq_int(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "eq_int");
    if (seq->count != 2) {
        fputs("flowarrow runtime: eq_int expected two inputs\n", stderr);
        exit(65);
    }
    return fa_bool(fa_expect_int(seq->items[0], "eq_int") == fa_expect_int(seq->items[1], "eq_int"));
}

static FaValue *fa_select(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "select");
    if (seq->count != 3 || !seq->items[0] || seq->items[0]->kind != FA_BOOL) {
        fputs("flowarrow runtime: select expected (Bool, T, T)\n", stderr);
        exit(65);
    }
    return seq->items[0]->b ? seq->items[1] : seq->items[2];
}

static FaValue *fa_range_step(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "range_step");
    if (seq->count != 3) {
        fputs("flowarrow runtime: range_step expected three Int inputs\n", stderr);
        exit(65);
    }
    int64_t start = fa_expect_int(seq->items[0], "range_step");
    int64_t stop = fa_expect_int(seq->items[1], "range_step");
    int64_t step = fa_expect_int(seq->items[2], "range_step");
    if (step == 0) {
        fputs("flowarrow runtime: range_step step cannot be zero\n", stderr);
        exit(65);
    }
    int64_t count = 0;
    for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) {
        count++;
    }
    FaValue *out = fa_seq_new(count);
    int64_t index = 0;
    for (int64_t i = start; step > 0 ? i < stop : i > stop; i += step) {
        fa_seq_set(out, index++, fa_int(i));
    }
    return out;
}

static FaValue *fa_write_stdout(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "write_stdout");
    size_t written = fwrite(bytes->bytes, 1, bytes->len, stdout);
    return fa_int(written == bytes->len ? 0 : 1);
}

FaValue *fa_builtin(const char *name, FaValue *input) {
    if (strcmp(name, "read_stdin") == 0) return fa_read_stdin(input);
    if (strcmp(name, "split_lines") == 0) return fa_split_lines(input);
    if (strcmp(name, "range_step") == 0) return fa_range_step(input);
    if (strcmp(name, "format_int") == 0) return fa_format_int(input);
    if (strcmp(name, "parse_int") == 0) return fa_parse_int(input);
    if (strcmp(name, "parse_real") == 0) return fa_parse_real(input);
    if (strcmp(name, "format_real") == 0) return fa_format_real(input);
    if (strcmp(name, "concat_bytes") == 0) return fa_concat_bytes(input);
    if (strcmp(name, "add_int") == 0) return fa_add_int(input);
    if (strcmp(name, "sub_int") == 0) return fa_sub_int(input);
    if (strcmp(name, "eq_int") == 0) return fa_eq_int(input);
    if (strcmp(name, "not_empty") == 0) return fa_not_empty(input);
    if (strcmp(name, "select") == 0) return fa_select(input);
    if (strcmp(name, "write_stdout") == 0) return fa_write_stdout(input);
    fprintf(stderr, "flowarrow runtime: unknown builtin `%s`\n", name);
    exit(65);
}

FaValue *fa_map(FaValue *input, FaValue *(*fn)(FaValue *)) {
    FaValue *seq = fa_expect_seq(input, "map");
    FaValue *out = fa_seq_new((int64_t)seq->count);
    for (size_t i = 0; i < seq->count; i++) {
        fa_seq_set(out, (int64_t)i, fn(seq->items[i]));
    }
    return out;
}

FaValue *fa_filter(FaValue *input, FaValue *(*pred)(FaValue *)) {
    FaValue *seq = fa_expect_seq(input, "filter");
    size_t count = 0;
    for (size_t i = 0; i < seq->count; i++) {
        FaValue *keep = pred(seq->items[i]);
        if (!keep || keep->kind != FA_BOOL) {
            fputs("flowarrow runtime: filter predicate must return Bool\n", stderr);
            exit(65);
        }
        if (keep->b) {
            count++;
        }
    }
    FaValue *out = fa_seq_new((int64_t)count);
    size_t index = 0;
    for (size_t i = 0; i < seq->count; i++) {
        if (pred(seq->items[i])->b) {
            fa_seq_set(out, (int64_t)index++, seq->items[i]);
        }
    }
    return out;
}

FaValue *fa_repeat(FaValue *initial, FaValue *count_value, FaValue *(*step)(FaValue *)) {
    int64_t count = fa_expect_int(count_value, "repeat count");
    if (count < 0) {
        fputs("flowarrow runtime: repeat count cannot be negative\n", stderr);
        exit(65);
    }
    FaValue *state = initial;
    for (int64_t i = 0; i < count; i++) {
        state = step(state);
    }
    return state;
}

FaValue *fa_reduce(FaValue *input, const char *op, FaValue *identity) {
    FaValue *seq = fa_expect_seq(input, "reduce");
    if (seq->count == 0) {
        return identity;
    }
    if (strcmp(op, "concat_bytes") == 0) {
        return fa_concat_bytes(seq);
    }
    if (strcmp(op, "add") == 0) {
        double total = fa_expect_real(identity, "reduce add identity");
        for (size_t i = 0; i < seq->count; i++) {
            total += fa_expect_real(seq->items[i], "reduce add");
        }
        return fa_real(total);
    }
    if (strcmp(op, "add_int") == 0) {
        int64_t total = fa_expect_int(identity, "reduce add_int identity");
        for (size_t i = 0; i < seq->count; i++) {
            total = fa_checked_add_int(total, fa_expect_int(seq->items[i], "reduce add_int"), "reduce add_int");
        }
        return fa_int(total);
    }
    fprintf(stderr, "flowarrow runtime: unsupported reduce op `%s`\n", op);
    exit(65);
}

int fa_value_to_exit_code(FaValue *value) {
    return (int)fa_expect_int(value, "program result");
}
