// SPDX-License-Identifier: GPL-3.0-or-later
//! The native `sqlite` module: an embedded SQLite database,
//! via the `rusqlite` crate (0.32) from crates.io, with the
//! `bundled` feature — SQLite is compiled from its C sources,
//! so no system SQLite library is needed.
//!
//! `sqlite.open(path)` returns a connection handle (a
//! `sqlite` value: `type(db)` is `"sqlite"`, `print db` is
//! `<sqlite>`, `<closed sqlite>` after `close`). The handle is
//! the connection itself, shared through `Rc<RefCell<...>>`
//! like the other mutable values, so `let db2 = db` aliases
//! the same connection — closing one closes the other too.
//!
//! The API is functional, not method calls: GG only calls
//! methods on instances, classes and modules, and a native
//! handle is none of those (a builtin function pointer cannot
//! carry per-connection state), so the connection is passed
//! explicitly as the first argument — the same shape as
//! `http.get(url)` and the rest of the native modules:
//!
//! * `sqlite.exec(db, sql)` — statements with no result
//!   (`CREATE TABLE`, `INSERT` without parameters, ...);
//!   `execute_batch`, so several statements separated by `;`
//!   run in one call. Returns Nil.
//! * `sqlite.execute(db, sql, params)` — a statement with `?`
//!   placeholders, `params` a List; returns the number of rows
//!   changed as an Int.
//! * `sqlite.query(db, sql)` / `sqlite.query(db, sql, params)`
//!   — a SELECT: a List of Dicts, one per row, keys are the
//!   column names in order. Column values map to GG as
//!   INTEGER → Int, REAL → Float, TEXT → Str, NULL → Nil and
//!   BLOB → a List of Int (one per byte).
//! * `sqlite.close(db)` — closes the connection; any later use
//!   of it (including a second `close`) is a runtime error.
//!
//! Parameters map the other way: Int → INTEGER, Float → REAL,
//! Str → TEXT, Bool → INTEGER 0/1, Nil → NULL; anything else
//! (a List, a Dict, a connection, ...) is an error naming the
//! function and the parameter position.
//!
//! Every SQLite failure — a syntax error, a missing table, a
//! violated constraint, a wrong number of parameters — is a GG
//! runtime error carrying SQLite's message (`sqlite.query():
//! no such table: utenti`), catchable with `try`/`except`
//! like any other runtime error.
//!
//! Out of scope: explicit transactions, prepared statements
//! exposed to GG, backup, pragmas and threading. Transactions
//! still work through plain SQL (`exec(db, "BEGIN")` ...,
//! `exec(db, "COMMIT")`) because `exec` runs any statement;
//! that is SQL, not API, so it needs no special support.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::value::{DictData, DictKey, SqliteData, Value};

/// The live connection inside a `sqlite` value, or an error
/// when the value is not a connection at all or the connection
/// is closed.
fn live_connection<'a>(
    name: &str,
    value: &'a Value,
) -> Result<std::cell::RefMut<'a, SqliteData>, RuntimeError> {
    let Value::Sqlite(data) = value else {
        return Err(RuntimeError::new(format!(
            "{name}() expects a sqlite connection, not {}",
            value.type_name()
        )));
    };
    let borrowed = data.borrow_mut();
    if borrowed.is_closed() {
        return Err(RuntimeError::new(format!("{name}(): connection is closed")));
    }
    Ok(borrowed)
}

/// A Str argument, or a clear error naming the function.
fn as_str(name: &str, value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        other => Err(RuntimeError::new(format!(
            "{name}() expects a Str, not {}",
            other.type_name()
        ))),
    }
}

/// A SQLite failure as a GG runtime error, naming the function
/// and carrying SQLite's message.
fn map_error(name: &str, err: rusqlite::Error) -> RuntimeError {
    RuntimeError::new(format!("{name}(): {err}"))
}

/// GG parameter values as SQLite values, in order. Bool becomes
/// INTEGER 0/1 (SQLite has no boolean type); Nil becomes NULL.
fn as_params(name: &str, value: &Value) -> Result<Vec<rusqlite::types::Value>, RuntimeError> {
    let Value::List(list) = value else {
        return Err(RuntimeError::new(format!(
            "{name}() params must be a List, not {}",
            value.type_name()
        )));
    };
    let mut params = Vec::new();
    for (i, item) in list.borrow().iter().enumerate() {
        let param = match item {
            Value::Int(n) => rusqlite::types::Value::Integer(*n),
            Value::Float(x) => rusqlite::types::Value::Real(*x),
            Value::Str(s) => rusqlite::types::Value::Text(s.clone()),
            Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
            Value::Nil => rusqlite::types::Value::Null,
            other => {
                return Err(RuntimeError::new(format!(
                    "{name}() parameter {i} must be Int, Float, Str, Bool or Nil, not {}",
                    other.type_name()
                )));
            }
        };
        params.push(param);
    }
    Ok(params)
}

/// A SQLite column value as a GG value. BLOB becomes a List of
/// Int, one per byte: GG has no byte-string type, and a List of
/// bytes round-trips through `execute` parameters only as
/// numbers — documented here so the choice is visible.
fn value_to_gg(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Nil,
        rusqlite::types::Value::Integer(n) => Value::Int(n),
        rusqlite::types::Value::Real(x) => Value::Float(x),
        rusqlite::types::Value::Text(s) => Value::Str(s),
        rusqlite::types::Value::Blob(bytes) => Value::List(Rc::new(RefCell::new(
            bytes.iter().map(|b| Value::Int(i64::from(*b))).collect(),
        ))),
    }
}

/// `sqlite.open(path)` — open (or create) the database file at
/// `path`, and return the connection. `":memory:"` opens a
/// temporary in-memory database that vanishes with the
/// connection.
pub fn open(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let path = as_str("sqlite.open", &args[0])?;
    let connection =
        rusqlite::Connection::open(path.as_str()).map_err(|err| map_error("sqlite.open", err))?;
    Ok(Value::Sqlite(Rc::new(RefCell::new(SqliteData::new(
        connection,
    )))))
}

/// `sqlite.exec(db, sql)` — run statements with no result
/// (`CREATE TABLE`, `INSERT` without parameters, `BEGIN`,
/// `COMMIT`, ...). Several statements separated by `;` run in
/// one call. Returns Nil.
pub fn exec(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let sql = as_str("sqlite.exec", &args[1])?;
    let data = live_connection("sqlite.exec", &args[0])?;
    let connection = data.connection.as_ref().expect("checked open above");
    connection
        .execute_batch(sql.as_str())
        .map_err(|err| map_error("sqlite.exec", err))?;
    Ok(Value::Nil)
}

/// `sqlite.execute(db, sql, params)` — run a statement with `?`
/// placeholders, `params` a List, and return the number of rows
/// changed as an Int.
pub fn execute(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let sql = as_str("sqlite.execute", &args[1])?;
    let params = as_params("sqlite.execute", &args[2])?;
    let data = live_connection("sqlite.execute", &args[0])?;
    let connection = data.connection.as_ref().expect("checked open above");
    let changed = connection
        .execute(sql.as_str(), rusqlite::params_from_iter(params))
        .map_err(|err| map_error("sqlite.execute", err))?;
    i64::try_from(changed)
        .map(Value::Int)
        .map_err(|_| RuntimeError::new("sqlite.execute(): rows changed out of range"))
}

/// `sqlite.query(db, sql)` / `sqlite.query(db, sql, params)` —
/// run a SELECT and return a List of Dicts, one per row, keys
/// the column names in order.
pub fn query(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let sql = as_str("sqlite.query", &args[1])?;
    let params = match args.get(2) {
        None => Vec::new(),
        Some(value) => as_params("sqlite.query", value)?,
    };
    let data = live_connection("sqlite.query", &args[0])?;
    let connection = data.connection.as_ref().expect("checked open above");
    let mut stmt = connection
        .prepare(sql.as_str())
        .map_err(|err| map_error("sqlite.query", err))?;
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params))
        .map_err(|err| map_error("sqlite.query", err))?;
    let mut out_rows = Vec::new();
    while let Some(row) = rows.next().map_err(|err| map_error("sqlite.query", err))? {
        let mut dict = DictData::new();
        for (i, name) in columns.iter().enumerate() {
            let value: rusqlite::types::Value =
                row.get(i).map_err(|err| map_error("sqlite.query", err))?;
            dict.insert(DictKey::Str(name.clone()), value_to_gg(value));
        }
        out_rows.push(Value::Dict(Rc::new(RefCell::new(dict))));
    }
    Ok(Value::List(Rc::new(RefCell::new(out_rows))))
}

/// `sqlite.close(db)` — close the connection. Any later use of
/// it, including a second `close`, is a runtime error.
pub fn close(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Sqlite(data) = &args[0] else {
        return Err(RuntimeError::new(format!(
            "sqlite.close() expects a sqlite connection, not {}",
            args[0].type_name()
        )));
    };
    let mut borrowed = data.borrow_mut();
    if borrowed.is_closed() {
        return Err(RuntimeError::new("sqlite.close(): connection is closed"));
    }
    borrowed.connection.take();
    Ok(Value::Nil)
}

#[cfg(test)]
mod tests {
    /// Run GG `source` and return its printed output, or the
    /// error text. (Same shape as the helpers in `lib.rs`.)
    fn run_ok(source: &str) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        crate::run_source(source, &mut input, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn run_err(source: &str) -> String {
        let mut input = std::io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        let err = crate::run_source(source, &mut input, &mut out).unwrap_err();
        err.to_string()
    }

    /// A fresh database path for one test: the tests run in
    /// parallel, so each test gets its own file, removed first.
    fn db_path(name: &str) -> String {
        let path = std::env::temp_dir().join(format!("gg-sqlite-test-{name}.db"));
        let _ = std::fs::remove_file(&path);
        path.to_str().unwrap().to_string()
    }

    /// The shared setup: a table with one row, as a GG prelude
    /// for `path`. The BLOB column is filled with the SQL
    /// literal `x'6869'` ("hi"): GG has no bytes, so a BLOB
    /// cannot come in as a parameter — only out, as a List.
    /// The table is dropped first, so the setup is idempotent:
    /// the file may already exist from an earlier test run.
    fn setup(path: &str) -> String {
        format!(
            "import sqlite\nimport json\nlet db = sqlite.open(\"{path}\")\nsqlite.exec(db, \"DROP TABLE IF EXISTS utenti\")\nsqlite.exec(db, \"CREATE TABLE utenti (id INTEGER PRIMARY KEY, nome TEXT, eta INTEGER, punteggio REAL, attivo INTEGER, nota TEXT, dati BLOB)\")\nlet n = sqlite.execute(db, \"INSERT INTO utenti (nome, eta, punteggio, attivo, nota, dati) VALUES (?, ?, ?, ?, ?, x'6869')\", [\"Giuliano\", 14, 9.5, true, json.parse(\"null\")])\nprint n\n"
        )
    }

    #[test]
    fn open_create_insert_select_round_trip() {
        // `json.parse("null")` is the only way to get Nil: GG
        // has no `nil` literal.
        let path = db_path("round-trip");
        assert_eq!(
            run_ok(&format!(
                "{setup}print sqlite.query(db, \"SELECT id, nome, eta FROM utenti\")\n",
                setup = setup(&path)
            )),
            "1\n[{\"id\": 1, \"nome\": \"Giuliano\", \"eta\": 14}]\n"
        );
    }

    #[test]
    fn in_memory_database() {
        assert_eq!(
            run_ok(
                "import sqlite\nlet db = sqlite.open(\":memory:\")\nsqlite.exec(db, \"CREATE TABLE t (x INTEGER)\")\nsqlite.execute(db, \"INSERT INTO t VALUES (?)\", [42])\nprint sqlite.query(db, \"SELECT x FROM t\")\n"
            ),
            "[{\"x\": 42}]\n"
        );
    }

    #[test]
    fn query_maps_every_type() {
        let path = db_path("types");
        assert_eq!(
            run_ok(&format!(
                "{setup}import json\nlet righe = sqlite.query(db, \"SELECT punteggio, attivo, nota, dati FROM utenti\")\nprint righe[0][\"punteggio\"]\nprint righe[0][\"attivo\"]\nprint righe[0][\"nota\"]\nprint righe[0][\"dati\"]\nprint type(righe[0][\"dati\"])\n",
                setup = setup(&path)
            )),
            "1\n9.5\n1\nnil\n[104, 105]\nlist\n"
        );
    }

    #[test]
    fn execute_returns_changed_rows() {
        let path = db_path("changed");
        assert_eq!(
            run_ok(&format!(
                "{setup}print sqlite.execute(db, \"UPDATE utenti SET eta = ? WHERE nome = ?\", [15, \"Giuliano\"])\nprint sqlite.execute(db, \"UPDATE utenti SET eta = 15 WHERE nome = ?\", [\"Nessuno\"])\nprint sqlite.execute(db, \"DELETE FROM utenti WHERE id = ?\", [1])\nprint sqlite.query(db, \"SELECT * FROM utenti\")\n",
                setup = setup(&path)
            )),
            "1\n1\n0\n1\n[]\n"
        );
    }

    #[test]
    fn query_with_and_without_params() {
        let path = db_path("params");
        assert_eq!(
            run_ok(&format!(
                "{setup}print sqlite.query(db, \"SELECT nome FROM utenti WHERE eta > ?\", [10])\nprint sqlite.query(db, \"SELECT nome FROM utenti WHERE eta > 100\")\nprint sqlite.query(db, \"SELECT count(*) AS n FROM utenti\")[0][\"n\"]\n",
                setup = setup(&path)
            )),
            "1\n[{\"nome\": \"Giuliano\"}]\n[]\n1\n"
        );
    }

    #[test]
    fn exec_runs_many_statements_and_transactions() {
        // `exec` runs several statements at once, so BEGIN and
        // COMMIT through plain SQL work too — that is SQL, not
        // API, and needs no special support.
        let path = db_path("exec-multi");
        assert_eq!(
            run_ok(&format!(
                "import sqlite\nlet db = sqlite.open(\"{path}\")\nsqlite.exec(db, \"CREATE TABLE a (x INTEGER); CREATE TABLE b (y TEXT)\")\nsqlite.exec(db, \"BEGIN; INSERT INTO a VALUES (1); INSERT INTO b VALUES ('z'); COMMIT\")\nprint sqlite.query(db, \"SELECT x FROM a\")\nprint sqlite.query(db, \"SELECT y FROM b\")\n"
            )),
            "[{\"x\": 1}]\n[{\"y\": \"z\"}]\n"
        );
    }

    #[test]
    fn file_persists_across_opens() {
        let path = db_path("persist");
        run_ok(&format!("{setup}sqlite.close(db)\n", setup = setup(&path)));
        assert_eq!(
            run_ok(&format!(
                "import sqlite\nlet db = sqlite.open(\"{path}\")\nprint sqlite.query(db, \"SELECT nome FROM utenti\")\nsqlite.close(db)\n"
            )),
            "[{\"nome\": \"Giuliano\"}]\n"
        );
    }

    #[test]
    fn type_and_display_of_a_connection() {
        assert_eq!(
            run_ok(
                "import sqlite\nlet db = sqlite.open(\":memory:\")\nprint type(db)\nprint db\nsqlite.close(db)\nprint db\n"
            ),
            "sqlite\n<sqlite>\n<closed sqlite>\n"
        );
    }

    #[test]
    fn open_failure_is_an_error() {
        let err = run_err(
            "import sqlite\nlet db = sqlite.open(\"/tmp/gg-sqlite-non-esiste-xyz/dir.db\")\n",
        );
        assert!(
            err.contains("sqlite.open(): unable to open database file"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sql_errors_carry_sqlites_message() {
        let path = db_path("errors");
        // A syntax error.
        let err = run_err(&format!(
            "import sqlite\nlet db = sqlite.open(\"{path}\")\nsqlite.exec(db, \"CREAT TABLE t (x)\")\n"
        ));
        assert!(
            err.contains("sqlite.exec(): near \"CREAT\": syntax error"),
            "unexpected error: {err}"
        );
        // A missing table.
        let err = run_err(&format!(
            "import sqlite\nlet db = sqlite.open(\"{path}\")\nprint sqlite.query(db, \"SELECT * FROM assente\")\n"
        ));
        assert!(
            err.contains("sqlite.query(): no such table: assente"),
            "unexpected error: {err}"
        );
        // A violated constraint (PRIMARY KEY twice).
        let err = run_err(&format!(
            "{setup}print sqlite.execute(db, \"INSERT INTO utenti (id, nome) VALUES (1, 'Altro')\", [])\n",
            setup = setup(&path)
        ));
        assert!(
            err.contains("sqlite.execute(): UNIQUE constraint failed: utenti.id"),
            "unexpected error: {err}"
        );
        // Too few parameters for the placeholders.
        let err = run_err(&format!(
            "{setup}print sqlite.execute(db, \"INSERT INTO utenti (nome) VALUES (?)\", [])\n",
            setup = setup(&path)
        ));
        assert!(
            err.contains(
                "sqlite.execute(): Wrong number of parameters passed to query. Got 0, needed 1"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sql_errors_have_position_and_are_catchable() {
        let path = db_path("position");
        // The failing call is on line 3; the error points at
        // the `exec` identifier (column 8), the call's span.
        let err = run_err(&format!(
            "import sqlite\nlet db = sqlite.open(\"{path}\")\nsqlite.exec(db, \"CREAT TABLE t (x)\")\n"
        ));
        assert!(
            err.starts_with("runtime error: 3:8: sqlite.exec():"),
            "unexpected error: {err}"
        );
        // The same error caught with try/except: the message is
        // the text after the position.
        assert_eq!(
            run_ok(&format!(
                "import sqlite\nlet db = sqlite.open(\"{path}\")\ntry then\n    sqlite.exec(db, \"CREAT TABLE t (x)\")\nexcept e then\n    print e\nend\nprint \"dopo\"\n"
            )),
            "sqlite.exec(): near \"CREAT\": syntax error in CREAT TABLE t (x) at offset 0\ndopo\n"
        );
    }

    #[test]
    fn closed_connection_is_an_error() {
        let path = db_path("closed");
        let err = run_err(&format!(
            "{setup}sqlite.close(db)\nprint sqlite.query(db, \"SELECT * FROM utenti\")\n",
            setup = setup(&path)
        ));
        assert!(
            err.contains("sqlite.query(): connection is closed"),
            "unexpected error: {err}"
        );
        let err = run_err(&format!(
            "{setup}sqlite.close(db)\nsqlite.close(db)\n",
            setup = setup(&path)
        ));
        assert!(
            err.contains("sqlite.close(): connection is closed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn wrong_argument_types_are_errors() {
        let path = db_path("arg-types");
        assert!(
            run_err("import sqlite\nprint sqlite.open(42)\n")
                .contains("sqlite.open() expects a Str, not Int"),
            "expected a type error for open(42)"
        );
        assert!(
            run_err(&format!(
                "{setup}print sqlite.exec(db, 5)\n",
                setup = setup(&path)
            ))
            .contains("sqlite.exec() expects a Str, not Int"),
            "expected a type error for a non-Str statement"
        );
        assert!(
            run_err(&format!(
                "{setup}print sqlite.exec(42, \"SELECT 1\")\n",
                setup = setup(&path)
            ))
            .contains("sqlite.exec() expects a sqlite connection, not Int"),
            "expected a type error for a non-connection db"
        );
        assert!(
            run_err(&format!(
                "{setup}print sqlite.execute(db, \"SELECT 1\", \"no\")\n",
                setup = setup(&path)
            ))
            .contains("sqlite.execute() params must be a List, not Str"),
            "expected a type error for non-List params"
        );
        assert!(
            run_err(&format!(
                "{setup}print sqlite.execute(db, \"SELECT ?\", [[1]])\n",
                setup = setup(&path)
            ))
            .contains(
                "sqlite.execute() parameter 0 must be Int, Float, Str, Bool or Nil, not List"
            ),
            "expected a type error for a List parameter"
        );
    }
}
