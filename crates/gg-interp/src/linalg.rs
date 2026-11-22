// SPDX-License-Identifier: GPL-3.0-or-later
//! Linear algebra (Phase 14, `rank` in Phase 18): the native `linalg` module.
//!
//! `import linalg` gives `solve`/`det`/`inv`/`rank` over the N-d
//! numeric arrays of Phase 11. Only 2D square matrices of
//! numbers are accepted for `det`/`inv`; `solve(A, b)` takes
//! a square matrix `A` and a 1D vector `b` of the same length
//! and returns the solution `x` of `A x = b` as a Float vector.
//! `rank(A)` accepts any 2D numeric matrix, square or
//! rectangular, and returns its rank as an Int.
//!
//! The solver is Gaussian elimination with partial pivoting.
//! A singular matrix — determinant zero, i.e. the system is
//! impossible or indeterminate, like the systems in the maths
//! ripasso — is a runtime error for `solve` and `inv` (the
//! GG convention: `matmul` on incompatible shapes and
//! `math.sqrt` of a negative number are errors too, never
//! silent `nil`). `det` of a singular matrix is simply `0`.
//! `rank` is what tells the two apart: a system is
//! indeterminate when `rank(A) == rank([A|b]) < n`, and
//! impossible when `rank(A) < rank([A|b])`.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::array::{fmt_shape, make_array};
use crate::error::RuntimeError;
use crate::value::{ArrayData, ElemType, Value};

/// A pivot smaller than this is treated as zero: the matrix is
/// singular. Absolute, not relative — GG systems are
/// school-scale, and a pivot this small means the equations
/// are (numerically) dependent.
const SINGULAR_TOL: f64 = 1e-12;

/// A value as an f64, if it is a number at all.
fn to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// A 2D array as a matrix of f64 (any shape, square or
/// rectangular), or a clear error naming the function.
fn as_rect_matrix(
    name: &str,
    data: &ArrayData,
) -> Result<(usize, usize, Vec<Vec<f64>>), RuntimeError> {
    let (rows, cols) = match data.shape.as_slice() {
        [r, c] => (*r, *c),
        _ => {
            return Err(RuntimeError::new(format!(
                "{name}() expects a 2D Array, not shape {}",
                fmt_shape(&data.shape)
            )));
        }
    };
    if !matches!(data.elem, ElemType::Int | ElemType::Float) {
        return Err(RuntimeError::new(format!(
            "{name}() expects numbers, not array of {}",
            data.elem.name()
        )));
    }
    let mut matrix = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut row = Vec::with_capacity(cols);
        for j in 0..cols {
            let Some(x) = to_f64(&data.items[i * cols + j]) else {
                unreachable!()
            };
            row.push(x);
        }
        matrix.push(row);
    }
    Ok((rows, cols, matrix))
}

/// A 2D array as a square matrix of f64, or a clear error
/// naming the function.
fn as_matrix(name: &str, data: &ArrayData) -> Result<(usize, Vec<Vec<f64>>), RuntimeError> {
    let (rows, cols, matrix) = as_rect_matrix(name, data)?;
    if rows != cols {
        return Err(RuntimeError::new(format!(
            "{name}() expects a square 2D Array, not shape {}",
            fmt_shape(&data.shape)
        )));
    }
    Ok((rows, matrix))
}

/// A 1D array of length `n` as a vector of f64, or a clear
/// error naming the function.
fn as_vector(name: &str, data: &ArrayData, n: usize) -> Result<Vec<f64>, RuntimeError> {
    match data.shape.as_slice() {
        [m] if *m == n => {}
        _ => {
            return Err(RuntimeError::new(format!(
                "{name}() expects a vector of length {n}, not shape {}",
                fmt_shape(&data.shape)
            )));
        }
    }
    if !matches!(data.elem, ElemType::Int | ElemType::Float) {
        return Err(RuntimeError::new(format!(
            "{name}() expects numbers, not array of {}",
            data.elem.name()
        )));
    }
    Ok(data.items.iter().map(|v| to_f64(v).unwrap()).collect())
}

/// Solve `A x = B` by Gaussian elimination with partial
/// pivoting: `B` is `n` rows of `m` right-hand sides, and the
/// result is `n` rows of `m` solutions. A (numerically) zero
/// pivot means `A` is singular.
fn solve_linear(
    name: &str,
    a: Vec<Vec<f64>>,
    b: Vec<Vec<f64>>,
) -> Result<Vec<Vec<f64>>, RuntimeError> {
    let n = a.len();
    let m = b[0].len();
    // The augmented matrix [A | B], one row per equation.
    let mut aug: Vec<Vec<f64>> = a
        .into_iter()
        .zip(b)
        .map(|(mut row, rhs)| {
            row.extend(rhs);
            row
        })
        .collect();
    for col in 0..n {
        // Partial pivoting: the largest remaining pivot in
        // this column, so a tiny (but non-zero) pivot does
        // not blow the result up.
        let (pivot_row, _) = (col..n)
            .map(|r| (r, aug[r][col].abs()))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        if aug[pivot_row][col].abs() < SINGULAR_TOL {
            return Err(RuntimeError::new(format!(
                "{name}(): singular matrix (determinant is zero)"
            )));
        }
        aug.swap(col, pivot_row);
        // Eliminate this column from every row below.
        for r in (col + 1)..n {
            let factor = aug[r][col] / aug[col][col];
            for c in col..(n + m) {
                aug[r][c] -= factor * aug[col][c];
            }
        }
    }
    // Back substitution, from the last equation up.
    let mut x = vec![vec![0.0; m]; n];
    for i in (0..n).rev() {
        for j in 0..m {
            let mut sum = aug[i][n + j];
            for k in (i + 1)..n {
                sum -= aug[i][k] * x[k][j];
            }
            x[i][j] = sum / aug[i][i];
        }
    }
    Ok(x)
}

/// The determinant, by LU decomposition with partial pivoting:
/// the product of the pivots, negated once per row swap. A
/// singular matrix has determinant exactly 0.
pub fn determinant(data: &ArrayData) -> Result<f64, RuntimeError> {
    let (n, mut a) = as_matrix("linalg.det", data)?;
    let mut det = 1.0;
    for col in 0..n {
        let (pivot_row, _) = (col..n)
            .map(|r| (r, a[r][col].abs()))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        if a[pivot_row][col].abs() < SINGULAR_TOL {
            return Ok(0.0);
        }
        if pivot_row != col {
            a.swap(col, pivot_row);
            det = -det;
        }
        det *= a[col][col];
        for r in (col + 1)..n {
            let factor = a[r][col] / a[col][col];
            for c in col..n {
                a[r][c] -= factor * a[col][c];
            }
        }
    }
    Ok(det)
}

/// The rank of a matrix, by Gaussian elimination with
/// partial pivoting to row echelon form: the number of rows
/// with a pivot. A pivot below `SINGULAR_TOL` counts as zero,
/// like in `solve` — a row that is (numerically) a combination
/// of the rows above it adds no rank. Works on rectangular
/// matrices too; an empty matrix (0 rows or 0 columns) has
/// rank 0.
pub fn matrix_rank(data: &ArrayData) -> Result<i64, RuntimeError> {
    let (rows, cols, mut a) = as_rect_matrix("linalg.rank", data)?;
    if rows == 0 || cols == 0 {
        return Ok(0);
    }
    let mut rank = 0;
    let mut r = 0;
    for col in 0..cols {
        if r >= rows {
            break;
        }
        // Partial pivoting: the largest remaining pivot in
        // this column. A column whose best pivot is (numerically)
        // zero has no pivot: the column is skipped, the rank
        // does not grow, and later columns can still pivot.
        let (pivot_row, _) = (r..rows)
            .map(|i| (i, a[i][col].abs()))
            .max_by(|x, y| x.1.partial_cmp(&y.1).unwrap())
            .unwrap();
        if a[pivot_row][col].abs() < SINGULAR_TOL {
            continue;
        }
        a.swap(r, pivot_row);
        // Eliminate this column from every row below.
        for i in (r + 1)..rows {
            let factor = a[i][col] / a[r][col];
            for c in col..cols {
                a[i][c] -= factor * a[r][c];
            }
        }
        r += 1;
        rank += 1;
    }
    Ok(rank)
}

/// `linalg.solve(A, b)` — the solution `x` of `A x = b` as a
/// Float vector of length `n`. A singular `A` is an error.
pub fn solve_system(a: &ArrayData, b: &ArrayData) -> Result<Value, RuntimeError> {
    let (n, matrix) = as_matrix("linalg.solve", a)?;
    let rhs = as_vector("linalg.solve", b, n)?;
    let cols: Vec<Vec<f64>> = rhs.into_iter().map(|x| vec![x]).collect();
    let x = solve_linear("linalg.solve", matrix, cols)?;
    let items: Vec<Value> = x.into_iter().map(|row| Value::Float(row[0])).collect();
    Ok(make_array(ElemType::Float, vec![n], items))
}

/// `linalg.inv(A)` — the inverse matrix, as a Float array.
/// A singular `A` is an error: it has no inverse.
pub fn inverse(data: &ArrayData) -> Result<Value, RuntimeError> {
    let (n, matrix) = as_matrix("linalg.inv", data)?;
    let identity: Vec<Vec<f64>> = (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    let x = solve_linear("linalg.inv", matrix, identity)?;
    let items: Vec<Value> = x.into_iter().flatten().map(Value::Float).collect();
    Ok(make_array(ElemType::Float, vec![n, n], items))
}

/// The argument as an Array, or a clear error naming the
/// function. Like `as_array` in `builtin.rs`: the `Rc` is
/// returned so the caller can borrow it for the call.
fn as_array<'a>(name: &str, value: &'a Value) -> Result<&'a Rc<RefCell<ArrayData>>, RuntimeError> {
    match value {
        Value::Array(array) => Ok(array),
        _ => Err(RuntimeError::new(format!(
            "{name}() expects an Array, not {}",
            value.type_name()
        ))),
    }
}

/// `linalg.solve(A, b)` — builtin wrapper (see `solve_system`).
pub fn solve(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("linalg.solve", &args[0])?;
    let b = as_array("linalg.solve", &args[1])?;
    solve_system(&a.borrow(), &b.borrow())
}

/// `linalg.det(A)` — builtin wrapper (see `determinant`).
pub fn det(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("linalg.det", &args[0])?;
    Ok(Value::Float(determinant(&a.borrow())?))
}

/// `linalg.inv(A)` — builtin wrapper (see `inverse`).
pub fn inv(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("linalg.inv", &args[0])?;
    inverse(&a.borrow())
}

/// `linalg.rank(A)` — builtin wrapper (see `matrix_rank`).
pub fn rank(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let a = as_array("linalg.rank", &args[0])?;
    Ok(Value::Int(matrix_rank(&a.borrow())?))
}
