//! Reading a Java or Kotlin file the way the Arthas idea plugin's PSI walk does,
//! minus the PSI.
//!
//! What a command needs from the caret is small and textual: the `package`, the
//! enclosing type chain (`Outer$Inner`), whether the caret sits on a method, a
//! field or a type, and the modifiers and types around it. Everything here is
//! derived from the text, so it never waits for a language server and works on
//! unsaved buffers. Ambiguity yields `None` rather than a guess: a wrong class
//! name produces a command that silently watches nothing.

/// Longest document read (bytes).
const MAX_TEXT: usize = 4 << 20;
/// Deepest type nesting spelled out (`Outer$Middle$Inner`).
const MAX_NESTING: usize = 4;

/// Words that open a statement rather than name a method.
const NOT_A_METHOD: [&str; 19] = [
    "if", "for", "while", "switch", "catch", "synchronized", "return", "new", "do", "else", "try",
    "assert", "throw", "case", "yield", "when", "super", "this", "print",
];

/// Spring stereotypes whose `value` names the bean, as the original plugin reads
/// them.
const STEREOTYPES: [&str; 6] = [
    "@Component",
    "@Service",
    "@Controller",
    "@Repository",
    "@RestController",
    "@Configuration",
];

/// A type declaration found above the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    /// Line of the declaration itself (annotations sit above it).
    pub line: usize,
}

/// What the caret is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Class,
    Method {
        name: String,
        params: Vec<String>,
        is_static: bool,
        is_constructor: bool,
    },
    Field {
        name: String,
        ty: String,
        is_static: bool,
        is_final: bool,
        /// `set<Name>(` appears in the file.
        has_setter: bool,
    },
}

/// Everything the command catalog needs about one caret position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caret {
    /// `com.acme.OrderService$Handler`, as Arthas addresses it.
    pub class: String,
    /// The innermost type's simple name.
    pub simple: String,
    /// `package` plus the enclosing types, without the innermost: `com.acme`.
    pub pkg: String,
    pub kind: Kind,
    /// From `@Service("orderService")` or the uncapitalised class name.
    pub bean: String,
    /// Type nesting depth, outermost first — used to tell a field from a local.
    pub chain: Vec<String>,
    /// How many declarations of this method name the file has. Arthas matches
    /// by name, so `watch` hits every overload: worth saying before it does.
    pub overloads: usize,
}

impl Caret {
    /// The method name, or `*` when the caret is not on one — the original
    /// plugin's wildcard for "everything in this class".
    pub fn method_pattern(&self) -> &str {
        match &self.kind {
            Kind::Method { name, .. } => name,
            _ => "*",
        }
    }

    /// The name a command can address: the method, else the field, else empty.
    pub fn member(&self) -> &str {
        match &self.kind {
            Kind::Method { name, .. } => name,
            Kind::Field { name, .. } => name,
            Kind::Class => "",
        }
    }
}

/// The `package` declaration's name, for a Java (`;`) or Kotlin file.
pub fn package_of(text: &str) -> Option<String> {
    for line in text.lines().take(64) {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("/*") || line.starts_with('*') {
            continue;
        }
        let rest = line.strip_prefix("package ")?;
        let name = rest.trim_end_matches(';').trim();
        return (!name.is_empty()).then(|| name.to_owned());
    }
    None
}

/// The file's class name: its stem, which is what one public type per file gives.
pub fn class_of(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next()?;
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_owned())
}

/// Brace depth at the start of every line, ignoring strings and comments: a
/// brace inside a literal shifts the numbers, which at worst loses the type
/// chain (the class field stays editable) and never invents one.
fn depths(text: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for line in text.lines() {
        out.push(depth);
        for c in line.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth = (depth - 1).max(0),
                _ => {}
            }
        }
    }
    out
}

/// The type declared on `line`.
fn type_decl(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
        return None;
    }
    for keyword in ["@interface ", "class ", "interface ", "enum ", "record ", "object "] {
        let Some(at) = line.find(keyword) else { continue };
        // `myclass `, `Some.class` and `Class ` are not declarations.
        if at > 0 && !line[..at].ends_with(|c: char| c.is_whitespace() || c == '>' || c == '.') {
            continue;
        }
        let rest = &line[at + keyword.len()..];
        let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
        if name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            return Some(name);
        }
    }
    None
}

/// The types enclosing `at`, outermost first.
///
/// A declaration encloses the line only if the depth never falls back to its own
/// before reaching it, which is what keeps a closed sibling class out of the
/// chain.
fn type_chain(lines: &[&str], depths: &[i32], at: usize) -> Vec<TypeDecl> {
    let mut chain: Vec<TypeDecl> = Vec::new();
    let Some(&start) = depths.get(at) else { return chain };
    let mut low = start;
    for i in (0..at).rev() {
        if depths[i] >= low {
            continue;
        }
        low = depths[i];
        if let Some(name) = type_decl(lines[i]) {
            if !chain.iter().any(|d| d.name == name) {
                chain.push(TypeDecl { name, line: i });
                if chain.len() == MAX_NESTING {
                    break;
                }
            }
        }
        if low == 0 {
            break;
        }
    }
    chain.reverse();
    chain
}

/// Whether `line` declares a member of the innermost type rather than a local
/// inside one of its methods.
fn is_member(depths: &[i32], at: usize, decl: &TypeDecl) -> bool {
    depths.get(at).zip(depths.get(decl.line)).is_some_and(|(here, body)| *here == *body + 1)
}

/// The modifier keywords in front of a declaration, and the type text.
fn split_modifiers(head: &str) -> (String, String) {
    let mut modifiers = String::new();
    let mut ty = String::new();
    for word in head.split_whitespace() {
        match word {
            "public" | "private" | "protected" | "static" | "final" | "transient" | "volatile"
            | "abstract" | "synchronized" | "native" | "default" | "strictfp" => {
                modifiers.push_str(word);
                modifiers.push(' ');
            }
            _ => {
                ty.push_str(word);
                ty.push(' ');
            }
        }
    }
    (modifiers, ty.trim().to_owned())
}

/// The field declared on `lines[index]`, if any.
fn field_on(line: &str) -> Option<(String, String, bool, bool)> {
    let line = line.trim();
    if line.is_empty()
        || line.starts_with("//")
        || line.starts_with('*')
        || line.starts_with("/*")
        || line.starts_with("import ")
        || line.starts_with("package ")
        || line.starts_with("return ")
        || !line.ends_with(';')
        || line.contains('(')
    {
        return None;
    }
    // Drop the initialiser, then the `;`; what is left ends with the name.
    let before_init = line.split('=').next().unwrap_or(line).trim_end_matches(';').trim_end();
    let cut = before_init.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$').len();
    let name = before_init[cut..].trim();
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let (modifiers, ty) = split_modifiers(before_init[..cut].trim_end());
    if ty.is_empty() || ty.ends_with('.') {
        return None;
    }
    Some((name.to_owned(), ty, modifiers.contains("static"), modifiers.contains("final")))
}

/// The method declared on `lines[index]`: its name and its parameter types.
///
/// A declaration opens a body on this line (`{` after the parameter list, with
/// or without `throws`) or on the next one. A call (`foo(bar);`) does neither,
/// which is what separates the two; `before` additionally has to hold a return
/// type, so `obj.foo(` never looks like a declaration.
fn method_on(lines: &[&str], index: usize) -> Option<(String, Vec<String>, bool)> {
    let line = lines.get(index)?.trim();
    if line.is_empty() || line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
        return None;
    }
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close <= open {
        return None;
    }
    let tail = &line[close + 1..];
    let next = lines.get(index + 1).map(|l| l.trim()).unwrap_or_default();
    if !(tail.contains('{') || (tail.trim().is_empty() && next.starts_with('{'))) {
        return None;
    }
    let head = line[..open].trim_end();
    let at = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$').len();
    let name = &head[at..];
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) || NOT_A_METHOD.contains(&name) {
        return None;
    }
    let before = head[..at].trim_end();
    if before.is_empty() || before.ends_with('.') {
        return None;
    }
    let (modifiers, _return_type) = split_modifiers(before);
    Some((name.to_owned(), split_params(&line[open + 1..close]), modifiers.contains("static")))
}

/// Drops leading annotations, which may carry arguments of their own
/// (`@Size(max = 3)`).
fn strip_annotations(param: &str) -> &str {
    let mut rest = param.trim_start();
    while let Some(after) = rest.strip_prefix('@') {
        let mut depth = 0usize;
        let mut cut = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                c if c.is_whitespace() && depth == 0 => {
                    cut = i;
                    break;
                }
                _ => {}
            }
        }
        rest = after[cut..].trim_start();
    }
    rest
}

/// One parameter as the type Arthas matches on: `String id` → `String`,
/// `id: String` (Kotlin) → `String`.
fn param_type(param: &str) -> String {
    let mut param = strip_annotations(param);
    for prefix in ["val ", "var ", "vararg ", "final "] {
        if let Some(rest) = param.strip_prefix(prefix) {
            param = rest.trim_start();
        }
    }
    // Kotlin names the parameter first and puts no space around the colon.
    if let Some((name, ty)) = param.split_once(':') {
        let bare = name.trim();
        if !bare.is_empty() && !bare.contains(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '[') {
            return ty.trim().to_owned();
        }
    }
    // Java puts the name last and separates it with a space; generics keep their
    // own spaces, which is why the split is at the last one.
    match param.rsplit_once(char::is_whitespace) {
        Some((ty, name)) if !ty.trim().is_empty() && !name.trim().is_empty() => ty.trim().to_owned(),
        _ => param.trim().to_owned(),
    }
}

/// Splits a parameter list at top-level commas, so `Map<String, Integer> rows`
/// stays one parameter.
fn split_params(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in params.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(params[start..i].trim().to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(params[start..].trim().to_owned());
    out.iter().filter(|p| !p.is_empty()).map(|p| param_type(p)).collect()
}

/// A literal to stand in for a value of `ty` in a generated expression: what a
/// caller must replace before the command means anything.
pub fn default_value(ty: &str) -> String {
    let bare = ty.trim().trim_end_matches("[]").trim();
    match bare {
        "byte" | "short" | "int" | "long" => "0".to_owned(),
        "float" | "double" => "0.0".to_owned(),
        "boolean" => "false".to_owned(),
        "char" => "'a'".to_owned(),
        "String" | "java.lang.String" | "CharSequence" => "\"\"".to_owned(),
        _ if ty.trim().ends_with("[]") => "null".to_owned(),
        _ => "null".to_owned(),
    }
}

/// `run(String, int)` — the call an OGNL expression has to make, with each
/// argument standing in for a value the user fills in.
pub fn call_with_defaults(name: &str, params: &[String]) -> String {
    let args: Vec<String> = params.iter().map(|p| default_value(p)).collect();
    format!("{name}({})", args.join(","))
}

/// Capitalises the first letter, as `set<Name>` needs.
pub fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn uncapitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The annotations in front of the type declared at `line`: on the same line and
/// stacked above it.
fn annotations_at(lines: &[&str], line: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(text) = lines.get(line) {
        if let Some(at) = text.find(|c: char| c.is_alphanumeric() || c == '@') {
            let prefix = text[..at].trim_end();
            // `public @Service(...)` is unusual; a leading `@` is the common case.
            let _ = prefix;
        }
        let head = text.split("class ").next().unwrap_or("");
        if head.contains('@') {
            out.push(head.to_owned());
        }
    }
    let mut i = line;
    while i > 0 {
        i -= 1;
        let text = lines[i].trim();
        if text.is_empty() || text.starts_with("//") || text.starts_with('*') {
            break;
        }
        if text.starts_with('@') {
            out.push(text.to_owned());
            continue;
        }
        break;
    }
    out
}

/// The bean name a `@Service("x")`-style annotation gives, else the
/// uncapitalised class name — the original plugin's rule.
fn bean_name(annotations: &[String], class: &str) -> String {
    for annotation in annotations {
        for stereotype in STEREOTYPES {
            let Some(rest) = annotation.split(stereotype).nth(1) else { continue };
            // `("name")`, `(value = "name")`, `(value="name")`, or nothing.
            if !rest.starts_with('(') {
                return uncapitalize(class);
            }
            let quoted = rest.split('"').nth(1).unwrap_or_default().trim();
            return if quoted.is_empty() { uncapitalize(class) } else { quoted.to_owned() };
        }
    }
    uncapitalize(class)
}

/// Whether the file declares `set<Name>(` anywhere.
fn has_setter(lines: &[&str], field: &str) -> bool {
    let setter = format!("set{}(", capitalize(field));
    lines.iter().any(|l| l.contains(&setter))
}

/// Everything the catalog needs about the caret at `line` in `text`.
pub fn caret_at(path: &str, text: &str, line: u32) -> Option<Caret> {
    if text.len() > MAX_TEXT {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let at = (line as usize).min(lines.len().checked_sub(1)?);
    let depths = depths(text);
    let chain = type_chain(&lines, &depths, at);
    let stem = class_of(path)?;
    let pkg = package_of(text).unwrap_or_default();

    let inner = chain.last().cloned();
    let names = if chain.is_empty() {
        stem.clone()
    } else {
        chain.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join("$")
    };
    let simple = inner.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| stem.clone());
    let class = if pkg.is_empty() { names } else { format!("{pkg}.{names}") };
    let annotations = inner.as_ref().map(|d| annotations_at(&lines, d.line)).unwrap_or_default();
    let bean = bean_name(&annotations, &simple);

    // A field only when the caret line declares a member of the innermost type:
    // the same shape inside a method is a local variable.
    let on_field = inner.as_ref().is_some_and(|decl| is_member(&depths, at, decl));
    let kind = if on_field {
        match lines.get(at).and_then(|l| field_on(l)) {
            Some((name, ty, is_static, is_final)) => Kind::Field {
                has_setter: has_setter(&lines, &name),
                name,
                ty,
                is_static,
                is_final,
            },
            None => member_above(&lines, at, &simple),
        }
    } else {
        member_above(&lines, at, &simple)
    };

    Some(Caret {
        overloads: match &kind {
            Kind::Method { name, .. } => {
                (0..lines.len()).filter(|i| matches!(method_on(&lines, *i), Some((n, ..)) if &n == name)).count()
            }
            _ => 0,
        },
        class,
        simple,
        pkg,
        kind,
        bean,
        chain: chain.into_iter().map(|d| d.name).collect(),
    })
}

/// The nearest method above `at`, else the type itself.
///
/// A constructor is reported as such: Arthas has no `<init>` to watch, so the
/// catalog refuses those rather than emit a name that matches nothing.
fn member_above(lines: &[&str], at: usize, class: &str) -> Kind {
    let mut i = at.min(lines.len().saturating_sub(1));
    loop {
        if let Some((name, params, is_static)) = method_on(lines, i) {
            return Kind::Method { is_constructor: name == class, is_static, name, params };
        }
        if i == 0 {
            return Kind::Class;
        }
        i -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = r#"package com.demo.order;

import java.util.List;

@Service("orderService")
public class OrderService {
    @Value("${timeout}")
    private static final long TIMEOUT = 3000L;

    private static int counter;

    private String channel = "APP";

    private List<String> rows;

    private static class Handler {
        void run(String id) {
        }
    }

    public String createOrder(Order order, @Nullable User user) {
        return null;
    }

    void audit(List<String> rows, Map<String, Integer> counts) {
    }

    public void setChannel(String channel) {
        this.channel = channel;
    }
}
"#;

    fn at(text: &str, needle: &str) -> u32 {
        text.lines().position(|l| l.contains(needle)).expect("the line") as u32
    }

    fn caret(text: &str, needle: &str) -> Caret {
        caret_at("/src/com/demo/order/OrderService.java", text, at(text, needle)).expect("a caret")
    }

    #[test]
    fn a_method_caret_reports_the_class_method_and_parameters() {
        let c = caret(SERVICE, "return null;");
        assert_eq!(c.class, "com.demo.order.OrderService");
        assert_eq!(c.simple, "OrderService");
        assert_eq!(c.bean, "orderService");
        assert_eq!(c.method_pattern(), "createOrder");
        match c.kind {
            Kind::Method { ref params, is_static, is_constructor, .. } => {
                assert_eq!(params, &["Order", "User"]);
                assert!(!is_static);
                assert!(!is_constructor);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_static_final_field_caret_reports_field_modifiers_and_type() {
        let c = caret(SERVICE, "TIMEOUT = 3000L");
        match c.kind {
            Kind::Field { ref name, ref ty, is_static, is_final, has_setter } => {
                assert_eq!(name, "TIMEOUT");
                assert_eq!(ty, "long");
                assert!(is_static && is_final);
                assert!(!has_setter);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(c.method_pattern(), "*");
        assert_eq!(c.member(), "TIMEOUT");
    }

    #[test]
    fn a_field_with_a_setter_is_marked_so_the_set_command_is_offered() {
        let c = caret(SERVICE, "private String channel");
        match c.kind {
            Kind::Field { ref name, ref ty, is_static, is_final, has_setter } => {
                assert_eq!(name, "channel");
                assert_eq!(ty, "String");
                assert!(!is_static && !is_final);
                assert!(has_setter, "setChannel(String) is in the file");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_generic_field_type_is_kept_whole() {
        let c = caret(SERVICE, "private List<String> rows");
        match c.kind {
            Kind::Field { ref ty, .. } => assert_eq!(ty, "List<String>"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_local_variable_is_not_a_field() {
        // `String tmp = ...` inside a method body has the same shape as a field.
        let text = "class A {\n  void m() {\n    String tmp = \"x\";\n  }\n}\n";
        let c = caret_at("/p/A.java", text, 2).expect("a caret");
        assert!(matches!(c.kind, Kind::Method { .. }), "{:?}", c.kind);
    }

    #[test]
    fn a_caret_on_the_type_itself_is_a_class() {
        // Depth 0 and no method above: the builder offers class-level commands.
        let text = "package a.b;\n\nclass A {\n}\n";
        let c = caret_at("/p/A.java", text, 0).expect("a caret");
        assert_eq!(c.class, "a.b.A");
        assert_eq!(c.kind, Kind::Class);
    }

    #[test]
    fn a_closed_sibling_class_stays_out_of_the_chain() {
        let c = caret(SERVICE, "void audit(");
        assert_eq!(c.class, "com.demo.order.OrderService");
        assert_eq!(c.chain, ["OrderService"]);
    }

    #[test]
    fn nested_types_use_the_dollar_form_and_keep_the_package() {
        let c = caret(SERVICE, "void run(String id)");
        assert_eq!(c.class, "com.demo.order.OrderService$Handler");
        assert_eq!(c.chain, ["OrderService", "Handler"]);
        // The inner type declares no stereotype, so it falls back to its name.
        assert_eq!(c.bean, "handler");
    }

    #[test]
    fn a_constructor_is_reported_so_the_catalog_can_refuse_it() {
        let text = "class A {\n  public A(String id) {\n  }\n}\n";
        let c = caret_at("/p/A.java", text, 1).expect("a caret");
        match c.kind {
            Kind::Method { is_constructor, ref name, .. } => {
                assert!(is_constructor);
                assert_eq!(name, "A");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bean_names_come_from_the_stereotype_then_the_class() {
        assert_eq!(bean_name(&["@Service(\"svc\")".into()], "OrderService"), "svc");
        assert_eq!(bean_name(&["@Component(value = \"x\")".into()], "Foo"), "x");
        assert_eq!(bean_name(&["@Service".into()], "OrderService"), "orderService");
        assert_eq!(bean_name(&[], "OrderService"), "orderService");
        assert_eq!(bean_name(&["@Data".into()], "URLBuilder"), "uRLBuilder");
    }

    #[test]
    fn default_values_stand_in_for_each_parameter_type() {
        assert_eq!(default_value("int"), "0");
        assert_eq!(default_value("boolean"), "false");
        assert_eq!(default_value("String"), "\"\"");
        assert_eq!(default_value("long"), "0");
        assert_eq!(default_value("Order"), "null");
        assert_eq!(default_value("List<String>"), "null");
        assert_eq!(call_with_defaults("save", &["Long".into(), "String".into()]), "save(null,\"\")");
        assert_eq!(call_with_defaults("tick", &[]), "tick()");
    }

    #[test]
    fn kotlin_parameters_and_missing_semicolons() {
        let text = "package com.demo\n\nclass Repo {\n  fun save(id: Long, rows: List<String>) {\n  }\n}\n";
        assert_eq!(package_of(text).as_deref(), Some("com.demo"));
        let c = caret_at("/p/Repo.kt", text, 3).expect("a caret");
        assert_eq!(c.class, "com.demo.Repo");
        match c.kind {
            Kind::Method { ref params, .. } => assert_eq!(params, &["Long", "List<String>"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn overloads_are_counted_because_arthas_matches_by_name() {
        let text = "class A {\n  void m(int a) {\n  }\n  void m(String b) {\n  }\n  void other() {\n  }\n}\n";
        assert_eq!(caret_at("/p/A.java", text, 1).unwrap().overloads, 2);
        assert_eq!(caret_at("/p/A.java", text, 5).unwrap().overloads, 1);
        // On the type itself there is no method to overload.
        assert_eq!(caret_at("/p/A.java", text, 0).unwrap().overloads, 0);
    }
}
