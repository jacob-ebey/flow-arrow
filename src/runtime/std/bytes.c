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

static FaValue *fa_join_bytes(FaValue *input) {
    FaValue *pair = fa_expect_seq(input, "join_bytes");
    if (pair->count != 2) {
        fputs("flowarrow runtime: join_bytes expected (Seq[Bytes], Bytes)\n", stderr);
        exit(65);
    }
    FaValue *seq = fa_expect_seq(pair->items[0], "join_bytes");
    FaValue *sep = fa_expect_bytes(pair->items[1], "join_bytes");
    if (seq->count == 0) {
        return fa_bytes_from_slice("", 0);
    }
    size_t total = 0;
    for (size_t i = 0; i < seq->count; i++) {
        total += fa_expect_bytes(seq->items[i], "join_bytes")->len;
    }
    total += sep->len * (seq->count - 1);
    FaValue *out = fa_alloc(FA_BYTES);
    out->len = total;
    out->bytes = (char *)malloc(total + 1);
    if (!out->bytes) {
        fputs("flowarrow runtime: allocation failed\n", stderr);
        exit(70);
    }
    size_t offset = 0;
    for (size_t i = 0; i < seq->count; i++) {
        FaValue *part = fa_expect_bytes(seq->items[i], "join_bytes");
        memcpy(out->bytes + offset, part->bytes, part->len);
        offset += part->len;
        if (i + 1 < seq->count) {
            memcpy(out->bytes + offset, sep->bytes, sep->len);
            offset += sep->len;
        }
    }
    out->bytes[total] = '\0';
    return out;
}
