FaValue *fa_builtin(const char *name, FaValue *input) {
    if (strcmp(name, "read_stdin") == 0) return fa_read_stdin(input);
    if (strcmp(name, "split_lines") == 0) return fa_split_lines(input);
    if (strcmp(name, "range_step") == 0) return fa_range_step(input);
    if (strcmp(name, "format_int") == 0) return fa_format_int(input);
    if (strcmp(name, "parse_int") == 0) return fa_parse_int(input);
    if (strcmp(name, "parse_real") == 0) return fa_parse_real(input);
    if (strcmp(name, "format_real") == 0) return fa_format_real(input);
    if (strcmp(name, "concat_bytes") == 0) return fa_concat_bytes(input);
    if (strcmp(name, "add") == 0) return fa_add(input);
    if (strcmp(name, "sub") == 0) return fa_sub(input);
    if (strcmp(name, "mul") == 0) return fa_mul(input);
    if (strcmp(name, "div") == 0) return fa_div(input);
    if (strcmp(name, "rem") == 0) return fa_rem(input);
    if (strcmp(name, "eq") == 0) return fa_eq(input);
    if (strcmp(name, "lt") == 0) return fa_lt(input);
    if (strcmp(name, "gt") == 0) return fa_gt(input);
    if (strcmp(name, "le") == 0) return fa_le(input);
    if (strcmp(name, "ge") == 0) return fa_ge(input);
    if (strcmp(name, "max") == 0) return fa_max(input);
    if (strcmp(name, "not_empty") == 0) return fa_not_empty(input);
    if (strcmp(name, "is_empty") == 0) return fa_is_empty(input);
    if (strcmp(name, "has_faults") == 0) return fa_has_faults(input);
    if (strcmp(name, "format_faults") == 0) return fa_format_faults(input);
    if (strcmp(name, "select") == 0) return fa_select(input);
    if (strcmp(name, "and") == 0) return fa_and(input);
    if (strcmp(name, "or") == 0) return fa_or(input);
    if (strcmp(name, "xor") == 0) return fa_xor(input);
    if (strcmp(name, "not") == 0) return fa_not(input);
    if (strcmp(name, "all") == 0) return fa_all(input);
    if (strcmp(name, "any") == 0) return fa_any(input);
    if (strcmp(name, "join_bytes") == 0) return fa_join_bytes(input);
    if (strcmp(name, "write_stdout") == 0) return fa_write_stdout(input);
    if (strcmp(name, "write_stderr") == 0) return fa_write_stderr(input);
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

FaValue *fa_fault_map(FaValue *input, FaValue *(*fn)(FaValue *)) {
    FaValue *seq = fa_expect_seq(input, "fault map");
    FaValue *ok = fa_seq_new((int64_t)seq->count);
    FaValue *faults = fa_seq_new((int64_t)seq->count);
    int64_t ok_count = 0;
    int64_t fault_count = 0;

    for (size_t i = 0; i < seq->count; i++) {
        if (fn == fa_parse_real) {
            double value = 0.0;
            if (fa_try_parse_real(seq->items[i], &value)) {
                fa_seq_set(ok, ok_count++, fa_real(value));
            } else {
                FaValue *bytes = fa_expect_bytes(seq->items[i], "parse_real");
                char message[512];
                snprintf(message, sizeof(message), "line %zu: expected Real, got \"%.*s\"", i + 1, (int)bytes->len, bytes->bytes);
                fa_seq_set(faults, fault_count++, fa_fault_from_cstr(message));
            }
        } else {
            fa_seq_set(ok, ok_count++, fn(seq->items[i]));
        }
    }

    FaValue *trimmed_ok = fa_seq_new(ok_count);
    for (int64_t i = 0; i < ok_count; i++) {
        fa_seq_set(trimmed_ok, i, ok->items[i]);
    }
    FaValue *trimmed_faults = fa_seq_new(fault_count);
    for (int64_t i = 0; i < fault_count; i++) {
        fa_seq_set(trimmed_faults, i, faults->items[i]);
    }
    FaValue *pair = fa_seq_new(2);
    fa_seq_set(pair, 0, trimmed_ok);
    fa_seq_set(pair, 1, trimmed_faults);
    return pair;
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
        if (identity && identity->kind == FA_INT) {
            int64_t total = fa_expect_int(identity, "reduce add identity");
            for (size_t i = 0; i < seq->count; i++) {
                total = fa_checked_integer_add(
                    total, fa_expect_int(seq->items[i], "reduce add"), "reduce add");
            }
            return fa_int(total);
        }
        double total = fa_expect_number(identity, "reduce add identity");
        for (size_t i = 0; i < seq->count; i++) {
            total += fa_expect_number(seq->items[i], "reduce add");
        }
        return fa_real(total);
    }
    fprintf(stderr, "flowarrow runtime: unsupported reduce op `%s`\n", op);
    exit(65);
}

int fa_value_to_exit_code(FaValue *value) {
    return (int)fa_expect_int(value, "program result");
}
