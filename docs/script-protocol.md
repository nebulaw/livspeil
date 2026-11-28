# Script worker protocol (version 1)

Scripts (Python first; TypeScript and Lua later) run **out of process**
(decisions D5 and D13). They never get in-process access to the kernel:
they talk to their host over this protocol, they read the graph through
queries, and they change a document **only** by sending commands to the
command bus. So validation, undo/redo and the journal apply to script edits
exactly as to UI edits, and every script edit is journaled with origin
`script`. What a script may do is decided by **capability tokens** granted
by the host.

Implementations: host `crates/livspeil-script` (Rust, GPL-3.0-or-later),
client `sdk/python/livspeil` (Python 3.10+, MPL-2.0, standard library
only). The command line runs scripts with
`livspeil run <room> <script.py> [--grant CAP]...`.

## 1. Transport

The host starts the script as a child process (for Python:
`python3 -B <script>`, with the SDK on `PYTHONPATH`) and talks to it over
the child's **stdin** (host to script) and **stdout** (script to host).
Framing is **JSON Lines**: every message is one JSON object encoded as
UTF-8 on one line, terminated by `\n`. Blank lines are ignored; a trailing
`\r` is tolerated.

Why lines rather than length prefixes: JSON escapes newlines inside
strings, so a line is always exactly one message; every language can read
a line and parse JSON with its standard library; messages are readable in a
terminal and in logs; and the kernel's journal already uses JSON Lines. The
cost, no binary payloads, does not matter here (values are numbers, short
strings and small tables). Length-prefixed framing would add nothing but
another place to get wrong.

- **stdout is reserved for the protocol.** A script must not write
  anything else there. The Python SDK moves file descriptor 1 to stderr
  when it connects, so `print()` and stray library output end up on
  stderr, and replaces stdin with `/dev/null`.
- **stderr** is free text. The host keeps each line as a log line of level
  `stderr` (tracebacks land here).
- Limits: a line from the script longer than `max_message_bytes` (8 MiB by
  default) kills the script. Floats must be finite (JSON has no NaN); the
  host sends a non-finite output value as `null`.
- The worker's environment is cleared except for `PATH`, `HOME`, `TMPDIR`
  and the locale variables, so secrets in the host's environment do not
  leak to scripts. The host sets `LIVSPEIL_PROTOCOL=1` (the SDK refuses to
  connect without it) and `LIVSPEIL_MEMORY_LIMIT` (see section 8).
  `livspeil run` starts the script in the room package directory.

## 2. Messages

Three shapes, modeled on JSON-RPC 2.0 without the `jsonrpc` member and
with string error codes:

| Shape | Members | Answered |
|---|---|---|
| request | `id`, `method`, `params` | by exactly one response with the same `id` |
| response | `id` and **either** `result` **or** `error` | never |
| notification | `method`, `params` (no `id`) | never |

```json
{"id":7,"method":"graph.node","params":{"node":"/lamp"}}
{"id":7,"result":{"id":"0192f5a0-...","name":"lamp", "...": "..."}}
{"method":"log","params":{"level":"info","message":"hello"}}
```

- `id` is an integer or a string, chosen by the sender. Each side numbers
  its own requests; a response always answers a request of the *other*
  side, so the two id spaces never collide.
- `params` is an object (absent means `{}`). Unknown members of `params`
  and of results are ignored, so both can grow compatibly.
- Both sides may send requests. The host serves script requests one at a
  time, in order; while the host waits for the answer to one of its own
  requests (`tick`, `event`, `shutdown`) it keeps serving the script's
  requests, so a handler may query and edit the graph.
- Unknown notifications are ignored. Unknown request methods are answered
  with `unknown_method`.

## 3. Lifecycle

```text
host                                   script
 | spawn ------------------------------> |
 | <------------------ hello (request)   |  must be the first request
 | result: protocol, capabilities ---->  |
 | <----- graph.* requests / log ...     |  the main body
 |  ... responses ------------------->   |
 |                                       |
 |  either: the script exits (status 0): done
 |  or:     <------ listen (request)     |  main body done, react from now on
 | result ---------------------------->  |
 | event / tick (requests) ---------->   |  per frame; the script may send
 | <-------------- graph.* / responses   |  requests while handling them
 | shutdown (request) --------------->   |
 | <------------------------- result     |
 |  stdin closed; the script exits       |
```

1. **Handshake.** The script's first request must be `hello`. Any other
   request before it is answered with `handshake_required`. A script that
   does not say hello within the handshake timeout (10 s) is killed.
2. **Main body.** The script sends requests and notifications. When it
   exits with status 0 the session is over (`Outcome::Exited`).
3. **Listening (optional).** `listen` ends the main body; afterwards the
   script only acts while handling host requests. The host sends an
   `event` per fired scene event and a `tick` per frame, then `shutdown`.
4. **Shutdown.** The host sends `shutdown`, the script answers and exits;
   the host then closes stdin and waits up to `exit_grace` (2 s) before
   killing it. A script that exits on its own at any time is fine; a
   non-zero exit status is a crash.

## 4. Methods

### Script to host

| Method | Capability | Params | Result |
|---|---|---|---|
| `hello` | none | `protocol` (integer, `1`), `client` (string, optional) | `protocol`, `host`, `session`, `capabilities` (tokens), `room` (`{name}`), `limits` (`request_timeout_ms`, `run_timeout_ms`, `max_message_bytes`) |
| `graph.nodes` | `graph.read` | `network` (subnet path, default root), `recursive` (default `true`) | `nodes`: `[{id, name, path, type, parent, subnet}]`, sorted by path |
| `graph.node` | `graph.read` | `node` | `{id, name, path, type, summary, parent, subnet, port_type, charm, inputs, outputs}` |
| `graph.snapshot` | `graph.read` | none | the kernel's `GraphSnapshot` (`nodes`, `links`) plus `paths` (id to path) |
| `graph.apply` | `graph.write` | `command` (a command, below), `group` (integer, optional) | `changes` (list), `undo_len` |
| `graph.new_id` | `graph.write` | `count` (1 to 1024, default 1) | `ids`: fresh node ids |
| `graph.cook` | `graph.cook` | `nodes` (list of node refs), `time`, `frame`, `dt` (optional) | `clock` (`{frame, time, dt}`), `results`: `[{node, id, outputs, error?}]` |
| `listen` | none; `graph.read` for topic `event` | `topics`: subset of `["tick", "event"]` (default `["tick"]`) | `topics` |
| `log` (notification) | none | `level` (`debug`, `info`, `warn`, `error`; default `info`), `message` | |
| `cancel` (notification) | none | `id` | (accepted and ignored: the host answers synchronously) |

**Node references** (`node`, `nodes`): a node id (UUID string) or a path
from the root network, `/lamp/rig/bulb` or `lamp/rig/bulb`.

**`graph.node` ports.** `inputs`: `[{name, type, value, default, link,
expression, animated}]` where `value` is the explicitly set static value
(`null` when unset), `default` the spec default, `link` the feeding output
`{node, id, port}` or `null`, `expression` the source text or `null`.
`outputs`: `[{name, type, links}]` with the inputs each output feeds.

**Values** use the kernel's serde form (externally tagged): `{"Float":1.5}`,
`{"Int":3}`, `{"Bool":true}`, `{"String":"hi"}`, `{"Vec2":[x,y]}`,
`{"Vec3":[...]}`, `{"Vec4":[...]}`, `{"Color":[l,c,h,a]}` (OKLCH plus
alpha), `"Event"`, and objects for `Table` and `Curve`. An `Int` is
accepted for a `Float` input.

**`graph.apply`.** `command` is exactly the JSON form of
`livspeil_cmd::Command`, the same as in the journal:

```json
{"set_input":{"node":"0192f5a0-7c3e-7a10-8000-000000000009","port":"out_max","value":{"Float":0.16}}}
{"transaction":[{"set_input":{...}},{"link":{"from":"…","from_port":"out","to":"…","to_port":"a"}}]}
```

`{"set_position":{"node":"…","position":[360.0,40.0]}}` places a node in the
node editor; `"position":null` unplaces it, and the field is required.

Commands name nodes by **id** (the SDK resolves paths for you). New nodes
need ids: take them from `graph.new_id`. The command is applied with
`History::apply` and `Origin::Script`: it is validated (an invalid command
changes nothing and is answered with `command_failed`), it becomes an undo
step, and its journal entry (`{"kind":"do","origin":"script",...}`) is
appended to the scene's journal. A transaction is all or nothing and one
undo step. Consecutive applies with the same `group` join one undo step;
script groups are mapped to fresh host groups, so they never merge with a
user's gesture. `changes` lists the document changes as
`{"kind": "input_changed" | "node_added" | "node_removed" | "linked" |
"unlinked" | "renamed" | "expression_changed" | "animation_changed" |
"node_moved" | "charm_changed" | "position_changed", ...}` with `node` (id), `path`, `port` and
the like.

**`graph.cook`.** Without `time`/`frame`/`dt` the nodes are cooked on the
scene's cooker at its current clock (inside a `tick` handler: that tick,
with the scene's event state). With any of them, the host cooks on a
**session-private** cooker at that clock (missing fields from the scene's
clock), so a script never moves the scene's clock. `outputs` maps output
names to values; a node that failed has `error` (and, if it ever produced
one, its last good `outputs`). Cooking never changes the document.

### Host to script

| Method | Params | Expected result |
|---|---|---|
| `event` (request) | `kind`: `"scene"` with `node` (path), `id`, `port`, `payload` (value or `null`) for an event fired by a node output during a tick; other kinds are host-defined, for example `{"kind":"doc_changed","origin":"ui","changes":[...]}` | `null` |
| `tick` (request) | `frame`, `time`, `dt` (seconds), `fired` (number of scene events in the frame) | `null` |
| `shutdown` (request) | `reason` | `null`, then exit |
| `cancel` (notification) | `id` of a host request | |

`event` and `tick` are *requests*, not notifications, so the host knows
when a handler has finished (and can time it) before it sends the next
frame. Per frame the host runs the scene cooker's tick, sends one `event`
per fired event (in firing order) if the script listens to `event`, then
one `tick` if it listens to `tick`.

## 5. Errors

A failed request is answered with
`{"id":…,"error":{"code":"…","message":"…","data":{…}}}`. `code` is a
stable string; `message` is for people; `data` is optional detail.

| Code | Meaning | `data` |
|---|---|---|
| `parse_error` | a line was not JSON (answered with `id: null`) | |
| `invalid_request` | not a valid message, `hello` twice, `listen` at the wrong time, or a request while shutting down | |
| `unknown_method` | | |
| `invalid_params` | params do not fit the method (including a malformed command) | |
| `handshake_required` | a request before `hello` | |
| `unsupported_protocol` | `hello` asked for another version; the host then kills the script | `supported`: `[1]` |
| `capability_denied` | the session lacks the capability | `capability`, `method` |
| `not_found` | a node reference names no node | `node` |
| `command_failed` | the command bus rejected the command; nothing changed | `step`: indices into nested transactions, when inside one |
| `cancelled` | the request was cancelled | |
| `handler_failed` | a script's `tick`/`event` handler raised | |
| `internal` | a host bug | |

Errors are per request: a denied or failed request does not end the
session. The SDK raises them as exceptions (`CapabilityDenied`, `NotFound`,
`CommandFailed`, `RemoteError`); an uncaught one ends the script with a
non-zero status, which the host reports as a crash.

The host's own failures (`livspeil_script::ScriptError`): `Spawn`,
`Handshake`, `UnsupportedProtocol`, `Timeout { method }`, `RunTimeout`,
`MessageTooLarge`, `Crashed { code, stderr }` (with the last stderr lines,
so a Python traceback is visible), `Exited { method }` (the script exited
while the host waited for its answer), `HandlerFailed { method, error }`
and `InvalidState`. All but the last two end the session with the worker
gone. Edits applied before a failure stay applied, and each stays
undoable; `livspeil run` does not save the room when the script failed.

## 6. Timeouts and cancellation

| Limit | Default | On expiry |
|---|---|---|
| handshake | 10 s | killed (`Timeout { method: "hello" }`) |
| per host request (`tick`, `event`, `shutdown`) | 5 s | `cancel` notification, then `cancel_grace` (250 ms); killed if still unanswered |
| total run time | 60 s | killed (`RunTimeout`) |
| exit after stdout closes or `shutdown` | 2 s | killed |

Cancellation is cooperative. On `cancel {id}` the script should stop the
handler and answer the request (the Python SDK answers `cancelled` when a
handler returns after its request was cancelled; long handlers can poll
`session.cancelled()`). If the script answers within the grace period, the
host reports a timeout for that request but the session continues;
otherwise the worker is killed. Clients may also time out their own
requests and send `cancel`; since the host answers every request
synchronously and promptly, it ignores the notification, and the client
skips the late answer.

## 7. Capabilities

Tokens are granted by the host when the session starts (`livspeil run
--grant`) and reported in the `hello` result. The default is **none**: a
script without grants can say hello, log, and listen to ticks.

| Token | Allows |
|---|---|
| `graph.read` | `graph.nodes`, `graph.node`, `graph.snapshot`, listening to `event` |
| `graph.write` | `graph.apply`, `graph.new_id` |
| `graph.cook` | `graph.cook` |
| `fs.read:<root>` | when the script cooks, nodes that read the file system (`core.fs.*`) see files under `<root>`; without it they have none. One root per session; it must be an existing directory. |

Every request is checked against the table before it runs. Capabilities
scope **the kernel API only**: they are not an operating-system sandbox. A
Python script can still open files and sockets itself, as any program the
user runs. OS-level confinement (Landlock and seccomp on Linux, the macOS
sandbox, or bubblewrap) is future work; until then, run only scripts you
trust, as with any program.

## 8. Resource guards

Enforced by the host: the timeouts above, the message size limit, a cap on
kept log text (1 MiB; further lines are counted, not kept), and the
cleared environment. Advisory: `LIVSPEIL_MEMORY_LIMIT` (1 GiB by default)
is passed to the worker, and the Python SDK applies it as `RLIMIT_AS` where
the platform allows; a worker that ignores it is still bounded by the run
time limit.

## 9. Versioning

`protocol` is a single integer. Compatible additions (new methods, new
optional params, new result members, new error codes, new event kinds) keep
version 1; clients ignore what they do not know. Anything else bumps the
version, and the host lists what it supports in the `unsupported_protocol`
error.

## 10. Example session

Script lines are marked `>`, host lines `<`:

```text
> {"id":1,"method":"hello","params":{"protocol":1,"client":"livspeil-python/0.0.0"}}
< {"id":1,"result":{"protocol":1,"host":"livspeil-script 0.0.0","session":"dim_lamps.py","capabilities":["graph.read","graph.write"],"room":{"name":"Sun and Lamp"},"limits":{"request_timeout_ms":5000,"run_timeout_ms":60000,"max_message_bytes":8388608}}}
> {"id":2,"method":"graph.node","params":{"node":"lamp_intensity"}}
< {"id":2,"result":{"id":"0192f5a0-7c3e-7a10-8000-000000000009","name":"lamp_intensity","path":"/lamp_intensity","type":"core.math.remap@1","inputs":[...],"outputs":[...]}}
> {"id":3,"method":"graph.apply","params":{"command":{"set_input":{"node":"0192f5a0-7c3e-7a10-8000-000000000009","port":"out_max","value":{"Float":0.16}}}}}
< {"id":3,"result":{"changes":[{"kind":"input_changed","node":"0192f5a0-7c3e-7a10-8000-000000000009","path":"/lamp_intensity","port":"out_max"}],"undo_len":1}}
> {"id":4,"method":"graph.cook","params":{"nodes":["sun"],"time":5.0}}
< {"id":4,"result":{"clock":{"frame":0,"time":5.0,"dt":0.0},"results":[{"node":"/sun","id":"…","outputs":{"direction":{"Vec3":[-0.917,0.265,-0.297]},"elevation":{"Float":15.35},"azimuth":{"Float":287.9}}}]}}
> {"method":"log","params":{"level":"info","message":"done"}}
```

The script then exits with status 0.
