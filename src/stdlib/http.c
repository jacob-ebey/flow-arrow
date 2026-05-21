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
  return stream;
}

static FaFaultable_Int fa_http_serve(FaTuple_HttpListener_Stream_HttpResponse input) {
  (void)input;
  return FaFaultable_Int_fault(fa_fault_cstr("std.http: H2O serving loop is not implemented yet"));
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
