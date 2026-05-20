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

static FaValue *fa_write_stdout(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "write_stdout");
    size_t written = fwrite(bytes->bytes, 1, bytes->len, stdout);
    return fa_int(written == bytes->len ? 0 : 1);
}

static FaValue *fa_write_stderr(FaValue *input) {
    FaValue *bytes = fa_expect_bytes(input, "write_stderr");
    size_t written = fwrite(bytes->bytes, 1, bytes->len, stderr);
    return fa_int(written == bytes->len ? 0 : 1);
}
