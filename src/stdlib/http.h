#include <h2o.h>
#include <h2o/http1.h>
#include <h2o/http2.h>
#include <arpa/inet.h>
#include <limits.h>
#include <netdb.h>
#include <netinet/in.h>
#include <openssl/err.h>
#include <openssl/ssl.h>
#include <signal.h>
#include <sys/socket.h>

typedef struct {
  FaBytes host;
  int64_t port;
  bool tls;
  FaBytes cert_path;
  FaBytes key_path;
  bool http2;
  bool http3;
} FaHttpServerConfig;

typedef struct {
  FaHttpServerConfig config;
  void *state;
} FaHttpListener;

typedef struct {
  FaBytes method;
  FaBytes path;
  FaBytes body;
  void *h2o_req;
} FaHttpRequest;

typedef struct {
  FaHttpRequest request;
  int64_t status;
  FaSeq_Bytes header_names;
  FaSeq_Bytes header_values;
  FaBytes body;
  FaBytes content_type;
} FaHttpResponse;

typedef struct {
  FaHttpRequest f0;
  FaBytes f1;
  FaBytes f2;
} FaTuple_HttpRequest_Bytes_Bytes;

typedef struct {
  bool is_fault;
  FaFault fault;
  FaHttpListener value;
} FaFaultable_HttpListener;

typedef struct {
  FaHttpListener f0;
  FaStream f1;
} FaTuple_HttpListener_Stream_HttpResponse;

typedef struct {
  FaHttpResponse f0;
  int64_t f1;
} FaTuple_HttpResponse_i64;

typedef struct {
  FaHttpResponse f0;
  FaBytes f1;
} FaTuple_HttpResponse_Bytes;

typedef struct {
  FaHttpResponse f0;
  FaBytes f1;
  FaBytes f2;
} FaTuple_HttpResponse_Bytes_Bytes;

static FaFaultable_HttpListener FaFaultable_HttpListener_ok(FaHttpListener value) {
  FaFaultable_HttpListener out;
  out.is_fault = false;
  out.value = value;
  return out;
}

static FaFaultable_HttpListener FaFaultable_HttpListener_fault(FaFault fault) {
  FaFaultable_HttpListener out;
  out.is_fault = true;
  out.fault = fault;
  return out;
}
