// Copyright 2021-2022 the Tectonic Project
// Licensed under the MIT License.

//! The [XeTeX] program as a reusable crate.
//!
//! [XeTeX]: http://www.xetex.org/
//!
//! This crate provides the core TeX engine implementation used by [Tectonic].
//! However, in order to obtain the full Tectonic user experience, it must be
//! combined with a variety of other utilities: the `xdvipdfmx` engine, code to
//! fetch support files, and so on. Rather than using this crate directly you
//! should probably use the main [`tectonic`] crate, which combines all of these
//! pieces into a (semi) coherent whole.
//!
//! [Tectonic]: https://tectonic-typesetting.github.io/
//! [`tectonic`]: https://docs.rs/tectonic/

// TODO: the internal interface we're using here is pretty janky. The bibtex
// engine has a nicer approach that we should probably start using.

use std::{ffi::CString, io::Write, sync::Arc, time::SystemTime};
use tectonic_bridge_core::{
    CoreBridgeLauncher, CoreBridgeState, EngineAbortedError, FileFormat, InputId, OutputId,
};
use tectonic_errors::prelude::*;

pub mod commands;
pub mod diag;
pub mod expand;
pub mod fmtio;
pub mod fonts;
pub mod ini;
pub mod input;
pub mod output;
pub mod primitives;
pub mod scan;
pub mod scanner;
pub mod shipout;
pub mod state;
pub mod strpool;
pub mod synctex;
pub mod teckit;
pub mod texmfmp;
pub mod unicode;

pub use primitives::{primitive_defs, PrimitiveDef, PrimitiveExtraInit, PrimitiveRegistryError};
pub use scanner::{
    ControlSequence, ControlSequenceTable, InputState, TeXMouthError, CS_TOKEN_FLAG,
    END_MATCH_TOKEN, LEFT_BRACE_LIMIT, LEFT_BRACE_TOKEN, MATCH_TOKEN, MAX_CHAR_VAL,
    OUT_PARAM_TOKEN, PROTECTED_TOKEN, RIGHT_BRACE_LIMIT, RIGHT_BRACE_TOKEN, SPACE_TOKEN,
};
pub use state::{
    badness, half, tex_round, CapturedDiagnostic, CommandCode, DiagSeverity, InputFileMetadata,
    InputResolver, InputResolverFn, MemoryWord, MetadataResolver, MetadataResolverFn,
    ShellEscapeFn, ShellEscapeHook, TeXEngineState, TeXMemoryError, TeXMode, TwoHalves,
};

/// A serial number describing the detailed binary layout of the TeX "format
/// files" used by this crate. This number will occasionally increment,
/// indicating that the format file structure has changed. There is no provision
/// for partial forwards or backwards compatibility: if the number changes, you
/// need to regenerate your format files. If you’re generating format files, you
/// should munge this serial number in the filename, or something along those
/// lines, to make sure that when the engine is updated you don’t attempt to
/// reuse old files.
//
// DEVELOPER NOTE: if you change this, rerun cbindgen! This value is exported
// into the C/C++ code as a #define.
pub const FORMAT_SERIAL: u32 = 33;

/// A possible outcome from a (Xe)TeX engine invocation.
///
/// The classic TeX implementation provides a fourth outcome: "fatal error". In
/// Tectonic, this outcome is represented as an `Err` result rather than a
/// [`TexOutcome`].
///
/// The `Errors` possibility will only occur if the `halt_on_error` engine
/// option is false: if it’s true, errors get upgraded to fatals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TexOutcome {
    /// Nothing bad happened.
    Spotless,

    /// Warnings were issued by the TeX engine. Note that, due to the ways that
    /// people are used to using TeX, warnings are *extremely* common in the
    /// wild. It’s rare to find a real document that *doesn’t* compile with
    /// warnings.
    Warnings,

    /// Errors were issued by the TeX engine. Note that, in TeX terminology,
    /// errors are not necessarily *fatal* errors: the engine will try extremely
    /// hard to proceed when it encounters them. It is not uncommon to find TeX
    /// documents in the wild that produce errors.
    Errors,
}

/// A struct for invoking the (Xe)TeX engine.
///
/// This struct has a fairly straightforward "builder" interface: you create it,
/// apply any settings that you wish, and eventually run the
/// [`process()`](Self::process) method.
///
/// Due to constraints of the gnarly C/C++ code underlying the engine
/// implementation, only one engine may run at once in one process. The engine
/// execution framework uses a global mutex to ensure that this is the case.
/// This restriction applies not only to the [`TexEngine`] type but to *all*
/// Tectonic engines. I.e., you can't run this engine and the BibTeX engine at
/// the same time.
#[derive(Debug)]
pub struct TexEngine {
    // One day, the engine will hold its own state. For the time being,
    // though, it's just a proxy for the global constants in the C code.
    halt_on_error: bool,
    initex_mode: bool,
    synctex_enabled: bool,
    semantic_pagination_enabled: bool,
    shell_escape_enabled: bool,
    build_date: SystemTime,
    use_rust_backend: bool,
}

impl Default for TexEngine {
    fn default() -> Self {
        TexEngine {
            halt_on_error: true,
            initex_mode: false,
            synctex_enabled: false,
            semantic_pagination_enabled: false,
            shell_escape_enabled: false,
            build_date: SystemTime::UNIX_EPOCH,
            use_rust_backend: false,
        }
    }
}

impl TexEngine {
    /// Configure whether the engine will halt on errors.
    ///
    /// The default setting is true. If false, the engine will plunge on ahead
    /// in the face of all but the most catastrophic problems. It’s really quite
    /// impressive!
    pub fn halt_on_error_mode(&mut self, halt_on_error: bool) -> &mut Self {
        self.halt_on_error = halt_on_error;
        self
    }

    /// Configure the engine to run in "initex" mode, in which it generates a
    /// "format" file that serializes the engine state rather than a PDF
    /// document. The default is false.
    pub fn initex_mode(&mut self, initex: bool) -> &mut Self {
        self.initex_mode = initex;
        self
    }

    /// Configure the engine to produce SyncTeX data.
    ///
    /// The default is false.
    pub fn synctex(&mut self, synctex_enabled: bool) -> &mut Self {
        self.synctex_enabled = synctex_enabled;
        self
    }

    /// Configure the engine to use "semantic pagination".
    ///
    /// **Important:** this mode is essentially unimplemented.
    ///
    /// The goal of this mode is to set up the engine to create HTML-friendly
    /// output by altering how paragraphs and pages are constructed. When this
    /// mode is activated, the engine output type changes from XDV to SPX
    /// (although the two formats are quite similar).
    ///
    /// The default is false.
    pub fn semantic_pagination(&mut self, enabled: bool) -> &mut Self {
        self.semantic_pagination_enabled = enabled;
        self
    }

    /// Configure whether the "shell escape" TeX feature is enabled.
    ///
    /// The default is false.
    pub fn shell_escape(&mut self, shell_escape_enabled: bool) -> &mut Self {
        self.shell_escape_enabled = shell_escape_enabled;
        self
    }

    /// Sets the date and time used by the TeX engine. This affects things like
    /// LaTeX's \today command.
    ///
    /// The default vaue is the Unix epoch, so you should almost always override
    /// this setting. If you are aiming to achieve reproducible builds, you will
    /// need a way to fix this parameter from one engine invocation to the next.
    pub fn build_date(&mut self, date: SystemTime) -> &mut Self {
        self.build_date = date;
        self
    }

    /// Select the pure-Rust [`TeXEngineState`] implementation instead of the
    /// C engine for [`Self::process`]. The Rust backend is fully re-entrant
    /// and does not rely on the C global-state machinery, but it is still
    /// validated against the C implementation before it becomes the default.
    ///
    /// The default is false (the C engine runs).
    pub fn rust_backend(&mut self, use_rust_backend: bool) -> &mut Self {
        self.use_rust_backend = use_rust_backend;
        self
    }

    /// Process a document using the current engine configuration.
    ///
    /// The *launcher* parameter gives overarching environmental context in
    /// which the engine will be run.
    ///
    /// The *format_file_name* is the name for the TeX "format file" giving
    /// preloaded engine state. It must be findable in the I/O stack, using the
    /// special hooks that are provided for handing format files, which allow
    /// updates to the file format to be handed (see [`FORMAT_SERIAL`]). If in
    /// "initex" mode, this parameter will be ignored.
    ///
    /// The *input_file_name* is used to name the "primary input file". The I/O
    /// system has special hooks for opening this primary input, so be aware
    /// that this filename is *not* opened using the usual mechanisms. This
    /// setting affects some of the names used by the engine internally,
    /// including the name it uses to create its main output files. The
    /// traditional default value is `"texput"`.
    pub fn process(
        &mut self,
        launcher: &mut CoreBridgeLauncher,
        format_file_name: &str,
        input_file_name: &str,
    ) -> Result<TexOutcome> {
        if self.use_rust_backend {
            return launcher.with_global_lock(|state| {
                self.process_rust(state, format_file_name, input_file_name)
            });
        }

        let cformat = CString::new(format_file_name)?;
        let cinput = CString::new(input_file_name)?;

        launcher.with_global_lock(|state| {
            // Note that we have to do all of this setup while holding the
            // lock, because we're modifying static state variables.

            // SAFETY: All methods are called with valid C-strings and while the global lock is held.
            let r = unsafe {
                use c_api::*;
                tt_xetex_set_int_variable(
                    c"shell_escape_enabled".as_ptr(),
                    self.shell_escape_enabled.into(),
                );
                tt_xetex_set_int_variable(c"halt_on_error_p".as_ptr(), self.halt_on_error.into());
                tt_xetex_set_int_variable(c"in_initex_mode".as_ptr(), self.initex_mode.into());
                tt_xetex_set_int_variable(c"synctex_enabled".as_ptr(), self.synctex_enabled.into());
                tt_xetex_set_int_variable(
                    c"semantic_pagination_enabled".as_ptr(),
                    self.semantic_pagination_enabled.into(),
                );

                tt_engine_xetex_main(
                    state,
                    cformat.as_ptr(),
                    cinput.as_ptr(),
                    self.build_date
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("invalid build date")
                        .as_secs(),
                )
            };

            match r {
                0 => Ok(TexOutcome::Spotless),
                1 => Ok(TexOutcome::Warnings),
                2 => Ok(TexOutcome::Errors),
                3 => Err(EngineAbortedError::new_abort_indicator().into()),
                x => Err(anyhow!("internal error: unexpected 'history' value {}", x)),
            }
        })
    }

    /// Runs the pure-Rust [`TeXEngineState`] engine against the bridge's I/O
    /// stack — the [`TeXEngineState::run`]-based equivalent of
    /// `tt_engine_xetex_main`.
    ///
    /// The bridge state is reached through [`BridgePtr`], a raw-pointer
    /// wrapper that satisfies the resolver seams' `Send + Sync` bounds.
    /// The pointer is valid for the duration of the synchronous
    /// [`TeXEngineState::run`] call, which executes on the thread holding
    /// the global engine lock, so no aliased mutable access can occur.
    fn process_rust(
        &self,
        cbs: &mut CoreBridgeState,
        format_file_name: &str,
        input_file_name: &str,
    ) -> Result<TexOutcome> {
        let mut engine = TeXEngineState::new(RUST_ENGINE_MEM_SIZE);
        engine.halt_on_error_p = self.halt_on_error;
        engine.in_initex_mode = self.initex_mode;
        engine.synctex_enabled = self.synctex_enabled;
        engine.semantic_pagination_enabled = self.semantic_pagination_enabled;
        engine.shell_escape_enabled = self.shell_escape_enabled;

        let bridge = BridgePtr::new(cbs);

        // Byte-side metadata cache: the bytes resolver records each open's
        // mtime and absolute path so the metadata resolver can answer
        // without re-opening the file (one provider open per file, as C
        // does in `u_open_in`).
        let meta_cache = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
            String,
            state::InputFileMetadata,
        >::new()));

        {
            let meta_cache = Arc::clone(&meta_cache);
            engine.set_input_resolver(Arc::new(move |name: &str| {
                // SAFETY: the resolver runs only while `run` executes
                // inside `with_global_lock` on this thread.
                let cbs = unsafe { bridge.state() };
                let id = cbs.input_open(name, file_format_for(name), false)?;
                let meta = state::InputFileMetadata {
                    mtime: Some(cbs.input_get_mtime(id)),
                    abspath: cbs.last_input_abspath(),
                };
                meta_cache.lock().unwrap().insert(name.to_string(), meta);
                read_input(cbs, id)
            }));
        }

        {
            engine.set_primary_input_resolver(Arc::new(move |_name: &str| {
                // SAFETY: as above.
                let cbs = unsafe { bridge.state() };
                let id = cbs.input_open_primary()?;
                read_input(cbs, id)
            }));
        }

        {
            let meta_cache = Arc::clone(&meta_cache);
            engine.set_fmt_input_resolver(Arc::new(move |name: &str| {
                // SAFETY: as above. Format images always open with
                // `FileFormat::Format`, matching C's `open_fmt_file`.
                let cbs = unsafe { bridge.state() };
                let id = cbs.input_open(name, FileFormat::Format, false)?;
                let meta = state::InputFileMetadata {
                    mtime: Some(cbs.input_get_mtime(id)),
                    abspath: cbs.last_input_abspath(),
                };
                meta_cache.lock().unwrap().insert(name.to_string(), meta);
                read_input(cbs, id)
            }));
        }

        {
            let meta_cache = Arc::clone(&meta_cache);
            engine.set_input_metadata_resolver(Arc::new(move |name: &str| {
                if let Some(meta) = meta_cache.lock().unwrap().get(name) {
                    return Some(meta.clone());
                }
                // A file queried but never opened for bytes (e.g.
                // `\pdffilemoddate` on an unreferenced file): open it
                // once for metadata as C's `getfilemoddate` does.
                // SAFETY: as above.
                let cbs = unsafe { bridge.state() };
                let id = cbs.input_open(name, file_format_for(name), false)?;
                let meta = state::InputFileMetadata {
                    mtime: Some(cbs.input_get_mtime(id)),
                    abspath: cbs.last_input_abspath(),
                };
                cbs.input_close(id);
                meta_cache
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), meta.clone());
                Some(meta)
            }));
        }

        {
            engine.set_native_font_resolver(Arc::new(move |name: &str| {
                // C's `createFontFromFile` opens the name through the
                // OpenType/TrueType/Type1 formats in that order; the
                // provider's extension fallback covers `name.otf` etc.
                // SAFETY: as above.
                let cbs = unsafe { bridge.state() };
                let id = cbs
                    .input_open(name, FileFormat::OpenType, false)
                    .or_else(|| cbs.input_open(name, FileFormat::TrueType, false))
                    .or_else(|| cbs.input_open(name, FileFormat::Type1, false))?;
                read_input(cbs, id)
            }));
        }

        {
            engine.set_shell_escape_hook(Arc::new(move |command: &str| {
                // SAFETY: as above.
                unsafe { bridge.state() }.shell_escape(command)
            }));
        }

        {
            let wants_names = cbs.wants_expansion_names();
            engine.set_expansion_hook(
                Arc::new(move |ev: &state::ExpansionEvent| {
                    // SAFETY: as above.
                    unsafe { bridge.state() }.expansion_event(
                        ev.name.as_deref(),
                        ev.depth,
                        ev.cur_cmd,
                        ev.cur_chr,
                    );
                }),
                wants_names,
            );
        }

        // C `tt_run_engine` opens the stdout handle before any input
        // processing (`rust_stdout = ttstub_output_open_stdout()`). The
        // driver tracks it under the empty name — which the primary-input
        // handle also carries — so this open must precede the first
        // `input_close` for the driver's `event_input_closed` lookup to
        // find its `FileSummary`.
        let stdout_id = cbs.output_open_stdout();

        let outcome = match engine.run(format_file_name, input_file_name, self.build_date) {
            Ok(history) => match history {
                0 => Ok(TexOutcome::Spotless),
                1 => Ok(TexOutcome::Warnings),
                2 => Ok(TexOutcome::Errors),
                3 => Err(anyhow!("{}", abort_detail(&engine))),
                x => Err(anyhow!("internal error: unexpected 'history' value {}", x)),
            },
            Err(e) => Err(anyhow!("{}", e)),
        };

        flush_rust_outputs(cbs, &mut engine, stdout_id);

        outcome
    }
}

/// TeX main-memory word capacity matching C's `MEM_TOP + 1`.
const RUST_ENGINE_MEM_SIZE: usize = 5_000_000;

/// A `*mut CoreBridgeState` wrapper with `Send`/`Sync` so it can be
/// captured by the engine's `Arc`-shared resolver closures.
///
/// The pointer is created inside [`CoreBridgeLauncher::with_global_lock`]
/// and is only dereferenced while the synchronous
/// [`TeXEngineState::run`] executes on the thread that created it — the
/// pointee `&mut CoreBridgeState` borrow is live for that entire span and
/// the lock serializes engine execution, so no aliased access can occur.
/// This mirrors the trust model of the C ABI, which hands the same
/// pointer to `tt_engine_xetex_main`.
#[derive(Clone, Copy)]
struct BridgePtr(*mut CoreBridgeState<'static>);

impl BridgePtr {
    fn new(state: &mut CoreBridgeState<'_>) -> Self {
        BridgePtr((state as *mut CoreBridgeState<'_>).cast())
    }

    /// # Safety
    ///
    /// Call only while the engine executes synchronously inside
    /// `with_global_lock` on the creating thread.
    #[allow(clippy::mut_from_ref)]
    unsafe fn state(&self) -> &mut CoreBridgeState<'static> {
        // SAFETY: upheld by the caller per the type-level invariant.
        unsafe { &mut *self.0 }
    }
}

// SAFETY: see the type-level invariant on [`BridgePtr`].
unsafe impl Send for BridgePtr {}
// SAFETY: see the type-level invariant on [`BridgePtr`].
unsafe impl Sync for BridgePtr {}

/// The `FileFormat` C would pass to `ttstub_input_open` for a file of the
/// given name, chosen by extension so the bridge's extension-fallback
/// lists match (`open_fmt_file` uses `Format`; everything else reaches
/// `input_open_name_with_abspath`).
fn file_format_for(name: &str) -> FileFormat {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "fmt" => FileFormat::Format,
        "tfm" => FileFormat::Tfm,
        "ofm" => FileFormat::Ofm,
        "ovf" => FileFormat::Ovf,
        "vf" => FileFormat::Vf,
        "afm" => FileFormat::Afm,
        "otf" | "dfont" => FileFormat::OpenType,
        "ttf" | "ttc" => FileFormat::TrueType,
        "pfa" | "pfb" => FileFormat::Type1,
        "enc" => FileFormat::Enc,
        "map" => FileFormat::FontMap,
        "sfd" => FileFormat::Sfd,
        "pk" => FileFormat::Pk,
        "tec" | "cfg" | "cnf" => FileFormat::MiscFonts,
        "pdf" | "png" | "jpg" | "jpeg" | "eps" | "epsi" | "bmp" | "jp2" | "gif" | "tif"
        | "tiff" => FileFormat::Pict,
        "bib" => FileFormat::Bib,
        "bst" => FileFormat::Bst,
        _ => FileFormat::Tex,
    }
}

/// Reads an open input to EOF and closes it, recording the close event
/// (`ttstub_input_read`/`ttstub_input_close` per open). Returns `None` on
/// a read error — the bridge already warned via `status`.
fn read_input(cbs: &mut CoreBridgeState, id: InputId) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    let ok = loop {
        match cbs.input_read_partial(id, &mut buf) {
            Ok(0) => break true,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
            Err(_) => break false,
        }
    };
    cbs.input_close(id);
    ok.then_some(bytes)
}

/// Writes `bytes` to the named output through the bridge
/// (`ttstub_output_open`/`_write`/`_close`), recording the output event.
fn write_output(cbs: &mut CoreBridgeState, name: &str, bytes: &[u8], is_gz: bool) {
    if let Some(id) = cbs.output_open(name, is_gz) {
        let _ = cbs.get_output(id).write_all(bytes);
        cbs.output_close(id);
    }
}

/// Flushes the engine's accumulated output sinks into the driver's I/O
/// stack after `run` completes: terminal text to the stdout handle, the
/// transcript log, `\openout` write streams, the XDV/SPX document,
/// gzipped SyncTeX data, and the INITEX format dump.
fn flush_rust_outputs(
    cbs: &mut CoreBridgeState,
    engine: &mut TeXEngineState,
    stdout_id: Option<OutputId>,
) {
    // Replay captured diagnostics to the status backend, matching C's
    // `ttstub_diag_finish` delivery (streamed live there; batched here).
    // A capture still open at abort time is the most recent diagnostic —
    // C's `ttstub_diag_finish` delivers it too, so drain it last.
    for diag in engine.diag_events.drain(..) {
        match diag.severity {
            state::DiagSeverity::Warning => cbs.issue_warning_diag(&diag.text),
            state::DiagSeverity::Error => cbs.issue_error_diag(&diag.text),
        }
    }
    if let Some(diag) = engine.diag_capture.take() {
        let severity = engine.diag_capture_severity;
        let text = String::from_utf8_lossy(&diag).into_owned();
        match severity {
            state::DiagSeverity::Warning => cbs.issue_warning_diag(&text),
            state::DiagSeverity::Error => cbs.issue_error_diag(&text),
        }
    }

    if let Some(id) = stdout_id {
        if !engine.term_out.is_empty() {
            let _ = cbs.get_output(id).write_all(&engine.term_out);
        }
        cbs.output_close(id);
    } else if !engine.term_out.is_empty() {
        if let Some(id) = cbs.output_open_stdout() {
            let _ = cbs.get_output(id).write_all(&engine.term_out);
            cbs.output_close(id);
        }
    }

    if engine.log_opened {
        if let Some(name) = engine.log_name.clone() {
            write_output(cbs, &name, &engine.log_out, false);
        }
    }

    for stream in engine.write_streams.iter().flatten() {
        write_output(cbs, &stream.name, stream.text.as_bytes(), false);
    }

    if engine.output_file_opened {
        write_output(cbs, &engine.output_file_name, &engine.dvi_bytes, false);
    }

    if let Some(name) = engine.synctex.output_name.clone() {
        write_output(cbs, &name, &engine.synctex.bytes, true);
    }

    if engine.in_initex_mode && !engine.fmt_bytes.is_empty() {
        // `pack_job_name(".fmt")`: the job name is already the stemmed
        // input name — append the extension verbatim.
        let stem = engine
            .job_name
            .clone()
            .unwrap_or_else(|| "texput".to_string());
        write_output(cbs, &format!("{stem}.fmt"), &engine.fmt_bytes, false);
    }
}

/// Builds a human-readable abort message from the engine's diagnostics —
/// the Rust-path equivalent of the C error buffer consumed by
/// [`EngineAbortedError::new_with_details`].
fn abort_detail(engine: &TeXEngineState) -> String {
    // The most recent diagnostic may still be in the open capture.
    if engine.diag_capture_severity == state::DiagSeverity::Error {
        if let Some(diag) = &engine.diag_capture {
            if !diag.is_empty() {
                return String::from_utf8_lossy(diag).into_owned();
            }
        }
    }
    if let Some(diag) = engine
        .diag_events
        .iter()
        .rev()
        .find(|d| d.severity == state::DiagSeverity::Error)
    {
        return diag.text.clone();
    }
    "TeX engine terminated with a fatal error".to_string()
}

#[doc(hidden)]
pub mod c_api {
    // If you change the interfaces here, rerun cbindgen as described in the README!

    use tectonic_bridge_core::CoreBridgeState;

    #[allow(improper_ctypes)] // for CoreBridgeState
    extern "C" {
        pub fn tt_xetex_set_int_variable(
            var_name: *const libc::c_char,
            value: libc::c_int,
        ) -> libc::c_int;

        pub fn tt_engine_xetex_main(
            api: &mut CoreBridgeState,
            dump_name: *const libc::c_char,
            input_file_name: *const libc::c_char,
            build_date: u64,
        ) -> libc::c_int;
    }
}

/// Import things from our bridge crates to ensure that we actually link with
/// them.
mod linkage {
    #[allow(unused_imports)]
    use tectonic_image_probe as clipyrenamehack1;

    #[allow(unused_imports)]
    use tectonic_xetex_layout as clipyrenamehack2;
}

/// Does our resulting executable link correctly?
#[test]
fn linkage() {}

#[cfg(test)]
mod rust_backend_tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;
    use tectonic_bridge_core::{
        DriverHooks, MacroExpansionContext, SecuritySettings, SecurityStance,
    };
    use tectonic_io_base::{InputHandle, InputOrigin, IoProvider, OpenResult, OutputHandle};
    use tectonic_status_base::{MessageKind, NoopStatusBackend, StatusBackend};

    /// A byte sink collecting one output file into the shared map
    /// (`MemoryIo`-equivalent test double).
    struct Sink(std::sync::Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// In-memory `IoProvider`: named inputs from `inputs`, the primary
    /// input from `primary` (like `SingleInputFileIo`), and all outputs
    /// captured into `outputs`.
    struct TestIo {
        inputs: HashMap<String, Vec<u8>>,
        primary: Option<Vec<u8>>,
        outputs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        /// Records every input-open request name (including extension
        /// fallbacks) for resolver-path assertions.
        opened: Arc<Mutex<Vec<String>>>,
    }

    impl IoProvider for TestIo {
        fn output_open_name(&mut self, name: &str) -> OpenResult<OutputHandle> {
            self.outputs
                .lock()
                .unwrap()
                .insert(name.to_string(), Vec::new());
            let inner = CommitSink {
                buf: Sink(Arc::new(Mutex::new(Vec::new()))),
                key: name.to_string(),
                outputs: Arc::clone(&self.outputs),
            };
            OpenResult::Ok(OutputHandle::new(name, inner))
        }

        fn input_open_name(
            &mut self,
            name: &str,
            _status: &mut dyn StatusBackend,
        ) -> OpenResult<InputHandle> {
            self.opened.lock().unwrap().push(name.to_string());
            match self.inputs.get(name) {
                Some(bytes) => OpenResult::Ok(InputHandle::new(
                    name,
                    Cursor::new(bytes.clone()),
                    InputOrigin::Other,
                )),
                None => OpenResult::NotAvailable,
            }
        }

        fn input_open_primary(
            &mut self,
            _status: &mut dyn StatusBackend,
        ) -> OpenResult<InputHandle> {
            match &self.primary {
                Some(bytes) => OpenResult::Ok(InputHandle::new(
                    "texput",
                    Cursor::new(bytes.clone()),
                    InputOrigin::Other,
                )),
                None => OpenResult::NotAvailable,
            }
        }

        fn input_open_format(
            &mut self,
            name: &str,
            status: &mut dyn StatusBackend,
        ) -> OpenResult<InputHandle> {
            self.input_open_name(name, status)
        }
    }

    /// An `OutputHandle` writer that commits buffered bytes into the shared
    /// map entry under `key` on every write.
    struct CommitSink {
        buf: Sink,
        key: String,
        outputs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    impl Write for CommitSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.buf.write(buf)?;
            let snapshot = self.buf.0.lock().unwrap().clone();
            self.outputs
                .lock()
                .unwrap()
                .insert(self.key.clone(), snapshot);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.buf.flush()
        }
    }

    struct TestHooks {
        io: TestIo,
    }

    impl DriverHooks for TestHooks {
        fn io(&mut self) -> &mut dyn IoProvider {
            &mut self.io
        }
    }

    fn test_hooks(primary: &[u8]) -> (TestHooks, Arc<Mutex<HashMap<String, Vec<u8>>>>) {
        let outputs = Arc::new(Mutex::new(HashMap::new()));
        (
            TestHooks {
                io: TestIo {
                    inputs: HashMap::new(),
                    primary: Some(primary.to_vec()),
                    outputs: Arc::clone(&outputs),
                    opened: Arc::new(Mutex::new(Vec::new())),
                },
            },
            outputs,
        )
    }

    /// INITEX through the bridge: `\dump` runs `store_fmt_file` and the
    /// driver flush lands `texput.fmt` in the provider's output sink.
    #[test]
    fn rust_backend_initex_writes_format_through_bridge() {
        let (mut hooks, outputs) = test_hooks(b"\\dump");
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");

        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        let files = outputs.lock().unwrap();
        assert!(
            files.get("texput.fmt").is_some_and(|b| !b.is_empty()),
            "format dump should land in the output sink: {:?}",
            files.keys().collect::<Vec<_>>()
        );
        assert!(files.contains_key("texput.log"));
    }

    /// A `\dump`-only INITEX image has an essentially empty `avail` free
    /// list — faithful to C, which dumps it verbatim. Churn a `\toks`
    /// register so the dumped format carries allocation headroom.
    fn busy_initex_input() -> Vec<u8> {
        let mut job = String::new();
        for _ in 0..50 {
            job.push_str("\\toks0={aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}");
        }
        job.push_str("\\dump");
        job.into_bytes()
    }

    /// Full cutover round-trip: dump a format via the Rust backend, then
    /// serve it through `input_open_format` for a production run.
    #[test]
    fn rust_backend_loads_format_and_ships_page() {
        let (mut hooks, outputs) = test_hooks(&busy_initex_input());
        let mut status = NoopStatusBackend::default();

        {
            let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);
            let r = TexEngine::default()
                .initex_mode(true)
                .rust_backend(true)
                .process(&mut launcher, "UNUSED.fmt", "texput");
            assert_eq!(r.unwrap(), TexOutcome::Spotless);
        }

        let fmt = outputs.lock().unwrap()["texput.fmt"].clone();
        hooks.io.inputs.insert("texput.fmt".to_string(), fmt);
        hooks.io.primary =
            Some(b"\\immediate\\openout1=auxfile.aux \\immediate\\write1{hello}\\end".to_vec());

        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);
        let r =
            TexEngine::default()
                .rust_backend(true)
                .process(&mut launcher, "texput.fmt", "texput");

        let outcome = r.unwrap();
        assert!(matches!(
            outcome,
            TexOutcome::Spotless | TexOutcome::Warnings
        ));
        let files = outputs.lock().unwrap();
        assert!(
            files.contains_key("texput.log"),
            "log file should be written: {:?}",
            files.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            files.get("auxfile.aux").map(|v| v.as_slice()),
            Some(b"hello\n".as_slice()),
            "write stream should flush through output_open_name"
        );
    }

    /// C `start_input` names the job from the primary input stem —
    /// `myjob.tex` produces `myjob.log`/`myjob.xdv`, not `texput.*`.
    #[test]
    fn rust_backend_derives_job_name_from_input() {
        let (mut hooks, outputs) = test_hooks(&busy_initex_input());
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "myjob.tex");

        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        let files = outputs.lock().unwrap();
        assert!(
            files.contains_key("myjob.fmt") && files.contains_key("myjob.log"),
            "outputs should be named for the input stem: {:?}",
            files.keys().collect::<Vec<_>>()
        );
    }

    /// The name-based resolver reaches `.fmt` files through
    /// `input_open_format` (the `FileFormat::Format` branch).
    #[test]
    fn rust_backend_missing_input_fails_cleanly() {
        let (mut hooks, _outputs) = test_hooks(b"");
        hooks.io.primary = None;
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");

        assert!(r.is_err(), "missing primary input must surface an error");
    }

    /// Hooks recording the expansion events and shell-escape requests the
    /// C engine would deliver through `ttstub_event_*`/`ttstub_shell_escape`.
    struct RecordingHooks {
        io: TestIo,
        expand_events: Arc<Mutex<Vec<String>>>,
        shell_calls: Arc<Mutex<Vec<String>>>,
        rich_hook: bool,
    }

    impl DriverHooks for RecordingHooks {
        fn io(&mut self) -> &mut dyn IoProvider {
            &mut self.io
        }
        fn is_macro_expansion_enabled(&self) -> bool {
            self.rich_hook
        }
        fn event_expand_on(&mut self) {
            self.expand_events.lock().unwrap().push("<on>".to_string());
        }
        fn event_macro_expand(&mut self, ctx: &MacroExpansionContext<'_>) {
            self.expand_events
                .lock()
                .unwrap()
                .push(ctx.name.to_string());
        }
        fn sysrq_shell_escape(
            &mut self,
            command: &str,
            _status: &mut dyn StatusBackend,
        ) -> Result<(), tectonic_bridge_core::SystemRequestError> {
            self.shell_calls.lock().unwrap().push(command.to_string());
            Ok(())
        }
    }

    fn recording_hooks(
        primary: &[u8],
        rich_hook: bool,
    ) -> (
        RecordingHooks,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ) {
        let expand_events = Arc::new(Mutex::new(Vec::new()));
        let shell_calls = Arc::new(Mutex::new(Vec::new()));
        let outputs = Arc::new(Mutex::new(HashMap::new()));
        (
            RecordingHooks {
                io: TestIo {
                    inputs: HashMap::new(),
                    primary: Some(primary.to_vec()),
                    outputs: Arc::clone(&outputs),
                    opened: Arc::new(Mutex::new(Vec::new())),
                },
                expand_events: Arc::clone(&expand_events),
                shell_calls: Arc::clone(&shell_calls),
                rich_hook,
            },
            expand_events,
            shell_calls,
            outputs,
        )
    }

    /// `ttstub_event_macro_expand` parity: with the rich hook enabled the
    /// macro name reaches `event_macro_expand`.
    #[test]
    fn rust_backend_fires_rich_expansion_events() {
        let (mut hooks, events, _shell, _outputs) = recording_hooks(b"\\def\\a{x}\\a\\dump", true);
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");
        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        assert!(
            events.lock().unwrap().iter().any(|n| n == "a"),
            "macro expansion should report the cs name: {:?}",
            events.lock().unwrap()
        );
    }

    /// `ttstub_event_expand_on` parity: without the rich hook the
    /// lightweight counter fires instead.
    #[test]
    fn rust_backend_fires_lightweight_expansion_events() {
        let (mut hooks, events, _shell, _outputs) = recording_hooks(b"\\def\\a{x}\\a\\dump", false);
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");
        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        let events = events.lock().unwrap();
        assert!(
            events.iter().all(|n| n == "<on>") && !events.is_empty(),
            "lightweight hook should fire without names: {events:?}"
        );
    }

    /// `ttstub_shell_escape` parity: an enabled `\write18` reaches the
    /// driver's `sysrq_shell_escape` through the security check.
    #[test]
    fn rust_backend_shell_escape_reaches_driver_hook() {
        let (mut hooks, _events, shell_calls, _outputs) =
            recording_hooks(b"\\immediate\\write18{touch foo}\\dump", true);
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new_with_security(
            &mut hooks,
            &mut status,
            SecuritySettings::new(SecurityStance::MaybeAllowInsecures),
        );

        let r = TexEngine::default()
            .initex_mode(true)
            .shell_escape(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");
        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        assert_eq!(
            shell_calls.lock().unwrap().as_slice(),
            &["touch foo"],
            "write18 should execute through sysrq_shell_escape"
        );
    }

    /// Disabled `\write18` emits the `runsystem(...)…disabled.` warning —
    /// replayed through `ttstub_diag`-equivalent status reporting.
    #[test]
    fn rust_backend_disabled_shell_escape_warns() {
        #[derive(Default)]
        struct Rec {
            kinds: Vec<MessageKind>,
            texts: Vec<String>,
        }
        impl StatusBackend for Rec {
            fn report(
                &mut self,
                kind: MessageKind,
                args: std::fmt::Arguments,
                _err: Option<&tectonic_errors::Error>,
            ) {
                self.kinds.push(kind);
                self.texts.push(args.to_string());
            }
            fn dump_error_logs(&mut self, _content: &[u8]) {}
        }

        let (mut hooks, _events, shell_calls, _outputs) =
            recording_hooks(b"\\immediate\\write18{touch foo}\\dump", false);
        let mut status = Rec::default();
        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);

        let r = TexEngine::default()
            .initex_mode(true)
            .rust_backend(true)
            .process(&mut launcher, "UNUSED.fmt", "texput");
        assert_eq!(r.unwrap(), TexOutcome::Spotless);
        assert!(shell_calls.lock().unwrap().is_empty());
        assert!(
            status
                .texts
                .iter()
                .any(|t| t.contains("runsystem(touch foo)...disabled")),
            "disabled shell escape should warn via the status backend: {:?}",
            status.texts
        );
    }

    /// `synctex_enabled` routes the `.synctex.gz` stream through the
    /// provider's gzip output path.
    #[test]
    fn rust_backend_synctex_writes_gzipped_output() {
        let (mut hooks, outputs) = test_hooks(&busy_initex_input());
        let mut status = NoopStatusBackend::default();

        {
            let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);
            let r = TexEngine::default()
                .initex_mode(true)
                .rust_backend(true)
                .process(&mut launcher, "UNUSED.fmt", "texput");
            assert_eq!(r.unwrap(), TexOutcome::Spotless);
        }

        let fmt = outputs.lock().unwrap()["texput.fmt"].clone();
        hooks.io.inputs.insert("texput.fmt".to_string(), fmt);
        // `\shipout` fires `synctex_sheet` unconditionally — the lazy
        // `.synctex.gz` open happens there, not at `\end`.
        hooks.io.primary = Some(b"\\shipout\\vbox{x}\\end".to_vec());

        let mut launcher = CoreBridgeLauncher::new(&mut hooks, &mut status);
        let r = TexEngine::default()
            .synctex(true)
            .rust_backend(true)
            .process(&mut launcher, "texput.fmt", "texput");
        assert!(r.is_ok());
        let files = outputs.lock().unwrap();
        let synctex = files
            .get("texput.synctex.gz")
            .expect("synctex output should be written");
        assert_eq!(
            &synctex[..2],
            &[0x1f, 0x8b],
            "synctex stream should be gzip-wrapped"
        );
    }
}
