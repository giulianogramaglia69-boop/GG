// SPDX-License-Identifier: GPL-3.0-or-later
//! N-dimensional numeric arrays (Phase 11).
//!
//! Arrays store their items flat in row-major order plus a shape
//! (one size per dimension). A 1D array of 3 elements has shape
//! `[3]`; a 2x2 matrix has shape `[2, 2]` and 4 items in row-major
//! order. This module holds the shape/index arithmetic, the
//! element-wise operations and the matrix operations shared by the
//! builtins and the interpreter's arithmetic.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::value::{ArrayData, ElemType, Value, values_equal};

/// Max number of elements an array constructor may create, so a
/// huge shape cannot exhaust memory by accident.
pub const MAX_ARRAY_SIZE: usize = 1_000_000;

/// Build an array value from its element type, shape and flat
/// items (row-major order).
pub fn make_array(elem: ElemType, shape: Vec<usize>, items: Vec<Value>) -> Value {
    Value::Array(Rc::new(RefCell::new(ArrayData { elem, shape, items })))
}

/// The shape as it appears in error messages: `[2, 2]`.
pub fn fmt_shape(shape: &[usize]) -> String {
    let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
    format!("[{}]", dims.join(", "))
}

/// Resolve a (possibly negative) index on one axis into a
/// position: `-1` is the last element, `-len` the first. Out of
/// range is an error. Messages match the 1D `resolve_index` in
/// the interpreter.
pub fn resolve_axis(len: usize, index: Value, kind: &str) -> Result<usize, RuntimeError> {
    let Value::Int(i) = index else {
        return Err(RuntimeError::new(format!(
            "{kind} index must be an integer"
        )));
    };
    let len = len as i64;
    let pos = if i < 0 { len + i } else { i };
    if pos < 0 || pos >= len {
        return Err(RuntimeError::new(format!("{kind} index out of range")));
    }
    Ok(pos as usize)
}

/// The flat (row-major) offset of a multi-dimensional index: one
/// integer per axis, negative from the end of its axis. The number
/// of indices must match the number of dimensions.
pub fn flat_index(shape: &[usize], indices: &[Value]) -> Result<usize, RuntimeError> {
    if indices.len() != shape.len() {
        return Err(RuntimeError::new(format!(
            "array has {} dimensions but got {} indices",
            shape.len(),
            indices.len()
        )));
    }
    let mut strides = vec![1; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }
    let mut offset = 0;
    for (axis, index) in indices.iter().enumerate() {
        let pos = resolve_axis(shape[axis], index.clone(), "array")?;
        offset += pos * strides[axis];
    }
    Ok(offset)
}

/// The sub-array at one index of the first axis: `a[0]` of a 2D
/// array is its first row (a 1D array). Only used when the array
/// has more than one dimension; a 1D array indexes to an element.
pub fn subarray(data: &ArrayData, index: Value) -> Result<Value, RuntimeError> {
    let pos = resolve_axis(data.shape[0], index, "array")?;
    let stride = data.strides()[0];
    let len: usize = data.shape[1..].iter().product();
    let start = pos * stride;
    Ok(make_array(
        data.elem,
        data.shape[1..].to_vec(),
        data.items[start..start + len].to_vec(),
    ))
}

/// Parse a shape argument: an Int `n` means the 1D shape `[n]`,
/// a List of Ints is the shape as given. Dimensions must be
/// positive integers; an empty list is an error.
pub fn parse_shape(value: &Value, name: &str) -> Result<Vec<usize>, RuntimeError> {
    match value {
        Value::Int(n) => {
            if *n <= 0 {
                return Err(RuntimeError::new(format!(
                    "{name}() shape must be positive, not {n}"
                )));
            }
            Ok(vec![*n as usize])
        }
        Value::List(list) => {
            let borrowed = list.borrow();
            if borrowed.is_empty() {
                return Err(RuntimeError::new(format!(
                    "{name}() shape must not be empty"
                )));
            }
            let mut shape = Vec::with_capacity(borrowed.len());
            for dim in borrowed.iter() {
                let Value::Int(n) = dim else {
                    return Err(RuntimeError::new(format!(
                        "{name}() shape dimensions must be integers, not {}",
                        dim.type_name()
                    )));
                };
                if *n <= 0 {
                    return Err(RuntimeError::new(format!(
                        "{name}() shape dimensions must be positive, not {n}"
                    )));
                }
                shape.push(*n as usize);
            }
            Ok(shape)
        }
        other => Err(RuntimeError::new(format!(
            "{name}() expects an Int or a List of Ints for the shape, not {}",
            other.type_name()
        ))),
    }
}

/// Total number of elements of a shape (checked product).
fn shape_total(shape: &[usize]) -> Option<usize> {
    shape.iter().try_fold(1usize, |acc, d| acc.checked_mul(*d))
}

/// Reject a shape whose total is above the cap.
pub fn check_shape_size(shape: &[usize], name: &str) -> Result<(), RuntimeError> {
    match shape_total(shape) {
        Some(total) if total <= MAX_ARRAY_SIZE => Ok(()),
        _ => Err(RuntimeError::new(format!(
            "{name}() array is too large (max {MAX_ARRAY_SIZE} elements)"
        ))),
    }
}

/// Element-wise arithmetic between two arrays, or an array and a
/// scalar (broadcast from either side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ArrayOp {
    fn verb(&self) -> &'static str {
        match self {
            ArrayOp::Add => "add",
            ArrayOp::Sub => "subtract",
            ArrayOp::Mul => "multiply",
            ArrayOp::Div => "divide",
        }
    }
}

/// One element pair of an element-wise operation, with the same
/// int/float promotion as scalar arithmetic (`/` is always float
/// division; integer overflow is an error).
fn apply_op(op: ArrayOp, a: &Value, b: &Value) -> Result<Value, RuntimeError> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => match op {
            ArrayOp::Add => x
                .checked_add(*y)
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("integer overflow")),
            ArrayOp::Sub => x
                .checked_sub(*y)
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("integer overflow")),
            ArrayOp::Mul => x
                .checked_mul(*y)
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("integer overflow")),
            ArrayOp::Div => {
                if *y == 0 {
                    return Err(RuntimeError::new("division by zero"));
                }
                Ok(Value::Float(*x as f64 / *y as f64))
            }
        },
        (a, b) => {
            let (Some(x), Some(y)) = (as_f64(a), as_f64(b)) else {
                return Err(RuntimeError::new(format!(
                    "cannot {} {} and {}",
                    op.verb(),
                    a.type_name(),
                    b.type_name()
                )));
            };
            if op == ArrayOp::Div && y == 0.0 {
                return Err(RuntimeError::new("division by zero"));
            }
            Ok(Value::Float(match op {
                ArrayOp::Add => x + y,
                ArrayOp::Sub => x - y,
                ArrayOp::Mul => x * y,
                ArrayOp::Div => x / y,
            }))
        }
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// The element type of an element-wise result: `/` is always
/// float; otherwise Int when both operands are Int.
fn result_elem(op: ArrayOp, a: ElemType, b: ElemType) -> ElemType {
    if op == ArrayOp::Div {
        return ElemType::Float;
    }
    if a == ElemType::Int && b == ElemType::Int {
        ElemType::Int
    } else {
        ElemType::Float
    }
}

/// An array's element type, or an error when it is not numeric:
/// element-wise arithmetic and matrix products need numbers.
fn numeric_elem(data: &ArrayData) -> Result<ElemType, RuntimeError> {
    match data.elem {
        ElemType::Int | ElemType::Float => Ok(data.elem),
        other => Err(RuntimeError::new(format!(
            "array operations require numbers, not array of {}",
            other.name()
        ))),
    }
}

/// `a + b`, `a - b`, `a * b`, `a / b` element by element: two
/// arrays need the same shape; an array and a number broadcast
/// the number over every element (from either side).
pub fn elementwise(op: ArrayOp, left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Array(a), Value::Array(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            if a.shape != b.shape {
                return Err(RuntimeError::new(format!(
                    "array shapes {} and {} do not match",
                    fmt_shape(&a.shape),
                    fmt_shape(&b.shape)
                )));
            }
            let elem_a = numeric_elem(&a)?;
            let elem_b = numeric_elem(&b)?;
            let items = a
                .items
                .iter()
                .zip(b.items.iter())
                .map(|(x, y)| apply_op(op, x, y))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(
                result_elem(op, elem_a, elem_b),
                a.shape.clone(),
                items,
            ))
        }
        (Value::Array(a), scalar) => {
            let a = a.borrow();
            let elem_a = numeric_elem(&a)?;
            let Some(_) = as_f64(scalar) else {
                return Err(RuntimeError::new(format!(
                    "cannot {} Array and {}",
                    op.verb(),
                    scalar.type_name()
                )));
            };
            let elem_b = match scalar {
                Value::Int(_) => ElemType::Int,
                _ => ElemType::Float,
            };
            let items = a
                .items
                .iter()
                .map(|x| apply_op(op, x, scalar))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(
                result_elem(op, elem_a, elem_b),
                a.shape.clone(),
                items,
            ))
        }
        (scalar, Value::Array(a)) => {
            let a = a.borrow();
            let elem_a = numeric_elem(&a)?;
            let Some(_) = as_f64(scalar) else {
                return Err(RuntimeError::new(format!(
                    "cannot {} {} and Array",
                    op.verb(),
                    scalar.type_name()
                )));
            };
            let elem_b = match scalar {
                Value::Int(_) => ElemType::Int,
                _ => ElemType::Float,
            };
            let items = a
                .items
                .iter()
                .map(|y| apply_op(op, scalar, y))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(
                result_elem(op, elem_b, elem_a),
                a.shape.clone(),
                items,
            ))
        }
        _ => Err(RuntimeError::new(format!(
            "cannot {} {} and {}",
            op.verb(),
            left.type_name(),
            right.type_name()
        ))),
    }
}

/// Unary minus on an array: negate every element.
pub fn negate_array(data: &ArrayData) -> Result<Value, RuntimeError> {
    let elem = numeric_elem(data)?;
    let items = data
        .items
        .iter()
        .map(|x| match x {
            Value::Int(n) => n
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("integer overflow")),
            Value::Float(x) => Ok(Value::Float(-x)),
            other => Err(RuntimeError::new(format!(
                "cannot negate {}",
                other.type_name()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(make_array(elem, data.shape.clone(), items))
}

/// `dot(a, b)` — the scalar product of two 1D vectors of the same
/// length. Int vectors give an Int, anything float a Float.
pub fn dot(a: &ArrayData, b: &ArrayData) -> Result<Value, RuntimeError> {
    let elem_a = numeric_elem(a)?;
    let elem_b = numeric_elem(b)?;
    if a.ndim() != 1 || b.ndim() != 1 {
        return Err(RuntimeError::new("dot() expects two 1D arrays (vectors)"));
    }
    if a.size() != b.size() {
        return Err(RuntimeError::new(format!(
            "dot: vectors of length {} and {} do not match",
            a.size(),
            b.size()
        )));
    }
    if elem_a == ElemType::Int && elem_b == ElemType::Int {
        let mut total: i64 = 0;
        for (x, y) in a.items.iter().zip(b.items.iter()) {
            let (Value::Int(x), Value::Int(y)) = (x, y) else {
                unreachable!()
            };
            total = x
                .checked_mul(*y)
                .and_then(|p| total.checked_add(p))
                .ok_or_else(|| RuntimeError::new("integer overflow"))?;
        }
        Ok(Value::Int(total))
    } else {
        let mut total: f64 = 0.0;
        for (x, y) in a.items.iter().zip(b.items.iter()) {
            let (Some(x), Some(y)) = (as_f64(x), as_f64(y)) else {
                unreachable!()
            };
            total += x * y;
        }
        Ok(Value::Float(total))
    }
}

/// `matmul(a, b)` — the matrix product. Supported shapes:
/// 2D x 2D, 1D x 2D (row vector times matrix) and 2D x 1D
/// (matrix times column vector). Two 1D vectors are an error —
/// use `dot()` for the scalar product.
pub fn matmul(a: &ArrayData, b: &ArrayData) -> Result<Value, RuntimeError> {
    let elem_a = numeric_elem(a)?;
    let elem_b = numeric_elem(b)?;
    // (rows, inner, cols, out_shape, a_at, b_at): the product's
    // element (i, j) is the sum over k of a(i, k) * b(k, j).
    let (rows, inner, cols, out_shape): (usize, usize, usize, Vec<usize>) =
        match (a.shape.as_slice(), b.shape.as_slice()) {
            ([n, m], [p, q]) if m == p => (*n, *m, *q, vec![*n, *q]),
            ([n], [p, q]) if n == p => (1, *n, *q, vec![*q]),
            ([n, m], [p]) if m == p => (*n, *m, 1, vec![*n]),
            ([_], [_]) => {
                return Err(RuntimeError::new(
                    "matmul() of two vectors: use dot() instead",
                ));
            }
            _ => {
                return Err(RuntimeError::new(format!(
                    "matmul: shapes {} and {} are incompatible",
                    fmt_shape(&a.shape),
                    fmt_shape(&b.shape)
                )));
            }
        };
    // Flat accessors: a(i, k) and b(k, j) in row-major order.
    let a_at = |i: usize, k: usize| -> &Value {
        match a.shape.len() {
            1 => &a.items[k],
            _ => &a.items[i * a.shape[1] + k],
        }
    };
    let b_at = |k: usize, j: usize| -> &Value {
        match b.shape.len() {
            1 => &b.items[k],
            _ => &b.items[k * b.shape[1] + j],
        }
    };
    let mut items = Vec::with_capacity(rows * cols);
    if elem_a == ElemType::Int && elem_b == ElemType::Int {
        for i in 0..rows {
            for j in 0..cols {
                let mut total: i64 = 0;
                for k in 0..inner {
                    let (Value::Int(x), Value::Int(y)) = (a_at(i, k), b_at(k, j)) else {
                        unreachable!()
                    };
                    total = x
                        .checked_mul(*y)
                        .and_then(|p| total.checked_add(p))
                        .ok_or_else(|| RuntimeError::new("integer overflow"))?;
                }
                items.push(Value::Int(total));
            }
        }
        Ok(make_array(ElemType::Int, out_shape, items))
    } else {
        for i in 0..rows {
            for j in 0..cols {
                let mut total: f64 = 0.0;
                for k in 0..inner {
                    let (Some(x), Some(y)) = (as_f64(a_at(i, k)), as_f64(b_at(k, j))) else {
                        unreachable!()
                    };
                    total += x * y;
                }
                items.push(Value::Float(total));
            }
        }
        Ok(make_array(ElemType::Float, out_shape, items))
    }
}

/// `transpose(a)` — swap the two axes of a 2D array; a 1D array
/// is returned unchanged (a vector has no second axis to swap).
pub fn transpose_array(data: &ArrayData) -> Result<Value, RuntimeError> {
    match data.shape.as_slice() {
        [_] => Ok(make_array(
            data.elem,
            data.shape.clone(),
            data.items.clone(),
        )),
        [rows, cols] => {
            let mut items = vec![Value::Nil; data.items.len()];
            for i in 0..*rows {
                for j in 0..*cols {
                    items[j * rows + i] = data.items[i * cols + j].clone();
                }
            }
            Ok(make_array(data.elem, vec![*cols, *rows], items))
        }
        _ => Err(RuntimeError::new(
            "transpose() supports 1D and 2D arrays only",
        )),
    }
}

/// `reshape(a, shape)` — the same items with a new shape; the
/// total number of elements must stay the same.
pub fn reshape_array(data: &ArrayData, shape: Vec<usize>) -> Result<Value, RuntimeError> {
    let total = shape_total(&shape).unwrap_or(usize::MAX);
    if total != data.size() {
        return Err(RuntimeError::new(format!(
            "reshape: cannot reshape array of size {} into shape {}",
            data.size(),
            fmt_shape(&shape)
        )));
    }
    Ok(make_array(data.elem, shape, data.items.clone()))
}

/// A comparison operator for `compare_arrays` (Phase 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

/// Compare two scalar values with a comparison operator: `==` and
/// `!=` use value equality (numbers compare across Int/Float);
/// the ordering operators need numbers.
fn compare_scalars(op: CompareOp, a: &Value, b: &Value) -> Result<bool, RuntimeError> {
    match op {
        CompareOp::Eq => Ok(values_equal(a, b)),
        CompareOp::NotEq => Ok(!values_equal(a, b)),
        _ => {
            let (Some(x), Some(y)) = (as_f64(a), as_f64(b)) else {
                return Err(RuntimeError::new(format!(
                    "cannot compare {} and {}",
                    a.type_name(),
                    b.type_name()
                )));
            };
            let ord = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            Ok(match op {
                CompareOp::Lt => ord == Ordering::Less,
                CompareOp::LtEq => ord != Ordering::Greater,
                CompareOp::Gt => ord == Ordering::Greater,
                CompareOp::GtEq => ord != Ordering::Less,
                CompareOp::Eq | CompareOp::NotEq => unreachable!(),
            })
        }
    }
}

/// `a == b`, `a < b`, ... element by element, when at least one
/// side is an array: two arrays need the same shape, an array and
/// a scalar broadcast the scalar over every element (from either
/// side). The result is a Bool array with the array's shape. An
/// array against anything else (a List, a Dict, ...) is not an
/// element-wise comparison — it falls back to whole-value
/// equality, so `array([1]) == [1]` is still false.
pub fn compare_arrays(op: CompareOp, left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Array(a), Value::Array(b)) => {
            let a = a.borrow();
            let b = b.borrow();
            if a.shape != b.shape {
                return Err(RuntimeError::new(format!(
                    "cannot compare arrays of shapes {} and {}",
                    fmt_shape(&a.shape),
                    fmt_shape(&b.shape)
                )));
            }
            let items = a
                .items
                .iter()
                .zip(b.items.iter())
                .map(|(x, y)| compare_scalars(op, x, y).map(Value::Bool))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(ElemType::Bool, a.shape.clone(), items))
        }
        (Value::Array(a), scalar) if is_scalar(scalar) => {
            let a = a.borrow();
            let items = a
                .items
                .iter()
                .map(|x| compare_scalars(op, x, scalar).map(Value::Bool))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(ElemType::Bool, a.shape.clone(), items))
        }
        (scalar, Value::Array(a)) if is_scalar(scalar) => {
            let a = a.borrow();
            let items = a
                .items
                .iter()
                .map(|y| compare_scalars(op, scalar, y).map(Value::Bool))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(make_array(ElemType::Bool, a.shape.clone(), items))
        }
        _ => match op {
            CompareOp::Eq => Ok(Value::Bool(values_equal(left, right))),
            CompareOp::NotEq => Ok(Value::Bool(!values_equal(left, right))),
            _ => Err(RuntimeError::new(format!(
                "cannot compare {} and {}",
                left.type_name(),
                right.type_name()
            ))),
        },
    }
}

/// A value that can be compared element-wise against an array:
/// a scalar (Int, Float, Str or Bool). Anything else (a List, a
/// Dict, ...) is not a scalar.
fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_)
    )
}
