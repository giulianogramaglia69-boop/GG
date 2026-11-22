// SPDX-License-Identifier: GPL-3.0-or-later
//! Native `gui` module (Phase 13, extended in Phase 17):
//! windows with egui + eframe.
//!
//! `import gui` gives a module with one event loop and a few
//! widgets. The script opens a window and hands eframe a GG
//! function; eframe calls it once per frame, and the function
//! builds the UI by calling widget functions in order (immediate
//! mode: there is no widget tree to keep in sync with GG state).
//! When the frame function returns `false`, the window closes and
//! `gui.run` returns.
//!
//! How the pieces fit together:
//!
//! * `gui.run` and `gui.horizontal` are `WithInterp` builtins
//!   (see `BuiltinFunc`): they receive the interpreter, so they
//!   can call GG functions back. Every other widget is a plain
//!   builtin: it only needs the `egui::Ui` of the current frame.
//! * The current `Ui` travels in a thread-local (`CURRENT_UI`):
//!   `GuiApp::ui` sets it before calling the frame function and
//!   the `UiScope` guard restores the previous one afterwards, so
//!   nested layouts (`gui.horizontal`) and panics cannot leave a
//!   dangling pointer behind. GG is single-threaded and eframe
//!   runs on the calling thread, so the pointer is always valid
//!   while it is set.
//! * `GuiApp` holds the interpreter as a raw pointer for the same
//!   reason: eframe owns the app for the whole event loop, so the
//!   interpreter cannot be borrowed into it and handed back. The
//!   pointer is only dereferenced while `run_native` runs, the app
//!   is dropped inside `run_native`, and nothing else touches the
//!   interpreter meanwhile (a nested `gui.run` is an error, see
//!   `Interpreter::gui_running`).
//! * The frame function runs on every frame: `GuiApp::ui` asks
//!   egui for a repaint each time, so the loop is continuous (no
//!   frame-rate control in v0.1 — keep frame functions light).
//! * `gui.run` needs a display: on Linux it fails with a clear
//!   error when neither `DISPLAY` nor `WAYLAND_DISPLAY` is set,
//!   instead of letting winit fail obscurely. Under Xvfb the
//!   wgpu renderer uses the lavapipe software Vulkan driver.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, Write};
use std::rc::Rc;

use crate::builtin::{Arity, Builtin, BuiltinFunc};
use crate::env::Environment;
use crate::error::RuntimeError;
use crate::interp::{CallArgs, Interpreter};
use crate::value::{ModuleData, Value};

/// Build the `gui` module value.
pub fn build() -> Value {
    let env = Environment::new();
    for (name, arity, func) in GUI_FUNCTIONS {
        env.define(
            name,
            Value::Builtin(Rc::new(Builtin {
                name,
                arity: *arity,
                func: *func,
            })),
        );
    }
    Value::Module(Rc::new(ModuleData {
        name: "gui".to_string(),
        env,
    }))
}

type GuiFn = (&'static str, Arity, BuiltinFunc);

const GUI_FUNCTIONS: &[GuiFn] = &[
    ("run", Arity::Exact(2), BuiltinFunc::WithInterp(run_gui)),
    (
        "horizontal",
        Arity::Exact(1),
        BuiltinFunc::WithInterp(horizontal),
    ),
    (
        "vertical",
        Arity::Exact(1),
        BuiltinFunc::WithInterp(vertical),
    ),
    ("heading", Arity::Exact(1), BuiltinFunc::Plain(heading)),
    ("label", Arity::Exact(1), BuiltinFunc::Plain(label)),
    ("separator", Arity::Exact(0), BuiltinFunc::Plain(separator)),
    ("button", Arity::Exact(1), BuiltinFunc::Plain(button)),
    ("radio", Arity::Exact(3), BuiltinFunc::Plain(radio)),
    ("combo", Arity::Exact(3), BuiltinFunc::Plain(combo)),
    ("progress", Arity::Exact(1), BuiltinFunc::Plain(progress)),
    (
        "text_input",
        Arity::Exact(2),
        BuiltinFunc::Plain(text_input),
    ),
    ("slider", Arity::Exact(4), BuiltinFunc::Plain(slider)),
    ("checkbox", Arity::Exact(3), BuiltinFunc::Plain(checkbox)),
];

// ---- the current frame's Ui ----

// The `Ui` of the frame (or layout) being built right now, or
// null outside a frame function. Thread-local: GG is
// single-threaded, and tests run one frame at a time per thread.
thread_local! {
    static CURRENT_UI: Cell<*mut eframe::egui::Ui> = const { Cell::new(std::ptr::null_mut()) };
}

/// Point `CURRENT_UI` at `ui` until the guard is dropped, then
/// restore the previous one. Nesting works: `gui.horizontal`
/// installs the inner `Ui` and the outer one comes back after.
struct UiScope {
    previous: *mut eframe::egui::Ui,
}

impl UiScope {
    fn new(ui: &mut eframe::egui::Ui) -> Self {
        let previous = CURRENT_UI.with(|cell| cell.replace(ui as *mut eframe::egui::Ui));
        Self { previous }
    }
}

impl Drop for UiScope {
    fn drop(&mut self) {
        CURRENT_UI.with(|cell| cell.set(self.previous));
    }
}

/// The `Ui` widgets must draw into, or a clear error when called
/// outside a frame function.
///
/// This returns a mutable reference built from a raw pointer: the
/// pointer is only ever set to a `Ui` that is alive on the stack
/// (see `UiScope`), and GG is single-threaded, so the reference
/// cannot alias anything else while it is used.
#[allow(clippy::mut_from_ref)]
fn current_ui(name: &str) -> Result<&mut eframe::egui::Ui, RuntimeError> {
    let ptr = CURRENT_UI.with(|cell| cell.get());
    if ptr.is_null() {
        return Err(RuntimeError::new(format!(
            "{name} can only be called inside a frame function passed to gui.run"
        )));
    }
    // SAFETY: the pointer is only ever set to a `Ui` that is alive
    // on the stack (the frame's `Ui`, or a layout's inner `Ui`),
    // by `UiScope`, and cleared when that `Ui` goes out of scope.
    // GG is single-threaded and eframe runs on this thread.
    Ok(unsafe { &mut *ptr })
}

// ---- argument helpers ----

/// A string argument, or a clear error naming the function.
fn as_str<'a>(name: &str, value: &'a Value) -> Result<&'a str, RuntimeError> {
    match value {
        Value::Str(s) => Ok(s),
        other => Err(RuntimeError::new(format!(
            "{name} expects a Str, not {}",
            other.type_name()
        ))),
    }
}

/// A number argument as an f64, or a clear error naming the function.
fn as_number(name: &str, value: &Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(x) => Ok(*x),
        other => Err(RuntimeError::new(format!(
            "{name} expects a number, not {}",
            other.type_name()
        ))),
    }
}

/// A bool argument, or a clear error naming the function.
fn as_bool(name: &str, value: &Value) -> Result<bool, RuntimeError> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(RuntimeError::new(format!(
            "{name} expects a Bool, not {}",
            other.type_name()
        ))),
    }
}

/// An Int argument, or a clear error naming the function.
fn as_int(name: &str, value: &Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(n) => Ok(*n),
        other => Err(RuntimeError::new(format!(
            "{name} expects an Int, not {}",
            other.type_name()
        ))),
    }
}

/// A List of Str arguments as owned Strings, or a clear error
/// naming the function.
fn as_str_list(name: &str, value: &Value) -> Result<Vec<String>, RuntimeError> {
    let Value::List(list) = value else {
        return Err(RuntimeError::new(format!(
            "{name} expects a List, not {}",
            value.type_name()
        )));
    };
    let borrowed = list.borrow();
    let mut out = Vec::with_capacity(borrowed.len());
    for item in borrowed.iter() {
        let Value::Str(s) = item else {
            return Err(RuntimeError::new(format!(
                "{name} expects a List of Str, not {}",
                item.type_name()
            )));
        };
        out.push(s.clone());
    }
    Ok(out)
}

// ---- plain widgets (called inside a frame function) ----

/// `gui.heading(testo)` — a large bold label.
fn heading(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let text = as_str("gui.heading", &args[0])?;
    let ui = current_ui("gui.heading")?;
    ui.heading(text);
    Ok(Value::Nil)
}

/// `gui.label(testo)` — a plain label.
fn label(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let text = as_str("gui.label", &args[0])?;
    let ui = current_ui("gui.label")?;
    ui.label(text);
    Ok(Value::Nil)
}

/// `gui.separator()` — a horizontal line.
fn separator(
    _args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let ui = current_ui("gui.separator")?;
    ui.separator();
    Ok(Value::Nil)
}

/// `gui.button(testo)` — a button; true only in the frame where
/// it is clicked.
fn button(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let text = as_str("gui.button", &args[0])?;
    let ui = current_ui("gui.button")?;
    Ok(Value::Bool(ui.button(text).clicked()))
}

/// `gui.text_input(id, valore)` — a single-line text field. The
/// `id` is a stable string that identifies the field (egui keeps
/// the cursor and the edit state under it); `valore` is the text
/// the script holds in a variable, and the return value is the
/// text after this frame (the script reassigns it).
fn text_input(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let id = as_str("gui.text_input", &args[0])?;
    let value = as_str("gui.text_input", &args[1])?;
    let ui = current_ui("gui.text_input")?;
    let mut text = value.to_string();
    ui.push_id(ui.scope_id().with(id), |ui| {
        ui.text_edit_singleline(&mut text);
    });
    Ok(Value::Str(text))
}

/// `gui.slider(id, valore, min, max)` — a numeric slider; same
/// `id` + value convention as `text_input`. Returns the value
/// after this frame (a Float, clamped to `min..=max` by egui).
fn slider(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let id = as_str("gui.slider", &args[0])?;
    let value = as_number("gui.slider", &args[1])?;
    let min = as_number("gui.slider", &args[2])?;
    let max = as_number("gui.slider", &args[3])?;
    if min > max {
        return Err(RuntimeError::new("gui.slider() expects min <= max"));
    }
    let ui = current_ui("gui.slider")?;
    let mut current = value;
    ui.push_id(ui.scope_id().with(id), |ui| {
        ui.add(eframe::egui::Slider::new(&mut current, min..=max));
    });
    Ok(Value::Float(current))
}

/// `gui.checkbox(id, testo, valore)` — a checkbox with a label;
/// returns the state after this frame.
fn checkbox(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let id = as_str("gui.checkbox", &args[0])?;
    let text = as_str("gui.checkbox", &args[1])?;
    let value = as_bool("gui.checkbox", &args[2])?;
    let ui = current_ui("gui.checkbox")?;
    let mut checked = value;
    ui.push_id(ui.scope_id().with(id), |ui| {
        ui.checkbox(&mut checked, text);
    });
    Ok(Value::Bool(checked))
}

/// Clamp a progress value to [0, 1] (a pure helper, unit-tested).
fn clamp01(value: f64) -> f32 {
    value.clamp(0.0, 1.0) as f32
}

/// `gui.radio(id, testo, selezionato)` — a radio button (a
/// circle); returns `true` only in the frame where it is
/// clicked, like `button`. For an exclusive group, call one
/// `radio` per option with `selezionato` true for the chosen
/// one and set the choice when a radio returns `true` — there
/// are no implicit groups. The `id` keeps each radio distinct
/// even when two share the same text.
fn radio(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let id = as_str("gui.radio", &args[0])?;
    let text = as_str("gui.radio", &args[1])?;
    let selected = as_bool("gui.radio", &args[2])?;
    let ui = current_ui("gui.radio")?;
    Ok(Value::Bool(
        ui.push_id(ui.scope_id().with(id), |ui| {
            ui.add(eframe::egui::RadioButton::new(selected, text))
                .clicked()
        })
        .inner,
    ))
}

/// `gui.combo(id, opzioni, selezionato)` — a dropdown menu.
/// `opzioni` is a List of Str, `selezionato` the current index
/// (an Int); returns the selected index after this frame. An
/// empty list or an out-of-range index is a runtime error (the
/// interpreter attaches the call position, Phase 16).
fn combo(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let id = as_str("gui.combo", &args[0])?;
    let options = as_str_list("gui.combo", &args[1])?;
    let selected = as_int("gui.combo", &args[2])?;
    if options.is_empty() {
        return Err(RuntimeError::new("gui.combo() expects at least one option"));
    }
    if selected < 0 || selected as usize >= options.len() {
        return Err(RuntimeError::new(format!(
            "gui.combo() selected index {selected} is out of range for {} options",
            options.len()
        )));
    }
    let ui = current_ui("gui.combo")?;
    let mut current = selected as usize;
    eframe::egui::ComboBox::from_id_salt(id)
        .show_index(ui, &mut current, options.len(), |i| options[i].as_str());
    Ok(Value::Int(current as i64))
}

/// `gui.progress(valore)` — a progress bar; `valore` is a
/// number in 0..1, values outside are clamped to [0, 1] (not
/// an error).
fn progress(
    args: &[Value],
    _input: &mut dyn BufRead,
    _out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let value = as_number("gui.progress", &args[0])?;
    let ui = current_ui("gui.progress")?;
    ui.add(eframe::egui::ProgressBar::new(clamp01(value)));
    Ok(Value::Nil)
}

// ---- layouts and the event loop (need the interpreter) ----

/// `gui.horizontal(fn)` — run the GG function `fn` laying out
/// the widgets it creates horizontally instead of vertically.
fn horizontal(
    interp: &mut Interpreter,
    args: &[Value],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let func = args[0].clone();
    let ui = current_ui("gui.horizontal")?;
    ui.horizontal(|inner| {
        let _scope = UiScope::new(inner);
        interp.call_value(&func, CallArgs::positional(vec![]), input, out)
    })
    .inner
}

/// `gui.vertical(fn)` — like `gui.horizontal`, but the widgets
/// the function creates are laid out vertically (the default
/// direction, useful inside a `horizontal` block to nest a
/// column of widgets in a row).
fn vertical(
    interp: &mut Interpreter,
    args: &[Value],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let func = args[0].clone();
    let ui = current_ui("gui.vertical")?;
    ui.vertical(|inner| {
        let _scope = UiScope::new(inner);
        interp.call_value(&func, CallArgs::positional(vec![]), input, out)
    })
    .inner
}

/// True when the frame function's return value means "close the
/// window": only `false` does; anything else (including `nil`)
/// keeps the loop running.
fn should_close(value: &Value) -> bool {
    matches!(value, Value::Bool(false))
}

/// `gui.run(titolo, fn)` — open a window titled `titolo` and run
/// the event loop, calling the GG function `fn` (no arguments)
/// once per frame. Returns `nil` when the window closes, either
/// because the frame function returned `false` or because the
/// user closed the window. A runtime error inside the frame
/// function closes the window and is reported by `gui.run`.
fn run_gui(
    interp: &mut Interpreter,
    args: &[Value],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<Value, RuntimeError> {
    let Value::Str(title) = &args[0] else {
        return Err(RuntimeError::new(format!(
            "gui.run() expects a Str title, not {}",
            args[0].type_name()
        )));
    };
    let title = title.clone();
    let func = args[1].clone();
    if !matches!(
        func,
        Value::Function(_) | Value::Builtin(_) | Value::BoundMethod(_) | Value::Class(_)
    ) {
        return Err(RuntimeError::new(format!(
            "gui.run() expects a function, not {}",
            func.type_name()
        )));
    }
    if interp.gui_running {
        return Err(RuntimeError::new(
            "gui.run() cannot be called while another gui.run() is active",
        ));
    }
    // No display at all (headless shell, no Xvfb): say so plainly
    // instead of letting winit fail with an obscure error.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(RuntimeError::new(
            "gui.run() needs a display: neither DISPLAY nor WAYLAND_DISPLAY is set (run on a desktop, or under Xvfb)",
        ));
    }
    interp.gui_running = true;
    // The app owns the interpreter for the whole event loop
    // through raw pointers (see the module docs): `run_native`
    // consumes the app and drops it before returning, so the
    // pointers are dead by the time this function continues.
    let error: Rc<RefCell<Option<RuntimeError>>> = Rc::new(RefCell::new(None));
    let app = GuiApp {
        interp: interp as *mut Interpreter,
        func,
        input: input as *mut (dyn BufRead + '_),
        out: out as *mut (dyn Write + '_),
        error: Rc::clone(&error),
    };
    let options = eframe::NativeOptions {
        persist_window: false,
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    let result = eframe::run_native(&title, options, Box::new(|_cc| Ok(Box::new(app))));
    interp.gui_running = false;
    if let Err(e) = result {
        return Err(RuntimeError::new(format!(
            "gui.run() failed to open a window: {e}"
        )));
    }
    if let Some(e) = error.borrow_mut().take() {
        return Err(e);
    }
    Ok(Value::Nil)
}

/// The eframe app: on every frame it calls the GG frame function
/// inside a `CentralPanel` (vertical layout) and closes the
/// window when the function returns `false` or fails.
struct GuiApp<'a> {
    /// Raw pointer: eframe owns the app, so the interpreter cannot
    /// be borrowed into it (see the module docs for the safety
    /// argument). Only dereferenced while `run_native` runs.
    interp: *mut Interpreter,
    func: Value,
    input: *mut (dyn BufRead + 'a),
    out: *mut (dyn Write + 'a),
    /// A runtime error from the frame function, reported by
    /// `gui.run` after the window closes.
    error: Rc<RefCell<Option<RuntimeError>>>,
}

impl eframe::App for GuiApp<'_> {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        // Continuous repaint: the frame function IS the program's
        // loop, so ask for the next frame every frame.
        ui.ctx().request_repaint();
        eframe::egui::CentralPanel::default().show(ui, |ui| {
            let _scope = UiScope::new(ui);
            // SAFETY: the pointers are set by `run_gui` to the
            // interpreter and streams that outlive `run_native`,
            // and this `ui` only runs inside `run_native`.
            let result = unsafe {
                (*self.interp).call_value(
                    &self.func.clone(),
                    CallArgs::positional(vec![]),
                    &mut *self.input,
                    &mut *self.out,
                )
            };
            match result {
                Ok(value) => {
                    if should_close(&value) {
                        ui.ctx()
                            .send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                    }
                }
                Err(e) => {
                    *self.error.borrow_mut() = Some(e);
                    ui.ctx()
                        .send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn empty_io() -> (Cursor<Vec<u8>>, Vec<u8>) {
        (Cursor::new(Vec::new()), Vec::new())
    }

    /// Calling a widget with no frame running is an error, never a
    /// panic: the thread-local `Ui` is null outside `gui.run`.
    #[test]
    fn widgets_outside_a_frame_are_errors() {
        let (mut input, mut out) = empty_io();
        let str_arg = [Value::Str("x".to_string())];
        for func in [heading, label, button] {
            let err = func(&str_arg, &mut input, &mut out).unwrap_err();
            assert!(err.to_string().contains("inside a frame function"), "{err}");
        }
        let err = separator(&[], &mut input, &mut out).unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = text_input(
            &[Value::Str("id".into()), Value::Str("v".into())],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = slider(
            &[
                Value::Str("id".into()),
                Value::Float(1.0),
                Value::Float(0.0),
                Value::Float(2.0),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = checkbox(
            &[
                Value::Str("id".into()),
                Value::Str("t".into()),
                Value::Bool(true),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = radio(
            &[
                Value::Str("id".into()),
                Value::Str("t".into()),
                Value::Bool(false),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = combo(
            &[
                Value::Str("id".into()),
                Value::List(Rc::new(RefCell::new(vec![Value::Str("a".into())]))),
                Value::Int(0),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
        let err = progress(&[Value::Float(0.5)], &mut input, &mut out).unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
    }

    /// `gui.vertical` outside a frame is the same error (it needs
    /// the interpreter, like `horizontal`).
    #[test]
    fn vertical_outside_a_frame_is_an_error() {
        let mut interp = Interpreter::new();
        let (mut input, mut out) = empty_io();
        let err = vertical(&mut interp, &[Value::Nil], &mut input, &mut out).unwrap_err();
        assert!(err.to_string().contains("inside a frame function"));
    }

    /// A slider with min > max is an error before any Ui is
    /// touched.
    #[test]
    fn slider_rejects_inverted_range() {
        let (mut input, mut out) = empty_io();
        let err = slider(
            &[
                Value::Str("id".into()),
                Value::Float(1.0),
                Value::Float(5.0),
                Value::Float(0.0),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("min <= max"));
    }

    /// Argument marshalling: wrong types are clear errors naming
    /// the function.
    #[test]
    fn widget_argument_types() {
        let (mut input, mut out) = empty_io();
        let err = button(&[Value::Int(3)], &mut input, &mut out).unwrap_err();
        assert!(err.to_string().contains("gui.button"));
        assert!(err.to_string().contains("Str"));
        let err = text_input(
            &[Value::Int(1), Value::Str("v".into())],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.text_input"));
        let err = checkbox(
            &[
                Value::Str("id".into()),
                Value::Str("t".into()),
                Value::Str("x".into()),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.checkbox"));
        assert!(err.to_string().contains("Bool"));
        let err = radio(
            &[
                Value::Str("id".into()),
                Value::Str("t".into()),
                Value::Str("x".into()),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.radio"));
        assert!(err.to_string().contains("Bool"));
        let err = progress(&[Value::Str("x".into())], &mut input, &mut out).unwrap_err();
        assert!(err.to_string().contains("gui.progress"));
        assert!(err.to_string().contains("number"));
    }

    /// `combo` validates its arguments before any Ui is touched:
    /// options must be a non-empty List of Str, the index an Int
    /// inside it.
    #[test]
    fn combo_validates_its_arguments() {
        let (mut input, mut out) = empty_io();
        let options = |items: Vec<Value>| Value::List(Rc::new(RefCell::new(items)));
        let two = options(vec![Value::Str("a".into()), Value::Str("b".into())]);
        // Index past the end.
        let err = combo(
            &[Value::Str("id".into()), two.clone(), Value::Int(2)],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
        assert!(err.to_string().contains("2"));
        // Negative index.
        let err = combo(
            &[Value::Str("id".into()), two.clone(), Value::Int(-1)],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
        // Empty options: there is nothing to select.
        let err = combo(
            &[Value::Str("id".into()), options(vec![]), Value::Int(0)],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("at least one option"));
        // Options must be a List of Str.
        let err = combo(
            &[
                Value::Str("id".into()),
                Value::Str("a".into()),
                Value::Int(0),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.combo"));
        assert!(err.to_string().contains("List"));
        let err = combo(
            &[
                Value::Str("id".into()),
                options(vec![Value::Str("a".into()), Value::Int(1)]),
                Value::Int(0),
            ],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("List of Str"));
        // The index must be an Int, not a Float.
        let err = combo(
            &[Value::Str("id".into()), two, Value::Float(0.0)],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.combo"));
        assert!(err.to_string().contains("Int"));
    }

    /// `progress` clamps to [0, 1]: values outside are not an
    /// error, the bar just stops at the end.
    #[test]
    fn progress_clamps_out_of_range_values() {
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(clamp01(0.0), 0.0);
        assert_eq!(clamp01(0.5), 0.5);
        assert_eq!(clamp01(1.0), 1.0);
        assert_eq!(clamp01(1.5), 1.0);
    }

    /// Only `false` closes the window; `true`, `nil` and anything
    /// else keep the loop running.
    #[test]
    fn only_false_closes_the_window() {
        assert!(should_close(&Value::Bool(false)));
        assert!(!should_close(&Value::Bool(true)));
        assert!(!should_close(&Value::Nil));
        assert!(!should_close(&Value::Int(0)));
        assert!(!should_close(&Value::Str("false".to_string())));
    }

    /// `gui.run` with a non-string title or a non-function is an
    /// error before any window is opened.
    #[test]
    fn run_gui_validates_its_arguments() {
        let mut interp = Interpreter::new();
        let (mut input, mut out) = empty_io();
        let err = run_gui(
            &mut interp,
            &[Value::Int(1), Value::Nil],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.run()"));
        assert!(err.to_string().contains("Str title"));
        let err = run_gui(
            &mut interp,
            &[Value::Str("T".into()), Value::Int(42)],
            &mut input,
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("gui.run()"));
        assert!(err.to_string().contains("expects a function"));
    }

    /// The module builds with all thirteen functions, and `type(gui)`
    /// is `"module"` like for the other native modules.
    #[test]
    fn module_shape() {
        let Value::Module(module) = build() else {
            panic!("gui.build() is not a module");
        };
        assert_eq!(module.name, "gui");
        for name in [
            "run",
            "horizontal",
            "vertical",
            "heading",
            "label",
            "separator",
            "button",
            "radio",
            "combo",
            "progress",
            "text_input",
            "slider",
            "checkbox",
        ] {
            assert!(module.env.get(name).is_ok(), "gui module is missing {name}");
        }
    }
}
