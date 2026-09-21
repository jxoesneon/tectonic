// Copyright 2020-2021 the Tectonic Project
// Licensed under the MIT License.

//! Font handling - specific fonts used by an engine while shaping text.

use crate::c_api::{GlyphBBox, GlyphID, PlatformFontRef};
use std::ffi::{CStr, CString};
use std::str::FromStr;
use std::sync::Arc;
use tectonic_bridge_core::{CoreBridgeState, FileFormat};
#[cfg(not(target_os = "macos"))]
use tectonic_bridge_fontconfig as fc;
use tectonic_bridge_graphite2 as gr;
#[cfg(target_os = "macos")]
use tectonic_mac_core::{
    CFArray, CFDictionary, CFType, CFUrl, CTFont, CTFontDescriptor, FontAttribute, FontNameKey,
};

struct FontDataHandle {
    data: Arc<Vec<u8>>,
    index: u32,
}

unsafe extern "C" fn graphite_get_table(
    app_face_handle: *const libc::c_void,
    name: libc::c_uint,
    len: *mut libc::size_t,
) -> *const libc::c_void {
    let handle = &*(app_face_handle as *const FontDataHandle);
    let tag = ttf_parser::Tag::from_bytes(&name.to_be_bytes());
    if let Ok(face) = ttf_parser::Face::parse(&handle.data, handle.index) {
        if let Some(table) = face.raw_face().table(tag) {
            *len = table.len();
            return table.as_ptr() as *const libc::c_void;
        }
    }
    *len = 0;
    std::ptr::null()
}

static GRAPHITE_OPS: gr::sys::gr_face_ops = gr::sys::gr_face_ops {
    size: std::mem::size_of::<gr::sys::gr_face_ops>(),
    get_table: graphite_get_table,
    release_table: None,
};

pub(crate) fn larger_script_list<'a>(
    face: &'a ttf_parser::Face<'a>,
) -> Option<ttf_parser::opentype_layout::ScriptList<'a>> {
    let sub = face.tables().gsub.map(|t| t.scripts);
    let pos = face.tables().gpos.map(|t| t.scripts);
    match (sub, pos) {
        (Some(s), Some(p)) => {
            if s.len() >= p.len() {
                Some(s)
            } else {
                Some(p)
            }
        }
        (Some(s), None) => Some(s),
        (None, Some(p)) => Some(p),
        (None, None) => None,
    }
}

enum FontKind {
    TtfFont,
    #[cfg(target_os = "macos")]
    Mac(CTFontDescriptor, Option<CTFont>),
}

/// The GPOS `size` feature's parameters (`OpSizeRec`): optical design
/// size and usable range in TeX points.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpSize {
    /// The design size in TeX points.
    pub design_size: f64,
    /// The lower end of the recommended range in TeX points.
    pub min_size: f64,
    /// The upper end of the recommended range in TeX points.
    pub max_size: f64,
    /// The subfamily identifier from the `size` parameters.
    pub sub_family_id: u32,
    /// The name-table entry identifying the subfamily.
    pub name_code: u32,
}

/// A font, with all the information needed by TeX to shape it for typesetting.
pub struct Font {
    units_per_em: u16,
    point_size: f32,
    ascent: f32,
    descent: f32,
    cap_height: f32,
    x_height: f32,
    italic_angle: f32,

    vertical: bool,

    filename: CString,
    index: u32,

    data: Option<Arc<Vec<u8>>>,
    gr_face: Option<Arc<gr::Face>>,
    #[allow(dead_code)]
    gr_handle: Option<Box<FontDataHandle>>,

    #[allow(dead_code)]
    kind: FontKind,
}

impl Font {
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn new(font: PlatformFontRef, point_size: f32) -> Result<Font, ()> {
        let path = font
            .as_ref()
            .get::<fc::pat::File>(0)
            .and_then(|s| s.to_str().map_err(|_| fc::FcErr::NoMatch))
            .ok();
        let index = font.as_ref().get::<fc::pat::Index>(0).unwrap_or(0);

        Font::new_path_index(path, index as usize, point_size)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn new(descriptor: PlatformFontRef, point_size: f32) -> Result<Font, ()> {
        let mut out = Font {
            units_per_em: 0,
            point_size,
            ascent: 0.0,
            descent: 0.0,
            cap_height: 0.0,
            x_height: 0.0,
            italic_angle: 0.0,
            vertical: false,
            filename: CString::new("").unwrap(),
            index: 0,
            data: None,
            gr_face: None,
            gr_handle: None,
            kind: FontKind::Mac(descriptor, None),
        };
        out.initialize_mac()?;
        Ok(out)
    }

    pub(crate) fn new_path_index(
        path: Option<&str>,
        index: usize,
        point_size: f32,
    ) -> Result<Font, ()> {
        let mut out = Font {
            units_per_em: 0,
            point_size,
            ascent: 0.0,
            descent: 0.0,
            cap_height: 0.0,
            x_height: 0.0,
            italic_angle: 0.0,
            vertical: false,
            filename: CString::new("").unwrap(),
            index: 0,
            data: None,
            gr_face: None,
            gr_handle: None,
            kind: FontKind::TtfFont,
        };
        if let Some(path) = path {
            out.initialize(path, index)?;
        }
        Ok(out)
    }

    fn initialize(&mut self, pathname: &str, index: usize) -> Result<(), ()> {
        let backing_data = CoreBridgeState::with_global_state(|engine| {
            let handle = engine
                .input_open(pathname, FileFormat::OpenType, false)
                .or_else(|| engine.input_open(pathname, FileFormat::TrueType, false))
                .or_else(|| engine.input_open(pathname, FileFormat::Type1, false));
            let Some(handle) = handle else {
                return Err(());
            };

            let sz = engine.input_get_size(handle);
            let mut data = vec![0; sz];
            engine
                .input_read(handle, &mut data)
                .expect("failed to read font file");

            engine.input_close(handle);
            Ok(data)
        })?;

        self.initialize_data(Arc::new(backing_data), pathname, index)
    }

    /// Creates a font directly from in-memory file bytes, without going
    /// through the bridge's global I/O state — used by the pure-Rust
    /// engine path, which resolves font bytes through its own resolver
    /// seam. `filename` is the name the font was requested under; it is
    /// what native-font XDV definitions carry.
    pub fn new_data(data: Vec<u8>, filename: &str, index: u32, point_size: f32) -> Option<Font> {
        Self::new_shared(Arc::new(data), filename, index, point_size)
    }

    /// Like [`Font::new_data`] but takes shared ownership of the font
    /// bytes, avoiding a copy when the caller already holds an `Arc`.
    pub fn new_shared(
        data: Arc<Vec<u8>>,
        filename: &str,
        index: u32,
        point_size: f32,
    ) -> Option<Font> {
        let mut out = Font {
            units_per_em: 0,
            point_size,
            ascent: 0.0,
            descent: 0.0,
            cap_height: 0.0,
            x_height: 0.0,
            italic_angle: 0.0,
            vertical: false,
            filename: CString::new("").unwrap(),
            index: 0,
            data: None,
            gr_face: None,
            gr_handle: None,
            kind: FontKind::TtfFont,
        };
        out.initialize_data(data, filename, index as usize).ok()?;
        Some(out)
    }

    fn initialize_data(
        &mut self,
        data: Arc<Vec<u8>>,
        filename: &str,
        index: usize,
    ) -> Result<(), ()> {
        let face = ttf_parser::Face::parse(&data, index as u32).map_err(|_| ())?;

        self.filename = CString::from_str(filename).map_err(|_| ())?;
        self.index = index as u32;
        self.units_per_em = face.units_per_em();
        self.ascent = self.units_to_points(face.ascender() as f64) as f32;
        self.descent = self.units_to_points(face.descender() as f64) as f32;
        self.italic_angle = face.italic_angle();
        self.cap_height = face
            .capital_height()
            .map(|ch| self.units_to_points(ch as f64) as f32)
            .unwrap_or(0.0);
        self.x_height = face
            .x_height()
            .map(|xh| self.units_to_points(xh as f64) as f32)
            .unwrap_or(0.0);

        let has_silf = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"Silf"))
            .is_some();

        self.data = Some(data);

        // Setup Graphite if available
        if has_silf {
            let handle = Box::new(FontDataHandle {
                data: Arc::clone(self.data.as_ref().unwrap()),
                index: self.index,
            });
            let handle_ptr = &*handle as *const FontDataHandle as *const libc::c_void;
            // SAFETY: handle_ptr and GRAPHITE_OPS are valid callbacks and state.
            let gr_face = unsafe {
                gr::Face::new_with_ops(handle_ptr, &GRAPHITE_OPS, gr::sys::gr_face_preloadAll)
            };
            self.gr_face = gr_face.map(Arc::new);
            self.gr_handle = Some(handle);
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn initialize_mac(&mut self) -> Result<(), ()> {
        let FontKind::Mac(descriptor, font_ref) = &mut self.kind else {
            return Err(());
        };

        let empty_cascade_list = CFArray::<CFType>::empty();
        let attributes =
            CFDictionary::new([(FontAttribute::CascadeList.to_str(), empty_cascade_list)]);

        *descriptor = descriptor.copy_with_attrs(&attributes);
        *font_ref = Some(CTFont::new_descriptor(
            descriptor,
            self.point_size as f64 * 72.0 / 72.27,
        ));
        let mut index = 0;
        let pathname = get_file_name_from_ct_font(font_ref.as_ref().unwrap(), &mut index).unwrap();
        self.initialize(pathname.to_str().unwrap(), index as usize)
    }

    /// The parsed `ttf_parser` face for this font, if backed by SFNT data.
    pub fn ttf_face(&self) -> Option<ttf_parser::Face<'_>> {
        self.data
            .as_ref()
            .and_then(|d| ttf_parser::Face::parse(d, self.index).ok())
    }

    /// The raw backing bytes of the font file, when held in memory.
    pub fn data(&self) -> Option<&[u8]> {
        self.data.as_deref().map(|v| v.as_slice())
    }

    /// The face index selected within the font file (for TTC/dfont).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The Graphite face handle, when the font carries a Silf table.
    pub fn gr_face(&self) -> Option<gr::FaceRef<'_>> {
        self.gr_face.as_ref().map(|f| gr::Face::as_ref(f))
    }

    /// A raw pointer to the Graphite face for FFI use.
    pub fn gr_face_ptr(&self) -> Option<std::ptr::NonNull<()>> {
        self.gr_face.as_ref().map(|f| f.raw_ptr())
    }

    /// Returns the PostScript name of glyph `gid`.
    pub fn get_glyph_name(&self, gid: u16) -> Option<CString> {
        let face = self.ttf_face()?;
        let name = face.glyph_name(ttf_parser::GlyphId(gid))?;
        CString::new(name).ok()
    }

    /// Returns the left and right sidebearings of `gid` in points.
    pub fn get_glyph_sidebearings(
        &mut self,
        gid: GlyphID,
        lsb: Option<&mut f32>,
        rsb: Option<&mut f32>,
    ) {
        let width = self.get_glyph_width(gid as u32);
        let bbox = self.get_glyph_bounds(gid);

        if let Some(lsb) = lsb {
            *lsb = bbox.x_min;
        }
        if let Some(rsb) = rsb {
            *rsb = width - bbox.x_max;
        }
    }

    /// Returns the italic correction of `gid` in points.
    pub fn get_glyph_ital_corr(&mut self, gid: GlyphID) -> f32 {
        let width = self.get_glyph_width(gid as u32);
        let bbox = self.get_glyph_bounds(gid);

        if bbox.x_max > width {
            bbox.x_max - width
        } else {
            0.0
        }
    }

    /// Maps a Unicode scalar to a glyph id (`mapCharToGlyph`), 0 if unmapped.
    pub fn map_char_to_glyph(&self, ch: u32) -> GlyphID {
        if let Some(face) = self.ttf_face() {
            if let Some(c) = char::from_u32(ch) {
                if let Some(gid) = face.glyph_index(c) {
                    return gid.0;
                }
            }
        }
        0
    }

    /// The lowest codepoint covered by the font's Unicode cmap.
    pub fn first_char_code(&self) -> u32 {
        let mut min_cp = u32::MAX;
        if let Some(face) = self.ttf_face() {
            if let Some(cmap) = face.tables().cmap {
                for sub in cmap.subtables {
                    if sub.is_unicode() {
                        sub.codepoints(|cp| {
                            if cp < min_cp {
                                min_cp = cp;
                            }
                        });
                    }
                }
            }
        }
        if min_cp == u32::MAX {
            0
        } else {
            min_cp
        }
    }

    /// The highest codepoint covered by the font's Unicode cmap.
    pub fn last_char_code(&self) -> u32 {
        let mut max_cp = 0;
        if let Some(face) = self.ttf_face() {
            if let Some(cmap) = face.tables().cmap {
                for sub in cmap.subtables {
                    if sub.is_unicode() {
                        sub.codepoints(|cp| {
                            if cp > max_cp {
                                max_cp = cp;
                            }
                        });
                    }
                }
            }
        }
        max_cp
    }

    /// Enumerates every Unicode codepoint→glyph-id pair across the
    /// font's Unicode `cmap` subtables. Used by the Rust engine to
    /// populate its `char_to_glyph` map eagerly.
    pub fn cmap_mappings(&self) -> Vec<(u32, u16)> {
        let mut out = Vec::new();
        if let Some(face) = self.ttf_face() {
            if let Some(cmap) = face.tables().cmap {
                for sub in cmap.subtables {
                    if !sub.is_unicode() {
                        continue;
                    }
                    sub.codepoints(|cp| {
                        if let Some(c) = char::from_u32(cp) {
                            if let Some(gid) = face.glyph_index(c) {
                                if gid.0 != 0 {
                                    out.push((cp, gid.0));
                                }
                            }
                        }
                    });
                }
            }
        }
        out.sort_unstable();
        out.dedup_by_key(|pair| pair.0);
        out
    }

    /// The font's design size in TeX points, from the GPOS `size`
    /// feature's parameters (`getOpSize`); defaults to 10.0 like C's
    /// `getDesignSize` when the feature is absent.
    pub fn design_size(&self) -> f64 {
        self.op_size().map(|rec| rec.design_size).unwrap_or(10.0)
    }

    /// Reads the GPOS `size` feature record (`getOpSize`): design size,
    /// subfamily id, menu-name code, and min/max size range in TeX
    /// points. `None` when the font lacks a `size` feature.
    pub fn op_size(&self) -> Option<OpSize> {
        let face = self.ttf_face()?;
        let data = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"GPOS"))?;
        if data.len() < 10 {
            return None;
        }
        let feature_list_offset = u16::from_be_bytes([data[6], data[7]]) as usize;
        let feat_data = data.get(feature_list_offset..)?;
        if feat_data.len() < 2 {
            return None;
        }
        let feature_count = u16::from_be_bytes([feat_data[0], feat_data[1]]) as usize;
        for i in 0..feature_count {
            let rec = feat_data.get(2 + i * 6..2 + (i + 1) * 6)?;
            if &rec[0..4] == b"size" {
                let offset = u16::from_be_bytes([rec[4], rec[5]]) as usize;
                let table = feat_data.get(offset..)?;
                if table.len() < 2 {
                    return None;
                }
                let params_offset = u16::from_be_bytes([table[0], table[1]]) as usize;
                if params_offset == 0 {
                    return None;
                }
                let params = table.get(params_offset..)?;
                if params.len() < 10 {
                    return None;
                }
                return Some(OpSize {
                    sub_family_id: u16::from_be_bytes([params[2], params[3]]) as u32,
                    name_code: u16::from_be_bytes([params[4], params[5]]) as u32,
                    design_size: u16::from_be_bytes([params[0], params[1]]) as f64 * 72.27
                        / 72.0
                        / 10.0,
                    min_size: u16::from_be_bytes([params[6], params[7]]) as f64 * 72.27
                        / 72.0
                        / 10.0,
                    max_size: u16::from_be_bytes([params[8], params[9]]) as f64 * 72.27
                        / 72.0
                        / 10.0,
                });
            }
        }
        None
    }

    /// Maps a PostScript glyph name to a glyph id (`mapGlyphToIndex`).
    pub fn map_glyph_to_index(&self, glyph_name: &CStr) -> GlyphID {
        if let Some(face) = self.ttf_face() {
            if let Ok(name_str) = glyph_name.to_str() {
                for i in 0..face.number_of_glyphs() {
                    let gid = ttf_parser::GlyphId(i);
                    if face.glyph_name(gid) == Some(name_str) {
                        return i;
                    }
                }
            }
        }
        0
    }

    /// Returns the raw bytes of SFNT table `tag`, when present.
    #[allow(dead_code)]
    pub fn load_font_table(&self, tag: u32) -> Option<Vec<u8>> {
        let tag = ttf_parser::Tag::from_bytes(&tag.to_be_bytes());
        self.ttf_face()
            .and_then(|f| f.raw_face().table(tag))
            .map(|s| s.to_vec())
    }

    /// Whether the font carries the SFNT table `tag` (e.g. `MATH`).
    pub fn has_font_table(&self, tag: u32) -> bool {
        let tag = ttf_parser::Tag::from_bytes(&tag.to_be_bytes());
        self.ttf_face()
            .map(|f| f.raw_face().table(tag).is_some())
            .unwrap_or(false)
    }

    /// The bounding box of `gid` in points (`getGlyphBounds`).
    pub fn get_glyph_bounds(&mut self, gid: GlyphID) -> GlyphBBox {
        if let Some(face) = self.ttf_face() {
            if let Some(rect) = face.glyph_bounding_box(ttf_parser::GlyphId(gid)) {
                return GlyphBBox {
                    x_min: self.units_to_points(rect.x_min as f64) as f32,
                    y_min: self.units_to_points(rect.y_min as f64) as f32,
                    x_max: self.units_to_points(rect.x_max as f64) as f32,
                    y_max: self.units_to_points(rect.y_max as f64) as f32,
                };
            }
        }
        GlyphBBox::default()
    }

    /// Writes `gid`'s height and depth in points (`getGlyphHeightDepth`).
    pub fn get_glyph_height_depth(
        &mut self,
        gid: GlyphID,
        height: Option<&mut f32>,
        depth: Option<&mut f32>,
    ) {
        let bbox = self.get_glyph_bounds(gid);
        if let Some(height) = height {
            *height = bbox.y_max;
        }
        if let Some(depth) = depth {
            *depth = -bbox.y_min;
        }
    }

    /// The file name this font was loaded under; writes the face index to `index`.
    pub fn filename(&self, index: &mut u32) -> &CStr {
        *index = self.index;
        &self.filename
    }

    /// The font's italic angle in degrees.
    pub fn italic_angle(&self) -> f32 {
        self.italic_angle
    }

    /// The number of glyphs in the font.
    pub fn num_glyphs(&self) -> usize {
        self.ttf_face()
            .map(|f| f.number_of_glyphs() as usize)
            .unwrap_or(0)
    }

    /// The horizontal advance of `gid` in points at this font's size.
    pub fn get_glyph_width(&self, gid: u32) -> f32 {
        if let Some(face) = self.ttf_face() {
            if let Some(adv) = face.glyph_hor_advance(ttf_parser::GlyphId(gid as u16)) {
                return self.units_to_points(adv as f64) as f32;
            }
        }
        0.0
    }

    /// Whether the layout direction is vertical.
    pub fn layout_dir_vertical(&self) -> bool {
        self.vertical
    }

    /// Sets the layout direction to vertical when `vertical` is true.
    pub fn set_layout_dir_vertical(&mut self, vertical: bool) {
        self.vertical = vertical;
    }

    /// The point size this font instance was created at.
    pub fn point_size(&self) -> f32 {
        self.point_size
    }

    /// The font ascent in points at this size.
    pub fn ascent(&self) -> f32 {
        self.ascent
    }

    /// The font descent in points at this size (negative below baseline).
    pub fn descent(&self) -> f32 {
        self.descent
    }

    /// The cap height in points at this size, or 0 if the font lacks one.
    pub fn cap_height(&self) -> f32 {
        self.cap_height
    }

    /// The x-height in points at this size, or 0 if the font lacks one.
    pub fn x_height(&self) -> f32 {
        self.x_height
    }

    /// Converts font units to points at this size.
    pub fn units_to_points(&self, units: f64) -> f64 {
        (units * self.point_size as f64) / (self.units_per_em as f64)
    }

    /// Converts points to font units at this size.
    pub fn points_to_units(&self, points: f64) -> f64 {
        (points * (self.units_per_em as f64)) / self.point_size as f64
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn get_file_name_from_ct_font(ct_font: &CTFont, index: &mut u32) -> Option<CString> {
    let url = ct_font
        .attr(FontAttribute::URL)
        // SAFETY: CFUrl has no generic parameters
        .and_then(|t| unsafe { t.downcast::<CFUrl>() }.ok())?;

    let pathname = url.fs_representation()?;
    *index = 0;

    let path_str = pathname.to_str().ok()?;
    let data = std::fs::read(path_str).ok()?;
    let num_faces = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    if num_faces > 1 {
        let ps_name1 = ct_font.name(FontNameKey::PostScript);
        *index = u32::MAX;
        for i in 0..num_faces {
            if let Ok(face) = ttf_parser::Face::parse(&data, i) {
                let ps_name2 = face
                    .names()
                    .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                    .and_then(|n| n.to_string());
                match (&ps_name1, ps_name2.as_deref()) {
                    (None, None) => {
                        *index = i;
                        break;
                    }
                    (Some(name1), Some(name2))
                        if name1
                            .as_cstr()
                            .is_some_and(|name1| name1.to_bytes() == name2.as_bytes()) =>
                    {
                        *index = i;
                        break;
                    }
                    _ => (),
                }
            }
        }
    }

    if *index != u32::MAX {
        Some(pathname)
    } else {
        None
    }
}
