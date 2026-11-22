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
`linalg` module (`solve`/`det`/`inv`, Gaussian elimination
with partial pivoting; a singular matrix is a runtime error,
never a silent `nil`) — see the Standard library section
below. **Phase 15 — for-in iteration: done.** `for x in xs
then ... end` iterates over lists, tuples, arrays (rows on
2D+), sets, dict keys, string characters, queues and stacks;
the iterable is evaluated once and snapshotted, and the loop
variable is scoped to the body (see the Loops section below).

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
an error, not a continuation). The REPL reads plain lines from stdin:
no arrow-key history and no line editing yet (that needs a line-editing
library like `rustyline`, a future upgrade).

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
  does not consume them). The iterable is evaluated once before
  the loop starts, so mutations inside the body do not change what
  the loop visits. Anything else is a runtime error (`Int is not
  iterable`). The loop variable exists only inside the loop body,
  like the numeric `for`.
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

- **Errors** are plain-English runtime errors (`undefined variable
  'x'`, `cannot add Int and Str`, `division by zero`,
  `cannot call Int`, `expected 1 argument(s) but got 2`,
  `return outside function`). **Known limitation:** the Phase 2 AST
  carries no source positions, so runtime errors have no line or
  column yet — positions will arrive in a later phase.

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

Forty-seven builtin functions are predefined in the global environment of
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

**Search order.** `import nome` looks for the module in three
levels, in this order: (1) a `.gg` file — the directory of the
file doing the import, then the `lib/` directories of the active
venvs, in flag order, then the current working directory; (2) an
embedded stdlib module written in GG (`stats`); (3) a native
module written in Rust (`math`, `random`). The first match wins:
a user file with the same name overrides a stdlib or a native
module (`math.gg` next to your script shadows the native
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

## Standard library

Phase 10 adds a minimal standard library in three layers, in
the import resolution order above: user `.gg` files first, then
embedded GG modules, then native Rust modules. Specialized
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

**`linalg`** — native, in Rust (Phase 14). Gaussian elimination
with partial pivoting over the N-d numeric arrays: `solve` and
`inv` return Float arrays, `det` a Float. A singular matrix —
determinant zero, i.e. an impossible or indeterminate system —
is a runtime error for `solve` and `inv` (the GG convention:
`matmul` on incompatible shapes is an error too, never a
silent `nil`); `det` of a singular matrix is `0`. Results are
f64, so a system like the classic 3x3 can print
`3.0000000000000004` instead of `3`: elimination divides by
numbers with no exact binary form, and a little rounding is
normal — the same happens in NumPy.

| Function | Description | Example |
| --- | --- | --- |
| `linalg.solve(A, b)` | The solution `x` of `A x = b`: `A` square 2D, `b` a 1D vector of the same length | `linalg.solve(reshape(array([2, 1, 1, 3]), [2, 2]), array([5, 10]))` → `array([1, 3])` |
| `linalg.det(A)` | The determinant of a square 2D matrix | `linalg.det(reshape(array([1, 2, 3, 4]), [2, 2]))` → `-2` |
| `linalg.inv(A)` | The inverse matrix | `linalg.inv(reshape(array([1, 2, 0, 1]), [2, 2]))` → `array([[1, -2], [0, 1]])` |

Full example: `examples/linalg_demo.gg` — the systems from the
maths ripasso, plus the classic 3x3 with its f64 rounding.

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
| `gui.horizontal(fn)` | Run the GG function `fn` laying out the widgets it creates horizontally instead of vertically |

`text_input`, `slider` and `checkbox` take a stable string `id`:
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
only, no menus, no canvas/images, no themes, no multiple
windows, and no frame-rate control yet.

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
           | "fn" ident "(" [ident ("," ident)*] ")" block "end"
           | "class" ident ["extends" ident] class_body "end"
           | "import" ident ["as" ident]
           | "return" [expr]
           | expr
class_body:= ("fn" ident "(" [ident ("," ident)*] ")" block "end")*
           ; a method's first parameter must be "self"
expr      := or
or        := and ("or" and)*
and       := equality ("and" equality)*
equality  := comparison (("==" | "!=") comparison)*
comparison:= term (("<" | "<=" | ">" | ">=") term)*
term      := factor (("+" | "-") factor)*
factor    := unary (("*" | "/" | "%") unary)*
unary     := ("-" | "not") unary | postfix
postfix   := primary ("[" index_item ("," index_item)* "]" | "." ident ["(" [expr ("," expr)*] ")"])*
index_item:= expr | [expr] ":" [expr] [":" [expr]] ; a slice — each bound optional
primary   := number | string | "true" | "false" | "self"
           | "super" "." ident ["(" [expr ("," expr)*] ")"]
           | "[" [expr ("," expr)*] "]"
           | "(" [expr ("," expr)*] ")"   ; one comma or more → tuple, else grouping
           | "{" [dict | set] "}"         ; "k: v" entries → dict, plain values → set, "{}" → empty dict
           | ident ["(" [expr ("," expr)*] ")"]
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
dotted module paths (`import sub.mod` is a parse error).

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
| Keywords | `let if then else end while for fn return print true false and or not to class extends super self import as` |
| Identifiers | `[A-Za-z_][A-Za-z0-9_]*` (keywords are reserved) |
| Numbers | integers `42`, floats `3.14` (`.5` and `5.` are errors) |
| Strings | `"..."` with escapes `\"` `\\` `\n` `\t` |
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
   (script dir → venv `lib/` → working dir), once-per-process
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
14. **Linear algebra** ✅ — native `linalg` module
    (`import linalg`): `solve(A, b)` / `det(A)` / `inv(A)` over
    2D square numeric arrays, Gaussian elimination with partial
    pivoting; a singular matrix is a runtime error for `solve`
    and `inv`, `det` of one is `0`; results are Float, with the
    usual f64 rounding (see the `linalg` table above)

Next: not decided yet — Fase 14 is done; the GUI's first real
interactive test on the laptop is still open.

Long term: more data libraries in the spirit of Pandas, and a
web framework in the spirit of Django — each is a large project
of its own, not a single phase.

## License

GPLv3 — see [LICENSE](LICENSE). All GG code is free software.
