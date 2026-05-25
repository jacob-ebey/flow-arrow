use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Type {
    Unit,
    Int,
    Real,
    Bool,
    Bytes,
    Args,
    HttpServerConfig,
    HttpListener,
    HttpRequest,
    HttpResponse,
    SqliteConnection,
    SqliteRow,
    SqliteValue,
    Stream(Box<Type>),
    Fault,
    Faultable(Box<Type>),
    Seq(Box<Type>),
    Tuple(Vec<Type>),
    Struct {
        name: String,
        fields: Vec<(String, Type)>,
    },
    OneOf(Vec<Type>),
    Var(String),
    EmptySeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Signature {
    pub input: Type,
    pub output: Type,
}

impl Type {
    pub(crate) fn contains_faultable(&self) -> bool {
        match self {
            Type::Faultable(_) => true,
            Type::Seq(item) | Type::Stream(item) => item.contains_faultable(),
            Type::Tuple(items) => items.iter().any(Type::contains_faultable),
            Type::Struct { fields, .. } => fields.iter().any(|(_, ty)| ty.contains_faultable()),
            Type::OneOf(items) => items.iter().any(Type::contains_faultable),
            _ => false,
        }
    }

    pub(crate) fn inner_faultable(&self) -> Type {
        match self {
            Type::Faultable(item) => (**item).clone(),
            other => other.clone(),
        }
    }

    pub(crate) fn strip_faultable(&self) -> Type {
        match self {
            Type::Faultable(item) => item.strip_faultable(),
            Type::Seq(item) => Type::Seq(Box::new(item.strip_faultable())),
            Type::Stream(item) => Type::Stream(Box::new(item.strip_faultable())),
            Type::Tuple(items) => Type::Tuple(items.iter().map(Type::strip_faultable).collect()),
            Type::Struct { name, fields } => Type::Struct {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(field, ty)| (field.clone(), ty.strip_faultable()))
                    .collect(),
            },
            Type::OneOf(items) => Type::OneOf(items.iter().map(Type::strip_faultable).collect()),
            other => other.clone(),
        }
    }
}

pub(crate) fn primitive_types() -> HashMap<String, Type> {
    HashMap::from([
        ("Unit".to_string(), Type::Unit),
        ("Int".to_string(), Type::Int),
        ("Real".to_string(), Type::Real),
        ("Bool".to_string(), Type::Bool),
        ("Bytes".to_string(), Type::Bytes),
        ("Fault".to_string(), Type::Fault),
        ("i1".to_string(), Type::Bool),
        ("i8".to_string(), Type::Int),
        ("i16".to_string(), Type::Int),
        ("i32".to_string(), Type::Int),
        ("i64".to_string(), Type::Int),
        ("f16".to_string(), Type::Real),
        ("float".to_string(), Type::Real),
        ("double".to_string(), Type::Real),
        ("ptr".to_string(), Type::Bytes),
        ("void".to_string(), Type::Unit),
    ])
}

pub(crate) fn stdlib_type_symbol(name: &str) -> Result<Type, String> {
    if name == "Stream" {
        Ok(Type::Stream(Box::new(Type::Var("V".to_string()))))
    } else if name == "Args" {
        Ok(Type::Args)
    } else if name == "Fault" {
        Ok(Type::Fault)
    } else if name == "ServerConfig" {
        Ok(Type::HttpServerConfig)
    } else if name == "Listener" {
        Ok(Type::HttpListener)
    } else if name == "Request" {
        Ok(Type::HttpRequest)
    } else if name == "Response" {
        Ok(Type::HttpResponse)
    } else if name == "Connection" {
        Ok(Type::SqliteConnection)
    } else if name == "Row" {
        Ok(Type::SqliteRow)
    } else if name == "Value" {
        Ok(Type::SqliteValue)
    } else {
        parse_type(name)
    }
}

pub(crate) fn is_stream_constructor(ty: &Type) -> bool {
    matches!(ty, Type::Stream(item) if matches!(item.as_ref(), Type::Var(_)))
}

pub(crate) fn single_or_tuple(mut items: Vec<Type>) -> Type {
    if items.len() == 1 {
        items.remove(0)
    } else {
        Type::Tuple(items)
    }
}

pub(crate) fn sequence_item_type(left: &Type, right: &Type) -> Result<Type, String> {
    if left == right {
        return Ok(left.clone());
    }
    match (left, right) {
        (Type::EmptySeq, other) | (other, Type::EmptySeq) => Ok(other.clone()),
        (Type::Faultable(inner), other) | (other, Type::Faultable(inner))
            if inner.as_ref() == other =>
        {
            Ok(Type::Faultable(inner.clone()))
        }
        (Type::Int, Type::Real) | (Type::Real, Type::Int) => Ok(Type::Real),
        _ => Err(format!("expected `{left}`, found `{right}`")),
    }
}

pub(crate) fn assignable_type(expected: &Type, actual: &Type) -> Result<(), String> {
    let mut vars = HashMap::new();
    match_types(expected, actual, &mut vars).or_else(|_| {
        if let Type::Faultable(inner) = expected {
            if let Some(actual) = unwrap_faultable_tuple_type(actual) {
                let mut vars = HashMap::new();
                return match_types(inner, &actual, &mut vars);
            }
            let mut vars = HashMap::new();
            match_types(inner, actual, &mut vars)
        } else {
            Err(format!("expected `{expected}`, found `{actual}`"))
        }
    })
}

pub(crate) fn common_assignable_type(
    current: &Type,
    next: &Type,
    label: &str,
) -> Result<Type, String> {
    if assignable_type(current, next).is_ok() {
        return Ok(current.clone());
    }
    if assignable_type(next, current).is_ok() {
        return Ok(next.clone());
    }
    Err(format!("{label} expected `{current}`, found `{next}`"))
}

pub(crate) fn unwrap_faultable_tuple_type(input: &Type) -> Option<Type> {
    let Type::Tuple(items) = input else {
        return None;
    };
    let mut saw_faultable = false;
    let unwrapped = items
        .iter()
        .map(|item| match item {
            Type::Faultable(inner) => {
                saw_faultable = true;
                inner.as_ref().clone()
            }
            Type::Tuple(_) => {
                if let Some(unwrapped) = unwrap_faultable_tuple_type(item) {
                    saw_faultable = true;
                    unwrapped
                } else {
                    item.clone()
                }
            }
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    saw_faultable.then_some(Type::Tuple(unwrapped))
}

pub(crate) fn contains_empty_seq(input: &Type) -> bool {
    match input {
        Type::EmptySeq => true,
        Type::Faultable(item) | Type::Seq(item) | Type::Stream(item) => contains_empty_seq(item),
        Type::Tuple(items) | Type::OneOf(items) => items.iter().any(contains_empty_seq),
        Type::Struct { fields, .. } => fields.iter().any(|(_, ty)| contains_empty_seq(ty)),
        _ => false,
    }
}

pub(crate) fn match_types(
    expected: &Type,
    actual: &Type,
    vars: &mut HashMap<String, Type>,
) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    match (expected, actual) {
        (Type::Seq(_), Type::EmptySeq) => Ok(()),
        (Type::Seq(expected), Type::Seq(actual)) if matches!(actual.as_ref(), Type::EmptySeq) => {
            match_types(expected, actual, vars)
        }
        (Type::Var(name), actual) => {
            if let Some(bound) = vars.get(name) {
                if bound == actual {
                    Ok(())
                } else if matches!(actual, Type::EmptySeq) && matches!(bound, Type::Seq(_)) {
                    Ok(())
                } else {
                    Err(format!(
                        "type variable `{name}` was `{bound}` then `{actual}`"
                    ))
                }
            } else {
                if matches!(actual, Type::EmptySeq) {
                    return Ok(());
                }
                vars.insert(name.clone(), actual.clone());
                Ok(())
            }
        }
        (Type::Faultable(expected), Type::Faultable(actual)) => match_types(expected, actual, vars),
        (Type::Seq(expected), Type::Seq(actual)) => match_types(expected, actual, vars),
        (Type::Stream(expected), Type::Stream(actual)) => match_types(expected, actual, vars),
        (Type::OneOf(expected), actual) => {
            let mut errors = Vec::new();
            for expected in expected {
                let mut candidate_vars = vars.clone();
                match match_types(expected, actual, &mut candidate_vars) {
                    Ok(()) => {
                        *vars = candidate_vars;
                        return Ok(());
                    }
                    Err(error) => errors.push(error),
                }
            }
            Err(errors.pop().unwrap_or_else(|| {
                format!(
                    "expected one of `{}`, found `{actual}`",
                    Type::OneOf(expected.clone())
                )
            }))
        }
        (expected, Type::OneOf(actual)) => {
            for actual in actual {
                match_types(expected, actual, vars)?;
            }
            Ok(())
        }
        (Type::Tuple(expected), Type::Tuple(actual)) if expected.len() == actual.len() => {
            for (expected, actual) in expected.iter().zip(actual) {
                match_types(expected, actual, vars)?;
            }
            Ok(())
        }
        (
            Type::Struct {
                name: expected_name,
                fields: expected,
            },
            Type::Struct {
                name: actual_name,
                fields: actual,
            },
        ) if expected_name == actual_name && expected.len() == actual.len() => {
            for ((expected_field, expected_ty), (actual_field, actual_ty)) in
                expected.iter().zip(actual)
            {
                if expected_field != actual_field {
                    return Err(format!(
                        "expected struct field `{expected_field}`, found `{actual_field}`"
                    ));
                }
                match_types(expected_ty, actual_ty, vars)?;
            }
            Ok(())
        }
        _ => Err(format!("expected `{expected}`, found `{actual}`")),
    }
}

pub(crate) fn substitute(ty: &Type, vars: &HashMap<String, Type>) -> Option<Type> {
    match ty {
        Type::Var(name) => vars.get(name).cloned(),
        Type::Faultable(item) => Some(Type::Faultable(Box::new(substitute(item, vars)?))),
        Type::Seq(item) => Some(Type::Seq(Box::new(substitute(item, vars)?))),
        Type::Stream(item) => Some(Type::Stream(Box::new(substitute(item, vars)?))),
        Type::OneOf(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute(item, vars)?);
            }
            Some(Type::OneOf(out))
        }
        Type::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(substitute(item, vars)?);
            }
            Some(Type::Tuple(out))
        }
        Type::Struct { name, fields } => {
            let mut out = Vec::with_capacity(fields.len());
            for (field, ty) in fields {
                out.push((field.clone(), substitute(ty, vars)?));
            }
            Some(Type::Struct {
                name: name.clone(),
                fields: out,
            })
        }
        Type::EmptySeq => Some(Type::EmptySeq),
        other => Some(other.clone()),
    }
}

pub(crate) fn substitute_partial(ty: &Type, vars: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Var(name) => vars
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::Var(name.clone())),
        Type::Faultable(item) => Type::Faultable(Box::new(substitute_partial(item, vars))),
        Type::Seq(item) => Type::Seq(Box::new(substitute_partial(item, vars))),
        Type::Stream(item) => Type::Stream(Box::new(substitute_partial(item, vars))),
        Type::OneOf(items) => Type::OneOf(
            items
                .iter()
                .map(|item| substitute_partial(item, vars))
                .collect(),
        ),
        Type::Tuple(items) => Type::Tuple(
            items
                .iter()
                .map(|item| substitute_partial(item, vars))
                .collect(),
        ),
        Type::Struct { name, fields } => Type::Struct {
            name: name.clone(),
            fields: fields
                .iter()
                .map(|(field, ty)| (field.clone(), substitute_partial(ty, vars)))
                .collect(),
        },
        other => other.clone(),
    }
}

pub(crate) fn parse_type(text: &str) -> Result<Type, String> {
    let mut parser = TypeParser {
        chars: text.chars().collect(),
        pos: 0,
    };
    let ty = parser.parse_union_type()?;
    parser.skip_ws();
    if parser.peek().is_some() {
        return Err(format!("unexpected type syntax near `{}`", parser.rest()));
    }
    Ok(ty)
}

struct TypeParser {
    chars: Vec<char>,
    pos: usize,
}

impl TypeParser {
    fn parse_union_type(&mut self) -> Result<Type, String> {
        let mut items = vec![self.parse_atom()?];
        while self.eat('|') {
            items.push(self.parse_atom()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Type::OneOf(items)
        })
    }

    fn parse_atom(&mut self) -> Result<Type, String> {
        self.skip_ws();
        match self.peek() {
            Some('(') => return self.parse_tuple_or_unit(),
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
            _ => return Err(format!("expected type, found `{}`", self.rest())),
        }

        let name = self.ident()?;
        if name == "Seq" && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Seq(Box::new(item)));
        }
        if name == "Faultable" && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Faultable(Box::new(item)));
        }
        if name.rsplit('.').next() == Some("Stream") && self.eat('[') {
            let item = self.parse_union_type()?;
            self.expect(']')?;
            return Ok(Type::Stream(Box::new(item)));
        }

        Ok(match name.as_str() {
            "Unit" => Type::Unit,
            "Int" => Type::Int,
            "Real" => Type::Real,
            "Bool" => Type::Bool,
            "Bytes" => Type::Bytes,
            "Fault" => Type::Fault,
            "Number" => Type::OneOf(vec![Type::Int, Type::Real]),
            "http.ServerConfig" => Type::HttpServerConfig,
            "http.Listener" => Type::HttpListener,
            "http.Request" => Type::HttpRequest,
            "http.Response" => Type::HttpResponse,
            "sqlite.Connection" => Type::SqliteConnection,
            "sqlite.Row" => Type::SqliteRow,
            "sqlite.Value" => Type::SqliteValue,
            "i1" => Type::Bool,
            "i8" | "i16" | "i32" | "i64" => Type::Int,
            "f16" | "float" | "double" => Type::Real,
            "ptr" => Type::Bytes,
            "void" => Type::Unit,
            _ => Type::Var(name),
        })
    }

    fn parse_tuple_or_unit(&mut self) -> Result<Type, String> {
        self.expect('(')?;
        if self.eat(')') {
            return Ok(Type::Unit);
        }
        let mut items = vec![self.parse_union_type()?];
        while self.eat(',') {
            items.push(self.parse_union_type()?);
        }
        self.expect(')')?;
        Ok(Type::Tuple(items))
    }

    fn ident(&mut self) -> Result<String, String> {
        self.skip_ws();
        let start = self.pos;
        while self
            .chars
            .get(self.pos)
            .map(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.')
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err("expected type name".to_string());
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(format!("expected `{expected}`, found `{}`", self.rest()))
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }

    fn skip_ws(&mut self) {
        while self
            .chars
            .get(self.pos)
            .map(|ch| ch.is_whitespace())
            .unwrap_or(false)
        {
            self.pos += 1;
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => write!(f, "()"),
            Type::Int => write!(f, "Int"),
            Type::Real => write!(f, "Real"),
            Type::Bool => write!(f, "Bool"),
            Type::Bytes => write!(f, "Bytes"),
            Type::Args => write!(f, "Args"),
            Type::HttpServerConfig => write!(f, "http.ServerConfig"),
            Type::HttpListener => write!(f, "http.Listener"),
            Type::HttpRequest => write!(f, "http.Request"),
            Type::HttpResponse => write!(f, "http.Response"),
            Type::SqliteConnection => write!(f, "sqlite.Connection"),
            Type::SqliteRow => write!(f, "sqlite.Row"),
            Type::SqliteValue => write!(f, "sqlite.Value"),
            Type::Stream(item) => write!(f, "Stream[{item}]"),
            Type::Fault => write!(f, "Fault"),
            Type::Faultable(item) => write!(f, "Faultable[{item}]"),
            Type::Seq(item) => write!(f, "Seq[{item}]"),
            Type::OneOf(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{item}")?;
                }
                Ok(())
            }
            Type::Tuple(items) => {
                write!(f, "(")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Type::Struct { name, .. } => write!(f, "{name}"),
            Type::Var(name) => write!(f, "{name}"),
            Type::EmptySeq => write!(f, "[]"),
        }
    }
}
