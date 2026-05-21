# sqlite-todos

Example for `std.sqlite`, backed by system SQLite 3 through `pkg-config`.

The example opens an in-memory database, creates a table, inserts parameterized
rows, queries a row stream, maps rows into text output, materializes the stream
with `stream.to_seq`, writes the result, and closes the connection.

Build and run:

```sh
cargo run -- build examples/sqlite-todos/main.flow
examples/sqlite-todos/build/<host-target>/main
```

The program prints:

```text
todo 1: write sqlite docs
todo 2: implement sqlite runtime
```
