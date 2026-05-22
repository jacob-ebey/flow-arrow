# c-interop

Native C interop example for LLVM-backed FlowArrow builds. The FlowArrow
program imports two C functions from `native_math.c` through
`foreign c header ... source ...`, calls them from the graph, and writes the
result to stdout.

```sh
flowarrow build examples/c-interop/main.flow
examples/c-interop/build/<host-target>/main
```

Expected output:

```text
score: 45
label: native-score:45
```
