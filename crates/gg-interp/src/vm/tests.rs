// SPDX-License-Identifier: GPL-3.0-or-later
//! Tests for the bytecode VM.
//!
//! The dual-backend corpus: every program runs on the VM and on
//! the tree-walker, and the two must agree exactly — same stdout
//! on success, same error text on failure. The corpus covers the
//! whole language, so a semantic drift in either engine shows up
//! here. Plus compiler unit tests and a `Backend` selection test.

use std::io::Cursor;

use crate::interp::Interpreter;
use crate::vm::Backend;
use crate::vm::chunk::{Constant, Op};
use crate::vm::compiler::compile_program;

/// Run `source` under one backend with `input` on stdin.
/// Returns the printed output, or the error text.
fn run_one(
    source: &str,
    input: &str,
    backend: Backend,
    base_dir: Option<std::path::PathBuf>,
) -> Result<String, String> {
    let tokens = gg_lexer::lex(source).map_err(|e| e.to_string())?;
    let program = gg_parser::parse(&tokens).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.set_backend(backend);
    if let Some(dir) = base_dir {
        interp.set_base_dir(Some(dir));
    }
    let mut input = Cursor::new(input.as_bytes().to_vec());
    let mut out = Vec::new();
    match interp.run(&program, &mut input, &mut out) {
        Ok(()) => Ok(String::from_utf8(out).unwrap()),
        // The same "runtime error: <line>:<col>: <msg>" text the
        // public `run_source` produces via `GgError`.
        Err(e) => Err(format!("runtime error: {e}")),
    }
}

/// The VM and the tree-walker must produce exactly the same
/// result on `source` (same stdout, or same error text).
fn assert_same(source: &str) {
    assert_same_in(source, "");
}

/// Like `assert_same`, with `input` fed to stdin.
fn assert_same_in(source: &str, input: &str) {
    let vm = run_one(source, input, Backend::Vm, None);
    let tree = run_one(source, input, Backend::Tree, None);
    assert_eq!(vm, tree, "VM and tree-walker disagree on:\n{source}");
}

/// The VM and the tree-walker must both print `expected`.
fn assert_same_out(source: &str, expected: &str) {
    assert_eq!(
        run_one(source, "", Backend::Vm, None),
        Ok(expected.to_string()),
        "VM output differs on:\n{source}"
    );
    assert_eq!(
        run_one(source, "", Backend::Tree, None),
        Ok(expected.to_string()),
        "tree-walker output differs on:\n{source}"
    );
}

/// The VM and the tree-walker must both fail with `expected`.
fn assert_same_err(source: &str, expected: &str) {
    assert_eq!(
        run_one(source, "", Backend::Vm, None),
        Err(expected.to_string()),
        "VM error differs on:\n{source}"
    );
    assert_eq!(
        run_one(source, "", Backend::Tree, None),
        Err(expected.to_string()),
        "tree-walker error differs on:\n{source}"
    );
}

/// A fresh temporary directory for one module test.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gg-vm-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_module(dir: &std::path::Path, name: &str, source: &str) {
    std::fs::write(dir.join(format!("{name}.gg")), source).unwrap();
}

#[test]
fn backend_from_env_value() {
    assert_eq!(Backend::from_env_value(None), Backend::Vm);
    assert_eq!(Backend::from_env_value(Some("vm")), Backend::Vm);
    assert_eq!(Backend::from_env_value(Some("tree")), Backend::Tree);
    // Anything else falls back to the VM.
    assert_eq!(Backend::from_env_value(Some("bogus")), Backend::Vm);
    assert_eq!(Backend::default(), Backend::Vm);
}

#[test]
fn compiler_emits_expected_opcodes() {
    let tokens = gg_lexer::lex("print 1 + 2\n").unwrap();
    let program = gg_parser::parse(&tokens).unwrap();
    let code = compile_program(&program, true);
    let ops: Vec<&Op> = code.instructions.iter().map(|i| &i.op).collect();
    assert_eq!(ops.len(), 4);
    assert!(matches!(ops[0], Op::LoadConst { .. }));
    assert!(matches!(ops[1], Op::LoadConst { .. }));
    assert!(matches!(ops[2], Op::Binary { .. }));
    assert!(matches!(ops[3], Op::Print));
    assert_eq!(code.constants.len(), 2);
    assert!(code.constants.contains(&Constant::Int(1)));
    assert!(code.constants.contains(&Constant::Int(2)));
    // Every instruction carries a real source span.
    assert!(code.instructions.iter().all(|i| i.span.line >= 1));
}

#[test]
fn loops() {
    assert_same_out(
        "for i = 1 to 3 then\n    print i\nend\nfor i = 3 to 1 then\n    print i\nend\nfor i = 5 to 5 then\n    print i\nend\n",
        "1\n2\n3\n3\n2\n1\n5\n",
    );
    assert_same_out(
        "let i = 0\nlet s = 0\nwhile i < 5 then\n    i = i + 1\n    s = s + i\nend\nprint s\n",
        "15\n",
    );
    assert_same_out(
        "let t = 0\nfor i = 1 to 3 then\n    for j = 1 to 3 then\n        t = t + i * j\n    end\nend\nprint t\n",
        "36\n",
    );
    assert_same_out(
        "let xs = [10, 20, 30]\nfor x in xs then\n    print x\nend\nlet voti = {\"a\": 1, \"b\": 2}\nfor k in voti then\n    print k\nend\nfor c in \"GG\" then\n    print c\nend\nfor i in range(3) then\n    print i\nend\n",
        "10\n20\n30\na\nb\nG\nG\n0\n1\n2\n",
    );
    // A `for` loop variable does not leak out of the loop.
    assert_same_err(
        "for i = 1 to 3 then\n    print i\nend\nprint i\n",
        "runtime error: 4:7: undefined variable 'i'",
    );
}

#[test]
fn closures_and_mutation() {
    assert_same_out(
        "fn contatore()\n    let n = 0\n    fn inc()\n        n = n + 1\n        return n\n    end\n    return inc\nend\nlet c = contatore()\nprint c()\nprint c()\nprint c()\n",
        "1\n2\n3\n",
    );
    // Two closures share the same captured binding.
    assert_same_out(
        "fn coppia()\n    let n = 0\n    fn inc()\n        n = n + 1\n    end\n    fn get()\n        return n\n    end\n    return [inc, get]\nend\nlet p = coppia()\np[0]()\np[0]()\nprint p[1]()\n",
        "2\n",
    );
    // Shadowing: the inner binding does not touch the outer one.
    assert_same_out(
        "let x = 1\nif true then\n    let x = 2\n    print x\nend\nprint x\n",
        "2\n1\n",
    );
    // A closure sees a later rebinding of a captured variable.
    assert_same_out("let x = 1\nlet f = fn() x end\nx = 2\nprint f()\n", "2\n");
}

#[test]
fn slot_locals_and_direct_upvalues() {
    // A closure over a numeric `for` variable sees the last
    // value the body ran with: one shared binding, and the
    // next value is only defined when the loop continues.
    assert_same_out(
        "let f0 = fn() 0 end\nlet f1 = fn() 0 end\nlet f2 = fn() 0 end\nfor i = 0 to 2 then\n    if i == 0 then f0 = fn() i end end\n    if i == 1 then f1 = fn() i end end\n    if i == 2 then f2 = fn() i end end\nend\nprint f0()\nprint f1()\nprint f2()\n",
        "2\n2\n2\n",
    );
    // Same for `for x in ...`: one shared binding, last item.
    assert_same_out(
        "let g0 = fn() 0 end\nlet g1 = fn() 0 end\nlet g2 = fn() 0 end\nlet k = 0\nfor x in [10, 20, 30] then\n    if k == 0 then g0 = fn() x end end\n    if k == 1 then g1 = fn() x end end\n    if k == 2 then g2 = fn() x end end\n    k = k + 1\nend\nprint g0()\nprint g1()\nprint g2()\n",
        "30\n30\n30\n",
    );
    // A comprehension variable is fresh per item: each closure
    // keeps its own value.
    assert_same_out(
        "let fs = [fn() x end for x in [1, 2, 3]]\nprint fs[0]()\nprint fs[1]()\nprint fs[2]()\n",
        "1\n2\n3\n",
    );
    // A closure over the `except` variable keeps the message.
    assert_same_out(
        "fn f()\n    try then\n        raise \"boom\"\n    except e then\n        return fn() e end\n    end\nend\nprint f()()\n",
        "boom\n",
    );
    // A closure over a `match` binding sees the bound value.
    assert_same_out(
        "match [1, 2] then\ncase [a, b] then\n    print (fn() a + b end)()\nend\n",
        "3\n",
    );
    // A grandchild reads a variable its parent already captured:
    // the same cell, shared through the parent's upvalue.
    assert_same_out(
        "fn outer()\n    let n = 1\n    fn mid()\n        print n\n        fn inner()\n            return n\n        end\n        return inner\n    end\n    return mid()\nend\nprint outer()()\n",
        "1\n1\n",
    );
}

#[test]
fn transitive_upvalues() {
    // A grandchild reads a variable of its grandparent, which
    // the parent never references: the parent gains an upvalue
    // for it, and the grandchild reads the same cell.
    assert_same_out(
        "fn outer()\n    let n = 1\n    fn mid()\n        let m = fn() n end\n        return m\n    end\n    return mid()\nend\nprint outer()()\n",
        "1\n",
    );
    // Three levels deep.
    assert_same_out(
        "fn a()\n    let x = 7\n    fn b()\n        fn c()\n            fn d()\n                return x\n            end\n            return d\n        end\n        return c\n    end\n    return b\nend\nprint a()()()()\n",
        "7\n",
    );
    // A grandchild writes the grandparent's variable: the
    // grandparent reads the new value through the shared cell.
    assert_same_out(
        "fn outer()\n    let n = 1\n    fn mid()\n        fn bump()\n            n = n + 10\n        end\n        return bump\n    end\n    let b = mid()\n    b()\n    print n\nend\nouter()\n",
        "11\n",
    );
    // A closure writes, the defining frame reads (direct).
    assert_same_out(
        "fn outer()\n    let n = 1\n    fn inc()\n        n = n + 1\n    end\n    inc()\n    inc()\n    return n\nend\nprint outer()\n",
        "3\n",
    );
    // A default expression reads a grandparent variable.
    assert_same_out(
        "fn outer()\n    let base = 10\n    fn mid()\n        fn f(x = base)\n            return x\n        end\n        return f\n    end\n    return mid()\nend\nprint outer()()\n",
        "10\n",
    );
    // Two grandchildren share one grandparent binding: one
    // writes, the other reads.
    assert_same_out(
        "fn outer()\n    let n = 0\n    fn a()\n        fn inc()\n            n = n + 1\n        end\n        return inc\n    end\n    fn b()\n        fn get()\n            return n\n        end\n        return get\n    end\n    let inc = a()\n    let get = b()\n    inc()\n    inc()\n    print get()\nend\nouter()\n",
        "2\n",
    );
}

#[test]
fn recursion() {
    assert_same_out(
        "fn fib(n)\n    if n < 2 then\n        return n\n    end\n    return fib(n - 1) + fib(n - 2)\nend\nprint fib(10)\n",
        "55\n",
    );
    assert_same_out(
        "fn fatt(n)\n    if n <= 1 then\n        return 1\n    end\n    return n * fatt(n - 1)\nend\nprint fatt(6)\n",
        "720\n",
    );
}

#[test]
fn classes_inheritance_and_super() {
    assert_same_out(
        "class Persona\n    fn init(self, nome, eta)\n        self.nome = nome\n        self.eta = eta\n    end\n    fn saluta(self)\n        return \"Ciao, sono \" + self.nome\n    end\nend\nlet p = Persona(\"Giuliano\", 14)\nprint p.saluta()\nprint p.nome\nprint p.eta\nprint type(p)\nprint p\n",
        "Ciao, sono Giuliano\nGiuliano\n14\nPersona\nPersona{nome: \"Giuliano\", eta: 14}\n",
    );
    assert_same_out(
        "class A\n    fn chi(self)\n        return \"A\"\n    end\nend\nclass B extends A\n    fn chi(self)\n        return \"B-\" + super.chi()\n    end\nend\nclass C extends B\n    fn chi(self)\n        return \"C-\" + super.chi()\n    end\nend\nlet c = C()\nprint c.chi()\nprint is_a(c, A)\nprint is_a(c, B)\nprint is_a(c, C)\nprint is_a(A(), C)\n",
        "C-B-A\ntrue\ntrue\ntrue\nfalse\n",
    );
    assert_same_out(
        "class Base\n    fn init(self, x)\n        self.x = x\n    end\nend\nclass Figlio extends Base\n    fn init(self, x, y)\n        super.init(x)\n        self.y = y\n    end\nend\nlet f = Figlio(1, 2)\nprint f.x\nprint f.y\n",
        "1\n2\n",
    );
    // A bound method saved in a variable keeps its instance.
    assert_same_out(
        "class Persona\n    fn init(self, nome)\n        self.nome = nome\n    end\n    fn saluta(self)\n        return \"Ciao, sono \" + self.nome\n    end\nend\nlet p = Persona(\"Giuliano\")\nlet m = p.saluta\nprint m()\nprint type(m)\n",
        "Ciao, sono Giuliano\nfn\n",
    );
    // `super` in a nested function uses the enclosing method.
    assert_same_out(
        "class A\n    fn chi(self)\n        return \"A\"\n    end\nend\nclass B extends A\n    fn chi(self)\n        let f = fn() super.chi() end\n        return f()\n    end\nend\nprint B().chi()\n",
        "A\n",
    );
    assert_same_err(
        "print super.chi()\n",
        "runtime error: 1:13: 'super' outside a method",
    );
    // A method calling another method on the same instance.
    assert_same_out(
        "class Calcolatrice\n    fn somma(self, a, b)\n        return a + b\n    end\n    fn doppio(self, x)\n        return self.somma(x, x)\n    end\nend\nlet c = Calcolatrice()\nprint c.somma(2, 3)\nprint c.doppio(4)\n",
        "5\n8\n",
    );
    // A field holding a function is called plainly (no `self`).
    assert_same_out(
        "class C\n    fn init(self)\n        self.f = fn(x) x * 3 end\n    end\nend\nlet c = C()\nprint c.f(7)\n",
        "21\n",
    );
    assert_same_err(
        "class C\nend\nlet c = C()\nprint c.manca\n",
        "runtime error: 4:9: instance of 'C' has no field or method 'manca'",
    );
}

#[test]
fn generators() {
    assert_same_out(
        "fn conta(n)\n    let i = 1\n    while i <= n then\n        yield i * 10\n        i = i + 1\n    end\nend\nlet g = conta(3)\nprint type(g)\nprint g\nprint next(g)\nprint next(g)\nfor x in g then\n    print f\"for: {x}\"\nend\n",
        "generator\n<generator>\n10\n20\nfor: 30\n",
    );
    // An infinite generator: `next()` only ever runs up to the
    // N-th `yield`, so it never loops forever.
    assert_same_out(
        "fn inf()\n    let i = 0\n    while true then\n        yield i\n        i = i + 1\n    end\nend\nlet h = inf()\nprint next(h)\nprint next(h)\nprint next(h)\n",
        "0\n1\n2\n",
    );
    // Exhaustion is an error, catchable with try/except.
    assert_same_out(
        "fn uno()\n    yield 1\nend\nlet g = uno()\nprint next(g)\ntry then\n    print next(g)\nexcept e then\n    print \"finito: \" + e\nend\n",
        "1\nfinito: generator exhausted\n",
    );
    // A generator method with `yield`.
    assert_same_out(
        "class C\n    fn gen(self)\n        yield self.x\n        yield self.x + 1\n    end\n    fn init(self, x)\n        self.x = x\n    end\nend\nlet g = C(5).gen()\nprint next(g)\nprint next(g)\n",
        "5\n6\n",
    );
    // `finally` runs when the generator body ends.
    assert_same(
        "fn g()\n    try then\n        yield 1\n    finally then\n        print \"pulizia\"\n    end\nend\nlet gen = g()\nprint next(gen)\ntry then\n    print next(gen)\nexcept e then\n    print \"finito: \" + e\nend\n",
    );
    // Side effects in a generator repeat on every replay.
    assert_same_out(
        "fn g()\n    print \"corpo\"\n    yield 1\n    yield 2\nend\nlet gen = g()\nprint next(gen)\nprint next(gen)\n",
        "corpo\n1\ncorpo\n2\n",
    );
}

#[test]
fn match_expressions() {
    assert_same_out(
        "fn descrivi(x)\n    match x then\n    case 0 then\n        print \"zero\"\n    case 1 then\n        print \"uno\"\n    case n if n > 10 then\n        print f\"{n} e grande\"\n    case n then\n        print f\"altro: {n}\"\n    end\nend\ndescrivi(0)\ndescrivi(1)\ndescrivi(7)\ndescrivi(42)\n",
        "zero\nuno\naltro: 7\n42 e grande\n",
    );
    assert_same_out(
        "match [3, 4] then\ncase [a, b] then\n    print f\"somma: {a + b}\"\ncase _ then\n    print \"non una coppia\"\nend\nmatch [1, 2, 3] then\ncase [a, b] then\n    print \"coppia\"\ncase _ then\n    print \"non una coppia\"\nend\n",
        "somma: 7\nnon una coppia\n",
    );
    // No case matches: nothing happens, and the bound name from a
    // failed pattern does not leak.
    assert_same_out(
        "match 5 then\ncase 0 then\n    print \"zero\"\nend\nprint \"dopo\"\n",
        "dopo\n",
    );
    // A match on a string literal and a nested pattern.
    assert_same_out(
        "match (\"a\", [1, 2]) then\ncase (\"a\", [x, y]) then\n    print x + y\ncase _ then\n    print \"no\"\nend\n",
        "3\n",
    );
}

#[test]
fn try_except_finally_paths() {
    // Normal path: finally still runs.
    assert_same_out(
        "try then\n    print \"corpo\"\nfinally then\n    print \"pulizia\"\nend\nprint \"dopo\"\n",
        "corpo\npulizia\ndopo\n",
    );
    // Caught error: the handler gets the message, and the
    // except variable is scoped to the handler.
    assert_same_out(
        "try then\n    print 1 / 0\nexcept e then\n    print \"preso: \" + e\nend\nprint \"dopo\"\n",
        "preso: division by zero\ndopo\n",
    );
    assert_same_err(
        "try then\n    print 1 / 0\nexcept e then\n    print e\nend\nprint e\n",
        "runtime error: 6:7: undefined variable 'e'",
    );
    // Uncaught error: propagates with its span.
    assert_same_err("raise \"boom\"\n", "runtime error: 1:1: boom");
    assert_same_err(
        "try then\n    print 1 / 0\nexcept e then\n    raise \"altro\"\nend\n",
        "runtime error: 4:5: altro",
    );
    // A `return` in the `try` runs `finally` first.
    assert_same_out(
        "fn f()\n    try then\n        return 1\n    finally then\n        print \"pulizia\"\n    end\nend\nprint f()\n",
        "pulizia\n1\n",
    );
    // A `return` in `finally` replaces the pending return.
    assert_same_out(
        "fn f()\n    try then\n        return 1\n    finally then\n        return 2\n    end\nend\nprint f()\n",
        "2\n",
    );
    // An error in `finally` replaces the pending return.
    assert_same_err(
        "fn f()\n    try then\n        return 1\n    finally then\n        raise \"in finally\"\n    end\nend\nprint f()\n",
        "runtime error: 5:9: in finally",
    );
    // An error in `finally` replaces a caught error.
    assert_same_err(
        "try then\n    print 1 / 0\nexcept e then\n    print \"preso\"\nfinally then\n    raise \"in finally\"\nend\n",
        "runtime error: 6:5: in finally",
    );
    // A variable defined inside `finally` does not leak out.
    assert_same_err(
        "try then\n    print 1\nfinally then\n    let dentro = 5\nend\nprint dentro\n",
        "runtime error: 6:7: undefined variable 'dentro'",
    );
    // Errors raised by a called function are caught by the caller.
    assert_same_out(
        "fn controlla(x)\n    if x < 0 then\n        raise \"negativo non ammesso\"\n    end\n    return x * 2\nend\ntry then\n    print controlla(-3)\nexcept e then\n    print \"errore: \" + e\nend\nprint controlla(21)\n",
        "errore: negativo non ammesso\n42\n",
    );
}

#[test]
fn comprehensions() {
    assert_same_out(
        "let xs = [1, 2, 3, 4, 5]\nprint [x * 2 for x in xs]\nprint [x * x for x in xs if x % 2 == 0]\nprint {x: x * x for x in xs}\nprint {k: v for k, v in [(\"a\", 1), (\"b\", 2)]}\nprint {x % 3 for x in xs}\n",
        "[2, 4, 6, 8, 10]\n[4, 16]\n{1: 1, 2: 4, 3: 9, 4: 16, 5: 25}\n{\"a\": 1, \"b\": 2}\n{1, 2, 0}\n",
    );
    // The comprehension variable does not leak, and a generator
    // as the iterable is an error.
    assert_same_err(
        "let xs = [x * 2 for x in [1, 2]]\nprint x\n",
        "runtime error: 2:7: undefined variable 'x'",
    );
    assert_same("fn g()\n    yield 1\nend\nprint [x for x in g()]\n");
    // Nested comprehension over a comprehension's result.
    assert_same_out(
        "print [y + 1 for y in [x * 2 for x in [1, 2, 3]]]\n",
        "[3, 5, 7]\n",
    );
}

#[test]
fn f_strings() {
    assert_same_out(
        "let nome = \"Giuliano\"\nlet eta = 14\nprint f\"Ciao, sono {nome} e ho {eta} anni\"\nprint f\"somma: {1 + 2}\"\nprint f\"{nome} ha {eta * 2} anni il doppio\"\n",
        "Ciao, sono Giuliano e ho 14 anni\nsomma: 3\nGiuliano ha 28 anni il doppio\n",
    );
    assert_same_out(
        "let xs = [1, 2]\nprint f\"{xs[1]}\"\nprint f\"{\"a\" + \"b\"}\"\n",
        "2\nab\n",
    );
}

#[test]
fn slicing_and_indexing() {
    assert_same_out(
        "let xs = [10, 20, 30, 40, 50]\nprint xs[1]\nprint xs[-1]\nprint xs[1:3]\nprint xs[:2]\nprint xs[3:]\nprint xs[:]\nprint \"ciao\"[1:3]\nprint xs[0:5:2]\n",
        "20\n50\n[20, 30]\n[10, 20]\n[40, 50]\n[10, 20, 30, 40, 50]\nia\n[10, 30, 50]\n",
    );
    assert_same_out(
        "let xs = [1, 2, 3]\nxs[0] = 9\nxs[1:3] = [8, 7]\nprint xs\n",
        "[9, 8, 7]\n",
    );
    assert_same_err(
        "print [1, 2][5]\n",
        "runtime error: 1:14: list index out of range",
    );
    assert_same_err(
        "print {\"a\": 1}[\"b\"]\n",
        "runtime error: 1:16: key not found",
    );
    assert_same_err("print 1 / 0\n", "runtime error: 1:9: division by zero");
    assert_same_err(
        "print sconosciuta\n",
        "runtime error: 1:7: undefined variable 'sconosciuta'",
    );
}

#[test]
fn dict_and_set_ops() {
    assert_same_out(
        "let d = {\"a\": 1}\nd[\"b\"] = 2\nprint d\nprint len(d)\nprint d[\"a\"]\nlet s = {1, 2, 2, 3}\nprint s\nprint len(s)\n",
        "{\"a\": 1, \"b\": 2}\n2\n1\n{1, 2, 3}\n3\n",
    );
    // `==` on collections compares element by element.
    assert_same_out(
        "print [1, 2] == [1, 2]\nprint {\"a\": 1} == {\"a\": 1}\nprint {1, 2} == {2, 1}\nprint [1, 2] != [1, 3]\n",
        "true\ntrue\ntrue\ntrue\n",
    );
}

#[test]
fn kwargs_defaults_args_and_spread() {
    assert_same_out(
        "fn saluta(nome, saluto = \"Ciao\")\n    return saluto + \", \" + nome\nend\nprint saluta(\"Giuliano\")\nprint saluta(\"Giuliano\", saluto = \"Buongiorno\")\nprint saluta(nome = \"Fabiana\", saluto = \"Ehi\")\n",
        "Ciao, Giuliano\nBuongiorno, Giuliano\nEhi, Fabiana\n",
    );
    assert_same_out(
        "fn somma(a, b, *resto)\n    let t = a + b\n    for x in resto then\n        t = t + x\n    end\n    return t\nend\nprint somma(1, 2)\nprint somma(1, 2, 3, 4)\nprint somma(*[1, 2, 3])\n",
        "3\n10\n6\n",
    );
    assert_same_out(
        "fn f(a, b)\n    return a * 10 + b\nend\nprint f(*[3, 4])\nprint f(1, b = 5)\n",
        "34\n15\n",
    );
    // Defaults are evaluated once, at definition time.
    assert_same_out(
        "let d = [1]\nfn f(x = d)\n    return x\nend\nd = [2]\nprint f()\n",
        "[1]\n",
    );
    assert_same_err(
        "fn f(a, b = 1)\n    return a\nend\nprint f(1, 2, 3)\n",
        "runtime error: 4:7: takes at most 2 positional argument(s) but got 3",
    );
}

#[test]
fn lambdas() {
    assert_same_out(
        "let doppia = fn(x) x * 2 end\nprint doppia(21)\nfn applica(f, v)\n    return f(v)\nend\nprint applica(fn(x) x + 1 end, 41)\nprint fn(x) x * 10 end(3)\n",
        "42\n42\n30\n",
    );
    assert_same_out(
        "let base = 10\nlet aggiungi = fn(x) x + base end\nprint aggiungi(5)\n",
        "15\n",
    );
    assert_same_out(
        "let fs = []\nfor i = 0 to 2 then\n    push(fs, fn() i end)\nend\nprint fs[0]()\nprint fs[1]()\nprint fs[2]()\n",
        "2\n2\n2\n",
    );
}

#[test]
fn nested_functions_and_shadowing() {
    assert_same_out(
        "fn esterna(x)\n    fn interna(y)\n        return x + y\n    end\n    return interna(10)\nend\nprint esterna(5)\n",
        "15\n",
    );
    assert_same_out(
        "let x = \"globale\"\nfn f()\n    let x = \"locale\"\n    return x\nend\nprint f()\nprint x\n",
        "locale\nglobale\n",
    );
    // `and`/`or` short-circuit and return operand values.
    assert_same_out(
        "fn niente()\nend\nprint true and 5\nprint false or \"d\"\nprint 0 or \"d\"\nprint niente() and 1\nprint niente() or \"d\"\nprint 1 < 2 and 2 < 3\nprint not false\n",
        "5\nd\n0\nnil\nd\ntrue\ntrue\n",
    );
}

#[test]
fn short_circuit_and_operators() {
    // Short-circuiting must not evaluate the right side.
    assert_same_out(
        "let x = false\nfn tocca()\n    x = true\n    return true\nend\nprint false and tocca()\nprint x\nprint true or tocca()\nprint x\n",
        "false\nfalse\ntrue\nfalse\n",
    );
    assert_same_out(
        "print 7 / 2\nprint 7 % 3\nprint 2.5 + 1\nprint \"a\" + \"b\"\nprint 1 == 1.0\nprint \"a\" < \"b\"\nprint -5\nprint 2 * 3 % 4\n",
        "3.5\n1\n3.5\nab\ntrue\ntrue\n-5\n2\n",
    );
    assert_same_err(
        "print 5 % 2.5\n",
        "runtime error: 1:9: cannot apply % to Int and Float",
    );
    assert_same_err(
        "print \"a\" - \"b\"\n",
        "runtime error: 1:11: cannot subtract Str and Str",
    );
    assert_same_err(
        "print [1] + [2]\n",
        "runtime error: 1:11: cannot add List and List",
    );
}

#[test]
fn input_builtin() {
    assert_same_in(
        "let nome = input()\nprint \"Ciao, \" + nome\n",
        "Giuliano\n",
    );
    assert_same_in("print input()\nprint input()\n", "uno\ndue\n");
}

#[test]
fn modules_file_and_stdlib() {
    let dir = temp_dir("vm-modules");
    write_module(
        &dir,
        "mate",
        "let PI = 3.14\nfn doppio(x)\n    return x * 2\nend\nclass Punto\n    fn init(self, x, y)\n        self.x = x\n        self.y = y\n    end\nend\n",
    );
    let src = "import mate\nprint mate.PI\nprint mate.doppio(21)\nlet p = mate.Punto(1, 2)\nprint p.x\nprint p.y\nprint type(mate)\n";
    let vm = run_one(src, "", Backend::Vm, Some(dir.clone()));
    let tree = run_one(src, "", Backend::Tree, Some(dir.clone()));
    assert_eq!(vm, tree);
    assert_eq!(vm, Ok("3.14\n42\n1\n2\nmodule\n".to_string()));
    // Nested import: a module importing another module.
    let dir2 = temp_dir("vm-modules-nested");
    write_module(&dir2, "base", "fn saluta()\n    return \"ciao\"\nend\n");
    write_module(
        &dir2,
        "alto",
        "import base\nfn saluta()\n    return base.saluta() + \"!\"\nend\n",
    );
    let src = "import alto\nprint alto.saluta()\n";
    assert_eq!(
        run_one(src, "", Backend::Vm, Some(dir2.clone())),
        run_one(src, "", Backend::Tree, Some(dir2))
    );
    // A stdlib module.
    assert_same_out(
        "import math\nprint math.sqrt(16)\nprint math.PI\nimport stats\nprint stats.mean([1, 2, 3, 4])\n",
        "4\n3.141592653589793\n2.5\n",
    );
}

#[test]
fn string_and_builtin_helpers() {
    assert_same_out(
        "fn niente()\nend\nprint len(\"ciao\")\nprint len([1, 2, 3])\nprint str(42)\nprint str(1.5)\nprint str(true)\nprint str(niente())\nprint str([1, \"a\"])\nprint type(1)\nprint type(1.5)\nprint type(\"s\")\nprint type([1])\nprint type({\"a\": 1})\nprint type(niente())\n",
        "4\n3\n42\n1.5\ntrue\nnil\n[1, \"a\"]\nint\nfloat\nstr\nlist\ndict\nnil\n",
    );
    assert_same_out(
        "print int(\"42\")\nprint float(\"1.5\")\nprint range(3)\nprint range(1, 4)\n",
        "42\n1.5\n[0, 1, 2]\n[1, 2, 3]\n",
    );
}

#[test]
fn os_module() {
    // A known directory, so listdir is deterministic on both
    // backends; getenv uses a variable set here, so it is too.
    let dir = temp_dir("vm-os");
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    let path = dir.to_str().unwrap().to_string();
    unsafe {
        std::env::set_var("GG_OS_DUAL_TEST_VAR", "ciao");
    }
    assert_same_out(
        &format!(
            "import os\nprint os.listdir(\"{path}\")\nprint os.getenv(\"GG_OS_DUAL_TEST_VAR\")\nprint os.getenv(\"GG_OS_DUAL_TEST_VAR_MISSING\", \"fallback\")\nprint type(os.cwd())\nprint os.cwd() != \"\"\nprint len(os.args()) >= 1\n"
        ),
        "[\"a.txt\", \"b.txt\"]\nciao\nfallback\nstr\ntrue\ntrue\n",
    );
    unsafe {
        std::env::remove_var("GG_OS_DUAL_TEST_VAR");
    }
    // Errors agree on both backends too.
    assert_same("import os\nprint os.listdir(\"/tmp/gg-os-non-esiste-xyz\")\n");
    assert_same("import os\nos.mkdir(\"/tmp/gg-os-non-esiste-xyz/figlia\")\n");
}

#[test]
fn sqlite_module() {
    // Both backends run the same program on the same file, one
    // after the other: DROP TABLE IF EXISTS first keeps the
    // second run identical to the first.
    let dir = temp_dir("vm-sqlite");
    let path = dir.join("test.db").to_str().unwrap().to_string();
    assert_same_out(
        &format!(
            "import sqlite\nlet db = sqlite.open(\"{path}\")\nsqlite.exec(db, \"DROP TABLE IF EXISTS utenti\")\nsqlite.exec(db, \"CREATE TABLE utenti (id INTEGER PRIMARY KEY, nome TEXT, eta INTEGER)\")\nprint sqlite.execute(db, \"INSERT INTO utenti (nome, eta) VALUES (?, ?)\", [\"Giuliano\", 14])\nprint sqlite.execute(db, \"INSERT INTO utenti (nome, eta) VALUES (?, ?)\", [\"Altro\", 20])\nprint sqlite.query(db, \"SELECT id, nome, eta FROM utenti WHERE eta > ?\", [15])\nprint sqlite.query(db, \"SELECT count(*) AS n FROM utenti\")[0][\"n\"]\nsqlite.close(db)\nprint type(db)\n"
        ),
        "1\n1\n[{\"id\": 2, \"nome\": \"Altro\", \"eta\": 20}]\n2\nsqlite\n",
    );
    // Errors agree on both backends too, position included.
    assert_same_err(
        "import sqlite\nlet db = sqlite.open(\":memory:\")\nsqlite.exec(db, \"CREAT TABLE t (x)\")\n",
        "runtime error: 3:8: sqlite.exec(): near \"CREAT\": syntax error in CREAT TABLE t (x) at offset 0",
    );
}
