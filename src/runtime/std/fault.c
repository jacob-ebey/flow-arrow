static FaValue *fa_has_faults(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "has_faults");
    return fa_bool(seq->count > 0);
}

static FaValue *fa_format_faults(FaValue *input) {
    FaValue *seq = fa_expect_seq(input, "format_faults");
    size_t total = 0;
    for (size_t i = 0; i < seq->count; i++) {
        FaValue *fault = seq->items[i];
        if (!fault || fault->kind != FA_FAULT) {
            fputs("flowarrow runtime: format_faults expected Fault items\n", stderr);
            exit(65);
        }
        total += fault->len + 1;
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
        FaValue *fault = seq->items[i];
        memcpy(out->bytes + offset, fault->bytes, fault->len);
        offset += fault->len;
        out->bytes[offset++] = '\n';
    }
    out->bytes[offset] = '\0';
    return out;
}
