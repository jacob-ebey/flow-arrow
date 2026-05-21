mod support;

use flowarrow::{build_file, typecheck_file};
use std::fs;

const HTTP_SOURCE: &str = r#"
    import std.cli { Args }
    import std.http as http

    program main(args: Args) -> exit_code: Faultable[Int] {
        () -> http.default_config -> $config
        $config -> http.listen -> $listener
        $listener -> http.requests -> $requests
        $requests -> map handle -> $responses
        ($listener, $responses) -> http.serve -> $exit_code
    }

    node handle(req: http.Request) -> response: http.Response {
        $req -> http.response -> $response0
        ($response0, 201) -> http.with_status -> $response1
        ($response1, "X-Test", "yes") -> http.with_header -> $response2
        ($response2, "{\"ok\":true}\n") -> http.json -> $response
    }
"#;

#[test]
fn std_http_build_reports_missing_h2o_pkg_config_or_builds() {
    let path = support::source_path("http-build-diagnostic");
    fs::write(&path, HTTP_SOURCE).expect("write source");
    typecheck_file(&path).expect("typecheck");
    match build_file(&path, None) {
        Ok(_) => {}
        Err(error) => {
            assert!(
                error.contains("std.http HTTP/H2O support requires H2O"),
                "unexpected error: {error}"
            );
            assert!(error.contains("libh2o-evloop"));
            assert!(error.contains("libh2o"));
        }
    }
}
