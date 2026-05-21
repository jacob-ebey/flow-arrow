#include <h2o.h>
#include <h2o/http1.h>

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
