<!-- Generated from crates/livspeil-nodes-core/specs/*.node.toml by
     `cargo xtask node-specs` (crates/livspeil-nodes-core/tests/node_specs.rs).
     Do not edit by hand. -->

# Core nodes

Every built-in node is described by a spec file
`crates/livspeil-nodes-core/specs/<id>.node.toml` (kernel spec, section 9;
`<id>@<version>.node.toml` for versions other than 1). The spec file is
the node's documentation and its table of test cases; the Rust `cook` body
is the only other hand-written part. How to add a node:
[CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-node).

## Spec file format

```toml
id = "core.math.lerp"           # generated from the Rust NodeSpec
version = 1                     # generated
summary = "Linear interpolation between a and b"   # generated
time_dependent = false          # generated: re-cooked every frame
reads_system = false            # generated: reads the host system (files)
description = '''
Optional, hand-written, multi-line documentation (Markdown).
'''

[[inputs]]                      # generated, one per input port, in order
name = "a"
type = "Float"
default = 0.0

[[outputs]]                     # generated, one per output port, in order
name = "out"
type = "Float"

[[tests]]                       # hand-written, any number
name = "spec example"           # optional
inputs = { b = 10.0, t = 0.25 } # inputs left out take their default
expect = { out = 2.5 }          # outputs left out are not checked

[[tests]]
inputs = { t = 0.1 }
expect = { out = 0.1 }
tolerance = 1e-12               # optional absolute tolerance for floats

[[tests]]                       # (as in core.math.div, which can fail)
inputs = { b = 0.0 }
expect_error = { port = "b", message = "division by zero" }
```

Everything above the first `[[tests]]` except `description` is generated
from the Rust `NodeSpec` by `cargo xtask node-specs` and must match it
exactly; the generator keeps `description` and the `[[tests]]` (copied
verbatim from the first `[[tests]]` line to the end of the file). Other
text above `[[tests]]`, such as comments, is not kept.

`[[tests]]` keys:

| Key | Meaning |
|---|---|
| `name` | Optional label shown when the case fails. |
| `note` | Optional free text. |
| `inputs` | Port name to value; other inputs take their defaults. |
| `context` | Optional `{ frame = 12, time = 0.2, dt = 0.016 }` cook context (default all zero). |
| `fs` | Optional in-memory file system for nodes that read the system: `{ dirs = [{ path = "a", modified = 0 }], files = [{ path = "a/b.txt", size = 10, modified = 3 }] }`. Without it the node gets no system access. |
| `expect` | Output port name to expected value. Floats compare exactly (`-0.0` differs from `0.0`; `nan` matches `nan`) unless `tolerance` is set. |
| `tolerance` | Optional absolute tolerance for every float compared (also inside vectors, colors and tables). |
| `expect_error` | The cook must fail: `port` (an invalid-input error on that port) and/or `message` (a substring of the error text); `{}` accepts any error. |

### Values

Values are written by port type:

| Type | TOML | Example |
|---|---|---|
| `Float` | float (an integer is accepted); `nan`, `inf`, `-inf` | `0.5`, `-0.0`, `1e-7` |
| `Int` | integer | `-42` |
| `Bool` | boolean | `true` |
| `String` | string | `"text"` |
| `Vec2`, `Vec3`, `Vec4` | array of 2, 3 or 4 numbers | `[1.0, 2.0, 3.0]` |
| `Color` | OKLCH plus alpha, `[l, c, h, alpha]` (alpha may be left out: 1.0) | `[0.7, 0.1, 250.0, 1.0]` |
| `Table` | `{ columns = [{ name, type, values = [...] }, ...] }` | `{ columns = [{ name = "name", type = "String", values = ["a", "b"] }] }` |
| `Curve` | `{ keys = [{ position, value, interpolation?, tangent? }, ...], pre?, post? }` | `{ keys = [{ position = 0.0, value = 0.0 }, { position = 1.0, value = 1.0 }] }` |

Curve keys: `interpolation` is `"linear"` (default), `"step"`, `"bezier"`
or `{ spring = { frequency = 2.0, damping = 0.3 } }`; `tangent` is
`"auto"` (default), `"flat"` or `{ incoming = 0.0, outgoing = 4.0 }`;
`pre` and `post` are `"hold"` (default), `"loop"` or `"ping_pong"`.

Large values (tables) read better as TOML sub-tables, for example
`[[tests.inputs.table.columns]]` with `name`, `type` and `values` keys.
A default of a type the format cannot write yet is recorded as
`default_debug` (its Rust `Debug` text) instead of `default`.

## Node index

43 nodes. *time* marks nodes re-cooked every frame, *system* nodes that read the host system.

| Node | Summary | Inputs | Outputs | Tests |
|---|---|---|---|---|
| [`core.color.compose@1`](#corecolorcompose1) | Build an OKLCH color (l, alpha clamped to [0, 1]; c >= 0; h wrapped to [0, 360)) | `l: Float = 0.7`<br>`c: Float = 0.1`<br>`h: Float = 250.0`<br>`alpha: Float = 1.0` | `out: Color` | 4 |
| [`core.color.gradient@1`](#corecolorgradient1) | Three-stop OKLCH gradient (c0 at 0, c1 at p1, c2 at 1) sampled at t, mixed in OKLab | `t: Float = 0.5`<br>`c0: Color = [0.3, 0.1, 270.0, 1.0]`<br>`c1: Color = [0.7, 0.15, 45.0, 1.0]`<br>`c2: Color = [0.95, 0.05, 95.0, 1.0]`<br>`p1: Float = 0.5` | `out: Color` | 4 |
| [`core.color.mix@1`](#corecolormix1) | Mix two OKLCH colors in OKLab space (t clamped to [0, 1]) | `a: Color = [0.8, 0.12, 60.0, 1.0]`<br>`b: Color = [0.9, 0.08, 80.0, 1.0]`<br>`t: Float = 0.5` | `out: Color` | 4 |
| [`core.curve.ramp@1`](#corecurveramp1) | Sample a curve at a position | `curve: Curve = { keys = [{ position = 0.0, value = 0.0 }, { position = 1.0, value = 1.0 }] }`<br>`position: Float = 0.0` | `out: Float` | 5 |
| [`core.fs.list_dir@1`](#corefslist_dir1) | List the files of a folder as a table (name, path, is_dir, size, modified) *(system)* | `path: String = "."`<br>`pattern: String = "*"`<br>`include_hidden: Bool = false`<br>`sort: String = "name"` | `entries: Table`<br>`count: Int` | 6 |
| [`core.logic.and@1`](#corelogicand1) | True when both a and b are true | `a: Bool = true`<br>`b: Bool = true` | `out: Bool` | 3 |
| [`core.logic.compare@1`](#corelogiccompare1) | Compare a and b with op (<, <=, ==, !=, >=, >) | `a: Float = 0.0`<br>`b: Float = 0.0`<br>`op: String = "<"` | `out: Bool` | 5 |
| [`core.logic.not@1`](#corelogicnot1) | Negate a | `a: Bool = false` | `out: Bool` | 2 |
| [`core.logic.or@1`](#corelogicor1) | True when a or b (or both) is true | `a: Bool = false`<br>`b: Bool = false` | `out: Bool` | 3 |
| [`core.logic.select@1`](#corelogicselect1) | Pick a when cond is true, else b | `cond: Bool = false`<br>`a: Float = 1.0`<br>`b: Float = 0.0` | `out: Float` | 3 |
| [`core.math.abs@1`](#coremathabs1) | Absolute value of x | `x: Float = 0.0` | `out: Float` | 3 |
| [`core.math.add@1`](#coremathadd1) | Add two numbers | `a: Float = 0.0`<br>`b: Float = 0.0` | `out: Float` | 3 |
| [`core.math.clamp@1`](#coremathclamp1) | Limit x to the range [min, max] (bounds are swapped if reversed) | `x: Float = 0.0`<br>`min: Float = 0.0`<br>`max: Float = 1.0` | `out: Float` | 4 |
| [`core.math.cos@1`](#coremathcos1) | Cosine of x (radians) | `x: Float = 0.0` | `out: Float` | 3 |
| [`core.math.div@1`](#coremathdiv1) | Divide a by b (b must not be zero) | `a: Float = 0.0`<br>`b: Float = 1.0` | `out: Float` | 5 |
| [`core.math.lerp@1`](#coremathlerp1) | Linear interpolation between a and b | `a: Float = 0.0`<br>`b: Float = 1.0`<br>`t: Float = 0.5` | `out: Float` | 3 |
| [`core.math.max@1`](#coremathmax1) | The larger of a and b | `a: Float = 0.0`<br>`b: Float = 0.0` | `out: Float` | 3 |
| [`core.math.min@1`](#coremathmin1) | The smaller of a and b | `a: Float = 0.0`<br>`b: Float = 0.0` | `out: Float` | 4 |
| [`core.math.mul@1`](#coremathmul1) | Multiply a by b | `a: Float = 0.0`<br>`b: Float = 1.0` | `out: Float` | 4 |
| [`core.math.pow@1`](#coremathpow1) | base raised to the power exponent | `base: Float = 0.0`<br>`exponent: Float = 1.0` | `out: Float` | 4 |
| [`core.math.remap@1`](#coremathremap1) | Map x linearly from [in_min, in_max] to [out_min, out_max] | `x: Float = 0.0`<br>`in_min: Float = 0.0`<br>`in_max: Float = 1.0`<br>`out_min: Float = 0.0`<br>`out_max: Float = 1.0`<br>`clamp: Bool = false` | `out: Float` | 5 |
| [`core.math.round@1`](#coremathround1) | Round x to the nearest multiple of step (halves away from zero) | `x: Float = 0.0`<br>`step: Float = 1.0` | `out: Float` | 5 |
| [`core.math.sin@1`](#coremathsin1) | Sine of x (radians) | `x: Float = 0.0` | `out: Float` | 3 |
| [`core.math.smoothstep@1`](#coremathsmoothstep1) | Smooth Hermite step from 0 to 1 as x goes from edge0 to edge1 | `edge0: Float = 0.0`<br>`edge1: Float = 1.0`<br>`x: Float = 0.0` | `out: Float` | 5 |
| [`core.math.sub@1`](#coremathsub1) | Subtract b from a | `a: Float = 0.0`<br>`b: Float = 0.0` | `out: Float` | 3 |
| [`core.math.wrap@1`](#coremathwrap1) | Wrap x into [min, max) (bounds are swapped if reversed) | `x: Float = 0.0`<br>`min: Float = 0.0`<br>`max: Float = 1.0` | `out: Float` | 5 |
| [`core.output@1`](#coreoutput1) | An output of the enclosing subnet, or a sink of the root network | `value: Float = 0.0` | `value: Float` | 2 |
| [`core.sky.sun_direction@1`](#coreskysun_direction1) | Sun direction (y up, x east, z south), elevation and azimuth from day, solar time and latitude | `day_of_year: Float = 172.0`<br>`solar_hour: Float = 12.0`<br>`latitude: Float = 41.7` | `direction: Vec3`<br>`elevation: Float`<br>`azimuth: Float` | 5 |
| [`core.subnet.input@1`](#coresubnetinput1) | An input port of the enclosing subnet | `value: Float = 0.0` | `value: Float` | 2 |
| [`core.subnet@1`](#coresubnet1) | A node that contains a network (a Charm when it has metadata) | none | none | 1 |
| [`core.table.column@1`](#coretablecolumn1) | One column of a table, as a list | `table: Table = { columns = [] }`<br>`name: String = "name"` | `out: Table` | 3 |
| [`core.table.count@1`](#coretablecount1) | Number of rows and columns of a table | `table: Table = { columns = [] }` | `count: Int`<br>`columns: Int` | 3 |
| [`core.table.filter_text@1`](#coretablefilter_text1) | Keep the rows whose text column contains a substring | `table: Table = { columns = [] }`<br>`column: String = "name"`<br>`text: String = ""`<br>`ignore_case: Bool = false` | `out: Table`<br>`count: Int` | 4 |
| [`core.table.get@1`](#coretableget1) | One cell of a table as text and number | `table: Table = { columns = [] }`<br>`row: Int = 0`<br>`column: String = "name"` | `text: String`<br>`number: Float`<br>`found: Bool` | 6 |
| [`core.table.sort@1`](#coretablesort1) | Sort the rows of a table by a column | `table: Table = { columns = [] }`<br>`column: String = "name"`<br>`ascending: Bool = true` | `out: Table` | 4 |
| [`core.text.format@1`](#coretextformat1) | Fill {a} and {b} in a template with numbers ({{ and }} for braces) | `template: String = "{a}"`<br>`a: Float = 0.0`<br>`b: Float = 0.0`<br>`precision: Int = 2` | `out: String` | 5 |
| [`core.time.clock@1`](#coretimeclock1) | The frame clock *(time)* | none | `frame: Int`<br>`time: Float`<br>`dt: Float` | 2 |
| [`core.time.timeofday@1`](#coretimetimeofday1) | Day phase (0 = midnight, 0.5 = noon) and hour from seconds since local midnight | `seconds: Float = 43200.0`<br>`offset: Float = 0.0` | `phase: Float`<br>`hour: Float` | 5 |
| [`core.vector.compose3@1`](#corevectorcompose31) | Build a Vec3 from x, y and z | `x: Float = 0.0`<br>`y: Float = 0.0`<br>`z: Float = 0.0` | `out: Vec3` | 2 |
| [`core.vector.decompose3@1`](#corevectordecompose31) | Split a Vec3 into x, y and z | `v: Vec3 = [0.0, 0.0, 0.0]` | `x: Float`<br>`y: Float`<br>`z: Float` | 2 |
| [`core.vector.dot3@1`](#corevectordot31) | Dot product of two Vec3s | `a: Vec3 = [0.0, 0.0, 0.0]`<br>`b: Vec3 = [0.0, 0.0, 0.0]` | `out: Float` | 3 |
| [`core.vector.length3@1`](#corevectorlength31) | Euclidean length of a Vec3 | `v: Vec3 = [0.0, 0.0, 0.0]` | `out: Float` | 4 |
| [`core.vector.normalize3@1`](#corevectornormalize31) | Scale a Vec3 to unit length (the zero vector stays zero) | `v: Vec3 = [0.0, 0.0, 0.0]` | `out: Vec3` | 4 |

## Nodes

### core.color.compose@1

Build an OKLCH color (l, alpha clamped to [0, 1]; c >= 0; h wrapped to [0, 360))

Builds an OKLCH color: `l` and `alpha` are clamped to `[0, 1]`, `c` to
`>= 0`, and `h` is wrapped to `[0, 360)`. Negative zeros come out as `+0.0`.
A `NaN` or infinite input is an invalid-input error on that port.

Spec: [`core.color.compose.node.toml`](../crates/livspeil-nodes-core/specs/core.color.compose.node.toml)

### core.color.gradient@1

Three-stop OKLCH gradient (c0 at 0, c1 at p1, c2 at 1) sampled at t, mixed in OKLab

Three-stop OKLCH gradient: `c0` at 0, `c1` at `p1` and `c2` at 1, sampled
at `t` (clamped to `[0, 1]`; `p1` too) and mixed in OKLab as in
`core.color.mix`. The number of stops is fixed at three for now. A `NaN`
or infinite `t`, `p1` or color component is an invalid-input error.

Spec: [`core.color.gradient.node.toml`](../crates/livspeil-nodes-core/specs/core.color.gradient.node.toml)

### core.color.mix@1

Mix two OKLCH colors in OKLab space (t clamped to [0, 1])

Mixes two OKLCH colors in OKLab: `L`, OKLab `a`/`b` and alpha are
interpolated linearly by `t` (clamped to `[0, 1]`), then converted back to
OKLCH with the hue in `[0, 360)`. When the resulting chroma is below
`CHROMA_EPSILON` the hue is 0.

Spec: [`core.color.mix.node.toml`](../crates/livspeil-nodes-core/specs/core.color.mix.node.toml)

### core.curve.ramp@1

Sample a curve at a position

Samples `curve` at `position`, exactly like a Float animation channel with
the position as time: step, linear, bezier and spring segments,
hold/loop/ping-pong outside the keys. A `NaN` position gives the first
key's value, infinities the end keys' values.

Spec: [`core.curve.ramp.node.toml`](../crates/livspeil-nodes-core/specs/core.curve.ramp.node.toml)

### core.fs.list_dir@1

List the files of a folder as a table (name, path, is_dir, size, modified)

Lists the folder `path` (relative to the host's file root; absolute paths
and `..` above the root are invalid-input errors on `path`) into a table
with the columns `name`, `path` (relative to the root, `/`-separated),
`is_dir`, `size` (bytes, 0 for folders) and `modified` (Unix seconds).
`pattern` is a glob (`*`, `?`; several separated by `;`, as in
`*.pdf;*.epub`) that applies to folders too; names starting with `.` are
left out unless `include_hidden`. `sort` is `name` (Unicode scalar values),
`size` or `modified` (ties by name), always ascending.

Reads the system: the result is a pure function of the inputs and the
host's snapshot revision. Without a file system the node fails.

Spec: [`core.fs.list_dir.node.toml`](../crates/livspeil-nodes-core/specs/core.fs.list_dir.node.toml)

### core.logic.and@1

True when both a and b are true

`out = a && b`. Both inputs default to `true` (the identity), so linking only one passes it through.

Spec: [`core.logic.and.node.toml`](../crates/livspeil-nodes-core/specs/core.logic.and.node.toml)

### core.logic.compare@1

Compare a and b with op (<, <=, ==, !=, >=, >)

Compares `a` with `b` using `op`, one of `<`, `<=`, `==`, `!=`, `>=`, `>`
(IEEE 754: `NaN` compares false except with `!=`, and `-0.0 == 0.0`). Any
other `op` is an invalid-input error on `op`. `op` is a String for now and
becomes an Enum parameter later.

Spec: [`core.logic.compare.node.toml`](../crates/livspeil-nodes-core/specs/core.logic.compare.node.toml)

### core.logic.not@1

Negate a

`out = !a`.

Spec: [`core.logic.not.node.toml`](../crates/livspeil-nodes-core/specs/core.logic.not.node.toml)

### core.logic.or@1

True when a or b (or both) is true

`out = a || b`. Both inputs default to `false` (the identity), so linking only one passes it through.

Spec: [`core.logic.or.node.toml`](../crates/livspeil-nodes-core/specs/core.logic.or.node.toml)

### core.logic.select@1

Pick a when cond is true, else b

`out = cond ? a : b`. Float-only for now; it becomes polymorphic once the node API has constraint classes.

Spec: [`core.logic.select.node.toml`](../crates/livspeil-nodes-core/specs/core.logic.select.node.toml)

### core.math.abs@1

Absolute value of x

Clears the sign bit, so `abs(-0.0) == +0.0`, `abs(-inf) == inf` and a `NaN` stays `NaN`.

Spec: [`core.math.abs.node.toml`](../crates/livspeil-nodes-core/specs/core.math.abs.node.toml)

### core.math.add@1

Add two numbers

`out = a + b`.

Spec: [`core.math.add.node.toml`](../crates/livspeil-nodes-core/specs/core.math.add.node.toml)

### core.math.clamp@1

Limit x to the range [min, max] (bounds are swapped if reversed)

Limits `x` to `[min, max]`. If `min > max` the bounds are swapped, so the
node never fails and the order in which the two sliders are dragged does
not matter. A `NaN` `x` passes through; a `NaN` bound is ignored (that side
is unbounded).

Spec: [`core.math.clamp.node.toml`](../crates/livspeil-nodes-core/specs/core.math.clamp.node.toml)

### core.math.cos@1

Cosine of x (radians)

`cos(±0) == 1`; an infinite or `NaN` `x` gives `NaN`. Platform math library (see `core.math.pow`).

Spec: [`core.math.cos.node.toml`](../crates/livspeil-nodes-core/specs/core.math.cos.node.toml)

### core.math.div@1

Divide a by b (b must not be zero)

`out = a / b`. A zero `b` (either sign) is an invalid-input error on `b`,
so the scheduler keeps the last good output instead of publishing an
infinity or `NaN`. Any other divisor follows IEEE 754 (`a / inf == 0`, a
`NaN` operand gives `NaN`).

Spec: [`core.math.div.node.toml`](../crates/livspeil-nodes-core/specs/core.math.div.node.toml)

### core.math.lerp@1

Linear interpolation between a and b

`out = a + (b - a) * t`, not clamped: `t` outside `[0, 1]` extrapolates.

Spec: [`core.math.lerp.node.toml`](../crates/livspeil-nodes-core/specs/core.math.lerp.node.toml)

### core.math.max@1

The larger of a and b

The larger of `a` and `b`, deterministic on every platform: a `NaN`
operand is ignored (the other one is returned; `NaN` only if both are
`NaN`), and `+0.0 > -0.0`.

Spec: [`core.math.max.node.toml`](../crates/livspeil-nodes-core/specs/core.math.max.node.toml)

### core.math.min@1

The smaller of a and b

The smaller of `a` and `b`, deterministic on every platform: a `NaN`
operand is ignored (the other one is returned; `NaN` only if both are
`NaN`), and `-0.0 < +0.0`.

Spec: [`core.math.min.node.toml`](../crates/livspeil-nodes-core/specs/core.math.min.node.toml)

### core.math.mul@1

Multiply a by b

`out = a * b`. `b` defaults to 1, so linking only `a` passes it through.

Spec: [`core.math.mul.node.toml`](../crates/livspeil-nodes-core/specs/core.math.mul.node.toml)

### core.math.pow@1

base raised to the power exponent

`base` raised to `exponent`, from the platform math library: the IEEE
special cases are exact everywhere, ordinary results may differ in the
last bit between platforms.

Spec: [`core.math.pow.node.toml`](../crates/livspeil-nodes-core/specs/core.math.pow.node.toml)

### core.math.remap@1

Map x linearly from [in_min, in_max] to [out_min, out_max]

`out = out_min + (out_max - out_min) * t` with
`t = (x - in_min) / (in_max - in_min)`. With `clamp` set, `t` is limited to
`[0, 1]`, so the output stays between `out_min` and `out_max` (in either
order). Reversed ranges are allowed and invert the mapping.
`in_min == in_max` is an invalid-input error on `in_max`.

Spec: [`core.math.remap.node.toml`](../crates/livspeil-nodes-core/specs/core.math.remap.node.toml)

### core.math.round@1

Round x to the nearest multiple of step (halves away from zero)

Rounds `x` to the nearest multiple of `step` (halves away from zero; the
sign of `step` does not matter). When `1 / step` is a whole number (steps
such as 0.1, 0.25 or 1) the node computes `round(x * (1/step)) / (1/step)`,
so `round(0.3, 0.1)` is `0.3` rather than `0.30000000000000004`. A zero,
`NaN` or infinite `step` is an invalid-input error on `step`.

Spec: [`core.math.round.node.toml`](../crates/livspeil-nodes-core/specs/core.math.round.node.toml)

### core.math.sin@1

Sine of x (radians)

`sin(±0) == ±0`; an infinite or `NaN` `x` gives `NaN`. Platform math library (see `core.math.pow`).

Spec: [`core.math.sin.node.toml`](../crates/livspeil-nodes-core/specs/core.math.sin.node.toml)

### core.math.smoothstep@1

Smooth Hermite step from 0 to 1 as x goes from edge0 to edge1

`t = clamp((x - edge0) / (edge1 - edge0), 0, 1)`, `out = 3t² - 2t³` (as in
GLSL). `edge0 > edge1` is allowed and gives the falling curve. When
`edge0 == edge1` (undefined in GLSL) the node is a step: `out = 0` for
`x < edge0`, else `1`. A `NaN` `x` gives `NaN`.

Spec: [`core.math.smoothstep.node.toml`](../crates/livspeil-nodes-core/specs/core.math.smoothstep.node.toml)

### core.math.sub@1

Subtract b from a

`out = a - b` (IEEE 754).

Spec: [`core.math.sub.node.toml`](../crates/livspeil-nodes-core/specs/core.math.sub.node.toml)

### core.math.wrap@1

Wrap x into [min, max) (bounds are swapped if reversed)

`out = min + (x - min) mod (max - min)` with a Euclidean (never negative)
remainder, so values below `min` wrap from the top: with the defaults
`wrap(-0.25) == 0.75` and `wrap(1.0) == 0.0`. If `min > max` the bounds are
swapped, as in `core.math.clamp`. Errors (invalid input): `min == max` (on
`max`), a `NaN` or infinite bound (on that bound), and a range
`max - min` too large to represent (on `max`). A `NaN` or infinite `x`
gives `NaN`.

Spec: [`core.math.wrap.node.toml`](../crates/livspeil-nodes-core/specs/core.math.wrap.node.toml)

### core.output@1

An output of the enclosing subnet, or a sink of the root network

One output port of the enclosing subnet (its input `value` is fed inside,
its output `value` is read outside). In the root network it marks a sink:
`livspeil cook` prints the root `core.output` nodes. Port-polymorphic like
`core.subnet.input`; cooking copies the input to the output.

Spec: [`core.output.node.toml`](../crates/livspeil-nodes-core/specs/core.output.node.toml)

### core.sky.sun_direction@1

Sun direction (y up, x east, z south), elevation and azimuth from day, solar time and latitude

The sun's position (Cooper's declination formula, apparent solar time) for
a `day_of_year` in `[1, 366]` (fractional days allowed), a local
`solar_hour` and a `latitude` in `[-90, 90]` degrees (north positive);
other days or latitudes are invalid-input errors. `direction` is a unit
vector in the world frame (+y up, +x east, +z south), `elevation` is in
degrees and `azimuth` in degrees clockwise from north.

Spec: [`core.sky.sun_direction.node.toml`](../crates/livspeil-nodes-core/specs/core.sky.sun_direction.node.toml)

### core.subnet.input@1

An input port of the enclosing subnet

One input port of the enclosing subnet. Its input `value` is fed from
outside the subnet (a link in the outer network, or its own static value:
the promoted parameter); its output `value` is read inside. The registered
spec uses `Float`; `livspeil-graph` retypes both ports to the interface
port's declared type. Cooking copies the input to the output.

Spec: [`core.subnet.input.node.toml`](../crates/livspeil-nodes-core/specs/core.subnet.input.node.toml)

### core.subnet@1

A node that contains a network (a Charm when it has metadata)

Contains a network. It has no ports of its own: its interface is made of
the `core.subnet.input` and `core.output` boundary nodes inside it.

Spec: [`core.subnet.node.toml`](../crates/livspeil-nodes-core/specs/core.subnet.node.toml)

### core.table.column@1

One column of a table, as a list

The single-column table (a list) of column `name` of `table`. An unknown
column is an invalid-input error on `name` that lists the columns there
are; a table without columns (the default) gives the empty table.

Spec: [`core.table.column.node.toml`](../crates/livspeil-nodes-core/specs/core.table.column.node.toml)

### core.table.count@1

Number of rows and columns of a table

Number of rows (`count`) and columns (`columns`) of `table`.

Spec: [`core.table.count.node.toml`](../crates/livspeil-nodes-core/specs/core.table.count.node.toml)

### core.table.filter_text@1

Keep the rows whose text column contains a substring

The rows of `table` whose String column `column` contains `text` (the
empty text keeps every row), in their original order; `count` is the
number of rows kept. With `ignore_case` both sides are lowercased first
(Unicode). A column that is not a String column is an invalid-input error.

Spec: [`core.table.filter_text.node.toml`](../crates/livspeil-nodes-core/specs/core.table.filter_text.node.toml)

### core.table.get@1

One cell of a table as text and number

One cell, three ways at once: `text` (the `str()` form: strings unchanged,
floats in shortest round-trip form, vectors as `vec2(1.0, 2.0)`), `number`
(Floats, Ints converted, Bools 0 or 1, anything else 0) and `found`. A row
out of range (negative or past the end) gives `found = false`, `text = ""`,
`number = 0`; an unknown column is an invalid-input error on `column`.

Spec: [`core.table.get.node.toml`](../crates/livspeil-nodes-core/specs/core.table.get.node.toml)

### core.table.sort@1

Sort the rows of a table by a column

`table` with its rows sorted by `column`, ascending unless `ascending` is
false. The sort is stable in both directions (rows with equal keys keep
their order). Numbers sort by IEEE total order (`-0.0` before `0.0`, `NaN`
after infinity), `false` before `true`, strings by Unicode scalar values,
vectors and colors component by component.

Spec: [`core.table.sort.node.toml`](../crates/livspeil-nodes-core/specs/core.table.sort.node.toml)

### core.text.format@1

Fill {a} and {b} in a template with numbers ({{ and }} for braces)

In `template`, `{a}` and `{b}` are replaced by the numbers printed with
`precision` decimals (platform-independent, half to even on the decimal
value, no `-0.00`; `NaN`, `inf`, `-inf` for the specials; precision is
clamped to `[0, 12]`), and `{{` and `}}` are literal braces. Placeholders
may repeat or be absent. Anything else in braces (`{c}`, `{}`, `{ a }`),
an unclosed `{` or a lone `}` is an invalid-input error on `template`.

Spec: [`core.text.format.node.toml`](../crates/livspeil-nodes-core/specs/core.text.format.node.toml)

### core.time.clock@1

The frame clock

Publishes the frame clock: `frame` counter, `time` in seconds since the clock started and `dt` since the previous frame. The only time-dependent core node.

Spec: [`core.time.clock.node.toml`](../crates/livspeil-nodes-core/specs/core.time.clock.node.toml)

### core.time.timeofday@1

Day phase (0 = midnight, 0.5 = noon) and hour from seconds since local midnight

`seconds` since local midnight plus `offset` hours, wrapped to one day:
`phase` in `[0, 1)` (0 = midnight, 0.5 = noon) and `hour` in `[0, 24)`. Any
finite input wraps, including negative values and values beyond one day;
`NaN` or infinite inputs are invalid-input errors.

Spec: [`core.time.timeofday.node.toml`](../crates/livspeil-nodes-core/specs/core.time.timeofday.node.toml)

### core.vector.compose3@1

Build a Vec3 from x, y and z

Spec: [`core.vector.compose3.node.toml`](../crates/livspeil-nodes-core/specs/core.vector.compose3.node.toml)

### core.vector.decompose3@1

Split a Vec3 into x, y and z

Spec: [`core.vector.decompose3.node.toml`](../crates/livspeil-nodes-core/specs/core.vector.decompose3.node.toml)

### core.vector.dot3@1

Dot product of two Vec3s

Dot product, summed in the order x, y, z. IEEE 754 throughout: `inf · 0` gives `NaN`.

Spec: [`core.vector.dot3.node.toml`](../crates/livspeil-nodes-core/specs/core.vector.dot3.node.toml)

### core.vector.length3@1

Euclidean length of a Vec3

Euclidean length, overflow- and underflow-safe (scaled hypot).

Spec: [`core.vector.length3.node.toml`](../crates/livspeil-nodes-core/specs/core.vector.length3.node.toml)

### core.vector.normalize3@1

Scale a Vec3 to unit length (the zero vector stays zero)

`v` scaled to unit length. The zero vector gives the zero vector (with
positive zeros); an infinite component dominates, and a `NaN` component
gives all `NaN`.

Spec: [`core.vector.normalize3.node.toml`](../crates/livspeil-nodes-core/specs/core.vector.normalize3.node.toml)
