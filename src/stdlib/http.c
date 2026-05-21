#include "http.h"

static FaHttpServerConfig fa_http_default_config(void) {
  FaHttpServerConfig config;
  config.host = fa_bytes_literal("127.0.0.1", 9);
  config.port = 8080;
  config.tls = false;
  config.cert_path = fa_bytes_literal("", 0);
  config.key_path = fa_bytes_literal("", 0);
  config.http2 = true;
  config.http3 = false;
  return config;
}

static FaHttpServerConfig fa_http_with_tcp_listener(FaHttpServerConfig config, FaBytes host, int64_t port) {
  config.host = host;
  config.port = port;
  return config;
}

static FaHttpServerConfig fa_http_with_tls(FaHttpServerConfig config, FaBytes cert_path, FaBytes key_path) {
  config.tls = true;
  config.cert_path = cert_path;
  config.key_path = key_path;
  return config;
}

static FaHttpServerConfig fa_http_with_http2(FaHttpServerConfig config, bool enabled) {
  config.http2 = enabled;
  return config;
}

static FaHttpServerConfig fa_http_with_http3(FaHttpServerConfig config, bool enabled) {
  config.http3 = enabled;
  return config;
}

static FaFaultable_HttpListener fa_http_listen(FaHttpServerConfig config) {
  if (config.port <= 0 || config.port > 65535) {
    return FaFaultable_HttpListener_fault(fa_fault_cstr("std.http: TCP listener port must be in range 1..65535"));
  }
  if (config.host.len == 0) {
    return FaFaultable_HttpListener_fault(fa_fault_cstr("std.http: TCP listener host must not be empty"));
  }
  if (config.http3) {
#ifndef H2O_USE_QUIC
    return FaFaultable_HttpListener_fault(fa_fault_cstr("std.http: installed H2O was not built with HTTP/3 support"));
#endif
  }
  FaHttpListener listener;
  listener.config = config;
  listener.state = NULL;
  return FaFaultable_HttpListener_ok(listener);
}

static FaStream fa_http_requests(FaHttpListener listener) {
  FaStream stream;
  stream.file = NULL;
  stream.fd = -1;
  stream.path = fa_bytes_literal("", 0);
  stream.state = listener.state;
  stream.map_fn = NULL;
  stream.next = NULL;
  stream.close = NULL;
  stream.item_size = 0;
  stream.closed = false;
  return stream;
}

typedef FaHttpResponse (*FaHttpHandlerFn)(FaHttpRequest);

typedef struct {
  h2o_handler_t super;
  FaHttpHandlerFn handler;
} FaHttpFlowHandler;

typedef struct {
  h2o_globalconf_t config;
  h2o_context_t ctx;
  h2o_accept_ctx_t accept_ctx;
  h2o_evloop_t *loop;
  h2o_socket_t *listener;
  SSL_CTX *ssl_ctx;
} FaHttpServeState;

static FaFault fa_http_fault2(const char *prefix, const char *detail) {
  size_t prefix_len = strlen(prefix);
  size_t detail_len = strlen(detail);
  size_t len = prefix_len + 2 + detail_len;
  char *message = (char *)malloc(len + 1);
  if (!message) fa_die_alloc();
  memcpy(message, prefix, prefix_len);
  memcpy(message + prefix_len, ": ", 2);
  memcpy(message + prefix_len + 2, detail, detail_len);
  message[len] = '\0';
  return fa_fault_bytes(fa_bytes_owned(message, len));
}

static const char *fa_http_reason(int64_t status) {
  switch (status) {
    case 100: return "Continue";
    case 101: return "Switching Protocols";
    case 200: return "OK";
    case 201: return "Created";
    case 202: return "Accepted";
    case 204: return "No Content";
    case 301: return "Moved Permanently";
    case 302: return "Found";
    case 304: return "Not Modified";
    case 400: return "Bad Request";
    case 401: return "Unauthorized";
    case 403: return "Forbidden";
    case 404: return "Not Found";
    case 405: return "Method Not Allowed";
    case 409: return "Conflict";
    case 415: return "Unsupported Media Type";
    case 422: return "Unprocessable Content";
    case 429: return "Too Many Requests";
    case 500: return "Internal Server Error";
    case 501: return "Not Implemented";
    case 502: return "Bad Gateway";
    case 503: return "Service Unavailable";
    default:
      if (status >= 100 && status < 200) return "Informational";
      if (status >= 200 && status < 300) return "OK";
      if (status >= 300 && status < 400) return "Redirect";
      if (status >= 400 && status < 500) return "Client Error";
      if (status >= 500 && status < 600) return "Server Error";
      return "Invalid Status";
  }
}

static void fa_http_add_response_headers(h2o_req_t *req, FaHttpResponse response) {
  if (response.content_type.len > 0) {
    h2o_add_header(&req->pool, &req->res.headers, H2O_TOKEN_CONTENT_TYPE, NULL, response.content_type.bytes, response.content_type.len);
  }
  size_t header_count = response.header_names.count < response.header_values.count
      ? response.header_names.count
      : response.header_values.count;
  for (size_t i = 0; i < header_count; i++) {
    FaBytes name = response.header_names.items[i];
    FaBytes value = response.header_values.items[i];
    if (name.len == 0) continue;
    h2o_add_header_by_str(&req->pool, &req->res.headers, name.bytes, name.len, 1, name.bytes, value.bytes, value.len);
  }
}

static void fa_http_send_response(h2o_req_t *req, FaHttpResponse response) {
  int64_t status = response.status;
  if (status < 100 || status > 599) status = 500;
  req->res.status = (int)status;
  req->res.reason = fa_http_reason(status);
  fa_http_add_response_headers(req, response);
  h2o_send_inline(req, response.body.bytes, response.body.len);
}

static FaBytes fa_http_req_path(h2o_req_t *req) {
  if (req->path_normalized.base != NULL) {
    return fa_bytes_owned(req->path_normalized.base, req->path_normalized.len);
  }
  size_t len = req->path.len;
  if (req->query_at != SIZE_MAX && req->query_at < len) {
    len = req->query_at;
  }
  return fa_bytes_owned(req->path.base, len);
}

static int fa_http_on_request(h2o_handler_t *self, h2o_req_t *req) {
  FaHttpFlowHandler *handler = (FaHttpFlowHandler *)self;
  if (!handler->handler) {
    h2o_send_error_500(req, "Internal Server Error", "std.http: missing request handler\n", 0);
    return 0;
  }

  FaHttpRequest request;
  request.method = fa_bytes_owned(req->method.base, req->method.len);
  request.path = fa_http_req_path(req);
  request.body = req->entity.base != NULL
      ? fa_bytes_owned(req->entity.base, req->entity.len)
      : fa_bytes_literal("", 0);
  request.h2o_req = req;

  FaHttpResponse response = handler->handler(request);
  fa_http_send_response(req, response);
  return 0;
}

static void fa_http_on_accept(h2o_socket_t *listener, const char *err) {
  if (err != NULL) return;
  FaHttpServeState *state = (FaHttpServeState *)listener->data;
  h2o_socket_t *sock = h2o_evloop_socket_accept(listener);
  if (sock == NULL) return;
  h2o_accept(&state->accept_ctx, sock);
}

static int fa_http_create_listener(FaHttpServeState *state, FaHttpServerConfig config, FaFault *fault) {
  char port[32];
  snprintf(port, sizeof(port), "%lld", (long long)config.port);
  char *host = fa_copy_bytes(config.host.bytes, config.host.len);

  struct addrinfo hints;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_flags = AI_PASSIVE;

  struct addrinfo *result = NULL;
  int gai = getaddrinfo(host, port, &hints, &result);
  free(host);
  if (gai != 0) {
    *fault = fa_http_fault2("std.http: failed to resolve TCP listener address", gai_strerror(gai));
    return -1;
  }

  int fd = -1;
  int saved_errno = 0;
  for (struct addrinfo *addr = result; addr != NULL; addr = addr->ai_next) {
    fd = socket(addr->ai_family, addr->ai_socktype, addr->ai_protocol);
    if (fd == -1) {
      saved_errno = errno;
      continue;
    }
    int reuseaddr = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &reuseaddr, sizeof(reuseaddr));
    if (bind(fd, addr->ai_addr, addr->ai_addrlen) == 0 && listen(fd, SOMAXCONN) == 0) {
      break;
    }
    saved_errno = errno;
    close(fd);
    fd = -1;
  }
  freeaddrinfo(result);

  if (fd == -1) {
    *fault = fa_http_fault2("std.http: failed to bind TCP listener", strerror(saved_errno ? saved_errno : errno));
    return -1;
  }

  state->listener = h2o_evloop_socket_create(state->loop, fd, H2O_SOCKET_FLAG_DONT_READ);
  if (state->listener == NULL) {
    saved_errno = errno;
    close(fd);
    *fault = fa_http_fault2("std.http: failed to create H2O listener socket", strerror(saved_errno));
    return -1;
  }
  state->listener->data = state;
  h2o_socket_read_start(state->listener, fa_http_on_accept);
  return 0;
}

static int fa_http_setup_tls(FaHttpServeState *state, FaHttpServerConfig config, FaFault *fault) {
  SSL_load_error_strings();
  SSL_library_init();
  OpenSSL_add_all_algorithms();

  state->ssl_ctx = SSL_CTX_new(SSLv23_server_method());
  if (state->ssl_ctx == NULL) {
    *fault = fa_fault_cstr("std.http: failed to create OpenSSL server context");
    return -1;
  }
  SSL_CTX_set_options(state->ssl_ctx, SSL_OP_NO_SSLv2);

  char *cert_path = fa_copy_bytes(config.cert_path.bytes, config.cert_path.len);
  char *key_path = fa_copy_bytes(config.key_path.bytes, config.key_path.len);
  if (SSL_CTX_use_certificate_file(state->ssl_ctx, cert_path, SSL_FILETYPE_PEM) != 1) {
    *fault = fa_http_fault2("std.http: failed to load TLS certificate", cert_path);
    free(cert_path);
    free(key_path);
    return -1;
  }
  if (SSL_CTX_use_PrivateKey_file(state->ssl_ctx, key_path, SSL_FILETYPE_PEM) != 1) {
    *fault = fa_http_fault2("std.http: failed to load TLS private key", key_path);
    free(cert_path);
    free(key_path);
    return -1;
  }
  free(cert_path);
  free(key_path);

  if (config.http2) {
#if H2O_USE_NPN
    h2o_ssl_register_npn_protocols(state->ssl_ctx, h2o_http2_npn_protocols);
#endif
#if H2O_USE_ALPN
    h2o_ssl_register_alpn_protocols(state->ssl_ctx, h2o_http2_alpn_protocols);
#endif
  }

  state->accept_ctx.ssl_ctx = state->ssl_ctx;
  return 0;
}

static FaFaultable_Int fa_http_serve(FaTuple_HttpListener_Stream_HttpResponse input) {
  FaHttpListener listener = input.f0;
  FaStream responses = input.f1;
  FaHttpHandlerFn handler = (FaHttpHandlerFn)responses.map_fn;
  if (!handler) {
    return FaFaultable_Int_fault(fa_fault_cstr("std.http: serve expects a response stream produced by `requests -> map handler`"));
  }
  if (listener.config.http3) {
    return FaFaultable_Int_fault(fa_fault_cstr("std.http: HTTP/3 serving is not available in this H2O runtime"));
  }

  signal(SIGPIPE, SIG_IGN);

  FaHttpServeState state;
  memset(&state, 0, sizeof(state));
  h2o_config_init(&state.config);
  h2o_hostconf_t *hostconf = h2o_config_register_host(&state.config, h2o_iovec_init(H2O_STRLIT("default")), 65535);
  h2o_pathconf_t *pathconf = h2o_config_register_path(hostconf, "/", 0);
  FaHttpFlowHandler *flow_handler = (FaHttpFlowHandler *)h2o_create_handler(pathconf, sizeof(*flow_handler));
  flow_handler->super.on_req = fa_http_on_request;
  flow_handler->handler = handler;

  state.loop = h2o_evloop_create();
  if (state.loop == NULL) {
    h2o_config_dispose(&state.config);
    return FaFaultable_Int_fault(fa_fault_cstr("std.http: failed to create H2O event loop"));
  }
  h2o_context_init(&state.ctx, state.loop, &state.config);
  state.accept_ctx.ctx = &state.ctx;
  state.accept_ctx.hosts = state.config.hosts;

  FaFault fault;
  if (listener.config.tls && fa_http_setup_tls(&state, listener.config, &fault) != 0) {
    h2o_context_dispose(&state.ctx);
    h2o_evloop_destroy(state.loop);
    h2o_config_dispose(&state.config);
    return FaFaultable_Int_fault(fault);
  }
  if (fa_http_create_listener(&state, listener.config, &fault) != 0) {
    if (state.ssl_ctx) SSL_CTX_free(state.ssl_ctx);
    h2o_context_dispose(&state.ctx);
    h2o_evloop_destroy(state.loop);
    h2o_config_dispose(&state.config);
    return FaFaultable_Int_fault(fault);
  }

  fprintf(stderr, "std.http listening on %s://%.*s:%lld\n",
      listener.config.tls ? "https" : "http",
      (int)listener.config.host.len,
      listener.config.host.bytes,
      (long long)listener.config.port);

  while (h2o_evloop_run(state.loop, INT32_MAX) == 0) {
  }

  if (state.listener) h2o_socket_close(state.listener);
  if (state.ssl_ctx) SSL_CTX_free(state.ssl_ctx);
  h2o_context_dispose(&state.ctx);
  h2o_evloop_destroy(state.loop);
  h2o_config_dispose(&state.config);
  return FaFaultable_Int_ok(0);
}

static bool fa_http_route(FaTuple_HttpRequest_Bytes_Bytes input) {
  FaHttpRequest request = input.f0;
  FaBytes method = input.f1;
  FaBytes path = input.f2;
  return request.method.len == method.len
      && memcmp(request.method.bytes, method.bytes, method.len) == 0
      && request.path.len == path.len
      && memcmp(request.path.bytes, path.bytes, path.len) == 0;
}

static FaBytes fa_http_body(FaHttpRequest request) {
  return request.body;
}

static FaHttpResponse fa_http_response(FaHttpRequest request) {
  FaHttpResponse response;
  response.request = request;
  response.status = 200;
  response.header_names = FaSeq_Bytes_new(0);
  response.header_values = FaSeq_Bytes_new(0);
  response.body = fa_bytes_literal("", 0);
  response.content_type = fa_bytes_literal("", 0);
  return response;
}

static FaHttpResponse fa_http_with_status(FaTuple_HttpResponse_Int input) {
  FaHttpResponse response = input.f0;
  response.status = input.f1;
  return response;
}

static FaHttpResponse fa_http_with_header(FaTuple_HttpResponse_Bytes_Bytes input) {
  FaHttpResponse response = input.f0;
  size_t count = response.header_names.count;
  FaSeq_Bytes names = FaSeq_Bytes_new(count + 1);
  FaSeq_Bytes values = FaSeq_Bytes_new(count + 1);
  for (size_t i = 0; i < count; i++) {
    names.items[i] = response.header_names.items[i];
    values.items[i] = response.header_values.items[i];
  }
  names.items[count] = input.f1;
  values.items[count] = input.f2;
  response.header_names = names;
  response.header_values = values;
  return response;
}

static FaHttpResponse fa_http_text(FaTuple_HttpResponse_Bytes input) {
  FaHttpResponse response = input.f0;
  response.body = input.f1;
  response.content_type = fa_bytes_literal("text/plain; charset=utf-8", 25);
  return response;
}

static FaHttpResponse fa_http_json(FaTuple_HttpResponse_Bytes input) {
  FaHttpResponse response = input.f0;
  response.body = input.f1;
  response.content_type = fa_bytes_literal("application/json", 16);
  return response;
}

static FaHttpResponse fa_http_not_found(FaHttpRequest request) {
  FaHttpResponse response = fa_http_response(request);
  response.status = 404;
  response.body = fa_bytes_literal("not found\n", 10);
  response.content_type = fa_bytes_literal("text/plain; charset=utf-8", 25);
  return response;
}
