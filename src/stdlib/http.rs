use super::*;

const MODULE: &str = "std.http";
pub const H: &[&str] = &[super::RUNTIME_H, include_str!("http.h")];
pub const C: &str = include_str!("http.c");

pub const SERVER_CONFIG: StdSymbol = ty(MODULE, "ServerConfig");
pub const LISTENER: StdSymbol = ty(MODULE, "Listener");
pub const REQUEST: StdSymbol = ty(MODULE, "Request");
pub const RESPONSE: StdSymbol = ty(MODULE, "Response");

pub const HEADER_TYPES: &[&str] = &[
    "FaHttpServerConfig",
    "FaHttpListener",
    "FaHttpRequest",
    "FaHttpResponse",
    "FaFaultable_HttpListener",
    "FaTuple_HttpListener_Stream_HttpResponse",
    "FaTuple_HttpRequest_Bytes_Bytes",
    "FaTuple_HttpResponse_Int",
    "FaTuple_HttpResponse_Bytes",
    "FaTuple_HttpResponse_Bytes_Bytes",
];

pub const DEFAULT_CONFIG: StdSymbol = io_node(MODULE, "default_config", "()", "http.ServerConfig");
pub const WITH_TCP_LISTENER: StdSymbol = io_node(
    MODULE,
    "with_tcp_listener",
    "(http.ServerConfig,Bytes,i64)",
    "http.ServerConfig",
);
pub const WITH_TLS: StdSymbol = io_node(
    MODULE,
    "with_tls",
    "(http.ServerConfig,Bytes,Bytes)",
    "http.ServerConfig",
);
pub const WITH_HTTP2: StdSymbol = io_node(
    MODULE,
    "with_http2",
    "(http.ServerConfig,Bool)",
    "http.ServerConfig",
);
pub const WITH_HTTP3: StdSymbol = io_node(
    MODULE,
    "with_http3",
    "(http.ServerConfig,Bool)",
    "http.ServerConfig",
);
pub const LISTEN: StdSymbol = io_node(
    MODULE,
    "listen",
    "http.ServerConfig",
    "Faultable[http.Listener]",
);
pub const REQUESTS: StdSymbol =
    io_node(MODULE, "requests", "http.Listener", "Stream[http.Request]");
pub const SERVE: StdSymbol = io_node(
    MODULE,
    "serve",
    "(http.Listener,Stream[http.Response])",
    "Faultable[i64]",
);

pub const ROUTE: StdSymbol = node(MODULE, "route", "(http.Request,Bytes,Bytes)", "Bool");
pub const BODY: StdSymbol = node(MODULE, "body", "http.Request", "Bytes");
pub const RESPONSE_NODE: StdSymbol = node(MODULE, "response", "http.Request", "http.Response");
pub const WITH_STATUS: StdSymbol = node(
    MODULE,
    "with_status",
    "(http.Response,i64)",
    "http.Response",
);
pub const WITH_HEADER: StdSymbol = node(
    MODULE,
    "with_header",
    "(http.Response,Bytes,Bytes)",
    "http.Response",
);
pub const TEXT: StdSymbol = node(MODULE, "text", "(http.Response,Bytes)", "http.Response");
pub const JSON: StdSymbol = node(MODULE, "json", "(http.Response,Bytes)", "http.Response");
pub const NOT_FOUND: StdSymbol = node(MODULE, "not_found", "http.Request", "http.Response");
