//! Native font machinery ported from `xetex-ext.c` and the `load_native_font`
//!/`read_font_info` dispatch in `xetex0.c`.
//!
//! This module is the TeX-side font pipeline: `splitFontName` decomposition,
//! feature-string parsing (`readCommonFeatures`/`readFeatureNumber`), the
//! `find_native_font`/`loadOTfont`/`load_native_font` sequence, TECkit mapping
//! seams (`check_for_tfm_font_mapping`/`load_tfm_font_mapping`/`apply_mapping`),
//! and the diagnostic/reporting helpers (`font_feature_warning`,
//! `font_mapping_warning`, `print_glyph_name`, `get_native_word_cp`).
//!
//! The platform font manager and shaping engine remain behind the registered
//! `font_sources`/`font_engine_values`/`tec_sources` seams so the engine keeps
//! zero global state and stays re-entrant; `font_specs` records the parsed
//! `FontSpec` per font where C keeps a `XeTeXLayoutEngine` pointer.

use crate::state::*;
use crate::TeXMouthError;

/// HarfBuzz-style feature setting (`hb_feature_t`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFeature {
    /// Four-byte OpenType feature tag.
    pub tag: u32,
    /// Feature value (`0` = disabled, `1` = enabled, `n` = selector index).
    pub value: i32,
    /// First cluster the feature applies to.
    pub start: u32,
    /// Last cluster the feature applies to (`u32::MAX` = to end).
    pub end: u32,
}

/// Parsed native font specification: the decoded product of `loadOTfont`'s
/// feature-string scan. Stored per font in `TeXEngineState::font_specs` where
/// C stores a `XeTeXLayoutEngine` pointer.
#[derive(Clone, Debug)]
pub struct FontSpec {
    /// `hb_tag_t` script code (`HB_TAG_NONE` = 0 when unset).
    pub script: u32,
    /// `language=` value bytes (`BCP 47`-style tag as raw bytes).
    pub language: Vec<u8>,
    /// Requested shaper names in order (`shaper=` values plus the
    /// `reqEngine`-derived default).
    pub shapers: Vec<Vec<u8>>,
    /// Ordered feature settings.
    pub features: Vec<FontFeature>,
    /// `extend=` horizontal scale (`1.0` default).
    pub extend: f32,
    /// `slant=` obliquing factor (`0.0` default).
    pub slant: f32,
    /// `embolden=` synthetic-bold factor, scaled to font units (`0.0`).
    pub embolden: f32,
    /// `letterspace=` factor (`0.0`).
    pub letterspace: f32,
    /// `color=` RGBA value; `0x000000FF` when unset or not colored.
    pub rgb: u32,
    /// `loaded_font_flags` bits (`FONT_FLAGS_COLORED`/`FONT_FLAGS_VERTICAL`).
    pub flags: u16,
    /// `mapping=` name when a text mapping was attached.
    pub mapping: Option<String>,
    /// `[file:idx]` face index (`splitFontName` `*index` output).
    pub face_index: u32,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            script: 0,
            language: Vec::new(),
            shapers: Vec::new(),
            features: Vec::new(),
            extend: 1.0,
            slant: 0.0,
            embolden: 0.0,
            letterspace: 0.0,
            rgb: 0x0000_00FF,
            flags: 0,
            mapping: None,
            face_index: 0,
        }
    }
}

/// A registered text/byte mapping (`TECkit_Converter` seam): the decoded
/// product of a `.tec` compiled mapping file. `apply_mapping` and
/// `apply_tfm_font_mapping` consume it.
#[derive(Clone, Debug)]
pub struct FontMapping {
    /// Whether the converter was created in `byteMapping` mode
    /// (`kForm_Bytes` output over the reverse pipeline, for TFM fonts).
    pub byte_mapping: bool,
    /// The decoded converter pipeline.
    pub converter: crate::teckit::Converter,
}

/// Result of `splitFontName`: the decomposed font specification string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitFontName {
    /// Font name or `[file]` path (leading `[` retained, as in C).
    pub name: Vec<u8>,
    /// Variant/style string between the `/` or `:` separator and the feature
    /// string (`/B`, `/AAT`, `/OT`, `/GR`, `index` …), without the separator.
    pub variant: Vec<u8>,
    /// Feature string after the last separator, without the separator.
    pub features: Vec<u8>,
    /// Face index for `[file:idx]` forms (`*index` output).
    pub index: i32,
}

/// `D2Fix` — IEEE double to 16.16 fixed point.
pub fn d2fix(d: f64) -> i32 {
    (d * 65_536.0) as i32
}

/// `Fix2D` — 16.16 fixed point to IEEE double.
pub fn fix2d(f: i32) -> f64 {
    f64::from(f) / 65_536.0
}

/// `hb_tag_from_string`: packs up to four bytes into a big-endian tag,
/// space-padding shorter strings.
pub fn hb_tag_from_string(s: &[u8]) -> u32 {
    let mut tag = 0u32;
    for i in 0..4 {
        tag = (tag << 8) | u32::from(s.get(i).copied().unwrap_or(b' ').min(127));
    }
    tag
}

/// `splitFontName`: decomposes a font specification into name, variant,
/// feature-string tail, and `[file:idx]` face index. Faithful to the C
/// scan: `[`...`]` names treat the first interior `:` as the index slot and
/// the first `:` after `]` as the feature separator; unbracketed names use
/// the first `/` for the variant and the first `:` for features.
pub fn split_font_name(name: &[u8]) -> SplitFontName {
    let mut var: Option<usize> = None;
    let mut feat: Option<usize> = None;
    let mut index = 0i32;
    let mut end = name.len();

    if name.first() == Some(&b'[') {
        let mut within_file_name = true;
        let mut i = 1;
        while i < name.len() {
            if within_file_name && name[i] == b']' {
                within_file_name = false;
                if var.is_none() {
                    var = Some(i);
                }
            } else if name[i] == b':' {
                if within_file_name && var.is_none() {
                    var = Some(i);
                    i += 1;
                    while i < name.len() && name[i].is_ascii_digit() {
                        index = index * 10 + i32::from(name[i] - b'0');
                        i += 1;
                    }
                    continue;
                } else if !within_file_name && feat.is_none() {
                    feat = Some(i);
                }
            }
            i += 1;
        }
    } else {
        for (i, &c) in name.iter().enumerate() {
            if c == b'/' && var.is_none() && feat.is_none() {
                var = Some(i);
            } else if c == b':' && feat.is_none() {
                feat = Some(i);
            }
        }
    }
    end = end.min(name.len());

    let feat_pos = feat.unwrap_or(end);
    let var_pos = var.unwrap_or(feat_pos);
    SplitFontName {
        name: name[..var_pos].to_vec(),
        variant: if feat_pos > var_pos {
            name[var_pos + 1..feat_pos].to_vec()
        } else {
            Vec::new()
        },
        features: if end > feat_pos {
            name[feat_pos + 1..end].to_vec()
        } else {
            Vec::new()
        },
        index,
    }
}

/// `strstartswith`: returns the tail of `s` after `prefix` when present.
fn str_starts_with<'a>(s: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    s.strip_prefix(prefix)
}

/// `read_double`: scans a decimal fraction, consuming leading whitespace and
/// an optional sign. Advances `pos` past the number.
pub fn read_double(s: &[u8], pos: &mut usize) -> f64 {
    let mut cp = *pos;
    while cp < s.len() && (s[cp] == b' ' || s[cp] == b'\t') {
        cp += 1;
    }
    let mut neg = false;
    if cp < s.len() {
        if s[cp] == b'-' {
            neg = true;
            cp += 1;
        } else if s[cp] == b'+' {
            cp += 1;
        }
    }
    let mut val = 0.0f64;
    while cp < s.len() && s[cp].is_ascii_digit() {
        val = val * 10.0 + f64::from(s[cp] - b'0');
        cp += 1;
    }
    if cp < s.len() && s[cp] == b'.' {
        let mut dec = 10.0f64;
        cp += 1;
        while cp < s.len() && s[cp].is_ascii_digit() {
            val += f64::from(s[cp] - b'0') / dec;
            dec *= 10.0;
            cp += 1;
        }
    }
    *pos = cp;
    if neg {
        -val
    } else {
        val
    }
}

/// `read_tag_with_param`: parses `TAG` or `TAG=n` (optionally signed);
/// returns `(tag, param)` where `param` is `0` when `=n` is absent (C
/// initializes `param` to 0 and only writes it inside the `=` branch).
fn read_tag_with_param(s: &[u8], pos: &mut usize) -> (u32, i32) {
    let mut cp = *pos;
    while cp < s.len() && s[cp] != b':' && s[cp] != b';' && s[cp] != b',' && s[cp] != b'=' {
        cp += 1;
    }
    let tag = hb_tag_from_string(&s[*pos..cp]);
    let mut param = 0i32;
    if cp < s.len() && s[cp] == b'=' {
        cp += 1;
        let mut neg = false;
        if cp < s.len() && s[cp] == b'-' {
            neg = true;
            cp += 1;
        }
        while cp < s.len() && s[cp].is_ascii_digit() {
            param = param * 10 + i32::from(s[cp] - b'0');
            cp += 1;
        }
        if neg {
            param = -param;
        }
    }
    *pos = cp;
    (tag, param)
}

/// `read_rgb_a`: reads exactly six or eight hexadecimal digits as a packed
/// RGBA value. Returns the value and advances `pos`; the caller checks the
/// consumed length to validate.
pub fn read_rgb_a(s: &[u8], pos: &mut usize) -> u32 {
    let mut cp = *pos;
    let mut value = 0u32;
    while cp < s.len() {
        let digit = match s[cp] {
            c @ b'0'..=b'9' => u32::from(c - b'0'),
            c @ b'A'..=b'F' => u32::from(c - b'A') + 10,
            c @ b'a'..=b'f' => u32::from(c - b'a') + 10,
            _ => break,
        };
        value = (value << 4) | digit;
        cp += 1;
    }
    *pos = cp;
    value
}

/// `readFeatureNumber`: parses a Graphite `id=setting` pair into
/// `(tag, value)`; returns `None` unless the entire `s[..e]` slice is a
/// well-formed `digits=digits` string.
fn read_feature_number(s: &[u8]) -> Option<(u32, i32)> {
    let mut i = 0usize;
    if s.is_empty() || !s[i].is_ascii_digit() {
        return None;
    }
    let mut f = 0u32;
    while i < s.len() && s[i].is_ascii_digit() {
        f = f * 10 + u32::from(s[i] - b'0');
        i += 1;
    }
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    if i >= s.len() || s[i] != b'=' {
        return None;
    }
    i += 1;
    if i >= s.len() || !s[i].is_ascii_digit() {
        return None;
    }
    let mut v = 0i32;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v * 10 + i32::from(s[i] - b'0');
        i += 1;
    }
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    if i != s.len() {
        return None;
    }
    Some((f, v))
}

/// Outcome of `readCommonFeatures` for one `feat..end` option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommonFeature {
    /// Option consumed; advance to the next.
    Consumed,
    /// Malformed common option (`=value` missing or bad color length).
    BadOption,
    /// Not a common option; try feature handling.
    NotCommon,
}

impl TeXEngineState {
    /// `readCommonFeatures`: handles `mapping`, `extend`, `slant`,
    /// `embolden`, `letterspace`, and `color` options within a font feature
    /// string, mutating `spec` and the loaded-font globals like C.
    fn read_common_features(&mut self, feat: &[u8], spec: &mut FontSpec) -> CommonFeature {
        if let Some(tail) = str_starts_with(feat, b"mapping") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let name = String::from_utf8_lossy(&tail[1..]).into_owned();
            spec.mapping = Some(name.clone());
            self.load_mapping_file(&name, false);
            return CommonFeature::Consumed;
        }
        if let Some(tail) = str_starts_with(feat, b"extend") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let mut pos = 1;
            spec.extend = read_double(tail, &mut pos) as f32;
            return CommonFeature::Consumed;
        }
        if let Some(tail) = str_starts_with(feat, b"slant") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let mut pos = 1;
            spec.slant = read_double(tail, &mut pos) as f32;
            return CommonFeature::Consumed;
        }
        if let Some(tail) = str_starts_with(feat, b"embolden") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let mut pos = 1;
            spec.embolden = read_double(tail, &mut pos) as f32;
            return CommonFeature::Consumed;
        }
        if let Some(tail) = str_starts_with(feat, b"letterspace") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let mut pos = 1;
            spec.letterspace = read_double(tail, &mut pos) as f32;
            return CommonFeature::Consumed;
        }
        if let Some(tail) = str_starts_with(feat, b"color") {
            if tail.first() != Some(&b'=') {
                return CommonFeature::BadOption;
            }
            let mut pos = 1;
            let value = read_rgb_a(tail, &mut pos);
            if pos == 7 || pos == 9 {
                spec.rgb = value;
                self.loaded_font_flags |= FONT_FLAGS_COLORED;
            } else {
                return CommonFeature::BadOption;
            }
            return CommonFeature::Consumed;
        }
        CommonFeature::NotCommon
    }

    /// `font_feature_warning`: reports a bad font feature option (and its
    /// optional `=setting` tail) to the transcript.
    pub fn font_feature_warning(&mut self, feature: &[u8], setting: Option<&[u8]>) {
        let name = String::from_utf8_lossy(feature).into_owned();
        let value = setting.map(|s| String::from_utf8_lossy(s).into_owned());
        self.transcript.push(match value {
            Some(v) => format!("! Unknown option `{v}' for feature `{name}'"),
            None => format!("! Unknown font feature `{name}'"),
        });
        self.error_count += 1;
    }

    /// `font_mapping_warning`: reports a `.tec` mapping problem
    /// (`warningType` 0 = tracing confirmation, 1 = not found,
    /// 2 = not loadable).
    pub fn font_mapping_warning(&mut self, name: &[u8], warning_type: i32) {
        let text = String::from_utf8_lossy(name).into_owned();
        self.transcript.push(match warning_type {
            0 => format!("-> mapping file {text}"),
            1 => format!("! Mapping file {text} not found"),
            _ => format!("! Mapping file {text} not loadable"),
        });
        if warning_type != 0 {
            self.error_count += 1;
        }
    }

    /// `load_mapping_file`: resolves a `.tec` mapping name against the
    /// registered `tec_sources` (the virtualized `ttstub_input_open` seam)
    /// and installs the decoded converter in `font_mappings`. Emits the C
    /// diagnostics for missing/unloadable mappings; returns whether a
    /// converter is now registered under `name`.
    pub fn load_mapping_file(&mut self, name: &str, byte_mapping: bool) -> bool {
        if self
            .font_mappings
            .get(name)
            .is_some_and(|m| m.byte_mapping == byte_mapping)
        {
            return true;
        }
        let Some(bytes) = self
            .tec_sources
            .get(name)
            .or_else(|| self.tec_sources.get(&format!("{name}.tec")))
            .cloned()
        else {
            self.font_mapping_warning(name.as_bytes(), 1);
            return false;
        };
        match crate::teckit::Converter::parse(&bytes, byte_mapping) {
            Some(converter) => {
                self.font_mappings.insert(
                    name.to_string(),
                    FontMapping {
                        byte_mapping,
                        converter,
                    },
                );
                if self.int_par(INT_PAR_XETEX_TRACING_FONTS).unwrap_or(0) > 1 {
                    self.font_mapping_warning(name.as_bytes(), 0);
                }
                true
            }
            None => {
                self.font_mapping_warning(name.as_bytes(), 2);
                false
            }
        }
    }

    /// `check_for_tfm_font_mapping`: extracts a trailing `:mapping=name`
    /// suffix from `name_of_file`, stripping it in place and recording the
    /// name in `saved_mapping_name` for `load_tfm_font_mapping`.
    pub fn check_for_tfm_font_mapping(&mut self) {
        self.saved_mapping_name = None;
        let text = String::from_utf8_lossy(&self.name_of_file).into_owned();
        let Some(pos) = text.find(":mapping=") else {
            return;
        };
        let tail = text[pos + 9..].trim_start_matches(|c: char| c <= ' ');
        if !tail.is_empty() {
            self.saved_mapping_name = Some(tail.to_string());
        }
        self.name_of_file.truncate(pos);
        self.name_length = self.name_of_file.len() as i32;
    }

    /// `load_tfm_font_mapping`: loads and clears `saved_mapping_name`,
    /// registering the byte converter under the pending TFM font.
    pub fn load_tfm_font_mapping(&mut self) -> Option<String> {
        let name = self.saved_mapping_name.take()?;
        if self.load_mapping_file(&name, true) {
            Some(name)
        } else {
            None
        }
    }

    /// `apply_tfm_font_mapping`: maps one character through the named
    /// converter; returns `0` when the mapping produces no output (C's
    /// `outUsed < 1` path).
    pub fn apply_tfm_font_mapping(&self, name: &str, c: u16) -> u16 {
        let Some(mapping) = self.font_mappings.get(name) else {
            return c;
        };
        mapping.converter.convert_byte(c)
    }

    /// `apply_mapping` for a named converter: greedy longest-match
    /// substitution over UTF-16 units (the non-byte converter path).
    pub fn apply_named_mapping(&self, name: &str, input: &[u16]) -> Vec<u16> {
        let Some(mapping) = self.font_mappings.get(name) else {
            return input.to_vec();
        };
        mapping.converter.convert_text(input)
    }

    /// `read_font_info`'s TFM half: builds a `FontRecord` from parsed TFM
    /// metrics at requested size `s` (C's `at`/`scaled` convention), and
    /// returns the record plus the scaled per-character metrics the
    /// caller installs into `font_char_metrics` under the new font index.
    ///
    /// The record's `lig_kern` table carries a sentinel at index 0 so
    /// `NON_ADDRESS` stays unambiguous exactly as C's nonzero
    /// `lig_kern_base` does; `LIG_TAG` remainders and `bchar_label` are
    /// stored one-past their raw TFM index.
    pub fn font_record_from_tfm(
        &mut self,
        tfm: &tectonic_fonts::TfmFont,
        s: i32,
    ) -> Result<(FontRecord, Vec<(u32, FontCharMetric)>), TeXMouthError> {
        let size = if s > 0 {
            s
        } else {
            self.xn_over_d(tfm.header.design_size_sp, -s, 1000)
        };
        let scaler = tectonic_fonts::TfmScaler::new(size);
        let mut record = FontRecord {
            dsize: tfm.header.design_size_sp,
            checksum: tfm.header.checksum,
            bc: u32::from(tfm.bc),
            ec: u32::from(tfm.ec),
            hyphen_char: self.int_par(INT_PAR_DEFAULT_HYPHEN_CHAR)?,
            skew_char: self.int_par(INT_PAR_DEFAULT_SKEW_CHAR)?,
            ..FontRecord::default()
        };

        // `font_params`: the slant ratio stays an unscaled FixWord; the
        // dimensions scale by `z` through Knuth's byte decomposition.
        // C pads the parameter count to at least 7 (`font_params[f]`).
        record.params = vec![0, tfm.params.slant.0];
        for w in [
            tfm.params.space,
            tfm.params.space_stretch,
            tfm.params.space_shrink,
            tfm.params.x_height,
            tfm.params.quad,
            tfm.params.extra_space,
        ] {
            record.params.push(w.scale(&scaler));
        }
        for w in &tfm.params.additional {
            record.params.push(w.scale(&scaler));
        }
        while record.params.len() < 8 {
            record.params.push(0);
        }

        // Lig/kern program behind the `lig_kern_base` sentinel: index 0
        // is the `NON_ADDRESS` slot, so raw TFM indices shift by one.
        record.lig_kern.push(LigKernInsn::default());
        for step in &tfm.lig_kern {
            record.lig_kern.push(LigKernInsn {
                skip: step.skip_byte,
                next_char: i32::from(step.next_char),
                op: step.op_byte,
                remainder: i32::from(step.remainder),
            });
        }

        record.kerns = tfm.kerns.iter().map(|k| k.scale(&scaler)).collect();

        for (i, ci) in tfm.char_info.iter().enumerate() {
            let c = u32::from(tfm.bc) + i as u32;
            match ci.tag {
                tectonic_fonts::CharTag::Vanilla => {}
                tectonic_fonts::CharTag::LigKern => {
                    record.char_tags.insert(c, LIG_TAG);
                    record
                        .char_remainders
                        .insert(c, i32::from(ci.remainder) + 1);
                }
                tectonic_fonts::CharTag::List => {
                    record.char_tags.insert(c, LIST_TAG);
                    record.char_remainders.insert(c, i32::from(ci.remainder));
                }
                tectonic_fonts::CharTag::Extensible => {
                    record.char_tags.insert(c, EXT_TAG);
                    record.char_remainders.insert(c, i32::from(ci.remainder));
                }
            }
        }

        for (i, recipe) in tfm.extensibles.iter().enumerate() {
            record.exten_recipes.insert(
                i as i32,
                [
                    i32::from(recipe.rep),
                    i32::from(recipe.top),
                    i32::from(recipe.bot),
                    i32::from(recipe.mid),
                ],
            );
        }

        // Boundary program: every `skip == 255` word is a label and the
        // last one wins (`bch_label`); a label on word 0 names `bchar`.
        let mut bch_label = 32767;
        let mut bchar = 256;
        for (k, step) in tfm.lig_kern.iter().enumerate() {
            if step.skip_byte == 255 {
                bch_label = 256 * i32::from(step.op_byte) + i32::from(step.remainder);
                if k == 0 {
                    bchar = i32::from(step.next_char);
                }
            }
        }
        if (bch_label as usize) < tfm.lig_kern.len() {
            record.bchar_label = bch_label + 1;
        }
        record.bchar = bchar;
        record.false_bchar = bchar;
        if bchar >= i32::from(tfm.bc)
            && bchar <= i32::from(tfm.ec)
            && tfm.char_info(bchar as u8).is_some_and(|ci| ci.exists())
        {
            record.false_bchar = TOO_BIG_CHAR;
        }

        let mut metrics = Vec::new();
        if tfm.bc <= tfm.ec {
            for code in tfm.bc..=tfm.ec {
                if let Some(ci) = tfm.char_info(code) {
                    if ci.exists() {
                        metrics.push((
                            u32::from(code),
                            FontCharMetric {
                                width: tfm.widths[ci.width_index as usize].scale(&scaler),
                                height: tfm.heights[ci.height_index as usize].scale(&scaler),
                                depth: tfm.depths[ci.depth_index as usize].scale(&scaler),
                                italic: tfm.italics[ci.italic_index as usize].scale(&scaler),
                            },
                        ));
                    }
                }
            }
        }

        Ok((record, metrics))
    }

    /// `open_tfm_file` + the TFM half of `read_font_info`: resolves
    /// `<name_of_file>.tfm` through the byte-source registry and injected
    /// input resolver, parses it, and installs the resulting font.
    /// Returns `Some(f)` on success, `Some(0)` when bytes were found but
    /// malformed (C's `bad_tfm` path), and `None` when no TFM bytes
    /// exist so the caller can fall back to `load_native_font`.
    pub fn load_tfm_font(&mut self, s: i32) -> Result<Option<i32>, TeXMouthError> {
        let base = String::from_utf8_lossy(&self.name_of_file).into_owned();
        let tfm_name = format!("{base}.tfm");
        let Some(bytes) = self.input_byte_source_bytes(&tfm_name) else {
            return Ok(None);
        };
        let Ok(tfm) = tectonic_fonts::TfmFont::from_bytes(&bytes) else {
            self.transcript.push(format!(
                "! Font ={base} not loadable: Bad Metric (TFM) file"
            ));
            self.error_count += 1;
            return Ok(Some(0));
        };
        let (mut record, metrics) = self.font_record_from_tfm(&tfm, s)?;
        record.name = self.cur_name.clone();
        record.area = self.cur_area.clone();
        record.ext = self.cur_ext.clone();
        record.size = if s > 0 {
            s
        } else {
            self.xn_over_d(record.dsize, -s, 1000)
        };
        let font = self.font_table.len() as i32;
        self.font_table.push(record);
        self.font_id_cs.push(-1);
        for (ch, metric) in metrics {
            self.set_font_char_metric(font, ch, metric);
        }
        // `font_mapping[f] = load_tfm_font_mapping()`.
        if let Some(name) = self.load_tfm_font_mapping() {
            self.font_table[font as usize].mapping_name = Some(name);
        }
        Ok(Some(font))
    }

    /// `findGraphiteFeature` seam: resolves a named Graphite feature or
    /// `id=setting` pair against the registered `font_engine_values`
    /// (`findGraphiteFeature` results are registered by the embedder).
    fn find_graphite_feature(&self, f: i32, s: &[u8]) -> Option<(u32, i32)> {
        let value = self.font_engine_get(
            f,
            font_query::FIND_GRAPHITE_FEATURE,
            &[hb_tag_from_string(s) as i32],
        );
        if value != 0 {
            Some((hb_tag_from_string(s), value))
        } else {
            None
        }
    }

    /// `loadOTfont`: parses the feature string tail of a font specification
    /// into a [`FontSpec`], performing C's full option scan — `script`,
    /// `language`, `shaper`, the common features, Graphite feature lookup
    /// under `req_engine == 'G'`, `+`/`-` feature toggles, and `vertical` —
    /// and updating `loaded_font_mapping`/`loaded_font_flags`/
    /// `loaded_font_letter_space`/`native_font_type_flag`.
    ///
    /// The engine-creation result is represented by the spec itself plus
    /// `Ok(())`; C's `createLayoutEngine` failure corresponds to no
    /// registered font source and is detected by `find_native_font`
    /// beforehand.
    pub fn load_ot_font(&mut self, font: i32, feat: &[u8], scaled_size: i32) -> FontSpec {
        let mut spec = FontSpec::default();
        let req_engine = self.req_engine;

        if req_engine == b'O' || req_engine == b'G' {
            spec.shapers.push(if req_engine == b'O' {
                b"ot".to_vec()
            } else {
                b"graphite2".to_vec()
            });
        }

        let mut cp1 = 0usize;
        while cp1 < feat.len() {
            if matches!(feat[cp1], b':' | b';' | b',') {
                cp1 += 1;
            }
            while cp1 < feat.len() && matches!(feat[cp1], b' ' | b'\t') {
                cp1 += 1;
            }
            if cp1 >= feat.len() {
                break;
            }
            let mut cp2 = cp1;
            while cp2 < feat.len() && !matches!(feat[cp2], b':' | b';' | b',') {
                cp2 += 1;
            }
            let option = &feat[cp1..cp2];

            if let Some(tail) = str_starts_with(option, b"script") {
                if tail.first() == Some(&b'=') {
                    spec.script = hb_tag_from_string(&tail[1..]);
                    cp1 = cp2;
                    continue;
                }
                self.font_feature_warning(option, None);
                cp1 = cp2;
                continue;
            }
            if let Some(tail) = str_starts_with(option, b"language") {
                if tail.first() == Some(&b'=') {
                    spec.language = tail[1..].to_vec();
                    cp1 = cp2;
                    continue;
                }
                self.font_feature_warning(option, None);
                cp1 = cp2;
                continue;
            }
            if let Some(tail) = str_starts_with(option, b"shaper") {
                if tail.first() == Some(&b'=') {
                    spec.shapers.push(tail[1..].to_vec());
                    cp1 = cp2;
                    continue;
                }
                self.font_feature_warning(option, None);
                cp1 = cp2;
                continue;
            }

            match self.read_common_features(option, &mut spec) {
                CommonFeature::Consumed => {
                    cp1 = cp2;
                    continue;
                }
                CommonFeature::BadOption => {
                    self.font_feature_warning(option, None);
                    cp1 = cp2;
                    continue;
                }
                CommonFeature::NotCommon => {}
            }

            if req_engine == b'G' {
                if let Some((tag, value)) =
                    read_feature_number(option).or_else(|| self.find_graphite_feature(font, option))
                {
                    spec.features.push(FontFeature {
                        tag,
                        value,
                        start: 0,
                        end: u32::MAX,
                    });
                    cp1 = cp2;
                    continue;
                }
            }

            if option.first() == Some(&b'+') {
                let mut pos = 1;
                let (tag, mut param) = read_tag_with_param(option, &mut pos);
                if param >= 0 {
                    param += 1;
                }
                spec.features.push(FontFeature {
                    tag,
                    value: param,
                    start: 0,
                    end: u32::MAX,
                });
                cp1 = cp2;
                continue;
            }
            if option.first() == Some(&b'-') {
                let tag = hb_tag_from_string(&option[1..]);
                spec.features.push(FontFeature {
                    tag,
                    value: 0,
                    start: 0,
                    end: u32::MAX,
                });
                cp1 = cp2;
                continue;
            }

            if str_starts_with(option, b"vertical").is_some() {
                // C trims the option end and compares `cp3 == cp1 + 8`,
                // i.e. the option is exactly "vertical".
                if option == b"vertical" {
                    self.loaded_font_flags |= FONT_FLAGS_VERTICAL;
                    cp1 = cp2;
                    continue;
                }
            }

            self.font_feature_warning(option, None);
            cp1 = cp2;
        }

        if spec.embolden != 0.0 {
            spec.embolden *= fix2d(scaled_size) as f32 / 100.0;
        }
        if spec.letterspace != 0.0 {
            self.loaded_font_letter_space =
                (f64::from(spec.letterspace) / 100.0 * f64::from(scaled_size)) as i32;
        }
        if self.loaded_font_flags & FONT_FLAGS_COLORED == 0 {
            spec.rgb = 0x0000_00FF;
        }
        spec.flags = self.loaded_font_flags;
        spec.mapping.clone_into(&mut self.loaded_font_mapping);
        self.native_font_type_flag = OTGR_FONT_FLAG;
        spec
    }

    /// `find_native_font`: resolves a font specification (`name_of_file`
    /// bytes) against registered `font_sources`, mirroring C's two paths —
    /// `[file[:idx]]` direct file load and platform-name lookup with
    /// `/AAT|/OT|/ICU|/GR` variant-driven `reqEngine` selection — then runs
    /// `loadOTfont` and appends the variant/feature strings to
    /// `name_of_file` like C. Returns the matched source name on success.
    pub fn find_native_font(&mut self, uname: &[u8], scaled_size: i32) -> Option<String> {
        self.loaded_font_mapping = None;
        self.loaded_font_flags = 0;
        self.loaded_font_letter_space = 0;

        let split = split_font_name(uname);
        let name_string = split.name.clone();
        let var_string = split.variant.clone();
        let feat_string = split.features.clone();
        let index = split.index;

        let font = self.font_ptr() + 1;

        if name_string.first() == Some(&b'[') {
            // `[file[:idx]]` form: direct file load, no name search.
            let stem = String::from_utf8_lossy(&name_string[1..]).into_owned();
            if scaled_size < 0 {
                let mut size = scaled_size;
                if let Some(record) = self.font_sources.get(&stem) {
                    let dsize = record.dsize;
                    size = if scaled_size == -1000 {
                        dsize
                    } else {
                        self.xn_over_d(dsize, -scaled_size, 1000)
                    };
                }
                self.find_native_font_scaled = size;
            }
            self.req_engine = 0;
            if var_string.starts_with(b"/AAT") {
                self.req_engine = b'A';
            } else if var_string.starts_with(b"/OT") || var_string.starts_with(b"/ICU") {
                self.req_engine = b'O';
            } else if var_string.starts_with(b"/GR") {
                self.req_engine = b'G';
            }
            let found = self.font_sources.contains_key(&stem);
            if found {
                if let Some(record) = self.font_sources.get(&stem) {
                    self.loaded_font_design_size = record.dsize;
                }
                let mut spec = self.load_ot_font(font, &feat_string, scaled_size);
                spec.face_index = index.max(0) as u32;
                self.pending_font_spec = Some(spec);
                if self.int_par(INT_PAR_XETEX_TRACING_FONTS).unwrap_or(0) > 0 {
                    self.transcript.push(format!("-> {stem}"));
                }
                Some(stem)
            } else {
                None
            }
        } else {
            let name = String::from_utf8_lossy(&name_string).into_owned();
            let variant = String::from_utf8_lossy(&var_string).into_owned();
            let key = if variant.is_empty() {
                name.clone()
            } else {
                format!("{name}/{variant}")
            };
            // C's findFontByName(name, var): exact key, then bare name.
            let resolved = if self.font_sources.contains_key(&key) {
                Some(key.clone())
            } else if self.font_sources.contains_key(&name) {
                Some(name.clone())
            } else {
                None
            };

            if self.int_par(INT_PAR_XETEX_TRACING_FONTS).unwrap_or(0) > 0 {
                if let Some(resolved) = &resolved {
                    self.transcript.push(format!("-> {resolved}"));
                }
            }

            resolved.inspect(|resolved| {
                if let Some(record) = self.font_sources.get(resolved) {
                    self.loaded_font_design_size = record.dsize;
                }
                let size = if scaled_size < 0 {
                    if scaled_size == -1000 {
                        self.loaded_font_design_size
                    } else {
                        self.xn_over_d(self.loaded_font_design_size, -scaled_size, 1000)
                    }
                } else {
                    scaled_size
                };
                self.find_native_font_scaled = size;

                // name_of_file := fullName + "/" + var + ":" + feat
                self.name_of_file = resolved.as_bytes().to_vec();
                if !var_string.is_empty() {
                    self.name_of_file.push(b'/');
                    self.name_of_file.extend_from_slice(&var_string);
                }
                if !feat_string.is_empty() {
                    self.name_of_file.push(b':');
                    self.name_of_file.extend_from_slice(&feat_string);
                }
                self.name_length = self.name_of_file.len() as i32;

                self.req_engine = 0;
                if var_string.starts_with(b"/AAT") {
                    self.req_engine = b'A';
                } else if var_string.starts_with(b"/OT") || var_string.starts_with(b"/ICU") {
                    self.req_engine = b'O';
                } else if var_string.starts_with(b"/GR") {
                    self.req_engine = b'G';
                }

                let spec = self.load_ot_font(font, &feat_string, size);
                self.pending_font_spec = Some(spec);
            })
        }
    }

    /// `release_font_engine`: drops the parsed spec of a native font.
    /// Registered sources have no held engine object, so this only clears
    /// the per-font spec.
    pub fn release_font_engine(&mut self, f: i32) {
        self.font_specs.remove(&f);
    }

    /// `ot_get_font_metrics`: engine-level metrics for a native font —
    /// ascent/descent/x-height/cap-height/slant. Reads the record's base
    /// metrics with C's OS/2-fallback chain: missing x-height falls back to
    /// the `x` glyph's height then `ascent/2`; missing cap-height to `X`'s
    /// height then `ascent`.
    ///
    /// Returns `(ascent, descent, xheight, capheight, slant)` in scaled
    /// points; `descent` here is positive-down like C's local `d`.
    pub fn ot_get_font_metrics(&self, f: i32) -> (i32, i32, i32, i32, i32) {
        let Some(record) = self.font_record(f) else {
            return (0, 0, 0, 0, 0);
        };
        let ascent = record.height_base;
        let descent = -record.depth_base;
        let param = |code: usize| record.params.get(code).copied().unwrap_or(0);
        let slant = param(1);
        let mut xheight = param(5);
        let mut capheight = param(8);
        if xheight == 0 {
            let gid = self.map_char_to_glyph(f, u32::from(b'x'));
            if gid != 0 {
                xheight = self.native_glyph_metric(f, gid as u32).height;
            } else {
                xheight = ascent / 2;
            }
        }
        if capheight == 0 {
            let gid = self.map_char_to_glyph(f, u32::from(b'X'));
            if gid != 0 {
                capheight = self.native_glyph_metric(f, gid as u32).height;
            } else {
                capheight = ascent;
            }
        }
        (ascent, descent, xheight, capheight, slant)
    }

    /// `load_native_font` (xetex0.c): the native branch of `read_font_info`.
    /// Runs `find_native_font`, computes `actual_size`, deduplicates on the
    /// resolved `full_name` + `native_font_type_flag`, then builds the font
    /// record with C's exact parameter layout — slant, space, space
    /// stretch/shrink, x-height, quad, extra space, cap-height, and 56
    /// `get_ot_math_constant` values for OT-MATH fonts (`num_font_dimens =
    /// 65`, else 8).
    pub fn load_native_font(&mut self, s: i32) -> Result<i32, TeXMouthError> {
        let uname = self.name_of_file.clone();
        let Some(source_name) = self.find_native_font(&uname, s) else {
            return Ok(FONT_BASE);
        };
        let pending_spec = self.pending_font_spec.take();

        let actual_size = if s >= 0 {
            s
        } else if s != -1000 {
            self.xn_over_d(self.loaded_font_design_size, -s, 1000)
        } else {
            self.loaded_font_design_size
        };

        // C interns name_of_file into the string pool for the dedup compare;
        // the compact model compares UTF-16 units directly.
        let full_name_units: Vec<i32> = String::from_utf8_lossy(&self.name_of_file)
            .encode_utf16()
            .map(i32::from)
            .collect();

        for f in 1..=self.font_ptr() {
            let matches = self.font_record(f).is_some_and(|r| {
                self.font_area.get(&f).copied().unwrap_or(0) == self.native_font_type_flag
                    && r.name == full_name_units
                    && r.size == actual_size
            });
            if matches {
                return Ok(f);
            }
        }

        let num_font_dimens: usize = if self.native_font_type_flag == OTGR_FONT_FLAG
            && self
                .font_sources
                .get(&source_name)
                .is_some_and(|r| r.has_math_table)
        {
            65
        } else {
            8
        };

        let font = self.font_ptr() + 1;
        let mut record = self
            .font_sources
            .get(&source_name)
            .cloned()
            .unwrap_or_default();
        record.name = full_name_units.clone();
        record.area = Vec::new();
        record.size = actual_size;
        record.dsize = self.loaded_font_design_size;
        record.checksum = 0;
        record.glue = TEX_NULL;
        record.used = false;
        record.hyphen_char = self.int_par(INT_PAR_DEFAULT_HYPHEN_CHAR)?;
        record.skew_char = self.int_par(INT_PAR_DEFAULT_SKEW_CHAR)?;
        record.letter_space = self.loaded_font_letter_space;
        record.font_flags = self.loaded_font_flags;
        record.mapping_name.clone_from(&self.loaded_font_mapping);

        let (ascent, descent) = (record.height_base, -record.depth_base);
        let mut params = vec![0; num_font_dimens + 1];
        // C measures the space glyph through a temp native node.
        let space_w = self
            .get_native_char_wd_for(&record, u32::from(b' '))
            .saturating_add(self.loaded_font_letter_space);
        // `ot_get_font_metrics` slant: font_slant * extend + spec slant.
        let (spec_extend, spec_slant) = pending_spec
            .as_ref()
            .map_or((1.0, 0.0), |sp| (f64::from(sp.extend), f64::from(sp.slant)));
        params[1] =
            d2fix(fix2d(record.params.get(1).copied().unwrap_or(0)) * spec_extend + spec_slant);
        params[2] = space_w;
        params[3] = space_w / 2; // space_stretch
        params[4] = space_w / 3; // space_shrink
        params[5] = {
            let registered = record.params.get(5).copied().unwrap_or(0);
            if registered != 0 {
                registered
            } else {
                let gid = record
                    .char_to_glyph
                    .get(&u32::from(b'x'))
                    .copied()
                    .unwrap_or(0);
                if gid != 0 {
                    record
                        .glyph_metrics
                        .get(&gid)
                        .map_or(ascent / 2, |m| m.height)
                } else {
                    ascent / 2
                }
            }
        };
        params[6] = actual_size; // quad
        params[7] = space_w / 3; // extra_space
        params[8] = {
            let registered = record.params.get(8).copied().unwrap_or(0);
            if registered != 0 {
                registered
            } else {
                let gid = record
                    .char_to_glyph
                    .get(&u32::from(b'X'))
                    .copied()
                    .unwrap_or(0);
                if gid != 0 {
                    record.glyph_metrics.get(&gid).map_or(ascent, |m| m.height)
                } else {
                    ascent
                }
            }
        };
        if num_font_dimens == 65 {
            params[9] = num_font_dimens as i32;
            for k in 0..=55i32 {
                let idx = 10 + k as usize;
                params[idx] = record.math_constants.get(&k).copied().unwrap_or(0);
            }
        }
        record.params = params;
        record.height_base = ascent;
        record.depth_base = -descent;
        record.bc = 0;
        record.ec = 65535;

        self.font_table.push(record);
        self.font_id_cs.push(-1);
        self.font_area.insert(font, self.native_font_type_flag);
        if let Some(spec) = pending_spec {
            if let Some(rec) = self.font_record_mut(font) {
                rec.layout_rgba = spec.rgb;
                rec.layout_extend = d2fix(f64::from(spec.extend));
                rec.layout_slant = d2fix(f64::from(spec.slant));
                rec.layout_embolden = d2fix(f64::from(spec.embolden));
                rec.layout_face_index = spec.face_index;
                rec.font_flags = spec.flags;
            }
            self.font_specs.insert(font, spec);
        }
        Ok(font)
    }

    /// `get_native_char_wd` against an unregistered record — used while a
    /// font is still being built (C's temporary `new_native_character`
    /// measure during `load_native_font`).
    fn get_native_char_wd_for(&self, record: &FontRecord, c: u32) -> i32 {
        let gid = record.char_to_glyph.get(&c).copied().unwrap_or(0);
        record.glyph_metrics.get(&gid).map_or(0, |m| m.width)
    }

    /// `print_glyph_name`: prints a native glyph's PostScript name.
    pub fn print_glyph_name(&mut self, font: i32, gid: u32) -> Result<(), TeXMouthError> {
        let name = self
            .glyph_name(font, gid)
            .map(str::to_string)
            .unwrap_or_default();
        for byte in name.bytes() {
            self.print_char(i32::from(byte))?;
        }
        Ok(())
    }

    /// `real_get_native_word_cp`: the lp/rp code of a native word node's
    /// edge glyph — first glyph for `LEFT_SIDE`, last for `RIGHT_SIDE`.
    pub fn real_get_native_word_cp(&self, p: i32, side: i32) -> i32 {
        let Some(info) = self.native_glyph_info.get(&p) else {
            return 0;
        };
        let Some(loc) = (if side == LEFT_SIDE {
            info.glyphs.first()
        } else {
            info.glyphs.last()
        }) else {
            return 0;
        };
        let f = self
            .word_at(p, 4)
            .map(|w| i32::from(w.b16_s2()))
            .unwrap_or(0);
        self.get_cp_code(f, u32::from(loc.glyph), side == 0)
    }
}

/// `font_engine_values`/`font_engine_named` query codes mirroring the C
/// `ot_font_get*`/`gr_*`/`aat_*` dispatch constants (`XeTeX_*` selector
/// ids). These are the `what` values consumed by `scan.rs`'s
/// `font_engine_get` calls; the real layout-engine implementations live
/// behind the registered-value seam.
pub mod font_query {
    /// `findGraphiteFeature` registration key for named Graphite features.
    pub const FIND_GRAPHITE_FEATURE: i32 = -1;
}

/// `LEFT_SIDE`/`RIGHT_SIDE` selector ids for `real_get_native_word_cp`.
pub const LEFT_SIDE: i32 = 0;
/// `RIGHT_SIDE` selector id for `real_get_native_word_cp`.
pub const RIGHT_SIDE: i32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TeXEngineState {
        TeXEngineState::new(2000)
    }

    /// Minimal byte-form `.tec` mapping (U->B reverse pipeline): 'A' ->
    /// 0x80, everything else -> '?' — same table as the teckit tests.
    fn ub_file() -> Vec<u8> {
        fn w32(v: &mut Vec<u8>, x: u32) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        fn w16(v: &mut Vec<u8>, x: u16) {
            v.extend_from_slice(&x.to_be_bytes());
        }
        let mut f = Vec::new();
        w32(&mut f, 0x714d_6170); // 'qMap'
        w32(&mut f, 0x0003_0000);
        w32(&mut f, 36);
        w32(&mut f, 1); // LHS bytes
        w32(&mut f, 0x0001_0000); // RHS Unicode
        w32(&mut f, 0);
        w32(&mut f, 0);
        w32(&mut f, 1);
        w32(&mut f, 36);
        let mut t = Vec::new();
        w32(&mut t, 0x552d_3e42); // 'U->B'
        w32(&mut t, 0x0001_0000);
        w32(&mut t, 824);
        w32(&mut t, 0);
        w32(&mut t, 48);
        w32(&mut t, 816);
        w32(&mut t, 824);
        w32(&mut t, 824);
        w32(&mut t, 824);
        w32(&mut t, 824);
        t.extend_from_slice(&[0, 0, 0, 1]);
        w32(&mut t, 0x003f);
        t.push(0);
        t.extend_from_slice(&[0xff; 255]);
        let mut cm = vec![0u16; 256];
        cm[0x41] = 1;
        for x in cm {
            w16(&mut t, x);
        }
        t.extend_from_slice(&[0xfd, 0, 0, 0]);
        t.extend_from_slice(&[0x01, 0x80, 0, 0]);
        assert_eq!(t.len(), 824);
        f.extend_from_slice(&t);
        f
    }

    /// Registers a bare native source record with ' ' and 'x'/'X' glyphs.
    fn register_native(state: &mut TeXEngineState, name: &str) {
        let mut record = FontRecord {
            dsize: 655_360, // 10pt
            height_base: 46_000,
            depth_base: -12_000,
            params: vec![0, 0, 0, 0, 0, 0, 0, 0, 0],
            ..FontRecord::default()
        };
        record.char_to_glyph.insert(u32::from(b' '), 3);
        record.char_to_glyph.insert(u32::from(b'x'), 24);
        record.char_to_glyph.insert(u32::from(b'X'), 56);
        record.glyph_metrics.insert(
            3,
            NativeGlyphMetric {
                width: 32_768,
                height: 0,
                depth: 0,
                lsb: 0,
                rsb: 0,
            },
        );
        for (gid, h) in [(24u32, 28_000i32), (56, 45_000)] {
            record.glyph_metrics.insert(
                gid,
                NativeGlyphMetric {
                    width: 35_000,
                    height: h,
                    depth: 0,
                    lsb: 0,
                    rsb: 0,
                },
            );
        }
        state.font_sources.insert(name.to_string(), record);
    }

    #[test]
    fn split_font_name_forms() {
        let s = split_font_name(b"cmr10");
        assert_eq!(s.name, b"cmr10");
        assert!(s.variant.is_empty() && s.features.is_empty());

        // plain variant+features
        let s = split_font_name(b"Some Font/B:+kern");
        assert_eq!(s.name, b"Some Font");
        assert_eq!(s.variant, b"B");
        assert_eq!(s.features, b"+kern");

        let s = split_font_name(b"file.otf:+smcp=2");
        assert_eq!(s.name, b"file.otf");
        assert_eq!(s.features, b"+smcp=2");

        let s = split_font_name(b"[path/to/font.ttf:3]:+liga");
        assert_eq!(s.name, b"[path/to/font.ttf");
        assert_eq!(s.index, 3);
        assert_eq!(s.features, b"+liga");

        // Bracketed path keeps '/' in the variant for the /AAT|OT|GR checks.
        let s = split_font_name(b"[font.ttf]/OT:mapping=tex-text");
        assert_eq!(s.variant, b"/OT");
        assert_eq!(s.features, b"mapping=tex-text");
    }

    #[test]
    fn numeric_parsers() {
        let mut pos = 0;
        assert_eq!(read_double(b"0.25", &mut pos), 0.25);
        assert_eq!(pos, 4);
        pos = 0;
        assert_eq!(read_double(b" -1.5x", &mut pos), -1.5);
        let mut pos = 0;
        assert_eq!(read_rgb_a(b"FF8800AA", &mut pos), 0xFF88_00AA);
        assert_eq!(pos, 8);
        let mut pos = 0;
        let (tag, param) = read_tag_with_param(b"kern=3;", &mut pos);
        assert_eq!(tag, hb_tag_from_string(b"kern"));
        assert_eq!(param, 3);
        let mut pos = 0;
        let (_, param) = read_tag_with_param(b"kern;", &mut pos);
        assert_eq!(param, 0);
        assert_eq!(read_feature_number(b"12=7"), Some((12, 7)));
        assert_eq!(read_feature_number(b"kern=3"), None);
    }

    #[test]
    fn feature_string_parsing() {
        let mut st = state();
        register_native(&mut st, "testfont");
        st.name_of_file =
            b"testfont:slant=0.25;extend=1.2;+kern;-liga;color=FF8800AA;vertical".to_vec();
        let f = st.load_native_font(655_360).unwrap();
        assert_eq!(f, 1);
        let spec = &st.font_specs[&f];
        assert_eq!(spec.slant, 0.25);
        assert_eq!(spec.extend, 1.2);
        assert_eq!(spec.rgb, 0xFF88_00AA);
        assert!(spec.flags & FONT_FLAGS_COLORED != 0);
        assert!(spec.flags & FONT_FLAGS_VERTICAL != 0);
        assert_eq!(spec.features.len(), 2);
        assert_eq!(spec.features[0].tag, hb_tag_from_string(b"kern"));
        assert_eq!(spec.features[0].value, 1); // '+' with no =n -> param 0 +1
        assert_eq!(spec.features[1].value, 0); // '-' disables
        let rec = st.font_record(f).unwrap();
        assert_eq!(rec.font_flags & FONT_FLAGS_COLORED, FONT_FLAGS_COLORED);
        assert_eq!(rec.layout_rgba, 0xFF88_00AA);
    }

    #[test]
    fn native_font_params() {
        let mut st = state();
        register_native(&mut st, "testfont");
        st.name_of_file = b"testfont".to_vec();
        // s = -1000 -> design size.
        let f = st.load_native_font(-1000).unwrap();
        let rec = st.font_record(f).unwrap();
        assert_eq!(rec.size, 655_360);
        assert_eq!(rec.params[2], 32_768); // space_w
        assert_eq!(rec.params[6], 655_360); // quad = actual size
        assert_eq!(rec.params[5], 28_000); // xheight via 'x' glyph
        assert_eq!(rec.params[8], 45_000); // capheight via 'X' glyph
        assert_eq!(st.font_area[&f], OTGR_FONT_FLAG);
        // Dedup: same spec returns the same font.
        st.name_of_file = b"testfont".to_vec();
        assert_eq!(st.load_native_font(-1000).unwrap(), f);
        // Different requested size loads a second record.
        st.name_of_file = b"testfont".to_vec();
        let g = st.load_native_font(327_680).unwrap();
        assert_ne!(g, f);
        assert_eq!(st.font_record(g).unwrap().size, 327_680);
    }

    #[test]
    fn tfm_mapping_suffix() {
        let mut st = state();
        st.tec_sources.insert("testmap".to_string(), ub_file());
        st.name_of_file = b"cmr10:mapping=testmap".to_vec();
        st.check_for_tfm_font_mapping();
        assert_eq!(st.name_of_file, b"cmr10");
        assert_eq!(st.saved_mapping_name.as_deref(), Some("testmap"));
        let name = st.load_tfm_font_mapping();
        assert_eq!(name.as_deref(), Some("testmap"));
        assert_eq!(st.apply_tfm_font_mapping("testmap", u16::from(b'A')), 0x80);
        assert_eq!(st.apply_tfm_font_mapping("testmap", u16::from(b'z')), 0x3f);
        // Missing source warns and yields no converter.
        st.saved_mapping_name = Some("missing".to_string());
        assert!(st.load_tfm_font_mapping().is_none());
    }

    fn cmr10_tfm() -> Vec<u8> {
        std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/assets/cmr10.tfm"),
        )
        .unwrap()
    }

    fn tfm_state() -> TeXEngineState {
        let mut st = state();
        st.name_of_file = b"cmr10".to_vec();
        st.cur_name = b"cmr10".iter().map(|&c| i32::from(c)).collect();
        st
    }

    #[test]
    fn load_tfm_font_installs_parsed_record() {
        let mut st = tfm_state();
        st.register_input_byte_source("cmr10.tfm", cmr10_tfm());
        let font = st.load_tfm_font(-1000).unwrap().unwrap();
        assert_eq!(font, 1);
        let record = st.font_record(font).unwrap();
        assert_eq!(record.dsize, 10 * 65_536);
        assert_eq!(record.size, 10 * 65_536);
        assert_eq!(record.checksum, 0x4BF1_6079);
        assert_eq!(record.bc, 0);
        assert_eq!(record.ec, 127);
        // Sentinel convention: index 0 is the NON_ADDRESS slot, so the
        // nl = 88 program words live at 1..=88.
        assert_eq!(record.lig_kern.len(), 89);
        assert_eq!(record.lig_kern[0], LigKernInsn::default());
        assert_eq!(record.kerns.len(), 10);
        assert_eq!(record.params.len(), 8);
        assert_eq!(record.params[1], 0); // slant is stored unscaled
        assert!(record.params[2] > 0); // space is scaled by the size
                                       // 'f' has a lig/kern program at raw index 2, shifted to 3.
        assert_eq!(
            record.char_tags.get(&u32::from(b'f')),
            Some(&crate::state::LIG_TAG)
        );
        assert_eq!(record.char_remainders.get(&u32::from(b'f')), Some(&3));
        // cmr10 has no boundary program.
        assert_eq!(record.bchar_label, crate::state::NON_ADDRESS);
        assert!(st.char_exists(font, u32::from(b'f')));
        let m = st.font_char_metric(font, u32::from(b'f'));
        assert!(m.width > 0 && m.height > 0);
    }

    #[test]
    fn load_tfm_font_scales_metrics_at_size() {
        let mut st = tfm_state();
        st.register_input_byte_source("cmr10.tfm", cmr10_tfm());
        let font = st.load_tfm_font(-1000).unwrap().unwrap();
        let w10 = st.font_char_metric(font, u32::from(b'f')).width;
        st.name_of_file = b"cmr10".to_vec();
        let big = st.load_tfm_font(20 * 65_536).unwrap().unwrap();
        let w20 = st.font_char_metric(big, u32::from(b'f')).width;
        assert_eq!(w20, 2 * w10);
    }

    #[test]
    fn load_tfm_font_reports_bad_tfm() {
        let mut st = tfm_state();
        st.register_input_byte_source("cmr10.tfm", b"not a tfm".to_vec());
        assert_eq!(st.load_tfm_font(-1000).unwrap(), Some(0));
        assert_eq!(st.error_count, 1);
        assert!(st
            .transcript
            .iter()
            .any(|line| line.contains("Bad Metric (TFM) file")));
    }

    #[test]
    fn load_tfm_font_miss_returns_none() {
        let mut st = tfm_state();
        assert_eq!(st.load_tfm_font(-1000).unwrap(), None);
    }

    #[test]
    fn load_tfm_font_installs_extensible_recipes() {
        let mut st = state();
        st.name_of_file = b"cmex10".to_vec();
        st.cur_name = b"cmex10".iter().map(|&c| i32::from(c)).collect();
        st.register_input_byte_source(
            "cmex10.tfm",
            std::fs::read(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/assets/cmex10.tfm"),
            )
            .unwrap(),
        );
        let font = st.load_tfm_font(-1000).unwrap().unwrap();
        let record = st.font_record(font).unwrap();
        assert_eq!(record.exten_recipes.len(), 28);
        // cmex10 char 12: EXT_TAG recipe 0 = {top:0, mid:0, bot:0, rep:12},
        // stored in C order [rep, top, bot, mid].
        assert_eq!(record.char_tags.get(&12), Some(&crate::state::EXT_TAG));
        assert_eq!(record.char_remainders.get(&12), Some(&0));
        assert_eq!(record.exten_recipes.get(&0), Some(&[12, 0, 0, 0]));
        // np = 13 math parameters plus the unused slot 0.
        assert_eq!(record.params.len(), 14);
        // nl = 0: the sentinel alone keeps NON_ADDRESS unambiguous.
        assert_eq!(record.lig_kern.len(), 1);
        assert_eq!(record.bchar_label, crate::state::NON_ADDRESS);
    }

    #[test]
    fn load_tfm_font_uses_injected_resolver_and_caches() {
        let mut st = tfm_state();
        let bytes = cmr10_tfm();
        st.set_input_resolver(std::sync::Arc::new(move |name| {
            (name == "cmr10.tfm").then(|| bytes.clone())
        }));
        let font = st.load_tfm_font(-1000).unwrap().unwrap();
        assert_eq!(font, 1);
        // The resolver hit was cached under the requested `.tfm` name.
        assert!(st.input_byte_sources.contains_key("cmr10.tfm"));
    }
}
