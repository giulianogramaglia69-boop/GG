# GG

GG is a small programming language written in Rust — a tree-walking
interpreter, built from scratch. It is designed to be **readable**:
English keywords, blocks closed by `end` (Ruby/Lua style), no
significant indentation, no braces.

```gg
let x = 10
if x > 5 then
    print "big"
else
    print "small"
end
```

## Status

**Phase 1 — lexer: done.** The lexer turns GG source into tokens
(`gg-lexer` crate). **Phase 2 — parser + AST: done.** The parser
(`gg-parser` crate) turns tokens into an AST; `gg --ast file.gg`
prints it back in canonical form. **Phase 3 — interpreter: done.**
The tree-walking interpreter (`gg-interp` crate) runs GG programs;
`gg run file.gg` executes them. **Phase 4 — builtins: done.**
Builtin functions (`len`, `type`, `input`...) are available in
every program. **Phase 5 — REPL: done.** The interactive shell
(`gg` with no arguments, or `gg repl`) evaluates line by line, keeps
state across lines, and lets blocks span several lines until `end`.
**Phase 6 — lists + file handling: done.** Lists and whole-file
helpers (`read_file`/`write_file`/`append_file`/`file_exists`).
**Phase 7 — collections: done.** Tuples, dicts, sets, arrays, queues
and stacks, with new builtins (`array`, `queue`, `stack`, `enqueue`,
`dequeue`, `peek`, `keys`, `contains`, `list`) and `len`/`push`/`pop`
extended to the new types. **Phase 8 — OOP: done.** Classes with
single inheritance, methods with explicit `self`, bound methods,
`super`, dynamic fields, and a new builtin `is_a`; `type`/`str`
extended to instances (see the OOP section below). **Phase 9 —
modules + virtual environments: done.** Modules are files loaded
with `import` (`Value::Module`, `type(m)` is `"module"`); a venv
holds project modules in `lib/` and is created with `gg venv`
(see the Modules section below). **Phase 10 — standard library:
done.** Eight new builtins (`sum`, `join`, `upper`, `lower`,
`trim`, `replace`, `starts_with`, `ends_with`), a native
`math` module, a native `random` module (SplitMix64, pure Rust)
and a `stats` module written in pure GG and embedded in the
binary (see the Standard library section below). **Phase 11 —
N-dimensional numeric arrays: done.** Arrays are now N-d with a
flat row-major storage plus a shape; new constructors (`zeros`,
`ones`, `arange`, `linspace`), shape helpers (`shape`, `ndim`,
`size`), multi-indexing `a[i, j]`, element-wise arithmetic
(array with array, array with scalar from either side), `dot`,
`matmul`, `transpose`, `reshape`, and `sum`/`min`/`max` over
whole arrays (see the N-dimensional arrays section below).
**Phase 12 — slicing and boolean indexing: done.** Slices
`start:stop:step` on lists, strings, tuples and arrays (Python
rules, always a copy); element-wise comparisons returning Bool
arrays; boolean indexing `a[mask]`; `where(cond, a, b)`; slice
assignment on lists and 1D arrays (see the Slicing section
below). **Phase 13 — graphical interfaces: done.** A native
`gui` module (egui + eframe, behind the `gui` Cargo feature,
on by default): `gui.run(titolo, fn)` opens a window and calls
the GG frame function once per frame until it returns `false`;
widgets (`heading`, `label`, `separator`, `button`,
`text_input`, `slider`, `checkbox`, `horizontal`) are called
inside the frame function (see the Graphical interfaces section
below). **Phase 14 — linear algebra: done.** A native
`linalg` module (`solve`/`det`/`inv`/`rank`, Gaussian elimination
with partial pivoting; a singular matrix is a runtime error,
never a silent `nil`; `rank` tells an indeterminate system
from an impossible one) — see the Standard library section
below. **Phase 15 — for-in iteration: done.** `for x in xs
then ... end` iterates over lists, tuples, arrays (rows on
2D+), sets, dict keys, string characters, queues and stacks;
the iterable is evaluated once and snapshotted, and the loop
variable is scoped to the body (see the Loops section below).
**Phase 16 — runtime errors with line and column: done.** Every
runtime error reports where it failed — `runtime error:
<line>:<col>: <message>` (1-based), pointing at the token that
failed (the identifier, the operator, the index, the call — see
the Errors bullet below and `examples/error_demo.gg`).
**Phase 17 — more GUI widgets: done.** `radio` (true only when
clicked, exclusive groups are the script's own logic),
`combo` (dropdown over a List of Str, returns the selected
index), `progress` (bar, value clamped to [0, 1]) and
`vertical` (like `horizontal`, vertical layout) — see
`examples/gui_widgets_demo.gg`. **Phase 18 — `linalg.rank`:
done.** `rank(A)` → Int, the rank of a 2D numeric matrix,
square or rectangular; with it, an indeterminate system is
told from an impossible one (`rank(A) == rank([A|b]) < n` vs
`rank(A) < rank([A|b])`) — see `examples/rank_demo.gg`.
**Phase 19 — error handling: done.** `try then ... except e
then ... finally then ... end` catches runtime errors (the
bound variable holds the error's message as a Str; `except`
without a binding ignores it; at least one of `except` and
`finally` is required), `raise "message"` throws one, and
`finally` runs on every exit — normal, error, and `return`
(see the Errors bullet below and `examples/try_demo.gg`).
**Phase 20 — comprehensions and lambdas: done.** List
`[x * 2 for x in xs]`, dict `{k: v for k, v in pairs}` and set
`{x for x in xs}` comprehensions, each with an optional `if`
filter and a single `for` clause (no nested fors); the loop
variable is scoped to the comprehension. Lambdas: `fn(x) x * 2
end` — `fn` without a name, as an expression, whose body is a
single expression; they capture the defining environment and
are values like any function, so they can be assigned, passed
as arguments, and called on the spot (`fn(x) x + 1 end(41)`) —
see the Comprehensions bullet below and
`examples/comprehension_demo.gg`. **Phase 21 — default
arguments, keyword arguments and `*args`: done.** Parameters
with defaults (`fn saluta(nome, saluto = "ciao")`), keyword
arguments (`saluta(nome = "x")`, in any order), a rest
parameter (`fn somma(*numeri)`, the extras as a List) and
spread at the call (`f(*xs)`, over lists, tuples and arrays);
defaults are evaluated once at definition time (Python rule —
a mutable default is shared between calls), and the same
mechanism works for named functions, lambdas and methods
(including `init` through the constructor and `super` calls);
builtins stay positional, and there is no `**kwargs` yet — see
the Arguments bullet below and `examples/args_demo.gg`.
**Phase 22 — f-strings: done.** `f"ciao {nome}, hai {eta} anni"`
interpolates any GG expression inside `{...}` — variables,
arithmetic, calls, indexing, comprehensions, lambdas — rendered
with the same Display as `print`; `{{` and `}}` are literal
braces, and the usual escapes work in the literal text. The `f`
must touch the quote (`f "..."` is still an identifier plus a
string). Errors have positions: an unclosed `{`, a lone `}`,
an empty `{}` and an unterminated f-string are lex/parse errors
pointing at the spot, and an error inside the braces points at
its own token and can be caught by `try`/`except`. Strings
inside the braces work, dict/set literals too, and f-strings
nest (`f"a {f"b {x}"} c"`). Out of scope: format specs
(`{x:.2f}`), `!r`/`!s` conversions and `{x=}` — see the
F-strings bullet below and `examples/fstring_demo.gg`.
**Phase 23 — `match` and generators: done.** `match x then
case 1 then ... case [a, b] then ... case n if n > 10 then ...
case _ then ... end` picks the first case whose pattern matches
(literals, `_`, bindings, list/tuple patterns, nested) and whose
guard, if any, is true; when no case matches, nothing happens.
A function whose body contains `yield` is a generator: calling
it returns a `<generator>` without running the body, and
`next(gen)` replays the body from the start up to the next
`yield` (lazy replay — see the Match and Generators bullets
below, with the limits in evidence, and
`examples/match_demo.gg` / `examples/generators_demo.gg`).
**Phase 24 — REPL history: done.** On a terminal the REPL runs
through `rustyline` (19.0.0): up/down arrows walk the history,
which persists in `~/.gg_history` across sessions, and a
multi-line block is a single history entry. Ctrl-C drops the
line (and the block in progress); Ctrl-D on an empty line
leaves. When stdin is piped, the REPL uses the plain stream
loop instead, so piped sessions behave exactly as before.
**Bytecode VM: done.** A bytecode compiler (`AST → Op`) and a
stack virtual machine now run GG programs, side by side with the
tree-walker, behind the same `Interpreter` API; the VM is the
default, the tree-walker stays as a fallback selectable with
`GG_BACKEND=tree` (see the Bytecode virtual machine section
below). Same programs, same values, same error texts and spans
— verified by a dual-backend test corpus that runs every program
on both engines and requires identical output. Honest benchmark
(release build, this VM): the VM is now faster on tight loops
(1M-iteration sum: ~240 ms vs ~440 ms) and on call-heavy
recursion too (`fib(26)`: ~295 ms vs ~355 ms) — see the
numbers and why in that section. **JSON module: done.** A native
`json` module (`import json`): `json.parse(text)` turns JSON into
GG values (objects → Dicts, arrays → Lists, `null` → Nil) and
`json.stringify(value)` / `json.stringify(value, indent)` renders
them back, compact or pretty-printed; a hand-written recursive
parser in pure Rust, no external crate — see the Standard library
section below and `examples/json_demo.gg`. **OS module: done.** A native
`os` module (`import os`): `os.args()` (the command line),
`os.getenv(name)` / `os.getenv(name, default)`, `os.cwd()`,
`os.listdir(path)` (names only, sorted), `os.mkdir(path)` and
`os.remove(path)` (a file, never a directory); pure Rust, no
external crate, and no command execution or environment writes,
by design — see the Standard library section below and
`examples/os_demo.gg`. **Datetime module: done.** A native
`datetime` module (`import datetime`): `datetime.now()` (the
Unix timestamp in seconds), `datetime.from_timestamp(ts)` /
`datetime.to_timestamp(dict)` (a Dict with `year`, `month`,
`day`, `hour`, `minute`, `second` and `weekday`, 0 = Monday ..
6 = Sunday), `datetime.format(ts_or_dict, fmt)` (a `strftime`
subset: `%Y %m %d %H %M %S %w`) and `datetime.date(year,
month, day)` (midnight UTC); everything is UTC, hand-written
calendar math in pure Rust, no external crate — see the
Standard library section below and `examples/datetime_demo.gg`.
**Regex module: done.** A native `regex` module (`import
regex`): `regex.test(pattern, text)` / `regex.find(pattern,
text)` / `regex.groups(pattern, text)` / `regex.replace(pattern,
text, replacement)` (with `$1` / `$nome` backreferences in the
replacement) / `regex.split(pattern, text)`; via the `regex`
crate (1.13.2) from crates.io — the first stdlib module with an
external dependency. Patterns match on Unicode text (`.` is one
Unicode character, `\d` is Unicode-aware), and an invalid
pattern is a runtime error carrying the crate's message,
catchable with `try`/`except` — see the Standard library
section below and `examples/regex_demo.gg`. **HTTP module:
done.** A native `http` module (`import http`): a blocking
HTTP client — `http.get(url)` / `http.get(url, headers)`,
`http.post(url, body)` / `http.post(url, body, content_type)`
and `http.request(method, url, body, headers)`; every call
returns a Dict with `status`, `headers` (keys lowercased) and
`body`, and an HTTP error status (404, 500) comes back in the
Dict instead of raising; only network failures are runtime
errors, with a 30-second timeout for the whole exchange; via
the `ureq` crate (3.5.2) from crates.io, TLS verified against
the platform trust store — see the Standard library section
below and `examples/http_demo.gg`. **SQLite module: done.** A
native `sqlite` module (`import sqlite`): an embedded database
— `sqlite.open(path)` / `sqlite.exec(db, sql)` /
`sqlite.execute(db, sql, params)` (returns rows changed) /
`sqlite.query(db, sql)` / `sqlite.query(db, sql, params)`
(returns a List of Dicts, keys are column names) /
`sqlite.close(db)`; via the `rusqlite` crate (0.32) from
crates.io with the `bundled` feature, so no system SQLite
library is needed; every SQLite failure is a runtime error
carrying SQLite's message, catchable with `try`/`except` —
see the Standard library section below and
`examples/sqlite_demo.gg`. **Package manager: done.** `gg
install` / `gg list` / `gg remove` and a `gg.lock` lockfile:
packages are `gg.toml` manifests (`name`, `version`,
`description`, `entry`, `dependencies`) installed from a
local directory or from GitHub (`owner/repo[@tag|@commit]`,
cloned with `git`) into `~/.gg/packages/<name>/<version>/`
(or `gg_packages/` with `--local`); `import <name>` finds
them after files, the stdlib and native modules, and the
lockfile pins the version — see the Package manager section
below and `examples/package_demo.gg` with
`examples/demo-package/`.

## The REPL

```sh
./target/release/gg
```

```text
GG 0.1.0 — interactive REPL. Type exit to quit.
gg> let x = 5
gg> x
5
gg> fn add(a, b)
... return a + b
... end
gg> add(2, 3)
5
gg> if x > 3 then
... print "grande"
... end
grande
gg> let = 5
error: line 1, col 5: expected variable name after 'let' but found '='
gg> x
5
gg> exit
```

Rules: an expression on its own echoes its value (`1 + 2` prints
`3`); other statements print nothing by themselves, and `nil` is
never echoed. `exit` (or `quit`) on its own line leaves; Ctrl-D
leaves too. Errors (lex, parse, runtime) are printed and the session
goes on with its state intact — a line that fails halfway may have
executed part of its statements before failing. Blocks wait for their
`end` with the `... ` prompt; note that `then` must stay on the same
line as the condition (`if x` followed by `then` on the next line is
an error, not a continuation). On a terminal the REPL has arrow-key
history (`rustyline`): up/down recall previous chunks, the history
persists in `~/.gg_history` across sessions, and a multi-line block
is a single history entry (up-arrow recalls the whole block).
Ctrl-C drops the line being typed and the block in progress — before
this, Ctrl-C killed the process — and `exit`/`quit` are not
recorded in the history. When stdin is piped (scripts, tests), the
REPL reads plain lines as before and never touches the history file.

## Semantics

Values: integers, floats, strings, booleans, `nil` (the value of a
function that returns nothing) and functions. Functions are
**closures**: they capture the environment where they were defined,
so they can use outer variables — and see themselves, so recursion
works.

- **Truthiness:** only `false` and `nil` are false; everything else
  (including `0` and `""`) is true. `and`/`or` short-circuit and
  return the operand value, not a boolean (`1 and 2` is `2`,
  `false or "x"` is `"x"`).
- **Numbers:** `int + int` is an int; any float operand makes the
  result a float. `/` is **always float division** (`7 / 2` is `3.5`).
  `%` works only between integers and keeps the dividend's sign
  (`-7 % 3` is `-1`). Integer overflow is a runtime error.
- **Strings:** `+` concatenates; `< <= > >=` compare
  lexicographically. `==`/`!=` between different types are
  `false`/`true`, never an error.
- **`for i = a to b then ... end`** includes both ends and steps +1
  if `a <= b`, -1 otherwise. The loop variable exists only inside
  the loop body.
- **`for x in xs then ... end`** iterates over a collection (Phase
  15): lists and tuples (the elements, in order), arrays (the
  elements of a 1D array, the rows of a 2D+ array), sets (insertion
  order), dicts (the keys, in insertion order), strings (one
  character at a time), queues and stacks (a snapshot — iterating
  does not consume them), and generators (Phase 23 — pulled
  lazily, one `next()` at a time, until exhaustion ends the loop).
  The iterable is evaluated once before the loop starts, so
  mutations inside the body do not change what the loop visits —
  except for a generator, whose body runs again on every pull
  (see the Generators bullet). Anything else is a runtime error
  (`Int is not iterable`). The loop variable exists only inside
  the loop body, like the numeric `for`.
- **Comprehensions (Phase 20):** `[expr for x in xs]`,
  `{k: v for k, v in pairs}`, `{x for x in xs}` — a list, dict or
  set built from any iterable of `for-in` (lists, tuples, arrays,
  sets, dict keys, strings, queues, stacks, `range`), with an
  optional `if` filter (`[x for x in xs if x > 0]`). One `for`
  clause only (no nested fors — nest a comprehension in the
  element instead: `[[y for y in row] for row in matrix]`). The
  loop variable exists only inside the comprehension, like a
  `for` variable; two variables unpack a pair (`for k, v in
  pairs`, each item a 2-tuple, 2-list or 2-element 1D array).
  Dict keys must be Int, Float, Str or Bool, and duplicate keys
  keep the last value; set duplicates are dropped.
- **Lambdas (Phase 20):** `fn(x) x * 2 end` is `fn` without a
  name, as an expression. The body is a single expression and its
  value is the return value (no statements, no `return` inside).
  A lambda captures the environment where it was defined, like a
  named function, and is a value: assign it (`let doppia = fn(x)
  x * 2 end`), pass it to a function, or call it on the spot
  (`fn(x) x + 1 end(41)`). `type` is `"fn"`, it prints as
  `<fn>`.
- **Arguments (Phase 21):** `fn saluta(nome, saluto = "ciao")
  ... end` gives `saluto` a default; a call may pass it
  positionally or by keyword (`saluta("x", saluto = "hey")`,
  keywords in any order, after the positionals). A default is
  evaluated once, at definition time (Python rule) — `let d =
  1` then `fn f(x = d)` then `d = 2` still gives `f()` as `1`,
  and a mutable default (a list) is shared between calls.
  `fn somma(*numeri) ... end` collects extra positionals
  into a List (empty when there are none); `*rest` must be the
  last parameter and there is at most one. `f(*xs)` spreads a
  list, tuple or array into positional arguments (anything
  else is an error). Unknown keywords, duplicates (the same
  parameter positional and by keyword, or twice by keyword)
  and missing required arguments are runtime errors pointing
  at the call — or at the keyword name for the first two.
  A parameter without a default after one with a default, a
  positional after a keyword, and a spread after a keyword
  are parse errors. The same mechanism works for lambdas and
  methods (`init` included, through the constructor, and
  `super` calls); builtins stay positional (a keyword
  argument to one is an error) and there is no `**kwargs`
  yet.
- **F-strings (Phase 22):** `f"ciao {nome}"` — the `f` must touch
  the quote. Inside `{...}` goes any GG expression, evaluated in
  the current environment and rendered with the same Display as
  `print` (`f"{1 + 2} e {xs[0]}"`, `f"{upper("ciao")}"`,
  `f"{[x * 2 for x in xs]}"`); the parts are concatenated left to
  right, and the result is an ordinary Str — assign it, pass it,
  print it. `{{` and `}}` are literal braces; the escapes
  `\"` `\\` `\n` `\t` work in the literal text (a raw `"` would
  close the string). Errors point at the spot: an unclosed `{`, a
  lone `}`, an empty `{}` and an unterminated f-string are
  lex/parse errors, and an error inside the braces (an undefined
  variable, a division by zero) points at its own token and can be
  caught by `try`/`except`. F-strings nest
  (`f"a {f"b {x}"} c"`). No format specs (`{x:.2f}`), no
  `!r`/`!s` conversions, no `{x=}`.
- **Match (Phase 23):** `match x then case 1 then ... case [a, b]
  then ... case n if n > 10 then ... case _ then ... end` — a
  statement, not an expression (no match expression yet). The
  first case whose pattern matches runs: literals (numbers,
  strings, `true`/`false`, also negative numbers), `_` (matches
  anything), a binding (a bare identifier, bound to the value),
  a list pattern `[a, b]` or a tuple pattern `(a, b)`, also
  nested (`[a, [b, c]]`). A list pattern matches only a List, a
  tuple pattern only a Tuple, and a length mismatch is simply no
  match, never an error. An optional guard `if <expr>` after the
  pattern is evaluated with the bindings in scope; a false guard
  skips the case. When no case matches, nothing happens — `case _`
  is the idiomatic fallback. Bindings live only in their case's
  body; a `return` inside a case returns from the enclosing
  function.
- **Generators (Phase 23):** a function whose body contains
  `yield` is a generator: calling it does **not** run the body —
  it returns a generator value (`type` is `"generator"`, it
  prints as `<generator>`), and `next(gen)` pulls the next value
  out of it. `yield expr` yields the value (a bare `yield` yields
  `nil`); `return` inside a generator ends it (the returned value
  is ignored). When the body ends before the next `yield`,
  `next()` fails with `"generator exhausted"` — catchable with
  `try`/`except`. `for x in gen` pulls lazily, one `next()` at a
  time, until exhaustion (which ends the loop silently). Two
  generators from one function are independent; a method with
  `yield` is a generator too. `yield` outside a function is a
  parse error.
  **Limits of the replay design — read this before using one:**
  GG has no threads and no state machine for real suspension
  (the interpreter shares everything through `Rc`), so `next()`
  works by **replay**: it runs the body from the start, counting
  the `yield`s, and stops at the next one after those already
  consumed. That means (1) the total cost is O(n²) over a whole
  iteration — every `next()` starts from the beginning; (2) the
  body's side effects **repeat** on every `next()` — a `print`
  before the target `yield` runs again, and a mutation of an
  outer variable runs again; the body must be deterministic for
  the values to be right. Infinite generators still work: a
  replay stops at the N-th `yield` without ever running forever.
  Comprehensions do not take generators (they would need a
  snapshot — use `for-in` or `next()` to collect into a list).
- **Scopes:** every block (`if`/`while`/`for`/function body) runs in
  its own child scope. `let` inside a block is invisible outside;
  assignment updates the variable where it was defined.
- **Lists:** a list is an ordered, mutable collection —
  `[1, 2, 3]`, `[]`, nested and mixed (`[1, "ciao", true]`).
  Indexing is `xs[0]`; negative indexes count from the end
  (`xs[-1]` is the last element); out of range is a runtime error.
  Assignment through an index works too: `xs[0] = 99`,
  `m[0][1] = 2`. A list is **shared**: `let b = a` makes `b` an
  alias of `a`, so a mutation through one name is visible through
  the other. Lists compare element by element (`[1, 2] == [1, 2]`);
  two distinct self-referential lists are reported unequal rather
  than recursing forever. Printing a list shows its elements, with
  strings quoted (`[1, "ciao"]`); past 16 levels of nesting it
  prints `...`, so a list containing itself cannot blow the stack.
  To visit every element, `for x in xs then print x end` (Phase 15);
  the index loop `for i = 0 to len(xs) - 1 then print xs[i] end`
  still works too.

```gg
let xs = [10, 20, 30]
push(xs, 40)
print xs[3]
print xs[-1]
print len(xs)
```

- **Collections (Phase 7):** five more containers besides lists.
  - **Tuples** — `(1, "a", true)`, `()` empty, `(1,)` with one
    element: the trailing comma is what makes it a tuple, `(1)` is
    still just grouping. Tuples are **immutable**: `t[0] = 99` is a
    runtime error. Indexing works like lists, negative indexes
    included; `type` is `"tuple"`.
  - **Dicts** — `{"nome": "Giuliano", "eta": 14}`, `{}` empty.
    Keys are Int, Float, Str or Bool only (a list, dict, set or
    tuple as a key is a runtime error); `1` and `1.0` are different
    keys. Reading a missing key is an error (`key not found`);
    assignment inserts a new key and updates keep the original
    position (`keys(d)` gives the keys in insertion order). Dicts
    compare by content, order ignored. `type` is `"dict"`.
  - **Sets** — `{1, 2, 3}`. Braces decide by content: entries with
    `:` make a dict, otherwise a set; `{}` is the empty **dict**
    (Python convention). Duplicates are ignored silently
    (`{1, 1, 2}` is `{1, 2}`). Elements must be hashable (Int,
    Float, Str, Bool). Sets have no indexes. `type` is `"set"`.
  - **Arrays** — like lists, but **fixed size** and **all elements
    of the same type**: `array([1, 2, 3])`. Built from a list only;
    an empty list is an error (`cannot infer array type`) and mixed
    types are an error. `a[0] = 99` works only when the new value
    has the same type as the others; there is no `push`/`pop` on
    arrays. `type` is `"array"`. Arrays are the first step toward
    numeric arrays in the spirit of NumPy.
  - **Queues** — `queue()` or `queue([1, 2])`, FIFO:
    `enqueue(q, x)` adds at the back, `dequeue(q)` removes and
    returns the front (`dequeue()` on an empty queue is an error).
    `type` is `"queue"`.
  - **Stacks** — `stack()` or `stack([1, 2])`, LIFO: the existing
    `push`/`pop` work on stacks too. `type` is `"stack"`.
  - **Common operations:** `len` works on every collection (for a
    dict it is the number of keys); `str` prints every collection
    the way `print` shows it; `peek` returns the top of a stack,
    the front of a queue or the last element of a list without
    removing it; `contains(coll, x)` tests membership (for a dict:
    the key exists; for a string: substring); `list(x)` converts a
    tuple, set, array, queue or stack — or a dict (its keys) — into
    a list (`list("ciao")` is an error: use `split` or `lines` for
    text). Equality is by content for all of them (a dict against
    another dict ignores order; a set against another set too).
    Display nests up to 16 levels, then prints `...`.

Example — count the words in a file with a dict and `split`:

```gg
let text = read_file("notes.txt")
let words = split(text, " ")
let counts = {}
for i = 0 to len(words) - 1 then
    let w = words[i]
    if contains(counts, w) then
        counts[w] = counts[w] + 1
    else
        counts[w] = 1
    end
end
print counts
```

```
$ cat notes.txt
pane pane latte pane
$ gg run wordcount.gg
{"pane": 3, "latte": 1}
```

**Lists vs arrays:** a list is the flexible container — any mix of
types, grows and shrinks with `push`/`pop`, shared by aliasing. An
array is the strict one — one element type, fixed size, no
`push`/`pop`; the type check on every assignment is what makes it
safe as a building block for numeric code later.

- **OOP (Phase 8):** classes, methods, single inheritance.
  - **Declaration** — `class Persona` ... `end`; a class body
    holds *only* method definitions (`fn`): anything else is a
    parse error (`only method definitions (fn) are allowed inside
    a class`). Single inheritance: `class Studente extends
    Persona` ... `end`. Classes are values: `let C = Persona`
    works, and a class can be defined inside a block (normal
    scope). `class`, `extends`, `super` and `self` are keywords and
    cannot be used as identifiers.
  - **Methods** — `fn nome(self, args)` ... `end`: `self` is the
    first parameter, explicit like in Python. The parser rejects a
    method whose first parameter is not `self`. Calling
    `p.saluta("ciao")` passes `self` implicitly. `p.saluta` on its
    own returns a **bound method**, which can be stored and called
    later (`let m = p.saluta` then `m()`). A method taken from the
    class (`Persona.saluta`) is unbound: `self` must be passed
    explicitly (`Persona.saluta(p)`).
  - **Fields** — dynamic, Python style: they are created by
    assigning inside a method, `self.nome = nome`. Reading an
    instance attribute looks in the instance's fields first, then
    in the class's methods (bound). A missing field is a runtime
    error (`no field 'x'`); assigning `p.campo = v` sets only
    fields, never methods — a field with the same name as a method
    shadows it. Fields keep insertion order.
  - **Constructor** — the method is called `init`:
    `fn init(self, nome, eta)` ... `end`. An instance is created
    by calling the class like a function:
    `let p = Persona("Giuliano", 14)` — the instance is created,
    then `init` runs with the same arguments. With no `init`,
    `C()` makes an empty instance and `C(1)` is an error
    (`takes no arguments (no init method)`). Calling something
    that is not a class or function is an error
    (`cannot call Int`).
  - **Inheritance** — method lookup walks up the chain (class →
    parent → ...); a method in the child hides the parent's
    (override). `super.metodo(args)` inside a method calls the
    parent's method with the same `self` — `super.init(nome, eta)`
    in the child's constructor is the usual case. `super` outside
    a method is a runtime error (`'super' outside a method`); a
    class with no parent has no super (`has no parent (no super)`).
    `super` is resolved **lexically**: a plain function defined
    outside a method has no super even when called from a method,
    while a function defined *inside* a method captures that
    method's class (like Python's `__class__` cell) and can use
    `super`.
  - **Values** — `type(p)` is the class name as a string
    (`"Persona"`), `type(Persona)` is `"class"`. `str(p)` is
    `Persona{nome: "Giuliano", eta: 14}` by default, but a
    `fn str(self)` method wins — for both `print p` and `str(p)`;
    inside a collection the default form is used instead (like
    Python's `repr` inside a list). `is_a(x, Classe)` is true when
    `x` is an instance of that class or any of its ancestors.
    **Instance equality is by identity** (the same object), not by
    fields: `p == p` is true, `Persona("A", 1) == Persona("A", 1)`
    is false. `len` and the other builtins on an instance are a
    runtime error (except `str` above).

```gg
class Persona
    fn init(self, nome, eta)
        self.nome = nome
        self.eta = eta
    end
    fn saluta(self)
        return "Ciao, sono " + self.nome
    end
    fn str(self)
        return self.nome + " (" + str(self.eta) + " anni)"
    end
end

class Studente extends Persona
    fn init(self, nome, eta, corso)
        super.init(nome, eta)
        self.corso = corso
    end
    fn saluta(self)
        return super.saluta() + ", studio " + self.corso
    end
end

let s = Studente("Giuliano", 14, "Data Science & AI")
print s.saluta()
print s
print is_a(s, Persona)
print is_a(s, Studente)
```

```
Ciao, sono Giuliano, studio Data Science & AI
Giuliano (14 anni)
true
true
```

- **Errors** are plain-English runtime errors with the position
  where they failed: `runtime error: <line>:<col>: <message>`
  (1-based). The position is the token that failed — the
  identifier for `undefined variable 'x'`, the operator for
  `division by zero` and `cannot add Int and Str`, the function
  name for `expected 1 argument(s) but got 2`, the index for an
  out-of-range index, the `return` keyword for
  `return outside function`. Example (`examples/error_demo.gg`):
  `print 10 / 0` fails with
  `runtime error: 10:10: division by zero`.

- **Error handling (Phase 19):** a runtime error can be caught
  instead of stopping the program:
  ```gg
  try then
      print 10 / 0
  except e then
      print "preso: " + e
  end
  ```
  `except e then` binds the error's message as a Str to `e`
  (scoped to the handler); `except then` without a binding
  ignores it. `raise "message"` throws an error pointing at the
  `raise` (the message must be a Str). `finally then ... end`
  runs on every exit — normal, error, and `return` — and a
  `return` or an error inside it replaces the pending outcome.
  At least one of `except` and `finally` is required, so a bare
  `try ... end` is a parse error. Errors unwind through function
  calls; lex and parse errors happen before any code runs and
  cannot be caught. See `examples/try_demo.gg`.

```gg
fn fib(n)
    if n < 2 then
        return n
    else
        return fib(n - 1) + fib(n - 2)
    end
end

for i = 0 to 10 then
    print fib(i)
end
```

```gg
for i = 1 to 15 then
    if i % 15 == 0 then
        print "FizzBuzz"
    else
        if i % 3 == 0 then
            print "Fizz"
        else
            if i % 5 == 0 then
                print "Buzz"
            else
                print i
            end
        end
    end
end
```

## Builtins

Forty-nine builtin functions are predefined in the global environment of
every program. They are first-class values: they can be assigned to
variables (`let f = len`), passed as arguments and returned from
functions. `type()` returns `"fn"` for user functions and builtins
alike, and `print` shows them as `<fn>`. `type()` returns `"list"`
for lists, `"tuple"`, `"dict"`, `"set"`, `"array"`, `"queue"` and
`"stack"` for the Phase 7 collections, the class name for an
instance and `"class"` for a class (Phase 8).

| Function | Description | Example |
| --- | --- | --- |
| `len(x)` | Length of a string, in characters (not bytes: `len("è")` is 1), or number of elements in a list, tuple, dict (keys), set, array, queue or stack | `len("ciao")` → `4`, `len([1, 2])` → `2`, `len({"a": 1})` → `1` |
| `type(x)` | Type name as a string | `type(1)` → `"int"`, `type([1])` → `"list"`, `type((1,))` → `"tuple"`, `type({})` → `"dict"`, `type(array([1]))` → `"array"`, `type(len)` → `"fn"` |
| `str(x)` | Convert to a string (same as `print` shows) | `str(42)` → `"42"`, `str([1, "x"])` → `"[1, \"x\"]"`, `str({"a": 1})` → `"{\"a\": 1}"` |
| `int(x)` | Float truncated toward zero, string parsed, `true`→`1`, `false`→`0` | `int(3.9)` → `3`, `int("42")` → `42` |
| `float(x)` | Int converted, string parsed, `true`→`1.0`, `false`→`0.0` | `float(3)` → `3.0` |
| `input(prompt)` | Print the prompt (no newline), read one line, return it without the newline; `""` at end of input | `input("Name: ")` |
| `abs(x)` | Absolute value of a number | `abs(-5)` → `5` |
| `min(a, b)` | The smaller of two numbers (mixed int/float gives a float); with one array argument, the smallest element of the array | `min(2, 5)` → `2`, `min(array([3, 1]))` → `1` |
| `max(a, b)` | The larger of two numbers; with one array argument, the largest element of the array | `max(2, 5)` → `5`, `max(array([3, 1]))` → `3` |
| `push(list, value)` | Append `value` to a list or a stack (mutates it) and return `nil`; an error on arrays, queues, tuples, dicts or sets | `push(xs, 3)`, `push(s, 3)` |
| `pop(list)` | Remove and return the last element of a list or the top of a stack; an error on an empty list or stack | `pop(xs)` → `3` |
| `range(n)` | `[0, 1, ..., n-1]` | `range(3)` → `[0, 1, 2]` |
| `range(a, b)` | `[a, ..., b-1]`; empty when `a >= b`; at most 1000000 elements | `range(2, 5)` → `[2, 3, 4]` |
| `split(s, sep)` | Split the string on the separator (an empty separator is an error) | `split("a,b", ",")` → `["a", "b"]` |
| `lines(s)` | The lines of the string (split on `\n`, trailing `\r` stripped, no empty line for a trailing newline) | `lines("a\nb\n")` → `["a", "b"]` |
| `read_file(path)` | The whole file as a string | `read_file("notes.txt")` |
| `write_file(path, text)` | Create or overwrite the file, return `nil` | `write_file("notes.txt", "ciao")` |
| `append_file(path, text)` | Append to the file (creating it if needed), return `nil` | `append_file("notes.txt", " mondo")` |
| `file_exists(path)` | `true` when the path exists | `file_exists("notes.txt")` → `true` |
| `array(list)` | An array from a list: all elements must have the same type (Int, Float, Str or Bool); an empty list is an error. The array is 1D, with shape `[n]` | `array([1, 2])` → `array([1, 2])` |
| `zeros(n)` / `zeros([r, c])` | An Int array of zeros with the given shape (an Int means the 1D shape `[n]`); dimensions must be positive | `zeros(3)` → `array([0, 0, 0])`, `zeros([2, 2])` → `array([[0, 0], [0, 0]])` |
| `ones(n)` / `ones([r, c])` | An Int array of ones with the given shape | `ones([2, 2])` → `array([[1, 1], [1, 1]])` |
| `arange(n)` / `arange(a, b)` / `arange(a, b, step)` | `[0, ..., n-1]`, `[a, ..., b-1]`, or stepping by `step` (negative steps count down); a step of 0 is an error; at most 1000000 elements | `arange(4)` → `array([0, 1, 2, 3])`, `arange(0, 10, 3)` → `array([0, 3, 6, 9])` |
| `linspace(a, b, n)` | `n` Float points evenly spaced from `a` to `b`, both ends included; `n` must be at least 1 | `linspace(0, 1, 3)` → `array([0, 0.5, 1])` |
| `shape(a)` | The shape of an array as a list of Ints | `shape(zeros([2, 3]))` → `[2, 3]` |
| `ndim(a)` | The number of dimensions of an array | `ndim(zeros([2, 3]))` → `2` |
| `size(a)` | The total number of elements of an array | `size(zeros([2, 3]))` → `6` |
| `dot(a, b)` | The scalar product of two 1D vectors of the same length | `dot(array([1, 2, 3]), array([4, 5, 6]))` → `32` |
| `matmul(a, b)` | The matrix product: 2D x 2D, 1D x 2D (row vector times matrix) and 2D x 1D (matrix times column vector); two 1D vectors are an error (use `dot()`) | `matmul(reshape(array([1, 2, 3, 4]), [2, 2]), reshape(array([5, 6, 7, 8]), [2, 2]))` → `array([[19, 22], [43, 50]])` |
| `transpose(a)` | Swap the two axes of a 2D array; a 1D array is returned unchanged | `transpose(reshape(array([1, 2, 3, 4]), [2, 2]))` → `array([[1, 3], [2, 4]])` |
| `reshape(a, shape)` | The same items with a new shape (an Int means the 1D shape `[n]`); the total number of elements must stay the same | `reshape(array([1, 2, 3, 4]), [2, 2])` → `array([[1, 2], [3, 4]])` |
| `queue()` | An empty queue (FIFO) | `queue()` → `queue([])` |
| `queue(list)` | A queue from a list, in order | `queue([1, 2])` → `queue([1, 2])` |
| `enqueue(q, x)` | Add `x` at the back of the queue, return `nil` | `enqueue(q, 3)` |
| `dequeue(q)` | Remove and return the front element; an error on an empty queue | `dequeue(q)` → `1` |
| `stack()` | An empty stack (LIFO) | `stack()` → `stack([])` |
| `stack(list)` | A stack from a list, in order (the last element is the top) | `stack([1, 2])` → `stack([1, 2])` |
| `peek(x)` | The top of a stack, the front of a queue or the last element of a list, without removing it; an error on empty | `peek(stack([1, 2]))` → `2` |
| `keys(d)` | The keys of a dict as a list, in insertion order | `keys({"b": 1, "a": 2})` → `["b", "a"]` |
| `contains(coll, x)` | Membership: in a list, tuple, array or set; a key in a dict; a substring in a string | `contains([1, 2], 2)` → `true`, `contains("ciao", "ia")` → `true` |
| `list(x)` | Convert a tuple, set, array, queue or stack — or a dict (its keys) — into a list; an error on strings and other types | `list((1, 2))` → `[1, 2]`, `list({"a": 1})` → `["a"]` |
| `next(gen)` | Pull the next value out of a generator (see the Generators bullet below); an error when the generator is exhausted (`"generator exhausted"`) or the argument is not a generator | `next(g)` → `10` |
| `is_a(x, Classe)` | `true` when `x` is an instance of `Classe` or of any class inheriting from it; the second argument must be a class | `is_a(s, Persona)` → `true`, `is_a(5, Persona)` → `false` |
| `sum(list)` | The sum of a list of numbers, or of all the elements of an array; all integers give an integer, any float gives a float; on a Bool array it counts the `true` values; an error on an empty list or a non-number | `sum([1, 2, 3])` → `6`, `sum([1.5, 2])` → `3.5`, `sum(reshape(array([1, 2, 3, 4]), [2, 2]))` → `10`, `sum(array([true, false]))` → `1` |
| `join(sep, list)` | The complement of `split`: join the strings of the list with the separator between them | `join(", ", ["a", "b"])` → `"a, b"` |
| `upper(s)` | The string in upper case (Unicode-aware) | `upper("ciao")` → `"CIAO"` |
| `lower(s)` | The string in lower case (Unicode-aware) | `lower("CIAO")` → `"ciao"` |
| `trim(s)` | The string without leading and trailing whitespace | `trim("  x  ")` → `"x"` |
| `replace(s, old, new)` | The string with every occurrence of `old` replaced by `new` (an empty `old` is an error) | `replace("aaa", "a", "b")` → `"bbb"` |
| `starts_with(s, prefix)` | `true` when the string starts with the prefix | `starts_with("ciao", "ci")` → `true` |
| `ends_with(s, suffix)` | `true` when the string ends with the suffix | `ends_with("ciao", "ao")` → `true` |
| `where(cond, a, b)` | Element-by-element selection: where the Bool array `cond` is true take the element from `a`, otherwise from `b`; `a` and `b` are arrays of the same shape as `cond`, or scalars (broadcast); Int+Float promotes to Float | `where(array([true, false]), array([1, 2]), 0)` → `array([1, 0])` |

Wrong arity is a runtime error naming the builtin
(`len() expects 1 argument(s) but got 0`); a wrong argument type
is a clear error too (`len() expects a Str`,
`int() cannot parse "ciao" as an integer`). `int("3.5")` is an
error — parse a float first (`int(float("3.5"))`).

**Shadowing:** a `let` with the same name hides a builtin in the
current scope, like any other variable (`let len = 5` makes `len`
the number 5 there); outside that scope the builtin is back.

Interactive example — a guessing game:

```gg
let name = input("Come ti chiami? ")
print "Ciao, " + name + "!"

let secret = 7
let guess = int(input("Indovina un numero da 1 a 10: "))
if guess == secret then
    print "Bravo, hai indovinato!"
else
    if guess < secret then
        print "Troppo piccolo, era 7"
    else
        print "Troppo grande, era 7"
    end
end
```

```
$ echo -e "Giuliano\n5" | gg run indovina.gg
Come ti chiami? Ciao, Giuliano!
Indovina un numero da 1 a 10: Troppo piccolo, era 7
```

## Files

File handling is whole-file only: GG has no open file handles.
Paths are strings, relative to the current directory.
`read_file`/`write_file`/`append_file` fail with a runtime error
carrying the operating system's message when the file cannot be
read or written (`read_file("x.gg"): No such file or directory`).

Example — read a file, split it into lines, print them numbered:

```gg
let path = "notes.txt"
if file_exists(path) then
    let rows = lines(read_file(path))
    for i = 0 to len(rows) - 1 then
        print str(i + 1) + ". " + rows[i]
    end
else
    print "file not found: " + path
end
```

```
$ cat notes.txt
comprare il pane
chiamare Fabiana
$ gg run numbered.gg
1. comprare il pane
2. chiamare Fabiana
```

## Modules and virtual environments

A module is a `.gg` file. `import nome` loads `nome.gg` and
binds the name `nome` to the module value in the current scope —
so an `import` inside a block or a function binds the name there,
like `let`. Everything defined at top level in the module file
(`let`, `fn`, `class`) becomes an attribute of the module:

```gg
import greeter
print greeter.saluta("Giuliano")   # a function of the module
print greeter.PI                   # a constant of the module
let c = shapes.Cerchio(2)          # a class of the module
print type(greeter)                # module
print greeter                      # <module greeter>
```

`import nome as alias` binds the module to `alias` instead (the
original name is not bound). There is no `from x import y` in
this phase — it is planned for a later one. A module is a
first-class value: it can be stored in a variable (`let m =
greeter`), passed to a function and compared (`greeter == m` is
`true`). Attribute access reads only the module's own names: a
builtin such as `len` is *not* an attribute (`module 'm' has no
attribute 'len'`), and a missing attribute is a runtime error
(`module 'm' has no attribute 'y'`).

**Search order.** `import nome` looks for the module in four
levels, in this order: (1) a `.gg` file — the directory of the
file doing the import, then the `lib/` directories of the active
venvs, in flag order, then the current working directory; (2) an
embedded stdlib module written in GG (`stats`); (3) a native
module written in Rust (`math`, `random`); (4) an installed
package — the package's `entry` from its `gg.toml`, from the
project `gg_packages/` first, then the global store
`~/.gg/packages/` (the `gg.lock` in the working directory
pins the version when it lists the package, otherwise the
newest installed version wins). The first match wins:
a user file with the same name overrides a stdlib, a native
module or a package (`math.gg` next to your script shadows the native
`math`). A module that is not found is a runtime error listing
every path that was tried (`module 'nope' not found (searched:
...)`). Nested imports resolve against the directory of the
module that imports, not the main script: if `sub/mod.gg` does
`import helper`, GG looks for `sub/helper.gg` (the interpreter
tracks the current file through the import chain). In the REPL
there is no script, so the base is the working directory.

**Cache and cycles.** Each module runs **once** per process: a
second `import` of the same file returns the same value, and a
module imported by several files is still executed a single time.
A circular import is a runtime error naming the whole chain
(`circular import: a -> b -> a`).

**Virtual environments.** A venv is a project folder with its
own `lib/` for modules:

```sh
$ gg venv myproj
Created venv 'myproj':
  myproj/
  myproj/lib/
  myproj/gg.toml
$ cat myproj/gg.toml
name = "myproj"
version = "0.1.0"
```

`gg venv <nome>` fails if the directory already exists. Modules
in `myproj/lib/` are found by `import` when the venv is active:

```sh
$ gg run --venv myproj main.gg        # the flag is repeatable: --venv a --venv b
$ GG_VENV=myproj gg run main.gg       # the environment variable, same effect
```

The flag wins over `GG_VENV` when both are given; a `--venv`
directory that does not exist is an error. `GG_VENV` pointing
nowhere is silently ignored (the search just finds nothing
there).

Full example: `examples/modules_demo/` — `main.gg` imports
`greeter.gg` (next to it) and `shapes` (a class module in
`demo-venv/lib/`):

```sh
$ cd examples/modules_demo
$ gg run --venv demo-venv main.gg
Ciao Giuliano!
PI greco: 3.14
area: 12.56
module
<module greeter>
```

## Package manager

Packages share code between projects. A package is a directory
with a `gg.toml` manifest and `.gg` files; `import <name>`
loads the package's `entry` file (default `lib.gg`).

**The manifest (`gg.toml`).** TOML, parsed with the `toml`
crate (0.8) — a syntax error reports its line and column:

```toml
name = "ciao"                    # required: the importable name (a GG identifier)
version = "1.0.0"                # required: X.Y.Z (no prerelease/build metadata)
description = "Saluti in GG"    # optional
entry = "lib.gg"                 # optional, default lib.gg: the file `import ciao` loads
dependencies = ["owner/altro@1.0.0", "./locale"]   # optional: package specs
```

A missing or mistyped field is a clear error (`gg.toml:
missing required field 'version'`). The `entry` must be a
relative `.gg` path inside the package (no `..`, no absolute
path). A dependency spec is the same as an install spec (see
below); a relative local dependency resolves against the
depending package's own directory (npm rule), so `./b` in
`a/gg.toml` is `a/b`.

**Commands.**

```sh
$ gg install owner/repo            # from GitHub, default branch
$ gg install owner/repo@v1.2.3     # from GitHub, a tag (must agree with the manifest version)
$ gg install owner/repo@abcdef0... # from GitHub, a commit (7-40 hex characters)
$ gg install ./examples/demo-package   # from a local directory
$ gg install --local owner/repo    # into the project's gg_packages/ instead of the global store
$ gg install                       # reinstall exactly what gg.lock pins (like `npm ci`)
$ gg list                          # installed packages: name, version, global/local
$ gg remove ciao                   # remove every installed version (and prune gg.lock)
```

**Layout.** Packages live in `<root>/<name>/<version>/`
(`gg.toml` + `.gg` files; `.git` and `target` are never
copied). The roots are the project-local `gg_packages/`
(searched first at import) and the global store
`~/.gg/packages/`. A GitHub install clones with `git`
(`--depth 1` for a tag or the default branch, a full clone +
checkout for a commit) — `git` must be installed; a missing
`git` or a failing clone is a clear error. The lockfile
`gg.lock` (JSON, in the project directory) records each
package's `source` (`github:owner/repo` or `local:<path>`),
`version`, `resolved` (the commit hash, or the canonical
path for a local package) and `location` (`global`/`local`).

**Dependencies.** After installing a package, its
`dependencies` are installed recursively. A dependency cycle
is an error naming the chain (`dependency cycle: a -> b ->
a`), and two explicitly demanded versions of the same
package are an error (`version conflict for package 'bar'`
— v1 has no version resolver: pick one version). A
dependency without a version (`owner/bar`) takes whatever
version its manifest declares.

**Security, honestly.** `gg install` only downloads and
copies code — it never runs anything (no install hooks, by
design). Importing a package runs its GG code exactly like
any other `import`, so a package is trusted the way any
dependency is trusted: install only code you have read, or
from authors you trust. A package whose name is also a
built-in module (`math`, `stats`, ...) still installs, but
`import <name>` loads the built-in instead — `gg install`
warns about it.

**Limits (v1).** No central registry, no version resolver
(a conflict is an error), no `gg publish`, no `gg update`,
no signatures. Dotted imports (`import a.b`) do not exist
in GG, so a package exposes only its entry file — a
package's own internal modules are reached through relative
imports inside the package.

Full example: `examples/demo-package/` (the `ciao` package)
and `examples/package_demo.gg`:

```sh
$ gg install ./examples/demo-package
Installed ciao 1.0.0 (~/.gg/packages/ciao/1.0.0)
Updated gg.lock
$ gg run examples/package_demo.gg
Ciao Giuliano!
42
1.0.0
```

## Standard library

Phase 10 adds a minimal standard library in three layers, in
the import resolution order above: user `.gg` files first, then
embedded GG modules, then native Rust modules (installed
packages are a fourth layer, after the native modules — see
the Package manager section). Specialized
domains live in modules, not as top-level builtins: math
functions are `math.sqrt`, never a bare `sqrt`.

**Top-level builtins** (also listed in the Builtins table):
`sum`, `join`, `upper`, `lower`, `trim`, `replace`,
`starts_with`, `ends_with`.

**`math`** — native, in Rust:

| Function / constant | Description | Example |
| --- | --- | --- |
| `math.sqrt(x)` | Square root; an error on a negative number | `math.sqrt(16)` → `4` |
| `math.floor(x)` | Greatest integer <= x, as an Int | `math.floor(3.7)` → `3` |
| `math.ceil(x)` | Smallest integer >= x, as an Int | `math.ceil(3.2)` → `4` |
| `math.round(x)` | Nearest integer, as an Int; halves round away from zero | `math.round(2.5)` → `3`, `math.round(-2.5)` → `-3` |
| `math.pow(a, b)` | a to the power of b; two integers give an exact integer (overflow is an error), anything else a float | `math.pow(2, 10)` → `1024` |
| `math.sin(x)` / `math.cos(x)` / `math.tan(x)` | Trigonometric functions, in radians | `math.sin(0)` → `0` |
| `math.log(x)` | Natural logarithm; an error on x <= 0 | `math.log(1)` → `0` |
| `math.exp(x)` | e to the power of x | `math.exp(0)` → `1` |
| `math.PI` / `math.E` | The constants π and e | `math.PI` → `3.141592653589793` |

**`random`** — native, in Rust. The generator is SplitMix64,
implemented in pure Rust with no external crate: fast and good
enough for games and sampling, **not** cryptographic, seeded
from the wall clock plus the process id, so every run gets a
different sequence.

| Function | Description | Example |
| --- | --- | --- |
| `random.randint(a, b)` | A random integer between a and b, both ends included; an error when a > b | `random.randint(1, 6)` → `4` |
| `random.random()` | A random float in [0, 1) | `random.random()` → `0.37...` |
| `random.choice(list)` | A random element of the list; an error on an empty list | `random.choice([10, 20])` → `20` |
| `random.shuffle(list)` | Shuffle the list in place (Fisher-Yates), return `nil` | `random.shuffle(xs)` |

**`linalg`** — native, in Rust (Phase 14, `rank` in Phase 18). Gaussian elimination
with partial pivoting over the N-d numeric arrays: `solve` and
`inv` return Float arrays, `det` a Float, `rank` an Int. A singular matrix —
determinant zero, i.e. an impossible or indeterminate system —
is a runtime error for `solve` and `inv` (the GG convention:
`matmul` on incompatible shapes is an error too, never a
silent `nil`); `det` of a singular matrix is `0`. `rank` is
what tells the two apart: a system is indeterminate when
`rank(A) == rank([A|b]) < n`, and impossible when
`rank(A) < rank([A|b])` (n = number of unknowns); it works on
rectangular matrices too, and an empty matrix has rank 0.
Results are
f64, so a system like the classic 3x3 can print
`3.0000000000000004` instead of `3`: elimination divides by
numbers with no exact binary form, and a little rounding is
normal — the same happens in NumPy.

| Function | Description | Example |
| --- | --- | --- |
| `linalg.solve(A, b)` | The solution `x` of `A x = b`: `A` square 2D, `b` a 1D vector of the same length | `linalg.solve(reshape(array([2, 1, 1, 3]), [2, 2]), array([5, 10]))` → `array([1, 3])` |
| `linalg.det(A)` | The determinant of a square 2D matrix | `linalg.det(reshape(array([1, 2, 3, 4]), [2, 2]))` → `-2` |
| `linalg.inv(A)` | The inverse matrix | `linalg.inv(reshape(array([1, 2, 0, 1]), [2, 2]))` → `array([[1, -2], [0, 1]])` |
| `linalg.rank(A)` | The rank of a 2D numeric matrix, square or rectangular: the number of independent rows (pivots in row echelon form, same 1e-12 tolerance as `solve`) | `linalg.rank(reshape(array([1, 2, 2, 4]), [2, 2]))` → `1` |

Full example: `examples/linalg_demo.gg` — the systems from the
maths ripasso, plus the classic 3x3 with its f64 rounding.
`examples/rank_demo.gg` shows `rank` telling an indeterminate
system from an impossible one.

**`json`** — native, in Rust. A hand-written recursive-descent
parser, pure Rust with no external crate. Objects become Dicts
(insertion order kept, a duplicate key keeps the last value),
arrays become Lists, integers that fit in i64 become Int (anything
else numeric becomes Float), `true`/`false` become Bool and
`null` becomes Nil. Parse errors are runtime errors with the
position inside the JSON text (`json.parse(): ... at line L,
column C`), catchable with `try`/`except`. `stringify` goes the
other way: Dict keys that are not strings are written as their
string form, Lists, Tuples, Sets and Arrays become JSON arrays,
floats print in the shortest form that round-trips (`1.0` stays
`1.0`, so `parse(stringify(x)) == x` for every finite float);
functions, classes, instances, modules, queues, stacks and
generators are runtime errors, never silent `nil`. Nesting
deeper than 128 levels is an error on both sides — that is also
what makes `stringify` terminate on a cyclic value (a list
containing itself) instead of recursing forever.

| Function | Description | Example |
| --- | --- | --- |
| `json.parse(text)` | Parse a JSON document into a GG value | `json.parse("{\"a\": [1, 2]}")` → `{"a": [1, 2]}` |
| `json.stringify(value)` | Render a GG value as compact JSON | `json.stringify({"a": 1})` → `"{\"a\":1}"` |
| `json.stringify(value, indent)` | Pretty-print with `indent` spaces per level (0 = compact); empty collections stay on one line | `json.stringify({"a": [1, 2]}, 2)` → `"{\n  \"a\": [\n    1,\n    2\n  ]\n}"` |

Full example: `examples/json_demo.gg` — write a document to a
file, read it back, parse it, change a field, and check the
round-trip, on both backends.

**`os`** — native, in Rust. The process and the filesystem, pure
Rust with no external crate. `os.args()` is the command line as
a List of Str, program name first (`gg run file.gg` gives
`["gg", "run", "file.gg"]` — `gg run` takes no script arguments
of its own). `os.getenv(name)` returns the variable as a Str, or
Nil when it is not set; `os.getenv(name, default)` returns the
default instead. `os.listdir(path)` returns the entry names
only, sorted, not recursive. `mkdir` creates one directory and
errors when it already exists or the parent is missing (no
recursive creation); `remove` deletes a file and errors on a
directory or a missing path. All errors are runtime errors with
a clear message, catchable with `try`/`except`. Out of scope, by
design: running commands (`os.system`) and writing environment
variables — GG programs observe the environment, they do not
change it or spawn processes.

| Function | Description | Example |
| --- | --- | --- |
| `os.args()` | The command line as a List of Str, program name first | `os.args()` → `["gg", "run", "file.gg"]` |
| `os.getenv(name)` | The value of an environment variable, or Nil when it is not set | `os.getenv("HOME")` → `"/home/giuliano"` |
| `os.getenv(name, default)` | The value, or the default when the variable is not set | `os.getenv("NOPE", "nessuna")` → `"nessuna"` |
| `os.cwd()` | The current working directory as a Str | `os.cwd()` → `"/home/giuliano/GG"` |
| `os.listdir(path)` | The names in a directory, sorted, not recursive | `os.listdir("examples")` → `["args_demo.gg", ...]` |
| `os.mkdir(path)` | Create one directory, return `nil`; an error when it exists or the parent is missing | `os.mkdir("/tmp/nuova")` |
| `os.remove(path)` | Delete a file, return `nil`; an error on a directory or a missing path | `os.remove("/tmp/vecchio.txt")` |

Full example: `examples/os_demo.gg` — cwd, args, an
environment variable and the `examples/` listing, on both
backends.

**`datetime`** — native, in Rust. Unix timestamps and UTC
calendar dates, pure Rust with no external crate: the
calendar math (days-from-civil / civil-from-days) is written
out by hand, and it works for negative timestamps too
(before 1970). **Everything is UTC** — there is no local
time here; that is a later step. A timestamp is an Int,
seconds since 1970-01-01 00:00:00 UTC. `from_timestamp`
returns a Dict with `year`, `month`, `day`, `hour`, `minute`,
`second` and `weekday`, where `weekday` is 0 for Monday
through 6 for Sunday (1970-01-01 was a Thursday, so
`from_timestamp(0)["weekday"]` is 3). `to_timestamp` is the
inverse: it reads the same fields — any `weekday` key is
ignored, it is recomputed — and validates them (month 1-12, a
day that exists in that month of that year, leap years
included, hour 0-23, minute and second 0-59); anything else
is a runtime error, catchable with `try`/`except`. `format`
takes an Int timestamp or a Dict like the one
`from_timestamp` returns, and a format string with a
`strftime` subset (`%Y` year, `%m` month, `%d` day, `%H`
hour, `%M` minute, `%S` second, `%w` weekday 0-6 as above);
`%%` is a literal `%`, any other directive — or a trailing
`%` — is an error.

| Function | Description | Example |
| --- | --- | --- |
| `datetime.now()` | The current Unix timestamp in seconds, as an Int | `datetime.now()` → `1700000000` |
| `datetime.from_timestamp(ts)` | The UTC date and time of a timestamp, as a Dict with `year`, `month`, `day`, `hour`, `minute`, `second` and `weekday` (0 = Monday .. 6 = Sunday) | `datetime.from_timestamp(0)` → `{"year": 1970, "month": 1, "day": 1, "hour": 0, "minute": 0, "second": 0, "weekday": 3}` |
| `datetime.to_timestamp(dict)` | The Unix timestamp of a Dict like the one `from_timestamp` returns, validating every field | `datetime.to_timestamp(datetime.from_timestamp(1700000000))` → `1700000000` |
| `datetime.format(ts_or_dict, fmt)` | The date and time as a Str, with a `strftime` subset (`%Y %m %d %H %M %S %w`); the first argument is an Int timestamp or a Dict | `datetime.format(1700000000, "%Y-%m-%d %H:%M:%S")` → `"2023-11-14 22:13:20"` |
| `datetime.date(year, month, day)` | The Unix timestamp of midnight UTC on that day, validating the date | `datetime.date(2024, 2, 29)` → `1709164800` |

Full example: `examples/datetime_demo.gg` — the current
time, today in UTC and a few known conversions, on both
backends.

**`regex`** — native, in Rust, via the `regex` crate (1.13.2)
from crates.io — the first stdlib module with an external
dependency. The pattern syntax is the crate's: character
classes (`[a-z]`, `\d`, `\w`, `\s`, `\p{L}` for Unicode
letters), anchors (`^`, `$`), quantifiers (`*`, `+`, `?`,
`{n,m}`), numbered `(...)` and named `(?P<nome>...)` groups,
and alternation (`|`). There are no backreferences in
patterns — the crate does not support them; backreferences
live in the replacement string of `replace` instead (see
below). Patterns match on Unicode text: `.` matches one
Unicode character (not one byte), `\d` matches any Unicode
decimal digit, and accented letters and emoji match literally.
`test` is true when the pattern matches anywhere in the text.
`find` returns every non-overlapping match, in order. `groups`
returns the first match as a List — index 0 is the whole match,
then one element per capture group, Nil for an optional group
that did not participate — and an empty List when there is no
match. `replace` replaces every occurrence; the replacement
supports `$1` (numbered group), `$nome` (named group),
`${nome}` (unambiguous form) and `$$` (a literal `$`) — the
`regex` crate's syntax, not GG's. `split` cuts the text at
every match. An invalid pattern is a runtime error carrying
the crate's message (`regex.test(): invalid regex: ...`),
catchable with `try`/`except`.

| Function | Description | Example |
| --- | --- | --- |
| `regex.test(pattern, text)` | True when the pattern matches anywhere in the text | `regex.test("^\\d+$", "42")` → `true` |
| `regex.find(pattern, text)` | Every non-overlapping match, in order, as a List of Str; `[]` when nothing matches | `regex.find("[0-9]+", "a12 b3")` → `["12", "3"]` |
| `regex.groups(pattern, text)` | The first match as a List: index 0 is the whole match, then the capture groups (Nil for a non-participating optional group); `[]` when no match | `regex.groups("(\\d+)-(\\d+)", "tel 555-1234")` → `["555-1234", "555", "1234"]` |
| `regex.replace(pattern, text, replacement)` | Replace every occurrence; the replacement supports `$1`, `$nome`, `${nome}` and `$$` | `regex.replace("(\\w+)@(\\w+)", "a@b", "$2@$1")` → `"b@a"` |
| `regex.split(pattern, text)` | Cut the text at every match, as a List of Str | `regex.split(",\\s*", "a, b,c")` → `["a", "b", "c"]` |

Full example: `examples/regex_demo.gg` — find, groups,
replace with `$1`/`$2` and split on a concrete text, on both
backends.

**`http`** — native, in Rust, via the `ureq` crate (3.5.2)
from crates.io — a blocking HTTP client (no async in GG).
Every call returns a Dict with `status` (the HTTP status
code as an Int), `headers` (a Dict of the response headers,
keys lowercased, values Str; a repeated header keeps the last
value) and `body` (the response body as a Str). An HTTP error
status (404, 500, ...) is NOT an error: it comes back in the
Dict with its status and body, like any serious HTTP library.
Only failures to perform the request are runtime errors — an
invalid URL, an unreachable host, a refused connection, a
timeout (30 seconds for the whole exchange, connect and read
included) — catchable with `try`/`except`. The body is bytes
on the wire and a Str in GG, so it is decoded with
`from_utf8_lossy`: bytes that are not valid UTF-8 are replaced
with U+FFFD instead of failing the request. TLS is verified
against the platform trust store, not just the bundled Mozilla
roots, so it also works behind a proxy whose CA is installed
in the system store. `request` takes Nil for "no body" and
"no headers" — GG has no `nil` literal, so Nil comes from a
call that returns it, e.g. `json.parse("null")`. Only POST,
PUT and PATCH take a body; a body with any other method is an
error, never silently dropped. The method name is
case-insensitive (`"get"` works like `"GET"`).

| Function | Description | Example |
| --- | --- | --- |
| `http.get(url)` | GET, as a Dict with `status`, `headers` and `body` | `http.get("https://example.com")["status"]` → `200` |
| `http.get(url, headers)` | GET with request headers, a Dict of Str to Str | `http.get(url, {"Accept": "application/json"})` |
| `http.post(url, body)` | POST the body as UTF-8 bytes, `Content-Type: text/plain` | `http.post(url, "{\"a\": 1}")["status"]` → `200` |
| `http.post(url, body, content_type)` | POST with an explicit Content-Type | `http.post(url, body, "application/json")` |
| `http.request(method, url, body, headers)` | Any method (`"GET"`, `"PUT"`, `"DELETE"`, ...); body is a Str or Nil, headers a Dict or Nil | `http.request("DELETE", url, json.parse("null"), json.parse("null"))["status"]` → `200` |

Full example: `examples/http_demo.gg` — GET from
example.com, POST of a JSON body to httpbin.org with the
echoed field parsed back with `json`, and a fallback GET when
httpbin does not answer, on both backends.

**`sqlite`** — native, in Rust, via the `rusqlite` crate
(0.32) from crates.io with the `bundled` feature, so SQLite
is compiled from its C sources and no system SQLite library
is needed: an embedded database, no server, no setup. The
API is functional, not method calls — GG only calls methods
on instances, classes and modules, and a native handle is
none of those, so the connection is passed explicitly as the
first argument. `open` returns a connection handle
(`type(db)` is `"sqlite"`, `print db` is `<sqlite>`,
`<closed sqlite>` after `close`); the handle is shared, so
`let db2 = db` aliases the same connection and closing one
closes the other too. Every SQLite failure — a syntax error,
a missing table, a violated constraint, a wrong number of
parameters — is a runtime error carrying SQLite's message,
catchable with `try`/`except`, with the position of the call.

| Function | Description | Example |
| --- | --- | --- |
| `sqlite.open(path)` | Open (or create) the database file; `":memory:"` is a temporary in-memory database | `sqlite.open("app.db")` |
| `sqlite.exec(db, sql)` | Statements with no result (`CREATE TABLE`, `BEGIN`, `COMMIT`, ...); several statements separated by `;` run in one call; returns Nil | `sqlite.exec(db, "CREATE TABLE utenti (id INTEGER PRIMARY KEY, nome TEXT)")` |
| `sqlite.execute(db, sql, params)` | A statement with `?` placeholders, `params` a List; returns the number of rows changed as an Int | `sqlite.execute(db, "INSERT INTO utenti (nome) VALUES (?)", ["Giuliano"])` → `1` |
| `sqlite.query(db, sql)` | A SELECT: a List of Dicts, one per row, keys are the column names in order | `sqlite.query(db, "SELECT * FROM utenti")` → `[{"id": 1, "nome": "Giuliano"}]` |
| `sqlite.query(db, sql, params)` | A SELECT with `?` placeholders | `sqlite.query(db, "SELECT * FROM utenti WHERE eta >= ?", [18])` |
| `sqlite.close(db)` | Close the connection; any later use of it, including a second `close`, is a runtime error | `sqlite.close(db)` |

Parameters map as Int → INTEGER, Float → REAL, Str → TEXT,
Bool → INTEGER 0/1 (SQLite has no boolean type), Nil →
NULL; anything else is an error naming the parameter
position. Query results map back as INTEGER → Int, REAL →
Float, TEXT → Str, NULL → Nil and BLOB → a List of Int, one
per byte — GG has no byte-string type, so a BLOB cannot come
in as a parameter either (insert one with the SQL literal
`x'6869'`). Transactions work through plain SQL
(`exec(db, "BEGIN")` ... `exec(db, "COMMIT")`); that is SQL,
not API. Out of scope: prepared statements exposed to GG,
backup, pragmas and threading.

Full example: `examples/sqlite_demo.gg` — create a table,
insert rows with parameters, select them back, update one,
on both backends.

**`stats`** — written in pure GG (`stdlib/stats.gg` in the
repo), embedded in the binary with `include_str!`, so it works
with no file on disk. The files in `stdlib/` are the source of
truth; a test checks the embedded copy matches them. It imports
`math` itself for the square root.

| Function | Description | Example |
| --- | --- | --- |
| `stats.mean(xs)` | The arithmetic mean | `stats.mean([1, 2, 3, 4])` → `2.5` |
| `stats.median(xs)` | The median (the list is sorted on a copy; the input is not modified) | `stats.median([1, 2, 3, 4])` → `2.5` |
| `stats.variance(xs)` | Population variance (divided by N, not N - 1) | `stats.variance([1, 2, 3, 4, 5])` → `2` |
| `stats.stdev(xs)` | Population standard deviation | `stats.stdev([1, 2, 3, 4, 5])` → `1.4142135623730951` |

An empty list is an error for every `stats` function. `type(math)`
is `"module"`, like for file modules.

Full example: `examples/stdlib_demo.gg` — grades, a die roll
and π:

```gg
import math
import random
import stats

let voti = [7, 8, 6, 9, 8, 7, 10, 6]
print "media: " + str(stats.mean(voti))
print "mediana: " + str(stats.median(voti))
print "deviazione standard: " + str(stats.stdev(voti))
print "dado: " + str(random.randint(1, 6))
print "radice di 2: " + str(math.sqrt(2))
print "pi greco: " + str(math.PI)
```

```sh
$ gg run examples/stdlib_demo.gg
media: 7.625
mediana: 7.5
deviazione standard: 1.3169567191065923
dado: 4
radice di 2: 1.4142135623730951
pi greco: 3.141592653589793
```

(The die roll differs on every run.)

## N-dimensional arrays

Phase 11 makes the `array` type from Phase 7 N-dimensional.
Internally an array is its flat items in row-major order plus a
shape (one size per dimension): a 1D array of 3 elements has
shape `[3]`, a 2x2 matrix has shape `[2, 2]`. The 1D arrays of
Phase 7 are shape `[n]` and keep working exactly as before.
`len(a)` is still the total number of elements, `type(a)` is
still `"array"`, and `str(a)` prints 1D arrays as before and 2D
arrays nested, NumPy style: `array([[1, 2], [3, 4]])`.

**Shape and indexing:**

| Expression | Description | Example |
| --- | --- | --- |
| `a[i]` | One element of a 1D array (negative from the end) | `array([10, 20])[1]` → `20`, `array([10, 20])[-1]` → `20` |
| `a[i, j]` | One element of a 2D array, one index per dimension (negative per axis) | `m[1, 0]` → `3` on `array([[1, 2], [3, 4]])` |
| `a[i]` on a 2D array | Its i-th row, as a 1D array (so `a[0][1]` is the same as `a[0, 1]`) | `m[0]` → `array([1, 2])` |
| `a[i, j] = v` | Assign one element (one index per dimension; a single index only for 1D arrays — a whole row cannot be assigned) | `m[0, 1] = 7` |
| `shape(a)` / `ndim(a)` / `size(a)` | The shape as a list, the number of dimensions, the total number of elements | `shape(m)` → `[2, 2]`, `ndim(m)` → `2`, `size(m)` → `4` |

**Constructors:** `zeros(n)` / `zeros([r, c])`, `ones(n)` /
`ones([r, c])`, `arange(n)` / `arange(a, b)` /
`arange(a, b, step)`, `linspace(a, b, n)` (see the Builtins
table). A shape takes an Int (1D) or a List of Ints; dimensions
must be positive integers, and constructors refuse arrays above
1000000 elements. A 2D array is built with `reshape`: there is
no nested-list literal for arrays — `array([[1, 2]])` is an
error, because `array()` takes a flat list of scalars.

**Element-wise arithmetic:** `+ - * /` between two arrays of
the same shape, or between an array and a scalar (broadcast from
either side: `a + 1` and `1 + a`). Int/float promotion follows
scalar arithmetic, and `/` is always float division. Unary minus
negates every element (`-a`). Shape mismatch, non-numeric
arrays (an array of strings plus an array of strings is an
error, not a concatenation) and division by zero are runtime
errors. **Phase 12 changed `==`/`!=` between arrays** (see the
Slicing section below): comparisons are now element-wise and
return a Bool array; the old whole-array `==` (one Bool for the
whole array) is gone.

**Matrix operations:**

| Function | Description |
| --- | --- |
| `dot(a, b)` | Scalar product of two 1D vectors of the same length |
| `matmul(a, b)` | Matrix product: 2D x 2D, 1D x 2D (row vector times matrix) and 2D x 1D (matrix times column vector); two 1D vectors are an error — use `dot()` |
| `transpose(a)` | Swap the two axes of a 2D array; a 1D array is returned unchanged |
| `reshape(a, shape)` | The same items with a new shape; the total number of elements must stay the same |

`sum(a)`, `min(a)` and `max(a)` work on whole arrays (all
elements); `min(a, b)` / `max(a, b)` on two numbers are
unchanged.

Full example: `examples/matmul_demo.gg` — matrix multiplication
and a 90-degree rotation of a 2D point:

```gg
let a = reshape(array([1, 2, 3, 4]), [2, 2])
let b = reshape(array([5, 6, 7, 8]), [2, 2])
print "A @ B:"
print matmul(a, b)
print "dot di [1, 2, 3] e [4, 5, 6]:"
print dot(array([1, 2, 3]), array([4, 5, 6]))
print "Rotazione di 90 gradi del punto (1, 0):"
let r = reshape(array([0, -1, 1, 0]), [2, 2])
print matmul(r, array([1, 0]))
```

```sh
$ gg run examples/matmul_demo.gg
A @ B:
array([[19, 22], [43, 50]])
dot di [1, 2, 3] e [4, 5, 6]:
32
Rotazione di 90 gradi del punto (1, 0):
array([0, 1])
```

## Slicing and boolean indexing

Phase 12 adds Python-style slices and boolean indexing. A slice
`start:stop:step` has three optional bounds: `stop` is excluded,
out-of-range bounds are clamped (never an error), a step of 0 is
a runtime error, and a negative step walks backwards. Slicing
always returns a **copy**, never a view.

**Slicing, by type:**

| Expression | Description | Example |
| --- | --- | --- |
| `xs[1:3]`, `xs[:2]`, `xs[2:]`, `xs[::2]`, `xs[::-1]` | A slice of a list (new list) | `[1, 2, 3, 4, 5][1:3]` → `[2, 3]` |
| `s[1:3]` | A slice of a string, by character (not by byte, like `len`) | `"ciao"[1:3]` → `"ia"`, `"aèb"[1:2]` → `"è"` |
| `t[1:3]` | A slice of a tuple (new tuple) | `(1, 2, 3, 4)[1:3]` → `(2, 3)` |
| `a[1:4]` | A slice of a 1D array (new 1D array) | `array([10, 20, 30, 40])[1:3]` → `array([20, 30])` |
| `a[0:2, 1:3]` | A sub-array of a 2D array | `m[0:2, 1:3]` → `array([[2, 3], [5, 6]])` on a 2x3 `m` |
| `a[:, 1]` | A whole column, as a 1D array | `m[:, 1]` → `array([2, 5])` |
| `a[0, :]` | A whole row, as a 1D array | `m[0, :]` → `array([1, 2, 3])` |
| `a[i, j]` and `a[i]` | Unchanged from Phase 11: one element, or a row on 2D arrays | `m[1, 0]` → `4` |

On arrays, each axis takes either an index or a slice; axes
without a selector are whole (`a[0:2]` on a 2D array keeps every
column). Indexed axes drop out of the shape, sliced axes keep
their selected length, in axis order. A single index on a string
is still an error — strings take a slice only.

**Slice assignment** (lists and 1D arrays only, same length
required):

```gg
let xs = [1, 2, 3, 4]
xs[1:3] = [9, 9]       # xs is now [1, 9, 9, 4]
let a = array([1, 2, 3, 4])
a[1:3] = array([9, 9]) # a is now array([1, 9, 9, 4])
```

A different length is a runtime error. Slice assignment on 2D
arrays is not done yet, and neither is assignment through a
mask (`a[a > 2] = 0` is an error).

**Comparisons on arrays → Bool arrays:** `== != < <= > >=`
work element by element, between two arrays of the same shape
or between an array and a scalar (broadcast from either side,
`2 < a` works too). Arrays can now hold Bool elements
(`array([true, false])`), and `sum` on a Bool array counts the
`true` values. Shape mismatch is a runtime error. An array
against a List or a Dict is not an element-wise comparison — it
falls back to whole-value equality, so `array([1]) == [1]` is
still `false`. Ordering (`<`, `<=`, `>`, `>=`) needs numbers;
`==`/`!=` work on any element type. `and`/`or` are **not**
extended to arrays — they stay scalar with short-circuit.

**Boolean indexing:** `a[mask]` keeps the elements where the
mask (a Bool array with the same shape as `a`) is true,
flattened to a 1D array in row-major order. It works on arrays
only — for lists, filter with a loop, as in Phase 6.

**`where(cond, a, b)`** (see the Builtins table): element-by-element
selection — where `cond` is true take from `a`, otherwise from
`b`. `a` and `b` are arrays of `cond`'s shape, or scalars.

Full example: `examples/slicing_demo.gg` — filter values above
a threshold with a mask and extract a sub-matrix:

```gg
let a = reshape(array([1, 2, 3, 4, 5, 6, 7, 8, 9]), [3, 3])
print "Valori sopra 5:"
print a[a > 5]
print "Sotto-matrice:"
print a[0:2, 1:3]
print "Prima riga:"
print a[0, :]
print "Ultima colonna:"
print a[:, 2]
print "Sopra 5, altrimenti zero:"
print where(a > 5, a, 0)
```

```sh
$ gg run examples/slicing_demo.gg
Valori sopra 5:
array([6, 7, 8, 9])
Sotto-matrice:
array([[2, 3], [5, 6]])
Prima riga:
array([1, 2, 3])
Ultima colonna:
array([3, 6, 9])
Sopra 5, altrimenti zero:
array([[0, 0, 0], [0, 0, 6], [7, 8, 9]])
```

## Graphical interfaces (`gui`)

Phase 13 adds windows: a native `gui` module built on egui 0.37
+ eframe 0.37 (immediate mode). The module lives behind the
`gui` Cargo feature, which is on by default — a headless library
build is `cargo test -p gg-interp --no-default-features`, and
then `import gui` fails with the usual "module not found" error.
(The `gg` binary always enables the feature, so
`--workspace --no-default-features` does not turn it off:
Cargo unifies features across the workspace.)

How it works: `gui.run(titolo, fn)` opens a window and enters
the event loop, calling the GG function `fn` (no arguments)
once per frame. The function builds the UI by calling widget
functions in order — there is no widget tree to keep in sync
with GG state, the script just redraws everything every frame.
When the frame function returns `false`, the window closes and
`gui.run` returns `nil`; closing the window with the X button
does the same. A runtime error inside the frame function also
closes the window, and `gui.run` reports the error. Variables
defined outside the frame function stay visible and mutable
inside it: the frame function is an ordinary GG closure.

**The `gui` API:**

| Function | Description |
| --- | --- |
| `gui.run(titolo, fn)` | Open a window titled `titolo`, run the loop, return `nil` when it closes |
| `gui.heading(testo)` | A large bold label |
| `gui.label(testo)` | A plain label |
| `gui.separator()` | A horizontal line |
| `gui.button(testo)` | A button; returns `true` only in the frame where it is clicked |
| `gui.text_input(id, valore)` | A single-line text field; returns the text after this frame (reassign it to the variable) |
| `gui.slider(id, valore, min, max)` | A numeric slider; returns the value after this frame (a Float) |
| `gui.checkbox(id, testo, valore)` | A checkbox with a label; returns the state after this frame |
| `gui.radio(id, testo, selezionato)` | A radio button (a circle); returns `true` only in the frame where it is clicked, like `button` — for an exclusive group, one `radio` per option, no implicit groups |
| `gui.combo(id, opzioni, selezionato)` | A dropdown menu; `opzioni` is a List of Str, `selezionato` the current index (an Int); returns the selected index after this frame (empty list or out-of-range index is an error) |
| `gui.progress(valore)` | A progress bar; `valore` is a number in 0..1, values outside are clamped to [0, 1] (not an error) |
| `gui.horizontal(fn)` | Run the GG function `fn` laying out the widgets it creates horizontally instead of vertically |
| `gui.vertical(fn)` | Like `horizontal`, but the widgets the function creates are laid out vertically (useful inside a `horizontal` block to nest a column in a row) |

`text_input`, `slider`, `checkbox`, `radio` and `combo` take a stable string `id`:
egui keeps the cursor, the edit state and the drag state under
that id, so the widget keeps its state across frames even when
the layout changes. Two widgets with the same `id` share state —
give each one its own id.

**Example** — a counter app (`examples/gui_demo.gg`); it closes
by itself after about 60 frames:

```gg
import gui

let count = 0
let frame = 0
let name = "Giuliano"
let size = 5
let loud = false

fn riga()
    gui.label("Nome:")
    name = gui.text_input("nome", name)
end

fn draw()
    frame = frame + 1
    gui.heading("Contatore GG")
    gui.label("Conteggio: " + str(count))
    if gui.button("Incrementa") then
        count = count + 1
    end
    gui.separator()
    gui.horizontal(riga)
    size = gui.slider("size", size, 1, 10)
    gui.label("Size: " + str(size))
    loud = gui.checkbox("loud", "Forte", loud)
    if frame >= 60 then
        return false
    end
end

gui.run("Demo GG", draw)
print "finestra chiusa dopo " + str(frame) + " frame, conteggio " + str(count)
```

Output (headless, under Xvfb):

```
finestra chiusa dopo 61 frame, conteggio 0
```

(The frame function returns `false` at frame 60; eframe runs
one more frame before the window actually closes.)

**How it is wired in:** `gui` is a native module like `math`
and `random` (a user file named `gui.gg` wins over it, like any
native module), but `gui.run` and `gui.horizontal` need the
interpreter to call GG functions back, and plain builtins do
not receive it. So builtins come in two kinds now:
`BuiltinFunc::Plain` (the old `BuiltinFn`) and
`BuiltinFunc::WithInterp` (a function that also takes
`&mut Interpreter`); the interpreter dispatches on the kind in
`call_value`. The current `egui::Ui` travels in a thread-local,
installed by a guard while the frame function runs — widgets
called outside a frame function are a clear runtime error, not
a panic. `GuiApp` holds the interpreter as a raw pointer while
eframe owns the app for the whole event loop (eframe cannot
borrow it and hand it back); GG is single-threaded, the
pointers are only dereferenced while `run_native` runs, and a
nested `gui.run` is an error. The frame loop repaints
continuously (no frame-rate control in v0.1 — keep frame
functions light).

**Honest limits (v0.1):** a display is required — on a machine
with no display `gui.run` fails with a clear error (on Linux it
checks `DISPLAY`/`WAYLAND_DISPLAY` first); on this VM the live
test runs under Xvfb with the lavapipe software Vulkan driver,
and the first real interactive test is on a laptop. One window
only, no canvas/images, no themes, no multiple windows, and no frame-rate control yet. (Phase 17 demo: `examples/gui_widgets_demo.gg` — radio, combo, progress and vertical; it closes by itself after about 60 frames.)

## Bytecode virtual machine

GG runs on a bytecode VM, like CPython: the compiler
(`crates/gg-interp/src/vm/compiler.rs`) turns the AST into a
`CodeObject` — a list of instructions, a constant table, a
name table and a slot count — and the machine (`vm/machine.rs`)
executes it on an operand stack, with one locals array per
frame. Every function, method and lambda compiles to its own
code object, eagerly, at definition time; a script or module
compiles as one code object. Compilation happens on the first
execution; there is no bytecode cache on disk.

The tree-walking interpreter is still in the code, behind the
same `Interpreter` API, and stays the fallback. Select the
engine with the `GG_BACKEND` environment variable: `vm` (the
default) or `tree`. Every public entry point (`run`, `eval`,
`call_value`, `call_method`, imports, the REPL, `gui.run`'s
frame function) dispatches on it, so switching backends changes
nothing but the engine.

### How it works

- **Values and scopes are shared with the tree-walker.** The VM
  reuses `Value`, the chained `Environment`s (for globals and
  module attributes), the builtins and the semantics helpers
  (`bind_arguments`, `index_value`, `binary_values`,
  `match_pattern`, ...), so closures, shadowing and mutable
  captured bindings behave exactly as before. Locals, though,
  are slot-based: the compiler assigns each local a numeric
  slot per code object (`LoadSlot` / `StoreSlot` /
  `DefineSlot`), and the frame reads it by index — no scope
  chain walk. A name bound in an enclosing function becomes
  an upvalue: a cell (`Rc<RefCell<Value>>`) shared between the
  defining frame and the nested one, so a closure sees later
  rebindings, two closures share one binding, and a grandchild
  reads the same cell as its grandparent (captured
  transitively, one upvalue per intermediate function). Only
  names bound nowhere in any enclosing function stay
  name-based (`LoadName` / `StoreName` / `DefineName`): globals,
  module attributes, and a REPL chunk's top level, which must
  persist across chunks.
- **Spans.** Every instruction carries the span of the token
  that generated it, and the VM loop uses it as the fallback
  position for errors — exactly like the statement span in the
  tree-walker. Error messages and positions are identical on
  both engines (a dual-backend test corpus in
  `crates/gg-interp/src/vm/tests.rs` runs every program on both
  and requires the same stdout or the same error text).
- **Calls.** Arguments are evaluated in source order into a
  positional list plus keyword values; `Call`/`CallMethod`/
  `CallSuper`/`Construct` bind them with the same
  `bind_arguments` the tree-walker uses (defaults evaluated
  once at definition time, `*args`, spread, kwargs).
- **Loops and comprehensions.** Loop state lives in slots of
  the loop's own scope (`$for_to`, `$for_iter`, `$comp_acc`,
  ... — `$`-prefixed names, which user code cannot spell, so
  they can never collide with a real variable). The numeric
  `for` keeps the last value the body ran with when it ends,
  like the tree-walker — a closure that captured the loop
  variable sees that value, not one past the end — and the
  loop variable is one shared binding across iterations, while
  a comprehension variable is fresh per item.
- **Try/except/finally.** A handler stack with Try/Except/
  Finally phases; `finally` runs on normal, error, `return`
  and `yield` exits, and a `return`/error inside `finally`
  replaces the pending outcome (the Python rule). The `except`
  variable and names defined in `finally` are scoped to the
  handler and do not leak out.
- **Generators.** The replay model is unchanged: `next()`
  re-runs the code object from the start with a yield counter,
  and `Yield` suspends at the target count. `for x in gen`
  pulls lazily.
- **Match.** Compiled to jumps: duplicate the value, try each
  pattern (`MatchPattern` binds into slots, with fresh cells
  per match), evaluate the guard, and jump to the next case on
  failure.
- **F-strings.** Compiled to concatenation (`BuildStr`).
- **Indexing/slicing.** Slices travel on a frame-local side
  stack (`PushIndexValue`/`PushIndexSlice`), not the operand
  stack, so no new `Value` variant was needed.

### Opcodes

`LoadConst`, `LoadName`, `StoreName`, `DefineName`,
`LoadSlot`, `StoreSlot`, `DefineSlot`, `LoadUpvalue`,
`StoreUpvalue`, `Pop`, `Dup`, `Jump`, `JumpIfFalse`,
`JumpIfTrue`, `Print`, `Return`, `Yield`, `Neg`, `Not`,
`Binary`, `BuildList`, `ListAppend`, `ListExtend`,
`BuildTuple`, `BuildDict`, `DictInsert`, `BuildSet`,
`SetInsert`, `BuildStr`, `MakeFunction`, `MakeClass`, `Call`,
`CallMethod`, `CallSuper`, `Construct`, `GetField`,
`SetField`, `GetIndex`, `SetIndex`, `PushIndexValue`,
`PushIndexSlice`, `EnterScope`, `ForPrep`, `ForCheck`,
`ForNext`, `ForInPrep`, `ForInNext`, `CompPrep`, `CompNext`,
`BindComp`, `CompAppend`, `CompDictInsert`, `CompSetInsert`,
`MatchPattern`, `PushHandler`, `FinishTry`, `EndFinally`,
`Raise`, `Import`.

### Benchmark (honest numbers)

Release build, this VM, `time` on `gg run` (median of 3 runs
each; outputs identical on both engines):

| program | tree-walker | VM | |
|---|---|---|---|
| `examples/bench_loop.gg` — sum 1..1,000,000 | ~440 ms | ~240 ms | VM ~1.8× **faster** |
| `examples/bench_fib.gg` — `fib(26)` | ~355 ms | ~295 ms | VM ~1.2× **faster** |

The loop is faster now, and the reason is concrete: slot-based
locals turned the loop body's variable accesses from `HashMap`
lookups on the scope chain into array indexes (before slots
the same loop took ~925 ms on the VM vs ~465 ms on the
tree-walker). `fib` was call-bound, not variable-bound, and
closed the same way — with cheaper calls, not faster
variables: a positional-only call to a function with no
defaults and no rest parameter skips the binding table
entirely and reuses the argument list's allocation as the
frame's locals, the cell array is only created when a local
is actually captured, the caller's argument list is taken
instead of cloned, and a plain function call goes straight
to the VM's call path instead of through the backend
dispatch. That took `fib(26)` from ~410 ms to ~295 ms on
the VM, with the loop untouched (~240 ms) and all 383 tests
still green on both engines.

### Limits and known deviations

- No bytecode cache on disk (compile on every run), no JIT.
- Garbage collection is unchanged: values are `Rc`, and cycles
  still leak.
- One known limitation of the single-pass compiler: a closure
  that references a variable declared *later* in its enclosing
  scope resolves it as a global (runtime "undefined variable"),
  while the tree-walker sees the later declaration. CPython
  avoids this with a symtable pass; GG has none yet.
- One known deviation from the tree-walker: in
  `super.foo(...)` the VM evaluates the arguments *before*
  the `super` checks, while the tree-walker checks `super`
  first. Both engines error on an invalid `super`; the only
  observable difference is side effects in the arguments
  combined with an invalid `super`.

## Grammar

```text
program   := statement* EOF
block     := statement* ("end" | "else")
statement := "let" ident "=" expr
           | ident "=" expr
           | index "=" expr
           | "print" expr
           | "if" expr "then" block ["else" block] "end"
           | "while" expr "then" block "end"
           | "for" ident "=" expr "to" expr "then" block "end"
           | "fn" ident "(" [param ("," param)*] ")" block "end"
           | "class" ident ["extends" ident] class_body "end"
           | "import" ident ["as" ident]
           | "try" "then" block ["except" [ident] "then" block] ["finally" "then" block] "end"
           | "raise" expr
           | "match" expr "then" (case)* "end"
           | "yield" [expr]
           | "return" [expr]
           | expr
case      := "case" pattern ["if" expr] "then" block
pattern   := number | string | "true" | "false" | "-" number
           | "_" | ident
           | "[" [pattern ("," pattern)*] "]"
           | "(" [pattern ("," pattern)*] ")" ; one comma or more → tuple, else grouping
class_body:= ("fn" ident "(" [param ("," param)*] ")" block "end")*
           ; a method's first parameter must be "self"
param     := ident ["=" expr] | "*" ident
           ; a parameter with a default, or the rest parameter —
           ; at most one rest parameter, and it must be last
arg       := "*" expr | ident "=" expr | expr
           ; a spread, a keyword argument, or a positional
expr      := or
or        := and ("or" and)*
and       := equality ("and" equality)*
equality  := comparison (("==" | "!=") comparison)*
comparison:= term (("<" | "<=" | ">" | ">=") term)*
term      := factor (("+" | "-") factor)*
factor    := unary (("*" | "/" | "%") unary)*
unary     := ("-" | "not") unary | postfix
postfix   := primary ("[" index_item ("," index_item)* "]" | "." ident ["(" [arg ("," arg)*] ")"] | "(" [arg ("," arg)*] ")")*
           ; the last alternative calls the value the primary
           ; produced: `fn(x) x end(2)`, `f()(3)`
index_item:= expr | [expr] ":" [expr] [":" [expr]] ; a slice — each bound optional
primary   := number | string | "true" | "false" | "self"
           | "f" string   ; an f-string: `f"ciao {nome}"`, `{expr}`
           ; interpolates any expression, `{{`/`}}` literal braces
           | "super" "." ident ["(" [arg ("," arg)*] ")"]
           | "[" [expr ("," expr)*] "]"
           | "[" expr "for" ident ["," ident] "in" expr ["if" expr] "]"
           ; a list comprehension — one `for` clause only
           | "(" [expr ("," expr)*] ")"   ; one comma or more → tuple, else grouping
           | "{" [dict | set] "}"         ; "k: v" entries → dict, plain values → set, "{}" → empty dict
           | "{" expr "for" ident ["," ident] "in" expr ["if" expr] "}"
           ; a set comprehension — one `for` clause only
           | "{" expr ":" expr "for" ident ["," ident] "in" expr ["if" expr] "}"
           ; a dict comprehension — one `for` clause only
           | "fn" "(" [param ("," param)*] ")" expr "end"
           ; a lambda — the body is a single expression
           | ident ["(" [arg ("," arg)*] ")"]
dict      := expr ":" expr ("," expr ":" expr)* [","]
set       := expr ("," expr)* [","]
index     := postfix "[" index_item ("," index_item)* "]"
field     := postfix "." ident
           ; ".5" lexes as Dot + Int(5) and the parser rejects it;
           ; "5." is still a lex error (malformed number)
```

Statements are separated by newlines and/or `;`. Assignment (`x = ...`)
is a statement, not an expression. `to` was added as a keyword in
Phase 2 for the `for` loop (`total` is still a plain identifier).
`class`, `extends`, `super` and `self` were added as keywords in
Phase 8 (`selfish` and `classy` are still plain identifiers); `.`
is the field-access operator since Phase 8, so `5.` is a lex error
and `.5` is rejected by the parser instead. `import` and `as`
were added as keywords in Phase 9 (`important` is still a plain
identifier); a module name is a single identifier — there are no
dotted module paths (`import sub.mod` is a parse error). `try`,
`except`, `finally` and `raise` were added as keywords in Phase 19
(`tryout` and `excepted` are still plain identifiers). `match`,
`case` and `yield` were added as keywords in Phase 23
(`matching` and `yielding` are still plain identifiers).

```gg
fn sum_to(n)
    let total = 0
    for i = 1 to n then
        total = total + i
    end
    return total
end

print sum_to(10)
```

## Tokens

| Kind | Examples |
| --- | --- |
| Keywords | `let if then else end while for fn return print true false and or not to class extends super self import as try except finally raise match case yield` |
| Identifiers | `[A-Za-z_][A-Za-z0-9_]*` (keywords are reserved) |
| Numbers | integers `42`, floats `3.14` (`.5` and `5.` are errors) |
| Strings | `"..."` with escapes `\"` `\\` `\n` `\t`; f-strings `f"..."` with `{expr}` interpolation, `{{`/`}}` literal braces |
| Operators | `+ - * / % == != < <= > >= =` (`!` alone is not an operator) |
| Punctuation | `( ) , [ ] { } : .` (braces are dict/set literals, not blocks; `.` is field access) |
| Separators | newline, `;` |
| Comments | `#` to end of line |

Every token carries its 1-based line and column. Lexing errors are
plain-English messages with line and column
(e.g. `line 2, col 3: unterminated string`).

## Usage

```sh
cargo build --release
./target/release/gg                 # start the interactive REPL
./target/release/gg repl            # same thing
./target/release/gg run hello.gg          # run a .gg program
./target/release/gg run --venv myproj hello.gg   # run with a venv's lib/ on the module path
./target/release/gg venv myproj            # create a virtual environment (myproj/ + lib/ + gg.toml)
./target/release/gg install owner/repo   # install a package from GitHub (or ./dir for a local one)
./target/release/gg list                 # list installed packages
./target/release/gg remove ciao          # remove an installed package
./target/release/gg --tokens hello.gg   # print the tokens, one per line
./target/release/gg --ast hello.gg      # parse and print the AST (canonical form)
./target/release/gg --help
./target/release/gg --version
```

Token output format: `<line>:<col> TYPE value`, e.g. `1:1 Let let`.
Runtime errors print as `runtime error: <message>` on stderr and
exit with code 1.

## Roadmap

1. **Lexer** ✅ — `gg-lexer`
2. **Parser + AST** ✅ — `gg-parser`
3. **Interpreter** ✅ — `gg-interp` (variables, `print`, `if`/`while`/`for`, operators, functions, closures)
4. **Builtins + standard library** ✅ — builtin functions (`len`, `type`, `str`, `int`, `float`, `input`, `abs`, `min`, `max`)
5. **REPL** ✅ — interactive shell (`gg` / `gg repl`): persistent state, multi-line blocks, expression echo, error recovery
6. **Lists + file handling** ✅ — lists (`[1, 2, 3]`, indexing, aliasing, `push`/`pop`/`range`/`split`/`lines`) and whole-file helpers (`read_file`/`write_file`/`append_file`/`file_exists`)
7. **Collections** ✅ — tuples `(1, "a")`, dicts `{"k": v}`, sets `{1, 2}`, arrays `array([1, 2])`, queues `queue()`/`enqueue`/`dequeue`, stacks `stack()`/`push`/`pop`, plus `peek`/`keys`/`contains`/`list`
8. **OOP** ✅ — classes, methods, inheritance (see below)
9. **Modules + virtual environments** ✅ — `import` (with `as`
   alias), module values (`type` is `"module"`), search order
   (script dir → venv `lib/` → working dir → installed
   packages), once-per-process
   cache, circular-import detection, nested imports relative to
   the importing module, `gg venv`, `gg run --venv <dir>`,
   `GG_VENV`, REPL imports
10. **Standard library** ✅ — new builtins (`sum`, `join`,
    `upper`, `lower`, `trim`, `replace`, `starts_with`,
    `ends_with`); native `math` and `random` (SplitMix64, pure
    Rust) modules; `stats` in pure GG, embedded in the binary;
    import resolution: user files → embedded stdlib → native
    modules
11. **N-dimensional numeric arrays** ✅ — flat row-major
    storage + shape; constructors (`zeros`, `ones`, `arange`,
    `linspace`), shape helpers (`shape`, `ndim`, `size`),
    multi-indexing `a[i, j]`, element-wise arithmetic
    (array/array, array/scalar both sides), `dot`, `matmul`,
    `transpose`, `reshape`, `sum`/`min`/`max` over whole arrays
12. **Slicing and boolean indexing** ✅ — slices
    `start:stop:step` on lists, strings, tuples and arrays
    (Python rules, always a copy); element-wise comparisons
    returning Bool arrays; boolean indexing `a[mask]`;
    `where(cond, a, b)`; slice assignment on lists and 1D arrays
    (same length required)
13. **Graphical interfaces** ✅ — native `gui` module
    (egui 0.37 + eframe 0.37, `gui` Cargo feature, on by
    default): `gui.run(titolo, fn)` opens a window and calls
    the GG frame function every frame until it returns `false`;
    widgets `heading`/`label`/`separator`/`button`/`text_input`/
    `slider`/`checkbox`/`horizontal`, called inside the frame
    function in order, vertical layout by default
17. **More GUI widgets** ✅ — `radio(id, testo, selezionato)`
    → Bool (true only when clicked; exclusive groups are the
    script's own logic, no implicit groups),
    `combo(id, opzioni, selezionato)` → Int (dropdown over a
    List of Str; empty list or out-of-range index is an error),
    `progress(valore)` → Nil (bar, value clamped to [0, 1]),
    `vertical(fn)` (like `horizontal`, vertical layout); see
    `examples/gui_widgets_demo.gg`
14. **Linear algebra** ✅ — native `linalg` module
    (`import linalg`): `solve(A, b)` / `det(A)` / `inv(A)` over
    2D square numeric arrays, `rank(A)` over any 2D numeric
    array (square or rectangular); Gaussian elimination with
    partial pivoting; a singular matrix is a runtime error for
    `solve` and `inv`, `det` of one is `0`; `rank` tells an
    indeterminate system from an impossible one
    (`rank(A) == rank([A|b]) < n` vs `rank(A) < rank([A|b])`);
    results are Float, with the usual f64 rounding (see the
    `linalg` table above)
15. **For-in iteration** ✅ — `for x in xs then ... end` over
    lists, tuples, arrays (rows on 2D+), sets, dict keys, string
    characters, queues and stacks (see the Loops section below)
16. **Runtime errors with line and column** ✅ — every runtime
    error reports `runtime error: <line>:<col>: <message>`
    (1-based), pointing at the failing token (identifier,
    operator, index, call); see `examples/error_demo.gg`
18. **`linalg.rank`** ✅ — `rank(A)` → Int: the rank of a 2D
    numeric matrix, square or rectangular, by Gaussian
    elimination to row echelon form with partial pivoting
    (same 1e-12 tolerance as `solve`); an empty matrix has
    rank 0. With it, an indeterminate system is told from an
    impossible one: `rank(A) == rank([A|b]) < n` vs
    `rank(A) < rank([A|b])`; see `examples/rank_demo.gg`
19. **Error handling** ✅ — `try then ... except e then ...
    finally then ... end` catches runtime errors (the bound
    variable holds the error's message as a Str; `except`
    without a binding ignores it; at least one of `except`
    and `finally` is required), `raise "message"` throws one
    (Str only, pointing at the `raise`), and `finally` runs on
    every exit — normal, error, and `return`; errors unwind
    through function calls, while lex/parse errors cannot be
    caught; see `examples/try_demo.gg`
20. **Comprehensions and lambdas** ✅ — list `[x * 2 for x in
    xs]`, dict `{k: v for k, v in pairs}` and set `{x for x in
    xs}` comprehensions (one `for` clause only, optional `if`
    filter; loop variable scoped to the comprehension; two
    variables unpack pairs; duplicate dict keys keep the last
    value) and lambdas `fn(x) x * 2 end` (single-expression
    body, closures, first-class values, callable on the spot);
    see `examples/comprehension_demo.gg`
21. **Default arguments, keyword arguments and `*args`** ✅ —
    parameters with defaults (`fn saluta(nome, saluto =
    "ciao")`, evaluated once at definition time), keyword
    arguments (`saluta(nome = "x")`, any order, after the
    positionals), a rest parameter (`fn somma(*numeri)`,
    extras as a List) and spread at the call (`f(*xs)`, over
    lists, tuples and arrays); same mechanism for named
    functions, lambdas and methods; builtins stay positional,
    no `**kwargs` yet; see `examples/args_demo.gg`
22. **F-strings** ✅ — `f"ciao {nome}"` interpolates any
    expression inside `{...}` (variables, arithmetic, calls,
    indexing, comprehensions, lambdas), rendered with the same
    Display as `print`; `{{`/`}}` are literal braces; errors
    point at the spot and are catchable by `try`/`except`;
    f-strings nest; no format specs, no `!r`/`!s`, no `{x=}`;
    see `examples/fstring_demo.gg`
23. **`match` and generators** ✅ — `match x then case 1 then
    ... case [a, b] then ... case n if n > 10 then ... case _
    then ... end` (first matching case wins; literals, `_`,
    bindings, list/tuple patterns, nested; optional `if` guard;
    no match when nothing matches; statement only, no match
    expression yet) and generators: a function with `yield` in
    its body returns a `<generator>` on call, `next(gen)` replays
    the body up to the next `yield`, `"generator exhausted"`
    when it ends (catchable with `try`/`except`), `for x in gen`
    pulls lazily; replay limits (O(n²) total, side effects
    repeat) documented in the Semantics section; see
    `examples/match_demo.gg` and `examples/generators_demo.gg`
24. **REPL history** ✅ — on a terminal the REPL runs through
    `rustyline` 19.0.0: up/down arrows walk the history, which
    persists in `~/.gg_history` across sessions, and a multi-line
    block is a single history entry; Ctrl-C drops the line and
    the block in progress, Ctrl-D on an empty line leaves;
    piped stdin keeps the plain stream loop (no history file
    touched). This closes the "migliora tutto" series (Fasi
    19–24)
25. **Bytecode VM** ✅ — bytecode compiler (AST → `Op`) +
    stack machine in `gg-interp` (`crates/gg-interp/src/vm/`),
    side by side with the tree-walker behind the same API;
    VM is the default, `GG_BACKEND=tree` selects the fallback;
    per-function code objects, spans on every instruction,
    try/except/finally with handler stack, generators by
    replay, match compiled to jumps; a dual-backend corpus
    (20 tests) runs every program on both engines and requires
    identical output; all 14 non-GUI examples + both GUI demos
    (Xvfb) verified identical; honest benchmark: VM ~1.8×
    faster on a 1M-iteration loop and ~1.2× faster on
    `fib(26)` (cheaper calls: no binding table for
    positional-only calls, argument list reused as locals,
    lazy cell array); slot-based locals with transitive
    upvalues, see the Bytecode virtual machine section
26. **JSON module** ✅ — native `json` module (`import json`):
    `json.parse(text)` / `json.stringify(value)` /
    `json.stringify(value, indent)`; hand-written recursive
    parser in pure Rust, no external crate; objects → Dicts
    (insertion order, last duplicate wins), integers that fit
    in i64 → Int, floats round-trip (`1.0` stays `1.0`); parse
    errors carry the position inside the JSON text; nesting
    past 128 levels is an error (also stops `stringify` on
    cyclic values); see `examples/json_demo.gg`
27. **OS module** ✅ — native `os` module (`import os`):
    `os.args()` / `os.getenv(name)` / `os.getenv(name, default)`
    / `os.cwd()` / `os.listdir(path)` (names only, sorted) /
    `os.mkdir(path)` / `os.remove(path)` (a file, never a
    directory); pure Rust, no external crate; no command
    execution and no environment writes, by design; see
    `examples/os_demo.gg`
28. **Datetime module** ✅ — native `datetime` module
    (`import datetime`): `datetime.now()` /
    `datetime.from_timestamp(ts)` /
    `datetime.to_timestamp(dict)` /
    `datetime.format(ts_or_dict, fmt)` (a `strftime` subset:
    `%Y %m %d %H %M %S %w`) / `datetime.date(year, month,
    day)`; everything is UTC, hand-written calendar math in
    pure Rust, no external crate; the Dict carries `year`,
    `month`, `day`, `hour`, `minute`, `second` and `weekday`
    (0 = Monday .. 6 = Sunday); invalid dates (month 13, 29
    February in a non-leap year, hour 25) are runtime errors;
    see `examples/datetime_demo.gg`
29. **Regex module** ✅ — native `regex` module
    (`import regex`): `regex.test(pattern, text)` /
    `regex.find(pattern, text)` / `regex.groups(pattern, text)`
    (whole match at index 0, then the capture groups, Nil for
    a non-participating optional group, `[]` when no match) /
    `regex.replace(pattern, text, replacement)` (replacement
    supports `$1`, `$nome`, `${nome}` and `$$`) /
    `regex.split(pattern, text)`; via the `regex` crate
    (1.13.2) from crates.io — the first stdlib module with an
    external dependency; patterns match on Unicode text, and
    an invalid pattern is a runtime error with the crate's
    message; see `examples/regex_demo.gg`
30. **HTTP module** ✅ — native `http` module
    (`import http`): a blocking HTTP client
    (`http.get(url)` / `http.get(url, headers)`,
    `http.post(url, body)` / `http.post(url, body,
    content_type)`, `http.request(method, url, body, headers)`);
    every call returns a Dict with `status`, `headers`
    (lowercased keys, a repeated header keeps the last value)
    and `body` (lossy UTF-8); HTTP error statuses come back in
    the Dict, only network failures are runtime errors (30s
    timeout); via the `ureq` crate (3.5.2) from crates.io, TLS
    verified against the platform trust store; see
    `examples/http_demo.gg`
31. **SQLite module** ✅ — native `sqlite` module
    (`import sqlite`): an embedded database
    (`sqlite.open(path)` / `sqlite.exec(db, sql)` /
    `sqlite.execute(db, sql, params)` returning rows changed /
    `sqlite.query(db, sql)` / `sqlite.query(db, sql, params)`
    returning a List of Dicts keyed by column name /
    `sqlite.close(db)`); results map INTEGER → Int, REAL →
    Float, TEXT → Str, NULL → Nil, BLOB → List of Int (bytes);
    every SQLite failure is a runtime error with SQLite's
    message, catchable with `try`/`except`; via the `rusqlite`
    crate (0.32) from crates.io with the `bundled` feature, so
    no system SQLite library is needed; see
    `examples/sqlite_demo.gg`
32. **Package manager** ✅ — `gg install` / `gg list` /
    `gg remove` and a `gg.lock` lockfile: packages are
    `gg.toml` manifests (`name`, `version`, `description`,
    `entry`, `dependencies`, parsed with the `toml` crate
    0.8) installed from a local directory or from GitHub
    (`owner/repo[@tag|@commit]`, cloned with `git`) into
    `~/.gg/packages/<name>/<version>/` (or `gg_packages/`
    with `--local`); `import <name>` finds them after files,
    the embedded stdlib and native modules, with the lockfile
    pinning the version (else the newest installed version);
    dependencies install recursively, cycles and version
    conflicts are clear errors (no resolver in v1), `gg
    install` with no spec reinstalls from the lockfile, and
    installing only copies code — it never runs anything; see
    the Package manager section, `examples/package_demo.gg`
    and `examples/demo-package/`

Next: the GUI's first real interactive test on the laptop, still open.

Long term: more data libraries in the spirit of Pandas, and a
web framework in the spirit of Django — each is a large project
of its own, not a single phase.

## License

GPLv3 — see [LICENSE](LICENSE). All GG code is free software.
