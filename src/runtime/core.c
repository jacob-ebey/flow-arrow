#include <math.h>
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
    FA_SEQ = 5,
    FA_FAULT = 6
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

static double fa_expect_number(FaValue *value, const char *op) {
    if (!value) {
        fprintf(stderr, "flowarrow runtime: %s expected numeric input\n", op);
        exit(65);
    }
    if (value->kind == FA_INT) {
        return (double)value->i;
    }
    if (value->kind == FA_REAL) {
        return value->real;
    }
    fprintf(stderr, "flowarrow runtime: %s expected numeric input\n", op);
    exit(65);
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

static FaValue *fa_fault_from_cstr(const char *message) {
    FaValue *value = fa_alloc(FA_FAULT);
    value->len = strlen(message);
    value->bytes = fa_copy_bytes(message, value->len);
    return value;
}
