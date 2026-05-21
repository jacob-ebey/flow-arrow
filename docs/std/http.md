# `std.http`

HTTP server boundary nodes backed by H2O.

`std.http` is an explicit I/O boundary module. It exposes listener setup,
request streams, response construction, and a serving boundary. Per-request
handlers remain ordinary FlowArrow nodes.

## Types

```text
http.ServerConfig
http.Listener
http.Request
http.Response
```

## Nodes

```text
default_config     : () -> http.ServerConfig
with_tcp_listener  : (http.ServerConfig, Bytes, Int) -> http.ServerConfig
with_tls           : (http.ServerConfig, Bytes, Bytes) -> http.ServerConfig
with_http2         : (http.ServerConfig, Bool) -> http.ServerConfig
with_http3         : (http.ServerConfig, Bool) -> http.ServerConfig
listen             : http.ServerConfig -> Faultable[http.Listener]
requests           : http.Listener -> Stream[http.Request]
serve              : (http.Listener, Stream[http.Response]) -> Faultable[Int]

route              : (http.Request, Bytes, Bytes) -> Bool
body               : http.Request -> Bytes
response           : http.Request -> http.Response
with_status        : (http.Response, Int) -> http.Response
with_header        : (http.Response, Bytes, Bytes) -> http.Response
text               : (http.Response, Bytes) -> http.Response
json               : (http.Response, Bytes) -> http.Response
not_found          : http.Request -> http.Response
```

## Semantics

`listen`, `requests`, and `serve` are boundary nodes. They make H2O connection
state, protocol negotiation, request receipt, and response writes explicit in
the graph.

`route`, `body`, and response builder nodes are pure value operations over
request and response values. Response builders are immutable: each builder
returns a new response value.

`serve` expects a response stream produced from the listener's request stream,
usually `$requests -> map handle_request -> $responses`. At runtime it casts the
stream mapper to a request handler, registers an H2O path handler, and runs the
H2O evloop accept loop.

Plain TCP listeners serve HTTP/1.x. TLS listeners install the certificate and
key on H2O's accept context; when `with_http2(true)` is set, the runtime
registers H2O's NPN/ALPN HTTP/2 protocols when the installed H2O/OpenSSL build
exposes them.

`with_http3(true)` requires an H2O build with HTTP/3 support. If the local H2O
installation does not provide that support, the program fails with a clear
diagnostic.

The implementation links through system `pkg-config` packages `libh2o-evloop`
or `libh2o` plus `openssl` and `libuv`, and compiles the generated runtime
against H2O's evloop backend.

## Example

```flow
import std.cli { Args }
import std.http as http

program main(args: Args) -> exit_code: Faultable[Int] {
    ()                         -> http.default_config    -> $config0
    ($config0, "0.0.0.0", 8080) -> http.with_tcp_listener -> $config
    $config                     -> http.listen            -> $listener
    $listener                   -> http.requests          -> $requests
    $requests                   -> map handle_request     -> $responses
    ($listener, $responses)     -> http.serve             -> $exit_code
}

node handle_request(req: http.Request) -> response: http.Response {
    $req -> match {
        http.route("GET", "/health") -> health_response
        _                            -> http.not_found
    } -> $response
}

node health_response(req: http.Request) -> response: http.Response {
    $req                            -> http.response    -> $response0
    ($response0, 200)               -> http.with_status -> $response1
    ($response1, "{\"ok\":true}\n") -> http.json        -> $response
}
```
