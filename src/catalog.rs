//! The command catalog: every shape the Arthas idea plugin can generate, as
//! templates plus the values behind them.
//!
//! Faithful to the original's `ShellScriptCommandEnum`: same flags, same default
//! values, same wildcard (`*`) for "every method of the class". Where the
//! original glues `" -x "` onto `" -n N "` and leaves a double space and a
//! trailing space, these render as one space — the command a shell sees is the
//! same, the text is just clean when pasted.
//!
//! One family can have several variants (the original's separate menu actions):
//! `watch` covers call, stack, static field, instance field and both
//! "execute-the-method" shapes. The panel switches between them locally, which
//! is why a family ships every variant at once.

use serde_json::{Value, json};

use crate::java::{Caret, Kind, call_with_defaults, capitalize, default_value};

/// A generated command, ready for the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub label: String,
    /// One line on what it is for, and what to look for in the output.
    pub describe: String,
    pub doc: String,
    pub variants: Vec<Variant>,
}

/// One shape of a family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub key: String,
    pub label: String,
    pub describe: String,
    /// `${NAME}` markers, replaced literally.
    pub template: String,
    pub fields: Vec<Field>,
}

/// A value the panel shows and the user may edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub value: String,
    /// For a select: the choices, `(value, label)`.
    pub choices: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Select,
}

impl Command {
    /// The command as the page sees it. The envelope (`type`, caret, focus) is
    /// the caller's job: one message carries every family at once, so a `type`
    /// on each entry would just be repeated noise.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "describe": self.describe,
            "doc": self.doc,
            "variants": self.variants.iter().map(Variant::to_json).collect::<Vec<_>>(),
        })
    }
}

impl Variant {
    /// The command this variant produces, with its current values.
    ///
    /// Arguments in, one line out: a shell never sees an unreplaced marker
    /// because the page only edits values, never the template.
    pub fn render(&self) -> String {
        let mut out = self.template.clone();
        for field in &self.fields {
            out = out.replace(&format!("${{{}}}", field.key), &field.value);
        }
        tidy(&out)
    }

    fn new(key: &str, label: &str, describe: &str, template: String) -> Variant {
        Variant { key: key.to_owned(), label: label.to_owned(), describe: describe.to_owned(), template, fields: Vec::new() }
    }

    fn field(mut self, key: &str, label: &str, kind: FieldKind, value: impl Into<String>) -> Variant {
        self.fields.push(Field {
            key: key.to_owned(),
            label: label.to_owned(),
            kind,
            value: value.into(),
            choices: Vec::new(),
        });
        self
    }

    fn select(mut self, key: &str, label: &str, value: &str, choices: &[(&str, &str)]) -> Variant {
        self.fields.push(Field {
            key: key.to_owned(),
            label: label.to_owned(),
            kind: FieldKind::Select,
            value: value.to_owned(),
            choices: choices.iter().map(|(v, l)| ((*v).to_owned(), (*l).to_owned())).collect(),
        });
        self
    }

    fn to_json(&self) -> Value {
        json!({
            "key": self.key,
            "label": self.label,
            "describe": self.describe,
            "template": self.template,
            // What these values produce. The page can compute it too — it does,
            // as soon as a field is edited — but shipping the first paint from
            // here keeps one renderer authoritative for "as generated".
            "rendered": self.render(),
            "fields": self.fields.iter().map(|f| json!({
                "key": f.key,
                "label": f.label,
                "kind": match f.kind { FieldKind::Text => "text", FieldKind::Number => "number", FieldKind::Select => "select" },
                "value": f.value,
                "choices": f.choices.iter().map(|(v, l)| json!([v, l])).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// The settings a command is built with — the original plugin's `AppSettingsState`.
#[derive(Clone, Debug)]
pub struct Settings {
    /// `-x`, how deep the result is expanded.
    pub depth: String,
    /// `-n` for watch/trace/stack/tt.
    pub count: String,
    /// `-n` for monitor.
    pub monitor_count: String,
    /// `--cycle` for monitor, in seconds.
    pub monitor_interval: String,
    /// `-v`: print the condition's result too.
    pub verbose: bool,
    /// Append `'1==1'` as the condition.
    pub condition: bool,
    /// Keep JDK frames in `trace`. Off adds `--skipJDKMethod false`.
    pub skip_jdk: bool,
    /// The OGNL expression that yields the Spring context.
    pub spring: String,
    /// The value field/logging commands write.
    pub value: String,
    /// `logger --level`.
    pub level: String,
    /// `profiler --event`.
    pub event: String,
    /// `profiler stop --format`.
    pub format: String,
    /// `profiler ... --duration`, in seconds.
    pub duration: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            depth: "3".into(),
            count: "5".into(),
            monitor_count: "10".into(),
            monitor_interval: "10".into(),
            verbose: false,
            condition: true,
            skip_jdk: false,
            spring: "@applicationContextProvider@context".into(),
            value: String::new(),
            level: "debug".into(),
            event: "cpu".into(),
            format: "html".into(),
            duration: "30".into(),
        }
    }
}

impl Settings {
    /// `-v`, or nothing.
    fn verbose(&self) -> String {
        if self.verbose { "-v".to_owned() } else { String::new() }
    }

    /// `'1==1'`, or nothing — the original's placeholder condition.
    fn condition(&self) -> String {
        if self.condition { "'1==1'".to_owned() } else { String::new() }
    }
}

/// Collapses runs of whitespace to one space, but only outside quotes.
///
/// The original concatenates fragments, so an optional flag that is off leaves
/// `"  "` behind; a shell does not care, a person pasting it does. Blanking
/// every run would be wrong the other way round: the Spring dump pads its
/// output with `#allProperties.add("                ")`, and OGNL string
/// literals may hold anything at all. Inside `'…'` and `"…"` — with `\` escapes
/// honoured — every character is kept.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut gap = false;

    for ch in text.chars() {
        if let Some(open) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                quote = Some(ch);
                out.push(ch);
            }
            // Leading and trailing whitespace never reaches `out`: it only
            // arms `gap`, which is spent by the next visible character.
            c if c.is_whitespace() => gap = true,
            c => {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                out.push(c);
            }
        }
    }
    out
}

/// `[{params,returnObj,throwExp}]`, quoted, or a custom expression.
fn observe_default() -> Vec<(&'static str, &'static str)> {
    vec![
        ("params,returnObj,throwExp", "入参 / 返回值 / 异常"),
        ("params", "只看入参"),
        ("returnObj", "只看返回值"),
        ("throwExp", "只看异常"),
        ("params,returnObj,throwExp,target", "连当前对象一起看"),
        ("new Throwable()", "打印调用栈"),
    ]
}

// ── Families ────────────────────────────────────────────────────────────────

/// `watch`, in the six shapes the original splits across menu actions.
pub fn watch(c: &Caret, s: &Settings) -> Option<Command> {
    let class = &c.class;
    let mut variants = Vec::new();

    // A method, or `*` when the caret is on the class or a field.
    let method = c.method_pattern();
    variants.push(
        Variant::new(
            "call",
            "方法调用",
            "看入参、返回值、抛出的异常：定位某个值不对，或异常没被捕获。",
            "watch ${CLASS} ${METHOD} '${OBSERVE}' ${V} -n ${COUNT} -x ${DEPTH} ${CONDITION}".to_owned(),
        )
        .field("CLASS", "类", FieldKind::Text, class)
        .field("METHOD", "方法（* 表示全部）", FieldKind::Text, method)
        .select("OBSERVE", "观测项", "params,returnObj,throwExp", &observe_default())
        .field("V", "-v（同时打印条件结果）", FieldKind::Text, s.verbose())
        .field("COUNT", "-n 次数", FieldKind::Number, &s.count)
        .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth)
        .field("CONDITION", "条件（OGNL）", FieldKind::Text, s.condition()),
    );

    if let Kind::Field { name, is_static, .. } = &c.kind {
        // A static field is read straight off the class; an instance field needs
        // an executing method to look at, hence the modifier guard the original
        // appends.
        let (expr, guard) = if *is_static {
            (format!("params,returnObj,throwExp,@{class}@{name}"), String::new())
        } else {
            (
                format!("params,returnObj,throwExp,target.{name}"),
                "'method.initMethod(),method.constructor!=null || !@java.lang.reflect.Modifier@isStatic(method.method.getModifiers())'".to_owned(),
            )
        };
        variants.push(
            Variant::new(
                "field",
                if *is_static { "静态字段" } else { "实例字段" },
                "方法执行时顺带看这个字段的值：判断它是不是被别处改坏了。",
                "watch ${CLASS} ${METHOD} '${OBSERVE}' ${V} -n ${COUNT} -x ${DEPTH} ${CONDITION}"
                    .to_owned(),
            )
            .field("CLASS", "类", FieldKind::Text, class)
            .field("METHOD", "方法（* 表示全部）", FieldKind::Text, "*")
            .field("OBSERVE", "观测项（含字段）", FieldKind::Text, expr)
            .field("V", "-v", FieldKind::Text, s.verbose())
            .field("COUNT", "-n 次数", FieldKind::Number, &s.count)
            .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth)
            .field("CONDITION", "条件", FieldKind::Text, if guard.is_empty() { s.condition() } else { guard }),
        );
    }

    // Call the watched method instead of waiting for it.
    if let Kind::Method { name, params, is_static, is_constructor } = &c.kind {
        if !is_constructor {
            let call = call_with_defaults(name, params);
            let (expr, guard) = if *is_static {
                (format!("params,returnObj,throwExp,@{class}@{call}"), String::new())
            } else {
                (
                    format!("params,returnObj,throwExp,target.{call}"),
                    "'method.initMethod(),method.constructor!=null || !@java.lang.reflect.Modifier@isStatic(method.method.getModifiers())'".to_owned(),
                )
            };
            variants.push(
                Variant::new(
                    "invoke",
                    if *is_static { "主动执行静态方法" } else { "主动执行方法" },
                    "不用等业务触发，任何方法被调用时就顺手执行一次这个方法：改缓存、刷新配置都能这么干。参数要自己填。",
                    "watch ${CLASS} * '${OBSERVE}' ${V} -n ${COUNT} -x ${DEPTH} ${CONDITION}".to_owned(),
                )
                .field("CLASS", "类", FieldKind::Text, class)
                .field("OBSERVE", "观测项（含执行的方法）", FieldKind::Text, expr)
                .field("V", "-v", FieldKind::Text, s.verbose())
                .field("COUNT", "-n 次数", FieldKind::Number, &s.count)
                .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth)
                .field("CONDITION", "条件", FieldKind::Text, if guard.is_empty() { s.condition() } else { guard }),
            );
        }
    }

    Some(Command {
        id: "watch".into(),
        label: "Watch".into(),
        describe: "观察指定方法的调用：入参、返回值、抛出的异常。".into(),
        doc: "https://arthas.aliyun.com/doc/watch.html".into(),
        variants,
    })
}

/// `trace` — the inner call path with per-node cost.
pub fn trace(c: &Caret, s: &Settings) -> Option<Command> {
    Some(Command {
        id: "trace".into(),
        label: "Trace".into(),
        describe: "方法内部调用路径与每个节点耗时：查性能热点、理清调用链。".into(),
        doc: "https://arthas.aliyun.com/doc/trace.html".into(),
        variants: vec![
            Variant::new(
                "trace",
                "调用路径",
                "输出方法内部的调用树与各节点耗时，耗时的节点是优化目标。",
                "trace ${CLASS} ${METHOD} ${V} -n ${COUNT} ${SKIPJDK} ${CONDITION}".to_owned(),
            )
            .field("CLASS", "类", FieldKind::Text, &c.class)
            .field("METHOD", "方法（* 表示全部）", FieldKind::Text, c.method_pattern())
            .field("V", "-v", FieldKind::Text, s.verbose())
            .field("COUNT", "-n 次数", FieldKind::Number, &s.count)
            .select(
                "SKIPJDK",
                "JDK 方法",
                if s.skip_jdk { "" } else { "--skipJDKMethod false" },
                &[("--skipJDKMethod false", "包含 JDK 方法"), ("", "只看业务代码")],
            )
            .field("CONDITION", "条件（如 '#cost>50'）", FieldKind::Text, s.condition()),
        ],
    })
}

/// `stack` — who called this method.
pub fn stack(c: &Caret, s: &Settings) -> Option<Command> {
    Some(Command {
        id: "stack".into(),
        label: "Stack".into(),
        describe: "看这个方法是「从哪被调进来的」：读源码、追调用入口最省事。".into(),
        doc: "https://arthas.aliyun.com/doc/stack.html".into(),
        variants: vec![
            Variant::new(
                "stack",
                "调用栈",
                "打印调用栈，从上往下就是进入路径。",
                "stack ${CLASS} ${METHOD} ${V} -n ${COUNT} ${CONDITION}".to_owned(),
            )
            .field("CLASS", "类", FieldKind::Text, &c.class)
            .field("METHOD", "方法（* 表示全部）", FieldKind::Text, c.method_pattern())
            .field("V", "-v", FieldKind::Text, s.verbose())
            .field("COUNT", "-n 次数", FieldKind::Number, &s.count)
            .field("CONDITION", "条件", FieldKind::Text, s.condition()),
        ],
    })
}

/// `monitor` — periodic statistics.
pub fn monitor(c: &Caret, s: &Settings) -> Option<Command> {
    Some(Command {
        id: "monitor".into(),
        label: "Monitor".into(),
        describe: "按周期统计方法的调用次数、成功率、平均耗时：看一段时间内的整体表现。".into(),
        doc: "https://arthas.aliyun.com/doc/monitor.html".into(),
        variants: vec![
            Variant::new(
                "monitor",
                "周期统计",
                "每 cycle 秒输出一行汇总，直到达到 -n 次数。",
                "monitor ${CLASS} ${METHOD} ${V} -n ${COUNT} --cycle ${CYCLE} ${CONDITION}".to_owned(),
            )
            .field("CLASS", "类", FieldKind::Text, &c.class)
            .field("METHOD", "方法（* 表示全部）", FieldKind::Text, c.method_pattern())
            .field("V", "-v", FieldKind::Text, s.verbose())
            .field("COUNT", "-n 周期数", FieldKind::Number, &s.monitor_count)
            .field("CYCLE", "--cycle 秒", FieldKind::Number, &s.monitor_interval)
            .field("CONDITION", "条件", FieldKind::Text, s.condition()),
        ],
    })
}

/// `tt` — record invocations, then inspect or replay them.
pub fn tt(c: &Caret, s: &Settings) -> Option<Command> {
    let variants = vec![
        Variant::new(
            "record",
            "记录调用",
            "把每次调用的入参和返回值存下来，事后再挑一条回看——ThreadLocal 会丢，引用对象的值按现在的算。",
            "tt -t ${CLASS} ${METHOD} -n ${COUNT} ${CONDITION}".to_owned(),
        )
        .field("CLASS", "类", FieldKind::Text, &c.class)
        .field("METHOD", "方法（* 表示全部）", FieldKind::Text, c.method_pattern())
        .field("COUNT", "-n 记录条数", FieldKind::Number, &s.count)
        .field("CONDITION", "条件", FieldKind::Text, s.condition()),
        Variant::new(
            "list",
            "列出记录",
            "看已经记录了哪些调用，拿到 index 再做别的。",
            "tt -l".to_owned(),
        ),
        Variant::new(
            "replay",
            "重新触发",
            "按记录的入参再执行一次：复现偶发问题最直接。index 是 `tt -l` 里那一列。",
            "tt -p -i ${INDEX} ${MODE}".to_owned(),
        )
        .field("INDEX", "记录 index", FieldKind::Number, "1000")
        .select(
            "MODE",
            "次数",
            "",
            &[("", "重新触发一次"), ("--replay-times 5 --replay-interval 2000", "触发 5 次，间隔 2 秒")],
        ),
        Variant::new(
            "inspect",
            "观测记录",
            "对某一次记录执行表达式，比重新触发更安全。",
            "tt -w '${EXPR}' -x ${DEPTH} -i ${INDEX}".to_owned(),
        )
        .field("INDEX", "记录 index", FieldKind::Number, "1000")
        .select(
            "EXPR",
            "表达式",
            "{method.name,params,returnObj,throwExp}",
            &[
                ("{method.name,params,returnObj,throwExp}", "方法名 / 入参 / 返回值 / 异常"),
                ("target", "目标对象"),
                ("params[0]", "第一个入参"),
                ("returnObj", "返回值"),
            ],
        )
        .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
        Variant::new(
            "clear",
            "清理记录",
            "记录会占内存，查完清掉。",
            "tt --delete-all".to_owned(),
        ),
    ];
    Some(Command {
        id: "tt".into(),
        label: "Time Tunnel".into(),
        describe: "记录方法每次调用的入参与返回，并能事后重新触发。".into(),
        doc: "https://arthas.aliyun.com/doc/tt.html".into(),
        variants,
    })
}

/// `ognl` — evaluate an expression against a static member, a Spring bean, or an
/// instance found by `vmtool`.
pub fn ognl(c: &Caret, s: &Settings) -> Option<Command> {
    let class = &c.class;
    let mut variants = Vec::new();

    if let Kind::Field { name, is_static, .. } = &c.kind {
        if *is_static {
            variants.push(
                Variant::new(
                    "static",
                    "静态字段",
                    "直接读一个静态字段，不用改代码加日志。",
                    "ognl -x ${DEPTH} '@${CLASS}@${EXPR}'".to_owned(),
                )
                .field("CLASS", "类", FieldKind::Text, class)
                .field("EXPR", "表达式", FieldKind::Text, name)
                .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
            );
        }
    }
    if let Kind::Method { name, params, is_static, is_constructor } = &c.kind {
        if !is_constructor {
            let call = call_with_defaults(name, params);
            variants.push(
                Variant::new(
                    if *is_static { "static" } else { "instance" },
                    if *is_static { "静态方法" } else { "实例方法" },
                    "执行一个方法并看返回值。参数按类型给了占位值，执行前改成真实的。",
                    if *is_static {
                        "ognl -x ${DEPTH} '@${CLASS}@${EXPR}'".to_owned()
                    } else {
                        "vmtool -x ${DEPTH} --action getInstances --className ${CLASS} --express 'instances[0].${EXPR}'".to_owned()
                    },
                )
                .field("CLASS", "类", FieldKind::Text, class)
                .field("EXPR", "表达式（参数需替换）", FieldKind::Text, call)
                .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
            );
        }
    }

    // A Spring bean is reachable through the context, whatever the member is.
    let member = match &c.kind {
        Kind::Method { name, params, is_constructor, .. } if !is_constructor => call_with_defaults(name, params),
        Kind::Field { name, .. } => name.clone(),
        _ => String::new(),
    };
    variants.push(
        Variant::new(
            "bean",
            "Spring Bean",
            "通过 Spring 上下文拿到 bean 再调用它：不用管它在哪个类、是不是代理。bean 名是按注解猜的，可以改。",
            "ognl -x ${DEPTH} '#springContext=${SPRING},#springContext.getBean(\"${BEAN}\").${EXPR}'".to_owned(),
        )
        .field("BEAN", "bean 名", FieldKind::Text, &c.bean)
        .field("EXPR", "表达式（参数需替换）", FieldKind::Text, member)
        .field("SPRING", "Spring 上下文表达式", FieldKind::Text, &s.spring)
        .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
    );

    Some(Command {
        id: "ognl".into(),
        label: "OGNL".into(),
        describe: "执行表达式：读静态变量、调方法、拿到 Spring 上下文后为所欲为。".into(),
        doc: "https://arthas.aliyun.com/doc/ognl.html".into(),
        variants,
    })
}

/// `getstatic` and the reflective field writes.
pub fn field(c: &Caret, s: &Settings) -> Option<Command> {
    let class = &c.class;
    let value = if s.value.is_empty() {
        match &c.kind {
            Kind::Field { ty, .. } => default_value(ty),
            _ => "null".into(),
        }
    } else {
        s.value.clone()
    };
    let mut variants = Vec::new();

    if let Kind::Field { name, is_static, is_final, has_setter, .. } = &c.kind {
        if *is_static {
            variants.push(
                Variant::new(
                    "get",
                    "读取",
                    "读静态字段当前的值。",
                    "getstatic ${CLASS} ${FIELD}".to_owned(),
                )
                .field("CLASS", "类", FieldKind::Text, class)
                .field("FIELD", "字段", FieldKind::Text, name),
            );
            if !*is_final {
                variants.push(
                    Variant::new(
                        "set",
                        "修改（非 final）",
                        "反射改静态字段：改了立马生效，用于临时开关、灰度验证。",
                        "ognl -x ${DEPTH} '#field=@${CLASS}@class.getDeclaredField(\"${FIELD}\"),#field.setAccessible(true),#field.set(null,${VALUE})'"
                            .to_owned(),
                    )
                    .field("CLASS", "类", FieldKind::Text, class)
                    .field("FIELD", "字段", FieldKind::Text, name)
                    .field("VALUE", "新值", FieldKind::Text, &value)
                    .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
                );
            } else {
                variants.push(
                    Variant::new(
                        "set",
                        "修改（static final）",
                        "static final 要先去掉 final 修饰符再写：反射两步走，能改但不能改字符串常量池里的字面量。",
                        "ognl -x ${DEPTH} '#field=@${CLASS}@class.getDeclaredField(\"${FIELD}\"),#modifiers=#field.getClass().getDeclaredField(\"modifiers\"),#modifiers.setAccessible(true),#modifiers.setInt(#field,#field.getModifiers() & ~@java.lang.reflect.Modifier@FINAL),#field.setAccessible(true),#field.set(null,${VALUE})'"
                            .to_owned(),
                    )
                    .field("CLASS", "类", FieldKind::Text, class)
                    .field("FIELD", "字段", FieldKind::Text, name)
                    .field("VALUE", "新值", FieldKind::Text, &value)
                    .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
                );
            }
        } else {
            // Instance fields are reached through an instance vmtool finds.
            if *is_final {
                variants.push(
                    Variant::new(
                        "set",
                        "修改（实例 final）",
                        "拿到实例后反射改 final 字段：要先清掉 final 位。",
                        "vmtool -x ${DEPTH} --action getInstances --className ${CLASS} --express '#field=instances[0].getClass().getDeclaredField(\"${FIELD}\"),#modifiers=#field.getClass().getDeclaredField(\"modifiers\"),#modifiers.setAccessible(true),#modifiers.setInt(#field,#field.getModifiers() & ~@java.lang.reflect.Modifier@FINAL),#field.setAccessible(true),#field.set(instances[0],${VALUE})'"
                            .to_owned(),
                    )
                    .field("CLASS", "类", FieldKind::Text, class)
                    .field("FIELD", "字段", FieldKind::Text, name)
                    .field("VALUE", "新值", FieldKind::Text, &value)
                    .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
                );
            } else {
                variants.push(
                    Variant::new(
                        "set",
                        "修改（实例）",
                        "拿到实例后反射改字段；有 setter 时也可以用 setter。",
                        "vmtool -x ${DEPTH} --action getInstances --className ${CLASS} --express '#field=instances[0].getClass().getDeclaredField(\"${FIELD}\"),#field.setAccessible(true),#field.set(instances[0],${VALUE})'"
                            .to_owned(),
                    )
                    .field("CLASS", "类", FieldKind::Text, class)
                    .field("FIELD", "字段", FieldKind::Text, name)
                    .field("VALUE", "新值", FieldKind::Text, &value)
                    .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
                );
                if *has_setter {
                    variants.push(
                        Variant::new(
                            "setter",
                            "修改（走 setter）",
                            "文件里有 set<Name>，走它而不是直接反射：setter 里的其他逻辑会一起生效。",
                            "vmtool -x ${DEPTH} --action getInstances --className ${CLASS} --express 'instances[0].set${NAME}(${VALUE})'"
                                .to_owned(),
                        )
                        .field("CLASS", "类", FieldKind::Text, class)
                        .field("NAME", "setter 后缀", FieldKind::Text, capitalize(name))
                        .field("VALUE", "新值", FieldKind::Text, &value)
                        .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
                    );
                }
            }
        }
    }

    variants.push(
        Variant::new(
            "instances",
            "查看实例",
            "列出这个类在 JVM 里的实例，先确认有对象再谈改。",
            "vmtool -x 1 --action getInstances --className ${CLASS} --limit ${LIMIT}".to_owned(),
        )
        .field("CLASS", "类", FieldKind::Text, class)
        .field("LIMIT", "最多几个", FieldKind::Number, "5"),
    );

    Some(Command {
        id: "field".into(),
        label: "Field".into(),
        describe: "读写字段：临时开关、灰度验证、修数据都靠它。".into(),
        doc: "https://arthas.aliyun.com/doc/getstatic.html".into(),
        variants,
    })
}

/// The Spring `Environment`, either one key or everything by priority.
///
/// No `Caret`: the environment is a Spring singleton, so the class under the
/// cursor has nothing to do with it. What the cursor *does* contribute is the
/// `selected` key, which is the page's selection, not a type name.
pub fn spring(s: &Settings, selected: &str) -> Option<Command> {
    // Verbatim from the original plugin: walk every property source in order and
    // flatten the ones that are maps.
    const ALL: &str = "#allProperties={},#propertySourceIterator=#standardServletEnvironment.getPropertySources().iterator(),#propertySourceIterator.{#key=#this.getName(),#allProperties.add(\"                \"),#allProperties.add(\"------------------------- name:\"+#key),#this.getSource() instanceof java.util.Map ?#this.getSource().entrySet().iterator.{#key=#this.key,#value=#this.getValue(),#allProperties.add(#key+\"=\"+#value)}:#{}},#allProperties";
    let key = if selected.trim().is_empty() { "spring.profiles.active" } else { selected.trim() };

    Some(Command {
        id: "spring".into(),
        label: "Spring Env".into(),
        describe: "读 Spring 的环境变量：排查「为什么这个配置没生效」最直接。".into(),
        doc: "https://arthas.aliyun.com/doc/vmtool.html".into(),
        variants: vec![
            Variant::new(
                "selected",
                "查一个配置项",
                "取某一项的最终值。把要查的 key 选中再执行命令，选中内容会自动填进来。",
                "vmtool -x ${DEPTH} --action getInstances --className org.springframework.core.env.ConfigurableEnvironment --express 'instances[0].getProperty(\"${KEY}\")'".to_owned(),
            )
            .field("KEY", "配置项", FieldKind::Text, key)
            .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
            Variant::new(
                "all",
                "全部（按优先级）",
                "按优先级把所有 property source 打出来：越靠前优先级越高，同名 key 以后者为准。",
                format!(
                    "vmtool -x ${{DEPTH}} --action getInstances --className org.springframework.core.env.ConfigurableEnvironment --express '#standardServletEnvironment=instances[0],{ALL}'"
                ),
            )
            .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
            Variant::new(
                "ognl",
                "全部（走静态上下文）",
                "没有 vmtool 时用静态上下文取：需要先把 Spring 上下文暴露成静态字段。",
                "ognl -x ${DEPTH} '${SPRING},#standardServletEnvironment=#springContext.getEnvironment(),".to_owned()
                    + ALL
                    + "'",
            )
            .field("SPRING", "Spring 上下文表达式", FieldKind::Text, &s.spring)
            .field("DEPTH", "-x 深度", FieldKind::Number, &s.depth),
        ],
    })
}

/// `logger` — read and change a logger's level.
pub fn logger(c: &Caret, s: &Settings) -> Option<Command> {
    let levels: [(&str, &str); 5] = [
        ("trace", "trace（最详细）"),
        ("debug", "debug"),
        ("info", "info"),
        ("warn", "warn"),
        ("error", "error（最简）"),
    ];
    Some(Command {
        id: "logger".into(),
        label: "Logger".into(),
        describe: "动态改日志级别：不用重启就能把某个类的日志打开。".into(),
        doc: "https://arthas.aliyun.com/doc/logger.html".into(),
        variants: vec![
            Variant::new(
                "set",
                "改级别",
                "级别从细到粗：trace > debug > info > warn > error。改完记得改回来。",
                "logger --name ${CLASS} --level ${LEVEL}".to_owned(),
            )
            .field("CLASS", "logger 名（通常等于类名）", FieldKind::Text, &c.class)
            .select("LEVEL", "级别", &s.level, &levels),
            Variant::new(
                "info",
                "查看当前",
                "先看这个 logger 现在是什么级别、挂了哪些 appender。",
                "logger --name ${CLASS}".to_owned(),
            )
            .field("CLASS", "logger 名", FieldKind::Text, &c.class),
        ],
    })
}

/// async-profiler through Arthas.
pub fn profiler(_c: &Caret, s: &Settings) -> Option<Command> {
    const EVENTS: [(&str, &str); 6] = [
        ("cpu", "cpu（CPU 时间）"),
        ("alloc", "alloc（内存分配）"),
        ("lock", "lock（锁竞争）"),
        ("wall", "wall（墙钟，含等待）"),
        ("itimer", "itimer（定时采样）"),
        ("ctimer", "ctimer（CPU 定时）"),
    ];
    const FORMATS: [(&str, &str); 4] = [
        ("html", "html（火焰图，浏览器直接开）"),
        ("jfr", "jfr（JFR 格式）"),
        ("collapsed", "collapsed（折叠栈）"),
        ("flat", "flat（平铺）"),
    ];
    Some(Command {
        id: "profiler".into(),
        label: "Profiler".into(),
        describe: "async-profiler 火焰图：X 轴越宽表示占用越多，Y 轴是调用栈，颜色无语义。".into(),
        doc: "https://arthas.aliyun.com/doc/profiler.html".into(),
        variants: vec![
            Variant::new(
                "start",
                "开始采样",
                "默认采 CPU。wall + --threads 能看「线程都在等什么」。",
                "profiler start --event ${EVENT} ${MODE} --interval 10000000 ${THREADS}".to_owned(),
            )
            .select("EVENT", "采样事件", &s.event, &EVENTS)
            .select("MODE", "范围", "", &[("", "全部"), ("--alluser", "只用户态"), ("--allkernel", "只内核态")])
            .select("THREADS", "线程", "", &[("", "按进程聚合"), ("--threads", "按线程分开")]),
            Variant::new(
                "auto",
                "采样并自动停止",
                "跑够时长自己停并出结果，适合挂在压测上。",
                "profiler start --event ${EVENT} ${MODE} --interval 10000000 --format ${FORMAT} --duration ${DURATION} ${THREADS}"
                    .to_owned(),
            )
            .select("EVENT", "采样事件", &s.event, &EVENTS)
            .select("MODE", "范围", "", &[("", "全部"), ("--alluser", "只用户态"), ("--allkernel", "只内核态")])
            .select("FORMAT", "输出格式", &s.format, &FORMATS)
            .field("DURATION", "--duration 秒", FieldKind::Number, &s.duration)
            .select("THREADS", "线程", "", &[("", "按进程聚合"), ("--threads", "按线程分开")]),
            Variant::new(
                "stop",
                "停止并输出",
                "手动停，按选定格式出结果。",
                "profiler stop --format ${FORMAT}".to_owned(),
            )
            .select("FORMAT", "输出格式", &s.format, &FORMATS),
            Variant::new(
                "other",
                "其它",
                "status / list / actions / version / dumpFlat 等。",
                "profiler ${ACTION}".to_owned(),
            )
            .select(
                "ACTION",
                "动作",
                "status",
                &[
                    ("status", "status（当前状态）"),
                    ("list", "list（可用事件）"),
                    ("actions", "actions（可用动作）"),
                    ("version", "version"),
                    ("getSamples", "getSamples（样本数）"),
                    ("dumpFlat", "dumpFlat"),
                    ("dumpTraces", "dumpTraces"),
                    ("dumpCollapsed", "dumpCollapsed"),
                ],
            ),
        ],
    })
}

/// `jad` — decompile what is actually loaded.
pub fn jad(c: &Caret, _s: &Settings) -> Option<Command> {
    let method = match &c.kind {
        Kind::Method { name, is_constructor, .. } if !is_constructor => name.clone(),
        _ => String::new(),
    };
    Some(Command {
        id: "jad".into(),
        label: "Jad".into(),
        describe: "反编译 JVM 里真正加载的那份字节码：确认线上跑的是不是你以为的代码。".into(),
        doc: "https://arthas.aliyun.com/doc/jad.html".into(),
        variants: vec![
            Variant::new("jad", "反编译", "线上类和源码不一致时，看这个最准。", "jad --source-only ${CLASS} ${METHOD}".to_owned())
                .field("CLASS", "类", FieldKind::Text, &c.class)
                .field("METHOD", "方法（留空为整个类）", FieldKind::Text, method),
        ],
    })
}

/// `sc` / `sm` — where a class or method actually lives.
pub fn search(c: &Caret, _s: &Settings) -> Option<Command> {
    Some(Command {
        id: "search".into(),
        label: "Search".into(),
        describe: "查类和方法的实际位置与 ClassLoader：类冲突、同名类加载了哪份，一查就知道。".into(),
        doc: "https://arthas.aliyun.com/doc/sc.html".into(),
        variants: vec![
            Variant::new("sc", "查类", "打印类的 code source 与 ClassLoader。", "sc -d ${CLASS}".to_owned())
                .field("CLASS", "类", FieldKind::Text, &c.class),
            Variant::new(
                "sm",
                "查方法",
                "打印方法签名，确认重载选了哪一个。",
                "sm -d ${CLASS} ${METHOD}".to_owned(),
            )
            .field("CLASS", "类", FieldKind::Text, &c.class)
            .field("METHOD", "方法", FieldKind::Text, c.method_pattern()),
        ],
    })
}

/// `dump` — take the loaded bytecode out.
pub fn dump(c: &Caret, _s: &Settings) -> Option<Command> {
    Some(Command {
        id: "dump".into(),
        label: "Dump".into(),
        describe: "把加载的字节码导出到目录，配合反编译工具离线看。".into(),
        doc: "https://arthas.aliyun.com/doc/dump.html".into(),
        variants: vec![
            Variant::new("dump", "导出字节码", "导出后可用 jad 或 IDE 反编译查看。", "dump ${CLASS} -d ${DIR}".to_owned())
                .field("CLASS", "类", FieldKind::Text, &c.class)
                .field("DIR", "输出目录", FieldKind::Text, "/tmp/output"),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java::caret_at;

    const SRC: &str = r#"package com.demo.order;

@Service("orderService")
public class OrderService {
    private static final long TIMEOUT = 3000L;
    private static int counter;
    private String channel = "APP";

    public String createOrder(Order order) {
        return null;
    }

    public void setChannel(String channel) {
        this.channel = channel;
    }
}
"#;

    fn caret(needle: &str) -> Caret {
        let line = SRC.lines().position(|l| l.contains(needle)).expect("the line") as u32;
        caret_at("/p/OrderService.java", SRC, line).expect("a caret")
    }

    /// Every assertion goes through the real renderer, so the template, the
    /// substitution and the whitespace pass are all under test.
    fn render(v: &Variant) -> String {
        v.render()
    }

    #[test]
    fn watch_on_a_method_matches_the_original_shape() {
        let c = caret("return null;");
        let cmd = watch(&c, &Settings::default()).expect("a command");
        let v = &cmd.variants[0];
        assert_eq!(
            render(v),
            "watch com.demo.order.OrderService createOrder 'params,returnObj,throwExp' -n 5 -x 3 '1==1'"
        );
        // The -v flag is its own field and empty by default.
        assert!(v.fields.iter().any(|f| f.key == "V" && f.value.is_empty()));
    }

    #[test]
    fn watch_on_a_static_final_field_reads_it_through_the_class() {
        let c = caret("TIMEOUT = 3000L");
        let cmd = watch(&c, &Settings::default()).expect("a command");
        let field = cmd.variants.iter().find(|v| v.key == "field").expect("the field variant");
        assert_eq!(field.fields.iter().find(|f| f.key == "METHOD").unwrap().value, "*");
        assert_eq!(
            field.fields.iter().find(|f| f.key == "OBSERVE").unwrap().value,
            "params,returnObj,throwExp,@com.demo.order.OrderService@TIMEOUT"
        );
    }

    #[test]
    fn watch_on_an_instance_field_keeps_the_modifier_guard() {
        let c = caret("private String channel");
        let cmd = watch(&c, &Settings::default()).expect("a command");
        let field = cmd.variants.iter().find(|v| v.key == "field").expect("the field variant");
        assert!(render(field).contains("target.channel"));
        assert!(render(field).contains("!@java.lang.reflect.Modifier@isStatic"));
    }

    #[test]
    fn trace_and_monitor_keep_their_flags() {
        let c = caret("return null;");
        let s = Settings::default();
        let t = &trace(&c, &s).unwrap().variants[0];
        assert_eq!(
            render(t),
            "trace com.demo.order.OrderService createOrder -n 5 --skipJDKMethod false '1==1'"
        );
        let m = &monitor(&c, &s).unwrap().variants[0];
        assert_eq!(
            render(m),
            "monitor com.demo.order.OrderService createOrder -n 10 --cycle 10 '1==1'"
        );
    }

    #[test]
    fn stack_and_tt_record_match() {
        let c = caret("return null;");
        let s = Settings::default();
        assert_eq!(render(&stack(&c, &s).unwrap().variants[0]), "stack com.demo.order.OrderService createOrder -n 5 '1==1'");
        assert_eq!(
            render(&tt(&c, &s).unwrap().variants[0]),
            "tt -t com.demo.order.OrderService createOrder -n 5 '1==1'"
        );
    }

    #[test]
    fn tt_replay_and_inspect_ask_for_the_record_index() {
        let c = caret("return null;");
        let cmd = tt(&c, &Settings::default()).expect("a command");
        let replay = cmd.variants.iter().find(|v| v.key == "replay").unwrap();
        assert_eq!(render(replay), "tt -p -i 1000");

        // Filling the optional mode keeps the flags in order and the gap tidy.
        let mut five = replay.clone();
        for f in &mut five.fields {
            if f.key == "MODE" {
                f.value = "--replay-times 5 --replay-interval 2000".into();
            }
        }
        assert_eq!(five.render(), "tt -p -i 1000 --replay-times 5 --replay-interval 2000");

        let inspect = cmd.variants.iter().find(|v| v.key == "inspect").unwrap();
        assert_eq!(render(inspect), "tt -w '{method.name,params,returnObj,throwExp}' -x 3 -i 1000");
        // The index is a field, not baked into the template.
        assert!(inspect.fields.iter().any(|f| f.key == "INDEX"));
    }

    #[test]
    fn ognl_offers_static_bean_and_instance_shapes() {
        let c = caret("return null;");
        let cmd = ognl(&c, &Settings::default()).expect("a command");
        let bean = cmd.variants.iter().find(|v| v.key == "bean").expect("bean");
        assert_eq!(
            render(bean),
            "ognl -x 3 '#springContext=@applicationContextProvider@context,#springContext.getBean(\"orderService\").createOrder(null)'"
        );
        let s = caret("TIMEOUT = 3000L");
        let cmd = ognl(&s, &Settings::default()).expect("a command");
        let stat = cmd.variants.iter().find(|v| v.key == "static").expect("static");
        assert_eq!(render(stat), "ognl -x 3 '@com.demo.order.OrderService@TIMEOUT'");
    }

    #[test]
    fn static_final_fields_need_the_modifier_dance() {
        let c = caret("TIMEOUT = 3000L");
        let cmd = field(&c, &Settings::default()).expect("a command");
        let set = cmd.variants.iter().find(|v| v.key == "set").expect("set");
        let out = render(set);
        assert!(out.starts_with("ognl -x 3 '#field=@com.demo.order.OrderService@class.getDeclaredField(\"TIMEOUT\")"), "{out}");
        // A final field is only writable after its FINAL bit is cleared, and the
        // stand-in value is the field's own default (long → 0).
        assert!(out.contains("#modifiers.setInt(#field,#field.getModifiers() & ~@java.lang.reflect.Modifier@FINAL)"), "{out}");
        assert!(out.ends_with("#field.set(null,0)'"), "{out}");
        // There is no plain getter variant here: getstatic covers static fields
        // of any kind, so it is offered once.
        assert_eq!(cmd.variants.iter().filter(|v| v.key == "get").count(), 1);
    }

    #[test]
    fn field_commands_use_a_typed_default_value() {
        let c = caret("private String channel");
        let mut s = Settings::default();
        s.value = String::new();
        // `default_value` is consulted only when the setting is empty.
        let cmd = field(&c, &s).expect("a command");
        let set = cmd.variants.iter().find(|v| v.key == "set").expect("set");
        let value = &set.fields.iter().find(|f| f.key == "VALUE").unwrap().value;
        assert_eq!(value, "\"\"", "a String field gets an empty string literal");
        // The setter variant exists because the file declares setChannel.
        assert!(cmd.variants.iter().any(|v| v.key == "setter"));
    }

    #[test]
    fn spring_lists_every_source_or_one_key() {
        let cmd = spring(&Settings::default(), "").expect("a command");
        let all = cmd.variants.iter().find(|v| v.key == "all").unwrap();
        assert!(render(all).contains("getPropertySources()"));
        let one = cmd.variants.iter().find(|v| v.key == "selected").unwrap();
        assert_eq!(
            render(one),
            "vmtool -x 3 --action getInstances --className org.springframework.core.env.ConfigurableEnvironment --express 'instances[0].getProperty(\"spring.profiles.active\")'"
        );
        // A selection wins over the placeholder key.
        let picked = spring(&Settings::default(), "  app.timeout  ").unwrap();
        assert!(render(picked.variants.iter().find(|v| v.key == "selected").unwrap()).contains("app.timeout"));
    }

    #[test]
    fn logger_profiler_and_search_render() {
        let c = caret("return null;");
        let s = Settings::default();
        assert_eq!(render(&logger(&c, &s).unwrap().variants[0]), "logger --name com.demo.order.OrderService --level debug");
        assert_eq!(
            render(&profiler(&c, &s).unwrap().variants[0]),
            "profiler start --event cpu --interval 10000000"
        );
        assert_eq!(render(&search(&c, &s).unwrap().variants[0]), "sc -d com.demo.order.OrderService");
        assert_eq!(render(&jad(&c, &s).unwrap().variants[0]), "jad --source-only com.demo.order.OrderService createOrder");
        assert_eq!(render(&dump(&c, &s).unwrap().variants[0]), "dump com.demo.order.OrderService -d /tmp/output");
    }
}

