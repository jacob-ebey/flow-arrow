#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StaticNodeRef {
    pub base: String,
    pub args: Vec<String>,
}

pub(crate) fn format_static_node_ref(base: &str, args: &[String]) -> String {
    if args.is_empty() {
        base.to_string()
    } else {
        format!("{base}<{}>", args.join(","))
    }
}

pub(crate) fn parse_static_node_ref(text: &str) -> StaticNodeRef {
    let Some(less) = text.find('<') else {
        return StaticNodeRef {
            base: text.to_string(),
            args: Vec::new(),
        };
    };
    if !text.ends_with('>') {
        return StaticNodeRef {
            base: text.to_string(),
            args: Vec::new(),
        };
    }
    let args_text = &text[less + 1..text.len() - 1];
    let args = if args_text.trim().is_empty() {
        Vec::new()
    } else {
        args_text
            .split(',')
            .map(|arg| arg.trim().to_string())
            .collect()
    };
    StaticNodeRef {
        base: text[..less].to_string(),
        args,
    }
}
