# http-server

Example for the `std.http` module backed by H2O. The API typechecks today and
builds only when system H2O development files are available through pkg-config.
The full H2O serving loop is still a runtime stub that returns a clear
`Faultable[Int]` fault.

The sketch proposes this application shape:

```flow
$config                 -> http.listen        -> $listener
$listener               -> http.requests      -> $requests
$requests               -> map handle_request -> $responses
($listener, $responses) -> http.serve         -> $exit_code
```

The main design choice is to keep HTTP serving as explicit boundary nodes while
keeping per-request handling as pure FlowArrow. H2O owns protocol negotiation,
connection state, multiplexing, and response writes behind `http.listen`,
`http.requests`, and `http.serve`. User code receives a stream of request values
and produces a stream of response values.

The route example assumes a proposed general-purpose `match` construct. This is
not HTTP-specific syntax. It models static alternatives with runtime-selected
evaluation: all arms are visible to the compiler, guards are pure, all arm bodies
return the same type, and only the selected arm body runs for a given input.

The proposed `std.http` surface used by `main.flow` is:

```text
http.default_config     : () -> http.ServerConfig
http.with_tcp_listener  : (http.ServerConfig, Bytes, Int) -> http.ServerConfig
http.with_http2         : (http.ServerConfig, Bool) -> http.ServerConfig
http.with_http3         : (http.ServerConfig, Bool) -> http.ServerConfig
http.with_tls          : (http.ServerConfig, Bytes, Bytes) -> http.ServerConfig
http.listen            : http.ServerConfig -> Faultable[http.Listener]
http.requests          : http.Listener -> Stream[http.Request]
http.serve             : (http.Listener, Stream[http.Response]) -> Faultable[Int]

http.route             : (http.Request, Bytes, Bytes) -> Bool
http.body              : http.Request -> Bytes
http.response          : http.Request -> http.Response
http.with_status       : (http.Response, Int) -> http.Response
http.with_header       : (http.Response, Bytes, Bytes) -> http.Response
http.text              : (http.Response, Bytes) -> http.Response
http.json              : (http.Response, Bytes) -> http.Response
http.not_found         : http.Request -> http.Response
```

Routes are expressed with ordinary dataflow:

```flow
$req -> match {
    http.route("GET", "/health")  -> health_response
    http.route("GET", "/hello")   -> hello_response
    http.route("GET", "/created") -> created_response
    http.route("POST", "/echo")   -> echo_response
    _                             -> http.not_found
} -> $response
```

This preserves FlowArrow's static-topology rule while introducing a control
dependency that `select` does not provide. The compiler can still compile each
arm as a known subgraph, but runtime request data chooses which arm is evaluated.

Responses are built as immutable values. A handler starts with `$req ->
http.response`, sets a status, adds any number of headers, and then attaches a
body:

```flow
$req                                                  -> http.response    -> $response0
($response0, 201)                                     -> http.with_status -> $response1
($response1, "Location", "/created/123")              -> http.with_header -> $response2
($response2, "X-FlowArrow-Example", "custom-headers") -> http.with_header -> $response3
($response3, "{\"id\":123,\"created\":true}\n")        -> http.json        -> $response
```
