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

        let data = Arc::new(backing_data);
        let face = ttf_parser::Face::parse(&data, index as u32).map_err(|_| ())?;

        self.filename = CString::from_str(pathname).unwrap();
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

        let has_silf = face.raw_face().table(ttf_parser::Tag::from_bytes(b"Silf")).is_some();

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
                gr::Face::new_with_ops(
                    handle_ptr,
                    &GRAPHITE_OPS,
                    gr::sys::gr_face_preloadAll,
                )
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

    pub(crate) fn ttf_face(&self) -> Option<ttf_parser::Face<'_>> {
        self.data
            .as_ref()
            .and_then(|d| ttf_parser::Face::parse(d, self.index).ok())
    }

    pub(crate) fn data(&self) -> Option<&[u8]> {
        self.data.as_deref().map(|v| v.as_slice())
    }

    pub(crate) fn index(&self) -> u32 {
        self.index
    }

    pub(crate) fn gr_face(&self) -> Option<gr::FaceRef<'_>> {
        self.gr_face.as_ref().map(|f| gr::Face::as_ref(f))
    }

    pub(crate) fn gr_face_ptr(&self) -> Option<std::ptr::NonNull<()>> {
        self.gr_face.as_ref().map(|f| f.raw_ptr())
    }

    pub(crate) fn get_glyph_name(&self, gid: u16) -> Option<CString> {
        let face = self.ttf_face()?;
        let name = face.glyph_name(ttf_parser::GlyphId(gid))?;
        CString::new(name).ok()
    }

    pub(crate) fn get_glyph_sidebearings(
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

    pub(crate) fn get_glyph_ital_corr(&mut self, gid: GlyphID) -> f32 {
        let width = self.get_glyph_width(gid as u32);
        let bbox = self.get_glyph_bounds(gid);

        if bbox.x_max > width {
            bbox.x_max - width
        } else {
            0.0
        }
    }

    pub(crate) fn map_char_to_glyph(&self, ch: u32) -> GlyphID {
        if let Some(face) = self.ttf_face() {
            if let Some(c) = char::from_u32(ch) {
                if let Some(gid) = face.glyph_index(c) {
                    return gid.0;
                }
            }
        }
        0
    }

    pub(crate) fn first_char_code(&self) -> u32 {
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

    pub(crate) fn last_char_code(&self) -> u32 {
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

    pub(crate) fn map_glyph_to_index(&self, glyph_name: &CStr) -> GlyphID {
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

    #[allow(dead_code)]
    pub(crate) fn load_font_table(&self, tag: u32) -> Option<Vec<u8>> {
        let tag = ttf_parser::Tag::from_bytes(&tag.to_be_bytes());
        self.ttf_face().and_then(|f| f.raw_face().table(tag)).map(|s| s.to_vec())
    }

    pub(crate) fn has_font_table(&self, tag: u32) -> bool {
        let tag = ttf_parser::Tag::from_bytes(&tag.to_be_bytes());
        self.ttf_face().map(|f| f.raw_face().table(tag).is_some()).unwrap_or(false)
    }

    pub(crate) fn get_glyph_bounds(&mut self, gid: GlyphID) -> GlyphBBox {
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

    pub(crate) fn get_glyph_height_depth(
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

    pub(crate) fn filename(&self, index: &mut u32) -> &CStr {
        *index = self.index;
        &self.filename
    }

    pub(crate) fn italic_angle(&self) -> f32 {
        self.italic_angle
    }

    pub(crate) fn num_glyphs(&self) -> usize {
        self.ttf_face()
            .map(|f| f.number_of_glyphs() as usize)
            .unwrap_or(0)
    }

    pub(crate) fn get_glyph_width(&self, gid: u32) -> f32 {
        if let Some(face) = self.ttf_face() {
            if let Some(adv) = face.glyph_hor_advance(ttf_parser::GlyphId(gid as u16)) {
                return self.units_to_points(adv as f64) as f32;
            }
        }
        0.0
    }

    pub(crate) fn layout_dir_vertical(&self) -> bool {
        self.vertical
    }

    pub(crate) fn set_layout_dir_vertical(&mut self, vertical: bool) {
        self.vertical = vertical;
    }

    pub(crate) fn point_size(&self) -> f32 {
        self.point_size
    }

    pub(crate) fn ascent(&self) -> f32 {
        self.ascent
    }

    pub(crate) fn descent(&self) -> f32 {
        self.descent
    }

    pub(crate) fn cap_height(&self) -> f32 {
        self.cap_height
    }

    pub(crate) fn x_height(&self) -> f32 {
        self.x_height
    }

    pub(crate) fn units_to_points(&self, units: f64) -> f64 {
        (units * self.point_size as f64) / (self.units_per_em as f64)
    }

    pub(crate) fn points_to_units(&self, points: f64) -> f64 {
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
                let ps_name2 = face.names().find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME).and_then(|n| n.to_string());
                match (&ps_name1, ps_name2.as_deref()) {
                    (None, None) => {
                        *index = i;
                        break;
                    }
                    (Some(name1), Some(name2))
                        if name1.as_cstr().is_some_and(|name1| name1.to_bytes() == name2.as_bytes()) =>
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
