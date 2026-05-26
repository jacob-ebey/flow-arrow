#[test]
fn static_playground_examples_compile() {
    let html = std::fs::read_to_string("static/index.html").expect("static index");
    for name in ["fibExample", "concurrencyExample", "jsGlobalExample"] {
        let source = extract_template(&html, name);
        flowarrow::compile_typescript_library_source(source)
            .unwrap_or_else(|error| panic!("{name} did not compile:\n{error}"));
    }
}

fn extract_template<'a>(html: &'a str, name: &str) -> &'a str {
    let marker = format!("const {name} = `");
    let start = html
        .find(&marker)
        .unwrap_or_else(|| panic!("missing {name}"))
        + marker.len();
    let rest = &html[start..];
    let end = rest
        .find("`;\n")
        .unwrap_or_else(|| panic!("unterminated {name}"));
    &rest[..end]
}
