// SPDX-License-Identifier: GPL-3.0-or-later
//! Slicing and boolean indexing (Phase 12).
//!
//! A slice `start:stop:step` follows Python's rules: `stop` is
//! excluded, out-of-range bounds are clamped (never an error), a
//! step of 0 is an error, and a negative step walks backwards.
//! Slicing always returns a copy, never a view. On arrays, each
//! axis takes either an index or a slice; the result keeps the
//! sliced axes (in order) and drops the indexed ones, so `a[:, 1]`
//! of a 2D array is a 1D column and `a[0:2, 1:3]` is a 2D
//! sub-array. Boolean indexing `a[mask]` keeps the elements where
//! the mask (same shape, Bool elements) is true, flattened to 1D.

use gg_lexer::Span;

use crate::array::{fmt_shape, make_array, resolve_axis};
use crate::error::RuntimeError;
use crate::value::{ArrayData, Value};

/// The evaluated bounds of a slice: each is `None` for "the
/// default for that end" (`start`/`stop` depend on the step's
/// sign, `step` defaults to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct SliceSpec {
    pub start: Option<i64>,
    pub stop: Option<i64>,
    pub step: Option<i64>,
}

impl SliceSpec {
    /// A slice over the whole axis: `a[:]`.
    pub fn full() -> Self {
        Self {
            start: None,
            stop: None,
            step: None,
        }
    }
}

/// One axis of an array selection: a single index, or a slice.
/// The span is the position of the index expression (or slice)
/// on that axis, so an out-of-range index points at it.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisSelector {
    Index(Value, Span),
    Slice(SliceSpec, Span),
}

/// One evaluated index argument: a plain value, or a slice with
/// its bounds already evaluated to integers (or `None` for the
/// default). The interpreter builds these from an `Index` node's
/// index expressions; the span is the position of that index
/// expression, so an out-of-range index points at it.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexArg {
    Value(Value, Span),
    Slice(SliceSpec, Span),
}

/// The positions a slice selects in a collection of `len`
/// elements, in order. Python semantics: a negative bound counts
/// from the end (`-1` is the last element); bounds past either
/// end are clamped; the default start/stop depend on the step's
/// sign (`0..len` forwards, `len-1..-1` backwards). A step of 0
/// is an error.
pub fn slice_positions(len: usize, spec: &SliceSpec) -> Result<Vec<usize>, RuntimeError> {
    let step = spec.step.unwrap_or(1);
    if step == 0 {
        return Err(RuntimeError::new("slice step must not be zero"));
    }
    let len_i = len as i64;
    // Negative bounds count from the end, then clamp into range.
    let normalize = |x: i64| if x < 0 { x + len_i } else { x };
    let (start, stop) = if step > 0 {
        (
            spec.start.map(normalize).unwrap_or(0).clamp(0, len_i),
            spec.stop.map(normalize).unwrap_or(len_i).clamp(0, len_i),
        )
    } else {
        (
            spec.start
                .map(normalize)
                .unwrap_or(len_i - 1)
                .clamp(-1, len_i - 1),
            spec.stop.map(normalize).unwrap_or(-1).clamp(-1, len_i - 1),
        )
    };
    let mut out = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < stop {
            out.push(i as usize);
            i += step;
        }
    } else {
        while i > stop {
            out.push(i as usize);
            i += step;
        }
    }
    Ok(out)
}

/// The slice of a list or tuple (a copy).
pub fn slice_values(items: &[Value], spec: &SliceSpec) -> Result<Vec<Value>, RuntimeError> {
    let positions = slice_positions(items.len(), spec)?;
    Ok(positions.iter().map(|&pos| items[pos].clone()).collect())
}

/// The slice of a string, by character (not by byte), like `len`.
pub fn slice_str(s: &str, spec: &SliceSpec) -> Result<String, RuntimeError> {
    let chars: Vec<char> = s.chars().collect();
    let positions = slice_positions(chars.len(), spec)?;
    Ok(positions.iter().map(|&pos| chars[pos]).collect())
}

/// The sub-array selected by one selector per axis: indexed axes
/// drop out of the shape, sliced axes keep their (selected)
/// length, in axis order. Missing axes are the caller's job —
/// pass a full slice for them.
pub fn slice_array(data: &ArrayData, selectors: &[AxisSelector]) -> Result<Value, RuntimeError> {
    debug_assert_eq!(selectors.len(), data.ndim());
    let strides = data.strides();
    let mut positions_per_axis: Vec<Vec<usize>> = Vec::with_capacity(selectors.len());
    let mut out_shape: Vec<usize> = Vec::new();
    for (axis, selector) in selectors.iter().enumerate() {
        match selector {
            AxisSelector::Index(index, span) => {
                let pos = resolve_axis(data.shape[axis], index.clone(), "array")
                    .map_err(|e| e.at(*span))?;
                positions_per_axis.push(vec![pos]);
            }
            AxisSelector::Slice(spec, span) => {
                let positions = slice_positions(data.shape[axis], spec).map_err(|e| e.at(*span))?;
                out_shape.push(positions.len());
                positions_per_axis.push(positions);
            }
        }
    }
    // Walk the cartesian product of the per-axis positions in
    // row-major order (the last axis varies fastest).
    let total: usize = out_shape.iter().product();
    let mut items = Vec::with_capacity(total);
    let mut counters = vec![0usize; selectors.len()];
    for _ in 0..total {
        let mut offset = 0;
        for (axis, positions) in positions_per_axis.iter().enumerate() {
            offset += positions[counters[axis]] * strides[axis];
        }
        items.push(data.items[offset].clone());
        // Odometer: advance the last axis, carry leftwards.
        for axis in (0..selectors.len()).rev() {
            counters[axis] += 1;
            if counters[axis] < positions_per_axis[axis].len() {
                break;
            }
            counters[axis] = 0;
        }
    }
    Ok(make_array(data.elem, out_shape, items))
}

/// `a[mask]` — the elements where the mask is true, flattened to
/// a 1D array in row-major order. The mask must be a Bool array
/// with the same shape as `data`.
pub fn mask_select(data: &ArrayData, mask: &ArrayData) -> Result<Value, RuntimeError> {
    if mask.shape != data.shape {
        return Err(RuntimeError::new(format!(
            "boolean mask shape {} does not match array shape {}",
            fmt_shape(&mask.shape),
            fmt_shape(&data.shape)
        )));
    }
    let items: Vec<Value> = data
        .items
        .iter()
        .zip(mask.items.iter())
        .filter_map(|(item, flag)| match flag {
            Value::Bool(true) => Some(item.clone()),
            _ => None,
        })
        .collect();
    let len = items.len();
    Ok(make_array(data.elem, vec![len], items))
}
