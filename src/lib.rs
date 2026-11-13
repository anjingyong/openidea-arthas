//! Arthas Commander for OpenIDEA: a tool window that builds the commands the
//! Arthas idea plugin offers, for a running JVM.
//!
//! The work is split in three: [`java`] turns the caret into a class, method or
//! field (textually — a plugin cannot wait for a language server); [`catalog`]
//! turns that into commands, one family per Arthas verb; this module is the
//! wiring — the menu, the page, the settings, the clipboard and the terminal.
//!
//! The page owns the form and renders the command by itself: a family arrives
//! with its templates and field values, so changing a count re-renders without a
//! round trip. What a page cannot know — which JVMs are running, whether a
//! command is safe to hand off, what the file says — stays here.

mod catalog;
mod java;

use openidea_plugin::host;
use openidea_plugin::views::{Body, Button, View, ViewEvent, Viewport, Views};
use openidea_plugin::{Error, Invocation, NotificationLevel, Plugin, register_plugin};
use serde_json::{Value, json};

use catalog::Settings;
use java::{Caret, Kind};

/// The tool window's id, as `[[contributes.views]]` declares it and as the
/// page's frame is addressed.
pub const VIEW: &str = "arthas.panel";
/// The page, read once at build time.
const PAGE: &str = include_str!("../webview/index.html");
/// Longest document read for a caret (bytes).
const MAX_TEXT: usize = 4 << 20;

/// Every family, in tab order. One menu entry each; `field` is the original
/// plugin's `getstatic` plus the reflective writes, `spring` its environment
/// readers, `search` its `sc` / `sm` pair.
pub const FAMILIES: [&str; 13] = [
    "watch", "trace", "stack", "monitor", "tt", "ognl", "field", "spring", "logger", "profiler",
    "jad", "search", "dump",
];

/// The first word a command may carry.
///
/// [`catalog::Variant::render`] only ever produces these, and `execute` accepts
/// nothing else — so a page, a stale selection or a hand-typed string can never
/// hand a terminal something that is not an Arthas command. The test
/// `every_generated_command_starts_with_a_known_verb` keeps the two in step.
pub const VERBS: [&str; 14] = [
    "watch", "trace", "stack", "monitor", "tt", "ognl", "getstatic", "vmtool", "logger",
    "profiler", "jad", "sc", "sm", "dump",
];

// ── Settings ────────────────────────────────────────────────────────────────

/// A count-like setting: a positive whole number, or the fallback. `-n 0` and
/// `-x 0` mean "nothing", which is never what someone typing a number wants.
fn whole(key: &str, fallback: &str) -> String {
    host::setting_number(key)
        .filter(|n| *n >= 1.0)
        .map(|n| (n as u64).to_string())
        .unwrap_or_else(|| fallback.to_owned())
}

/// A text setting, blank collapsed to the fallback.
fn text(key: &str, fallback: &str) -> String {
    host::setting_string(key)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// The settings' values as the catalog wants them.
///
/// Read on every request rather than cached: a setting edited in the IDE should
/// show up in the next command, and the cost is a handful of lookups.
fn settings() -> Settings {
    let d = Settings::default();
    Settings {
        depth: whole("depth", &d.depth),
        count: whole("count", &d.count),
        monitor_count: whole("monitorCount", &d.monitor_count),
        monitor_interval: whole("monitorInterval", &d.monitor_interval),
        verbose: host::setting_bool("verbose").unwrap_or(d.verbose),
        condition: host::setting_bool("condition").unwrap_or(d.condition),
        skip_jdk: host::setting_bool("skipJdkMethod").unwrap_or(d.skip_jdk),
        spring: text("springContext", &d.spring),
        // Empty is meaningful here: it means "use the field type's default",
        // which the catalog fills in from the declaration.
        value: host::setting_string("value").unwrap_or(d.value).trim().to_owned(),
        level: text("level", &d.level),
        event: text("event", &d.event),
        format: text("format", &d.format),
        duration: whole("duration", &d.duration),
    }
}

// ── The catalog message ─────────────────────────────────────────────────────

/// One family, or `None` when the caret cannot support it — `watch` on a class
/// with no method to name, for instance. A family that cannot be built is left
/// out of the message rather than shown empty.
fn family_command(family: &str, caret: &Caret, settings: &Settings, selected: &str) -> Option<catalog::Command> {
    match family {
        "watch" => catalog::watch(caret, settings),
        "trace" => catalog::trace(caret, settings),
        "stack" => catalog::stack(caret, settings),
        "monitor" => catalog::monitor(caret, settings),
        "tt" => catalog::tt(caret, settings),
        "ognl" => catalog::ognl(caret, settings),
        "field" => catalog::field(caret, settings),
        // The environment is a singleton: no caret, but the selected text is
        // the key someone wants to look up.
        "spring" => catalog::spring(settings, selected),
        "logger" => catalog::logger(caret, settings),
        "profiler" => catalog::profiler(caret, settings),
        "jad" => catalog::jad(caret, settings),
        "search" => catalog::search(caret, settings),
        "dump" => catalog::dump(caret, settings),
        _ => None,
    }
}

/// `com.acme.OrderService#createOrder(Order, User)` for the panel's status row.
fn signature(caret: &Caret) -> String {
    match &caret.kind {
        Kind::Method { name, params, .. } => format!("{}#{name}({})", caret.class, params.join(", ")),
        Kind::Field { name, ty, .. } => format!("{}#{name}: {ty}", caret.class),
        Kind::Class => caret.class.clone(),
    }
}

/// The message the page renders: where the caret is, and every command that
/// follows from it.
fn view_message(caret: &Caret, settings: &Settings, selected: &str, focus: Option<&str>) -> Value {
    let commands: Vec<Value> = FAMILIES
        .iter()
        .filter_map(|family| family_command(family, caret, settings, selected))
        .map(|command| command.to_json())
        .collect();
    let (kind, params) = match &caret.kind {
        Kind::Method { params, .. } => ("method", params.clone()),
        Kind::Field { ty, .. } => ("field", vec![ty.clone()]),
        Kind::Class => ("class", Vec::new()),
    };
    let mut message = json!({
        "type": "arthas.catalog",
        "cls": caret.class,
        "simple": caret.simple,
        "pkg": caret.pkg,
        "bean": caret.bean,
        "member": caret.member(),
        "kind": kind,
        "params": params,
        "overloads": caret.overloads,
        "signature": signature(caret),
        "selection": selected,
        "commands": commands,
    });
    if let Some(focus) = focus {
        message["focus"] = json!(focus);
    }
    message
}

/// The catalog for the focused editor's caret.
fn catalog_at(ctx: &Invocation, focus: Option<&str>) -> Result<Value, Error> {
    let editor = ctx.editor().ok_or_else(|| Error::Invalid("先在编辑器里打开一个 Java / Kotlin 文件".into()))?;
    let document = editor.document()?;
    if document.text.len() > MAX_TEXT {
        return Err(Error::Invalid(format!("{} 太大，读不出类名", editor.path())));
    }
    let selections = editor.selections().unwrap_or_default();
    let line = selections.first().map(|range| range.start.line).unwrap_or(0);
    // Whatever is selected is the Spring key, or the class to prefill; the page
    // decides which, so it travels as one string.
    let selected = selections
        .first()
        .and_then(|range| document.text_in(*range))
        .unwrap_or_default()
        .trim()
        .to_owned();
    // A file with no type around the caret has nothing to say: `package` and
    // `class` are what Arthas addresses, and a guess is a command that silently
    // watches nothing.
    let caret = java::caret_at(&editor.path(), &document.text, line)
        .ok_or_else(|| Error::Invalid("光标不在任何类里：把它放到类、方法或字段上".into()))?;
    Ok(view_message(&caret, &settings(), &selected, focus))
}

// ── The plugin ──────────────────────────────────────────────────────────────

/// The target JVM picked in the panel; `execute` attaches to it. Kept for as
/// long as the IDE runs — the `targetPid` setting is what survives a restart.
#[derive(Default)]
pub struct Arthas {
    pid: Option<String>,
    /// The family the menu last asked for. The panel's frame may not exist yet
    /// when a menu command runs, in which case the message lands nowhere; the
    /// page asks again when it loads, and this is what makes that answer land on
    /// the command the user actually picked.
    focus: Option<String>,
}

fn post(message: &Value) -> Result<(), Error> {
    host::post_view_message(VIEW, &message.to_string())
}

/// The attachable JVMs from `jps -l`, or nothing when `jps` is not granted or
/// not installed — the picker then just stays empty.
fn targets() -> Vec<Value> {
    let Ok(output) = host::run_process("jps", &["-l".to_owned()], None) else { return Vec::new() };
    if output.status != Some(0) {
        return Vec::new();
    }
    output
        .stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?;
            if !pid.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let app = fields.next().unwrap_or("—");
            // `jps` lists itself; that row is noise.
            if app.ends_with("Jps") {
                return None;
            }
            Some(json!({ "pid": pid, "app": app }))
        })
        .collect()
}

/// The command as it may be handed off, or `None`.
///
/// One of the catalog's verbs and one line: a newline would split it into a
/// second command in whatever terminal it is pasted into.
fn checked(command: &str) -> Option<&str> {
    let command = command.trim();
    let verb = command.split_whitespace().next()?;
    (VERBS.contains(&verb) && command.lines().count() == 1).then_some(command)
}

/// The answer for a command that is not one of ours.
fn refusal(command: &str) -> Value {
    json!({
        "type": "arthas.result", "ok": false, "copied": false, "terminal": false,
        "message": format!("不是 Arthas 命令：{}", command.trim()),
    })
}

impl Arthas {
    /// Validates a generated command and hands it off.
    ///
    /// Nothing is typed into a shell: the command is copied, and when
    /// `arthasJar` is configured the terminal that attaches to the target opens
    /// so it can be pasted in. The verb is checked first, so the clipboard and
    /// the terminal only ever see a shape the catalog produces.
    fn execute(&self, command: &str) -> Value {
        let Some(command) = checked(command) else { return refusal(command) };
        let copied = host::copy_to_clipboard(command).is_ok();
        let jar = host::setting_string("arthasJar").unwrap_or_default().trim().to_owned();
        let (terminal, message) = if jar.is_empty() {
            let message = if copied {
                "已复制命令。配置 arthas-boot.jar 后，这里会直接连上目标 JVM。"
            } else {
                "未配置 arthas-boot.jar，也没有剪贴板权限，命令只在面板里。"
            };
            (false, message.to_owned())
        } else {
            let mut args = vec!["-jar".to_owned(), jar];
            let picked = self.pid.clone().unwrap_or_default();
            let pid = if picked.trim().is_empty() {
                host::setting_string("targetPid").unwrap_or_default()
            } else {
                picked
            };
            let pid = pid.trim();
            if !pid.is_empty() {
                args.push(pid.to_owned());
            }
            match host::open_terminal("Arthas", "java", &args) {
                Ok(()) if copied => (true, "终端已连上目标 JVM；命令在剪贴板里，粘贴回车即可。".to_owned()),
                Ok(()) => (true, "终端已连上目标 JVM；命令请在面板里复制。".to_owned()),
                Err(e) => (false, format!("无法打开 Arthas 终端：{e}")),
            }
        };
        host::notify(NotificationLevel::Info, "Arthas", &message);
        json!({ "type": "arthas.result", "ok": true, "copied": copied, "terminal": terminal, "message": message })
    }

    /// Copies without attaching — the panel's Copy button.
    fn copy(&self, command: &str) -> Value {
        let Some(command) = checked(command) else { return refusal(command) };
        let copied = host::copy_to_clipboard(command).is_ok();
        let message = if copied { "已复制命令" } else { "剪贴板不可用：请手动选中命令复制" };
        json!({ "type": "arthas.result", "ok": copied, "copied": copied, "terminal": false, "message": message })
    }

    /// Answers a request for the catalog, spending a focus the menu left behind.
    fn refresh(&mut self, ctx: &Invocation) -> Result<(), Error> {
        let focus = self.focus.take();
        match catalog_at(ctx, focus.as_deref()) {
            Ok(message) => post(&message),
            Err(e) => post(&json!({ "type": "arthas.error", "message": e.to_string() })),
        }
    }
}

impl Plugin for Arthas {
    fn run_command(&mut self, id: &str, ctx: &Invocation, _arg: Option<&str>) -> Result<Option<String>, Error> {
        if id == "arthas.open" {
            return host::reveal_view(VIEW).map(|()| None);
        }
        if id == "arthas.targets" {
            post(&json!({ "type": "arthas.targets", "list": targets() }))?;
            return host::reveal_view(VIEW).map(|()| None);
        }
        let family = id.strip_prefix("arthas.").ok_or_else(|| Error::NotFound(format!("command {id}")))?;
        if !FAMILIES.contains(&family) {
            return Err(Error::NotFound(format!("command {id}")));
        }
        // A caret that yields nothing is an error the IDE shows; a frame that is
        // not up yet is not — the page asks again when it loads.
        let message = catalog_at(ctx, Some(family))?;
        self.focus = Some(family.to_owned());
        let _ = post(&message);
        host::reveal_view(VIEW).map(|()| None)
    }
}

impl Views for Arthas {
    fn render(&mut self, view: &str, _viewport: Option<Viewport>, _ctx: &Invocation) -> Result<View, Error> {
        if view != VIEW {
            return Err(Error::NotFound(view.into()));
        }
        // No message is posted here: the frame asks for the catalog once its
        // scripts run, which is the only moment the bridge is guaranteed ready.
        Ok(View::new(Body::Html(PAGE.to_owned()))
            .button(Button::new("refresh", "Fill from Editor").icon("refresh"))
            .button(Button::new("targets", "Refresh JVMs").icon("server")))
    }

    fn handle(&mut self, view: &str, event: ViewEvent, ctx: &Invocation) -> Result<(), Error> {
        if view != VIEW {
            return Err(Error::NotFound(view.into()));
        }
        match event {
            // Failures inside the frame become a message in the page: the frame
            // has no error surface of its own, and the command menu already
            // reported.
            ViewEvent::Button(id) if id == "refresh" => self.refresh(ctx),
            ViewEvent::Button(id) if id == "targets" => {
                post(&json!({ "type": "arthas.targets", "list": targets() }))
            }
            ViewEvent::Message(text) => {
                let Ok(message) = serde_json::from_str::<Value>(&text) else { return Ok(()) };
                match message.get("type").and_then(Value::as_str) {
                    Some("arthas.execute") => match message.get("command").and_then(Value::as_str) {
                        Some(command) => post(&self.execute(command)),
                        None => Ok(()),
                    },
                    // Copy-only: the panel's Copy button must not open a terminal.
                    Some("arthas.copy") => match message.get("command").and_then(Value::as_str) {
                        Some(command) => post(&self.copy(command)),
                        None => Ok(()),
                    },
                    Some("arthas.pick-target") => {
                        match message.get("pid").and_then(Value::as_str).map(str::trim) {
                            Some(pid) if !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) => {
                                self.pid = Some(pid.to_owned());
                                let app = message.get("app").and_then(Value::as_str).unwrap_or("—");
                                post(&json!({ "type": "arthas.target", "pid": pid, "app": app }))
                            }
                            // A PID that is not a number never reaches `arthas-boot`.
                            _ => Ok(()),
                        }
                    }
                    Some("arthas.refresh") => self.refresh(ctx),
                    Some("arthas.targets") => {
                        post(&json!({ "type": "arthas.targets", "list": targets() }))
                    }
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }
}

register_plugin!(Arthas, views);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::ProcessOutput;
    use openidea_plugin::mock::MockHost;

    const SERVICE: &str = r#"package com.demo.order;

import java.util.List;
import org.springframework.stereotype.Service;

@Service("orderService")
public class OrderService {
    private static final long TIMEOUT = 3000L;
    private static int counter;
    private String channel = "APP";

    private static class Handler {
        void run(String id) {
        }
    }

    public String createOrder(Order order, @Nullable User user) {
        return null;
    }

    public String createOrder(Order order) {
        return null;
    }

    void audit(List<String> rows, Map<String, Integer> counts) {
    }
}
"#;

    fn line_of(text: &str, needle: &str) -> u32 {
        text.lines().position(|l| l.contains(needle)).expect("the line") as u32
    }

    fn caret_at(text: &str, needle: &str) -> Caret {
        java::caret_at("/p/OrderService.java", text, line_of(text, needle)).expect("a caret")
    }

    #[test]
    fn the_menu_sends_every_family_and_focuses_the_one_asked_for() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        let line = line_of(SERVICE, "return null;");
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(line, 8, line, 8)]);
        let mut plugin = Arthas::default();
        plugin.run_command("arthas.trace", &host.invocation(), None).unwrap();
        let (view, message) = host.view_messages().pop().expect("a message");
        assert_eq!(view, VIEW);
        let message: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(message["type"], "arthas.catalog");
        assert_eq!(message["focus"], "trace");
        assert_eq!(message["cls"], "com.demo.order.OrderService");
        assert_eq!(message["member"], "createOrder");
        assert_eq!(message["kind"], "method");
        assert_eq!(message["overloads"], 2);
        assert_eq!(message["signature"], "com.demo.order.OrderService#createOrder(Order, User)");
        let ids: Vec<&str> = message["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        // Every family is offered for a method caret, in tab order.
        assert_eq!(ids, FAMILIES);
        assert_eq!(host.revealed_views(), [VIEW.to_owned()]);
    }

    #[test]
    fn a_nested_type_is_addressed_with_a_dollar_and_offers_the_same_families() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        // Inside `Handler.run`, a field of the outer class is out of reach but
        // every method-scoped family still applies.
        let line = line_of(SERVICE, "void run(String id)") + 1;
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(line, 8, line, 8)]);
        let mut plugin = Arthas::default();
        plugin.run_command("arthas.watch", &host.invocation(), None).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        let message: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(message["cls"], "com.demo.order.OrderService$Handler");
        assert_eq!(message["member"], "run");
    }

    #[test]
    fn the_package_line_still_resolves_the_class_through_the_file_name() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(0, 0, 0, 0)]);
        let mut plugin = Arthas::default();
        plugin.run_command("arthas.watch", &host.invocation(), None).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        let message: Value = serde_json::from_str(&message).unwrap();
        // Outside any method: the class is still addressable, `*` stands in for
        // the method — which is why `watch` is offered rather than hidden.
        assert_eq!(message["cls"], "com.demo.order.OrderService");
        assert_eq!(message["kind"], "class");
        assert_eq!(message["member"], "");
        assert_eq!(message["signature"], "com.demo.order.OrderService");
    }

    #[test]
    fn an_empty_buffer_has_no_class_to_address() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        host.open_editor("/p/Empty.java", "java", "", &[(0, 0, 0, 0)]);
        let mut plugin = Arthas::default();
        assert!(plugin.run_command("arthas.watch", &host.invocation(), None).is_err());
        assert!(host.view_messages().is_empty());
    }

    #[test]
    fn every_generated_command_starts_with_a_known_verb() {
        // The whitelist and the catalog must agree; this walks every variant.
        for (needle, text) in [
            ("return null;", SERVICE),
            ("TIMEOUT = 3000L", SERVICE),
            ("private String channel", SERVICE),
            ("void run(String id)", SERVICE),
        ] {
            let caret = caret_at(text, needle);
            for family in FAMILIES {
                let Some(command) = family_command(family, &caret, &Settings::default(), "app.timeout") else {
                    continue;
                };
                for variant in &command.variants {
                    let rendered = variant.render();
                    let verb = rendered.split_whitespace().next().unwrap_or_default();
                    assert!(VERBS.contains(&verb), "{} / {}: {rendered}", command.id, variant.key);
                    // One line, always: a newline would split it into two
                    // commands in the terminal it is pasted into.
                    assert_eq!(rendered.lines().count(), 1, "{rendered}");
                }
            }
        }
    }

    #[test]
    fn the_selected_text_becomes_the_spring_key() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        // Select the class name in `public class OrderService {`.
        let line = line_of(SERVICE, "public class OrderService");
        let column = SERVICE.lines().nth(line as usize).unwrap().find("OrderService").unwrap() as u32;
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(line, column, line, column + 12)]);
        let mut plugin = Arthas::default();
        plugin.run_command("arthas.spring", &host.invocation(), None).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        let message: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(message["selection"], "OrderService");
        let spring = message["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "spring")
            .expect("the spring family");
        let key = spring["variants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["key"] == "selected")
            .and_then(|v| v["fields"].as_array())
            .and_then(|fields| fields.iter().find(|f| f["key"] == "KEY"))
            .and_then(|f| f["value"].as_str())
            .expect("the key field");
        assert_eq!(key, "OrderService");
    }

    #[test]
    fn settings_reach_the_fields_and_clamp_nonsense() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        host.set_setting("depth", "7");
        host.set_setting("count", "0");
        host.set_setting("level", r#""  warn  ""#);
        host.set_setting("skipJdkMethod", "true");
        host.set_setting("springContext", r#""@ctx@bean""#);
        let mut plugin = Arthas::default();
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(15, 8, 15, 8)]);
        plugin.run_command("arthas.watch", &host.invocation(), None).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        let message: Value = serde_json::from_str(&message).unwrap();
        let commands = message["commands"].as_array().unwrap();

        let field_of = |family: &str, variant: &str, key: &str| {
            commands
                .iter()
                .find(|c| c["id"] == family)
                .and_then(|c| c["variants"].as_array())
                .and_then(|vs| vs.iter().find(|v| v["key"] == variant))
                .and_then(|v| v["fields"].as_array())
                .and_then(|fs| fs.iter().find(|f| f["key"] == key))
                .and_then(|f| f["value"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| panic!("{family}/{variant}/{key}"))
        };
        assert_eq!(field_of("watch", "call", "DEPTH"), "7");
        // `-n 0` is clamped back to the default: a zero count prints nothing.
        assert_eq!(field_of("watch", "call", "COUNT"), "5");
        assert_eq!(field_of("logger", "set", "LEVEL"), "warn");
        // `skipJdkMethod` on drops `--skipJDKMethod false`, so JDK frames stay out.
        assert_eq!(field_of("trace", "trace", "SKIPJDK"), "");
        assert_eq!(field_of("tt", "record", "CLASS"), "com.demo.order.OrderService");
    }

    #[test]
    fn execute_copies_and_names_the_missing_jar() {
        let host = MockHost::install().with_capabilities(&["editor.read", "clipboard"]);
        let result = Arthas::default().execute("watch com.demo.OrderService createOrder -n 5");
        assert_eq!(result["ok"], true);
        assert_eq!(result["copied"], true);
        assert_eq!(result["terminal"], false);
        assert_eq!(host.clipboard().as_deref(), Some("watch com.demo.OrderService createOrder -n 5"));
        assert_eq!(host.terminals().len(), 0);
    }

    #[test]
    fn execute_opens_the_attach_terminal_when_a_jar_is_configured() {
        let host = MockHost::install().with_capabilities(&["editor.read", "clipboard", "process:java"]);
        host.set_setting("arthasJar", r#""/opt/arthas/arthas-boot.jar""#);
        host.set_setting("targetPid", r#""4242""#);
        let result = Arthas::default().execute("trace com.demo.OrderService createOrder -n 5");
        assert_eq!(result["terminal"], true, "{result}");
        let terminals = host.terminals();
        assert_eq!(terminals[0].program, "java");
        assert_eq!(terminals[0].args, ["-jar", "/opt/arthas/arthas-boot.jar", "4242"]);
    }

    #[test]
    fn the_picked_jvm_beats_the_setting_and_bad_pids_never_reach_the_shell() {
        let host = MockHost::install().with_capabilities(&["clipboard", "process:java"]);
        host.set_setting("arthasJar", r#""/opt/arthas/arthas-boot.jar""#);
        host.set_setting("targetPid", r#""1111""#);
        let mut plugin = Arthas::default();

        // Not a number: ignored, and nothing is remembered.
        for bad in [r#"{"type":"arthas.pick-target","pid":"42; rm -rf /"}"#, r#"{"type":"arthas.pick-target"}"#] {
            plugin.handle(VIEW, ViewEvent::Message(bad.into()), &host.invocation()).unwrap();
        }
        assert!(host.view_messages().is_empty(), "a bad pid is not echoed back");

        plugin
            .handle(
                VIEW,
                ViewEvent::Message(r#"{"type":"arthas.pick-target","pid":"5151","app":"Gateway"}"#.into()),
                &host.invocation(),
            )
            .unwrap();
        let (_, reply) = host.view_messages().pop().expect("the picked target");
        assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["pid"], "5151");

        plugin.execute("monitor com.demo.OrderService createOrder -c 5");
        let terminals = host.terminals();
        assert_eq!(terminals[0].args, ["-jar", "/opt/arthas/arthas-boot.jar", "5151"]);
    }

    #[test]
    fn copy_never_opens_a_terminal_and_still_vets_the_command() {
        let host = MockHost::install().with_capabilities(&["clipboard", "process:java"]);
        host.set_setting("arthasJar", r#""/opt/arthas/arthas-boot.jar""#);
        host.set_setting("targetPid", r#""4242""#);
        let plugin = Arthas::default();

        let result = plugin.copy("getstatic com.demo.OrderService TIMEOUT");
        assert_eq!(result["ok"], true);
        assert_eq!(result["terminal"], false);
        assert_eq!(host.terminals().len(), 0, "copy is not run");
        assert_eq!(host.clipboard().as_deref(), Some("getstatic com.demo.OrderService TIMEOUT"));

        assert_eq!(plugin.copy("bash -c whoami")["ok"], false);
    }

    #[test]
    fn execute_refuses_anything_but_a_known_verb() {
        let host = MockHost::install().with_capabilities(&["clipboard", "process:java"]);
        let plugin = Arthas::default();
        for bad in ["rm -rf /", "watch x y\nrm -rf /", "", "arthas watch", "jps -l"] {
            let result = plugin.execute(bad);
            assert_eq!(result["ok"], false, "{bad}");
        }
        assert_eq!(host.clipboard(), None);
        assert_eq!(host.terminals().len(), 0);
    }

    #[test]
    fn every_verb_the_catalog_can_emit_is_accepted() {
        let host = MockHost::install().with_capabilities(&["clipboard"]);
        let plugin = Arthas::default();
        for verb in VERBS {
            let command = format!("{verb} com.demo.OrderService createOrder");
            assert_eq!(plugin.execute(&command)["ok"], true, "{verb}");
            assert_eq!(host.clipboard().as_deref(), Some(command.as_str()));
        }
    }

    #[test]
    fn the_jvm_list_comes_from_jps_and_skips_itself() {
        let host = MockHost::install().with_capabilities(&["process:jps"]);
        host.on_process("jps", |_, _| {
            Ok(ProcessOutput {
                status: Some(0),
                stdout: "4242 com.demo.Application\n5151 /opt/app/boot.jar\n909 jdk.jcmd/sun.tools.jps.Jps\n"
                    .to_owned(),
                stderr: String::new(),
            })
        });
        let mut plugin = Arthas::default();
        plugin.handle(VIEW, ViewEvent::Message(r#"{"type":"arthas.targets"}"#.into()), &host.invocation()).unwrap();
        let (_, reply) = host.view_messages().pop().expect("a message");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(
            reply["list"],
            json!([{ "pid": "4242", "app": "com.demo.Application" }, { "pid": "5151", "app": "/opt/app/boot.jar" }])
        );
    }

    #[test]
    fn a_refresh_from_the_page_answers_with_the_catalog_or_an_error() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        let mut plugin = Arthas::default();
        // No editor: the page still gets an answer, so it can say why it is empty.
        plugin.handle(VIEW, ViewEvent::Message(r#"{"type":"arthas.refresh"}"#.into()), &host.invocation()).unwrap();
        let (_, reply) = host.view_messages().pop().expect("an error");
        assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["type"], "arthas.error");

        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(15, 8, 15, 8)]);
        plugin.handle(VIEW, ViewEvent::Message(r#"{"type":"arthas.refresh"}"#.into()), &host.invocation()).unwrap();
        let (_, reply) = host.view_messages().pop().expect("the catalog");
        assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["type"], "arthas.catalog");
    }

    #[test]
    fn the_panel_keeps_the_family_the_menu_picked_until_it_asks() {
        let host = MockHost::install().with_capabilities(&["editor.read"]);
        let line = line_of(SERVICE, "return null;");
        host.open_editor("/p/OrderService.java", "java", SERVICE, &[(line, 8, line, 8)]);
        let mut plugin = Arthas::default();
        plugin.run_command("arthas.tt", &host.invocation(), None).unwrap();

        // The frame may not have been up when the menu ran: the page asks again
        // when it loads, and that answer must land on `tt`.
        plugin.handle(VIEW, ViewEvent::Message(r#"{"type":"arthas.refresh"}"#.into()), &host.invocation()).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        let message: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(message["type"], "arthas.catalog");
        assert_eq!(message["focus"], "tt");

        // Spent once: a later refresh keeps whatever the panel already shows.
        plugin.handle(VIEW, ViewEvent::Message(r#"{"type":"arthas.refresh"}"#.into()), &host.invocation()).unwrap();
        let (_, message) = host.view_messages().pop().expect("a message");
        assert!(serde_json::from_str::<Value>(&message).unwrap().get("focus").is_none());
    }

    #[test]
    fn garbage_from_the_frame_is_ignored() {
        let host = MockHost::install();
        let mut plugin = Arthas::default();
        plugin.handle(VIEW, ViewEvent::Message("not json".into()), &host.invocation()).unwrap();
        plugin.handle(VIEW, ViewEvent::Message("{}".into()), &host.invocation()).unwrap();
        assert!(host.view_messages().is_empty());
    }
}
