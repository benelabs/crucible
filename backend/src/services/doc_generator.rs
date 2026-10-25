#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Sanitizes HTML content to prevent XSS attacks
/// Uses a conservative allowlist approach - only allows safe tags
fn sanitize_html(input: &str) -> String {
    // Use a simple HTML escaping for documentation content
    // This prevents any HTML/JavaScript from being interpreted. `=` is escaped as well so an
    // attribute assignment (`onerror=`, `src=`) can never be reconstructed by a consumer that
    // decodes the entities a second time; it renders back as `=`.
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
        .replace('=', "&#x3d;")
}

/// Abstract Syntax Tree representing a Soroban/Rust type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeAst {
    /// Primitive or named identifier, e.g. `u32`, `Address`, `Symbol`.
    Named(String),
    /// Generic type instantiation, e.g. `Vec<T>`, `Result<T, E>`, `Map<K, V>`.
    Generic {
        name: String,
        type_params: Vec<TypeAst>,
    },
    /// Unnamed tuple struct or tuple, e.g. `()`, `(u32, bool)`, `((A, B), (C, D))`.
    Tuple(Vec<TypeAst>),
    /// Reference type, e.g. `&str`, `&mut T`.
    Reference { is_mut: bool, inner: Box<TypeAst> },
    /// Fixed-size or slice array, e.g. `[u8; 32]`, `[T]`.
    Array {
        inner: Box<TypeAst>,
        len: Option<String>,
    },
    /// Raw unparsed fallback string (guaranteed never to panic on malformed input).
    Raw(String),
}

/// Visitor trait for recursively traversing and formatting `TypeAst`.
pub trait TypeVisitor<R> {
    fn visit(&mut self, ast: &TypeAst) -> R;
}

/// Formatter that transforms `TypeAst` into human-readable Markdown/OpenAPI type strings.
pub struct MarkdownTypeFormatter;

impl TypeVisitor<String> for MarkdownTypeFormatter {
    fn visit(&mut self, ast: &TypeAst) -> String {
        match ast {
            TypeAst::Named(name) => name.clone(),
            TypeAst::Generic { name, type_params } => {
                if type_params.is_empty() {
                    name.clone()
                } else {
                    let formatted: Vec<String> =
                        type_params.iter().map(|p| self.visit(p)).collect();
                    format!("{}<{}>", name, formatted.join(", "))
                }
            }
            TypeAst::Tuple(elems) => {
                if elems.is_empty() {
                    "()".to_string()
                } else if elems.len() == 1 {
                    format!("({},)", self.visit(&elems[0]))
                } else {
                    let formatted: Vec<String> = elems.iter().map(|e| self.visit(e)).collect();
                    format!("({})", formatted.join(", "))
                }
            }
            TypeAst::Reference { is_mut, inner } => {
                let m = if *is_mut { "mut " } else { "" };
                format!("&{}{}", m, self.visit(inner))
            }
            TypeAst::Array { inner, len } => {
                if let Some(l) = len {
                    format!("[{}; {}]", self.visit(inner), l)
                } else {
                    format!("[{}]", self.visit(inner))
                }
            }
            TypeAst::Raw(s) => s.clone(),
        }
    }
}

impl MarkdownTypeFormatter {
    pub fn format(ast: &TypeAst) -> String {
        let mut formatter = Self;
        formatter.visit(ast)
    }
}

// ---------------------------------------------------------------------------
// Robust recursive TypeParser (Zero unwraps, zero panics)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    OpenParen,
    CloseParen,
    OpenAngle,
    CloseAngle,
    OpenBracket,
    CloseBracket,
    Comma,
    Semicolon,
    Ampersand,
}

pub struct TypeParser;

impl TypeParser {
    fn tokenize(input: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut chars = input.chars().peekable();

        while let Some(&ch) = chars.peek() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    chars.next();
                }
                '(' => {
                    tokens.push(Token::OpenParen);
                    chars.next();
                }
                ')' => {
                    tokens.push(Token::CloseParen);
                    chars.next();
                }
                '<' => {
                    tokens.push(Token::OpenAngle);
                    chars.next();
                }
                '>' => {
                    tokens.push(Token::CloseAngle);
                    chars.next();
                }
                '[' => {
                    tokens.push(Token::OpenBracket);
                    chars.next();
                }
                ']' => {
                    tokens.push(Token::CloseBracket);
                    chars.next();
                }
                ',' => {
                    tokens.push(Token::Comma);
                    chars.next();
                }
                ';' => {
                    tokens.push(Token::Semicolon);
                    chars.next();
                }
                '&' => {
                    tokens.push(Token::Ampersand);
                    chars.next();
                }
                _ => {
                    let mut ident = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' || c == ':' {
                            ident.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if !ident.is_empty() {
                        tokens.push(Token::Ident(ident));
                    } else {
                        chars.next();
                    }
                }
            }
        }

        tokens
    }

    /// Parse a type signature string into a recursive `TypeAst`.
    pub fn parse(input: &str) -> TypeAst {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return TypeAst::Named("()".to_string());
        }

        let tokens = Self::tokenize(trimmed);
        if tokens.is_empty() {
            return TypeAst::Raw(trimmed.to_string());
        }

        let mut idx = 0;
        let ast = Self::parse_type(&tokens, &mut idx);

        if idx < tokens.len() {
            if let TypeAst::Raw(_) = ast {
                TypeAst::Raw(trimmed.to_string())
            } else {
                ast
            }
        } else {
            ast
        }
    }

    fn parse_type(tokens: &[Token], idx: &mut usize) -> TypeAst {
        if *idx >= tokens.len() {
            return TypeAst::Raw(String::new());
        }

        match &tokens[*idx] {
            Token::Ampersand => {
                *idx += 1;
                let is_mut = if *idx < tokens.len() {
                    if let Token::Ident(id) = &tokens[*idx] {
                        if id == "mut" {
                            *idx += 1;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                let inner = Self::parse_type(tokens, idx);
                TypeAst::Reference {
                    is_mut,
                    inner: Box::new(inner),
                }
            }
            Token::OpenParen => {
                *idx += 1; // Consume '('
                let mut elems = Vec::new();

                while *idx < tokens.len() && tokens[*idx] != Token::CloseParen {
                    let elem = Self::parse_type(tokens, idx);
                    elems.push(elem);

                    if *idx < tokens.len() && tokens[*idx] == Token::Comma {
                        *idx += 1; // Consume ','
                    } else {
                        break;
                    }
                }

                if *idx < tokens.len() && tokens[*idx] == Token::CloseParen {
                    *idx += 1; // Consume ')'
                }

                TypeAst::Tuple(elems)
            }
            Token::OpenBracket => {
                *idx += 1; // Consume '['
                let inner = Self::parse_type(tokens, idx);
                let mut len = None;

                if *idx < tokens.len() && tokens[*idx] == Token::Semicolon {
                    *idx += 1; // Consume ';'
                    if *idx < tokens.len() {
                        if let Token::Ident(l) = &tokens[*idx] {
                            len = Some(l.clone());
                            *idx += 1;
                        }
                    }
                }

                if *idx < tokens.len() && tokens[*idx] == Token::CloseBracket {
                    *idx += 1; // Consume ']'
                }

                TypeAst::Array {
                    inner: Box::new(inner),
                    len,
                }
            }
            Token::Ident(name) => {
                let ident_name = name.clone();
                *idx += 1;

                if *idx < tokens.len() && tokens[*idx] == Token::OpenAngle {
                    *idx += 1; // Consume '<'
                    let mut type_params = Vec::new();

                    while *idx < tokens.len() && tokens[*idx] != Token::CloseAngle {
                        let param = Self::parse_type(tokens, idx);
                        type_params.push(param);

                        if *idx < tokens.len() && tokens[*idx] == Token::Comma {
                            *idx += 1; // Consume ','
                        } else {
                            break;
                        }
                    }

                    if *idx < tokens.len() && tokens[*idx] == Token::CloseAngle {
                        *idx += 1; // Consume '>'
                    }

                    TypeAst::Generic {
                        name: ident_name,
                        type_params,
                    }
                } else {
                    TypeAst::Named(ident_name)
                }
            }
            _ => {
                *idx += 1;
                TypeAst::Raw("unknown".to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Domain Documentation Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDoc {
    pub name: String,
    pub type_name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDoc {
    pub name: String,
    pub description: String,
    pub params: Vec<ParamDoc>,
    pub returns: String,
    pub visibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDoc {
    pub name: String,
    pub description: String,
    pub fields: Vec<ParamDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariantDoc {
    pub name: String,
    pub description: String,
    pub fields: Vec<ParamDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDoc {
    pub name: String,
    pub description: String,
    pub variants: Vec<EnumVariantDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDoc {
    pub contract_name: String,
    pub version: String,
    pub description: String,
    pub functions: Vec<FunctionDoc>,
    pub events: Vec<EventDoc>,
    #[serde(default)]
    pub enums: Vec<EnumDoc>,
    pub generated_at: chrono::DateTime<Utc>,
}

pub struct DocGeneratorService {
    #[allow(dead_code)]
    db: PgPool,
}

impl DocGeneratorService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Parse function signature safely extracting parameter types and complex return types.
    pub fn parse_function_line(line: &str, pending_doc: &str) -> Option<FunctionDoc> {
        let trimmed = line.trim();
        if !trimmed.starts_with("pub fn ") && !trimmed.starts_with("fn ") {
            return None;
        }

        let is_pub = trimmed.starts_with("pub ");
        let without_vis = if is_pub {
            trimmed.trim_start_matches("pub ")
        } else {
            trimmed
        };
        let without_fn = without_vis.trim_start_matches("fn ").trim();

        // Extract function name and parameter slice
        let open_paren = without_fn.find('(')?;
        let fn_name = without_fn[..open_paren].trim().to_string();
        if fn_name.is_empty() {
            return None;
        }

        // Find matching closing paren for parameter list
        let rest = &without_fn[open_paren..];
        let mut depth = 0;
        let mut close_paren_idx = None;

        for (i, c) in rest.char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close_paren_idx = Some(i);
                    break;
                }
            }
        }

        let param_str = if let Some(close_idx) = close_paren_idx {
            &rest[1..close_idx]
        } else {
            ""
        };

        let mut params = Vec::new();
        if !param_str.trim().is_empty() {
            let param_tokens = Self::split_top_level_comma(param_str);
            for p in param_tokens {
                let p = p.trim();
                if p == "&self" || p == "&mut self" || p == "self" {
                    continue;
                }
                if let Some(colon_idx) = p.find(':') {
                    let name = p[..colon_idx].trim().to_string();
                    let type_raw = p[colon_idx + 1..].trim();
                    let ast = TypeParser::parse(type_raw);
                    let type_name = MarkdownTypeFormatter::format(&ast);
                    params.push(ParamDoc {
                        name,
                        type_name,
                        description: String::new(),
                    });
                }
            }
        }

        // Extract return type after matching ')'
        let after_params = if let Some(close_idx) = close_paren_idx {
            &rest[close_idx + 1..]
        } else {
            ""
        };

        let returns = if let Some(arrow_idx) = after_params.find("->") {
            let ret_slice = after_params[arrow_idx + 2..].trim();
            let ret_raw = ret_slice
                .split('{')
                .next()
                .unwrap_or(ret_slice)
                .split(';')
                .next()
                .unwrap_or(ret_slice)
                .trim();

            let ast = TypeParser::parse(ret_raw);
            MarkdownTypeFormatter::format(&ast)
        } else {
            "()".to_string()
        };

        Some(FunctionDoc {
            name: fn_name,
            description: pending_doc.trim().to_string(),
            params,
            returns,
            visibility: if is_pub {
                "public".into()
            } else {
                "private".into()
            },
        })
    }

    /// Split comma-separated tokens at depth 0 (ignoring commas inside nested (), <>, []).
    fn split_top_level_comma(input: &str) -> Vec<String> {
        let mut items = Vec::new();
        let mut current = String::new();
        let mut paren_depth: usize = 0;
        let mut angle_depth: usize = 0;
        let mut bracket_depth: usize = 0;

        for c in input.chars() {
            match c {
                '(' => {
                    paren_depth += 1;
                    current.push(c);
                }
                ')' => {
                    paren_depth = paren_depth.saturating_sub(1);
                    current.push(c);
                }
                '<' => {
                    angle_depth += 1;
                    current.push(c);
                }
                '>' => {
                    angle_depth = angle_depth.saturating_sub(1);
                    current.push(c);
                }
                '[' => {
                    bracket_depth += 1;
                    current.push(c);
                }
                ']' => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    current.push(c);
                }
                ',' if paren_depth == 0 && angle_depth == 0 && bracket_depth == 0 => {
                    items.push(current.trim().to_string());
                    current.clear();
                }
                _ => {
                    current.push(c);
                }
            }
        }

        if !current.trim().is_empty() {
            items.push(current.trim().to_string());
        }

        items
    }

    pub async fn generate(
        &self,
        contract_name: &str,
        source_code: &str,
    ) -> Result<ContractDoc, sqlx::Error> {
        let mut functions = Vec::new();
        let mut events = Vec::new();
        let mut enums = Vec::new();
        let mut description = String::new();
        let mut pending_doc = String::new();

        for line in source_code.lines() {
            let trimmed = line.trim();
            if let Some(doc) = trimmed.strip_prefix("///") {
                pending_doc.push_str(doc.trim());
                pending_doc.push(' ');
            } else if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
                if let Some(mut func_doc) = Self::parse_function_line(trimmed, &pending_doc) {
                    // Sanitize description to prevent XSS in OpenAPI docs
                    func_doc.description = sanitize_html(&func_doc.description);
                    functions.push(func_doc);
                }
                pending_doc.clear();
            } else if trimmed.contains("events().publish") {
                let event_name = trimmed
                    .split('(')
                    .nth(1)
                    .and_then(|s| s.split(',').next())
                    .unwrap_or("unknown")
                    .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                    .to_string();
                if !event_name.is_empty() {
                    events.push(EventDoc {
                        name: event_name,
                        // Sanitize event description to prevent XSS
                        description: sanitize_html(pending_doc.trim()),
                        fields: vec![],
                    });
                }
                pending_doc.clear();
            } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
                let enum_name = trimmed
                    .trim_start_matches("pub ")
                    .trim_start_matches("enum ")
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !enum_name.is_empty() {
                    enums.push(EnumDoc {
                        name: enum_name,
                        description: pending_doc.trim().to_string(),
                        variants: vec![],
                    });
                }
                pending_doc.clear();
            } else if !trimmed.is_empty() {
                if description.is_empty() && !pending_doc.is_empty() {
                    // Sanitize contract description
                    description = sanitize_html(pending_doc.trim());
                }
                pending_doc.clear();
            }
        }

        Ok(ContractDoc {
            contract_name: contract_name.to_string(),
            version: "0.1.0".to_string(),
            description,
            functions,
            events,
            enums,
            generated_at: Utc::now(),
        })
    }

    pub fn generate_markdown(&self, doc: &ContractDoc) -> String {
        let mut md = format!("# {}\n\n", doc.contract_name);
        if !doc.description.is_empty() {
            // Markdown already sanitized from generate(), but double-sanitize for safety
            md.push_str(&sanitize_html(&doc.description));
            md.push_str("\n\n");
        }
        if !doc.functions.is_empty() {
            md.push_str("## Functions\n\n");
            for f in &doc.functions {
                md.push_str(&format!("### `{}`\n\n", f.name));
                if !f.description.is_empty() {
                    md.push_str(&format!("{}\n\n", sanitize_html(&f.description)));
                }
                if !f.params.is_empty() {
                    md.push_str("**Parameters:**\n");
                    for p in &f.params {
                        md.push_str(&format!("- `{}: {}`", p.name, p.type_name));
                        if !p.description.is_empty() {
                            md.push_str(&format!(" - {}", sanitize_html(&p.description)));
                        }
                        md.push('\n');
                    }
                    md.push('\n');
                }
                md.push_str(&format!("**Returns:** `{}`\n\n", f.returns));
            }
        }
        if !doc.events.is_empty() {
            md.push_str("## Events\n\n");
            for e in &doc.events {
                md.push_str(&format!(
                    "### `{}`\n\n{}",
                    e.name,
                    sanitize_html(&e.description)
                ));
                md.push_str("\n\n");
            }
        }
        md
    }

    pub fn generate_json(&self, doc: &ContractDoc) -> serde_json::Value {
        serde_json::to_value(doc).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_html_escapes_script_tags() {
        let malicious = "<script>alert('XSS')</script>";
        let sanitized = sanitize_html(malicious);
        assert_eq!(
            sanitized,
            "&lt;script&gt;alert(&#x27;XSS&#x27;)&lt;/script&gt;"
        );
        assert!(!sanitized.contains("<script>"));
    }

    #[test]
    fn test_sanitize_html_escapes_event_handlers() {
        let malicious = "<img src=x onerror=\"alert('XSS')\">";
        let sanitized = sanitize_html(malicious);
        assert!(!sanitized.contains("onerror="));
        assert!(sanitized.contains("&lt;"));
    }

    #[test]
    fn test_sanitize_html_escapes_entities() {
        let input = "Normal & safe <content>";
        let sanitized = sanitize_html(input);
        assert_eq!(sanitized, "Normal &amp; safe &lt;content&gt;");
    }

    #[test]
    fn test_sanitize_html_preserves_text() {
        let text = "This is a normal docstring";
        let sanitized = sanitize_html(text);
        assert_eq!(sanitized, text);
    }

    #[tokio::test]
    async fn test_doc_generation_sanitizes_descriptions() {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap();
        let service = DocGeneratorService::new(db);

        let source = r#"
        /// This function <script>alert('XSS')</script> does something
        pub fn dangerous_fn() {}
        "#;

        let doc = service.generate("TestContract", source).await.unwrap();
        assert!(doc.functions.len() > 0);
        assert!(!doc.functions[0].description.contains("<script>"));
        assert!(doc.functions[0].description.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_deeply_nested_unnamed_tuple_return() {
        let sig = "pub fn query_nested_state(&self) -> ((u32, String), (Address, (bool, i128))) {";
        let doc =
            DocGeneratorService::parse_function_line(sig, "Queries deeply nested state.").unwrap();
        assert_eq!(doc.name, "query_nested_state");
        assert_eq!(doc.returns, "((u32, String), (Address, (bool, i128)))");
    }

    #[test]
    fn test_complex_result_with_nested_tuple() {
        let sig = "pub fn complex_operation(input: (Address, u64)) -> Result<((u32, Symbol), Option<(bool, i128)>), Error>;";
        let doc = DocGeneratorService::parse_function_line(sig, "").unwrap();
        assert_eq!(doc.name, "complex_operation");
        assert_eq!(doc.params.len(), 1);
        assert_eq!(doc.params[0].type_name, "(Address, u64)");
        assert_eq!(
            doc.returns,
            "Result<((u32, Symbol), Option<(bool, i128)>), Error>"
        );
    }

    #[test]
    fn test_malformed_unbalanced_tuple_does_not_panic() {
        let sig = "pub fn malformed_fn() -> ((u32, (bool);";
        let doc = DocGeneratorService::parse_function_line(sig, "");
        assert!(doc.is_some());
        let d = doc.unwrap();
        assert_eq!(d.name, "malformed_fn");
        assert!(!d.returns.is_empty());
    }

    #[test]
    fn test_markdown_generation_includes_types() {
        let doc = ContractDoc {
            contract_name: "TokenContract".to_string(),
            version: "0.1.0".to_string(),
            description: "Token documentation".to_string(),
            functions: vec![FunctionDoc {
                name: "get_account".to_string(),
                description: "Query user account info".to_string(),
                params: vec![ParamDoc {
                    name: "user".to_string(),
                    type_name: "Address".to_string(),
                    description: "".to_string(),
                }],
                returns: "((u64, bool), Result<(i32, String), Error>)".to_string(),
                visibility: "public".to_string(),
            }],
            events: vec![],
            enums: vec![],
            generated_at: Utc::now(),
        };

        let dummy_pool = sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap();
        let service = DocGeneratorService::new(dummy_pool);
        let md = service.generate_markdown(&doc);

        assert!(md.contains("### `get_account`"));
        assert!(md.contains("**Parameters:**"));
        assert!(md.contains("- `user: Address`"));
        assert!(md.contains("**Returns:** `((u64, bool), Result<(i32, String), Error>)`"));
    }
}
