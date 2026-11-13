# Arthas Commander for OpenIDEA

Build [Arthas](https://arthas.aliyun.com/) commands from the code you are reading. Right-click a
Java or Kotlin file and pick what you want to know — the class, method, overloads and modifiers come
from the caret, so the command is complete before you paste it into a terminal.

The command set follows the official
[arthas-idea](https://plugins.jetbrains.com/plugin/13581-arthas-idea) plugin: thirteen families,
forty-odd variants, the same flags and the same defaults.

## What it builds

| Family | What you get |
| --- | --- |
| `watch` | Method entry/exit/exception, a static or instance field, or a forced invocation with the class, method and descriptors filled in |
| `trace` | The call path with depth, `-n` and an optional condition |
| `stack` | Who calls this method — the reverse of `trace` |
| `monitor` | Periodic statistics with a count and an interval |
| `tt` | Record, list, inspect a call, replay it with optional replay times/interval |
| `ognl` | A static field, an instance field through `@Class@`, or a Spring bean through the context |
| `field` | Read a `getstatic`, write a `setstatic` (including `static final`), call the setter, or list `instances` |
| `spring` | One property from your selection, or every property source of the context in priority order |
| `logger` | Change a logger level at runtime, or list the current ones |
| `profiler` | async-profiler `start` / `stop`, with an auto-flame-graph variant that also writes an HTML |
| `jad` / `sc` / `sm` / `dump` | Decompile, search classes and methods, dump loaded bytecode |

## The panel

`Arthas ›` in the editor context menu (or `⌘⌥A` / `Ctrl+Alt+A`) opens a tool window with the whole
command family for the caret you came from. Every `${FIELD}` is editable in place, the rendered
command updates as you type, and each one can be copied or run. An unusable family says why instead
of offering a broken command — "this caret is on a field, `trace` needs a method", and so on.

Settings › Plugins › Arthas Commander sets the defaults used when a field is first filled:
depth, `-n` counts, the monitor interval, `--verbose`, the watch condition, whether JDK methods are
skipped, the Spring context variable, the profiler event/format/duration and the logger level.

## Capabilities

The install page asks for these; here is what each is for.

| Capability | Risk | Why |
| --- | --- | --- |
| `editor.read` | Low | The only input: the file's package and type chain, the member around the caret, and the selected text (used as a Spring property key). Nothing is written to the editor. |
| `process:java` | High | Runs `java -jar arthas-boot.jar` in a terminal **only** when you configure a jar path and press *Run*; the command is otherwise only copied to the clipboard. The jar is the official Arthas boot launcher you point the setting at. |
| `clipboard` | Medium | *Copy* puts the built command on the clipboard, which is the main way it is used — paste it into a session that is already attached. |
| `process:jps` | Optional | Lists the local JVMs so the target process can be picked instead of typed. Without the grant the picker stays empty and you type the pid. |

No network access, no project files are read, and the plugin keeps no storage.

## Installing

From the marketplace once listed: Settings › Plugins › Available › search *Arthas*.

Off a package: Settings › Plugins › **Install from Disk…** and pick
`allen.arthas-0.2.1.oiplugin`.

Or link a checkout and iterate on it:

```sh
openidea-plugin build     # → dist/plugin.wasm, then `check`
openidea-plugin link      # load this folder in the IDE
```

`build` needs the `wasm32-wasip2` target and the Rust SDK from the OpenIDEA release the plugin
declares (`openidea-plugin = { git = … , tag = "v0.1.39" }`).

## Using it

1. Put the caret on a method, a field or a type in a Java or Kotlin file.
2. Right-click › `Arthas ›` (code-scoped families) or `Arthas App ›` (Spring, logger, profiler).
3. Edit the fields in the panel; *Copy* the command, or set **arthasJar** and press *Run* to open it
   in a terminal against the chosen pid.

Nothing is executed by looking at a command: *Copy* only fills the clipboard, and *Run* requires a
configured jar and a target process.

## Licence

MIT.
