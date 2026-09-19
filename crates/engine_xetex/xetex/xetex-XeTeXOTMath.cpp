/****************************************************************************\
 Part of the XeTeX typesetting system
 Copyright (c) 1994-2008 by SIL International
 Copyright (c) 2009 by Jonathan Kew
 Copyright (c) 2012-2015 by Khaled Hosny

 SIL Author(s): Jonathan Kew

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE COPYRIGHT HOLDERS BE LIABLE
FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

Except as contained in this notice, the name of the copyright holders
shall not be used in advertising or otherwise to promote the sale,
use or other dealings in this Software without prior written
authorization from the copyright holders.
\****************************************************************************/

#include "xetex-core.h"
#include "xetex-xetexd.h"

#include <algorithm>

#include "tectonic_xetex_layout.h"
#include "xetex-XeTeXOTMath.h"

enum {
    OT_MATH_CONSTANT_SCRIPT_PERCENT_SCALE_DOWN = 0,
    OT_MATH_CONSTANT_SCRIPT_SCRIPT_PERCENT_SCALE_DOWN = 1,
    OT_MATH_CONSTANT_DELIMITED_SUB_FORMULA_MIN_HEIGHT = 2,
    OT_MATH_CONSTANT_DISPLAY_OPERATOR_MIN_HEIGHT = 3,
    OT_MATH_CONSTANT_MATH_LEADING = 4,
    OT_MATH_CONSTANT_AXIS_HEIGHT = 5,
    OT_MATH_CONSTANT_ACCENT_BASE_HEIGHT = 6,
    OT_MATH_CONSTANT_FLATTENED_ACCENT_BASE_HEIGHT = 7,
    OT_MATH_CONSTANT_SUBSCRIPT_SHIFT_DOWN = 8,
    OT_MATH_CONSTANT_SUBSCRIPT_TOP_MAX = 9,
    OT_MATH_CONSTANT_SUBSCRIPT_BASELINE_DROP_MIN = 10,
    OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP = 11,
    OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP_CRAMPED = 12,
    OT_MATH_CONSTANT_SUPERSCRIPT_BOTTOM_MIN = 13,
    OT_MATH_CONSTANT_SUPERSCRIPT_BASELINE_DROP_MAX = 14,
    OT_MATH_CONSTANT_SUB_SUPERSCRIPT_GAP_MIN = 15,
    OT_MATH_CONSTANT_SUPERSCRIPT_BOTTOM_MAX_WITH_SUBSCRIPT = 16,
    OT_MATH_CONSTANT_SPACE_AFTER_SCRIPT = 17,
    OT_MATH_CONSTANT_UPPER_LIMIT_GAP_MIN = 18,
    OT_MATH_CONSTANT_UPPER_LIMIT_BASELINE_RISE_MIN = 19,
    OT_MATH_CONSTANT_LOWER_LIMIT_GAP_MIN = 20,
    OT_MATH_CONSTANT_LOWER_LIMIT_BASELINE_DROP_MIN = 21,
    OT_MATH_CONSTANT_STACK_TOP_SHIFT_UP = 22,
    OT_MATH_CONSTANT_STACK_TOP_DISPLAY_STYLE_SHIFT_UP = 23,
    OT_MATH_CONSTANT_STACK_BOTTOM_SHIFT_DOWN = 24,
    OT_MATH_CONSTANT_STACK_BOTTOM_DISPLAY_STYLE_SHIFT_DOWN = 25,
    OT_MATH_CONSTANT_STACK_GAP_MIN = 26,
    OT_MATH_CONSTANT_STACK_DISPLAY_STYLE_GAP_MIN = 27,
    OT_MATH_CONSTANT_STRETCH_STACK_TOP_SHIFT_UP = 28,
    OT_MATH_CONSTANT_STRETCH_STACK_BOTTOM_SHIFT_DOWN = 29,
    OT_MATH_CONSTANT_STRETCH_STACK_GAP_ABOVE_MIN = 30,
    OT_MATH_CONSTANT_STRETCH_STACK_GAP_BELOW_MIN = 31,
    OT_MATH_CONSTANT_FRACTION_NUMERATOR_SHIFT_UP = 32,
    OT_MATH_CONSTANT_FRACTION_NUMERATOR_DISPLAY_STYLE_SHIFT_UP = 33,
    OT_MATH_CONSTANT_FRACTION_DENOMINATOR_SHIFT_DOWN = 34,
    OT_MATH_CONSTANT_FRACTION_DENOMINATOR_DISPLAY_STYLE_SHIFT_DOWN = 35,
    OT_MATH_CONSTANT_FRACTION_NUMERATOR_GAP_MIN = 36,
    OT_MATH_CONSTANT_FRACTION_NUM_DISPLAY_STYLE_GAP_MIN = 37,
    OT_MATH_CONSTANT_FRACTION_RULE_THICKNESS = 38,
    OT_MATH_CONSTANT_FRACTION_DENOMINATOR_GAP_MIN = 39,
    OT_MATH_CONSTANT_FRACTION_DENOM_DISPLAY_STYLE_GAP_MIN = 40,
    OT_MATH_CONSTANT_SKEWED_FRACTION_HORIZONTAL_GAP = 41,
    OT_MATH_CONSTANT_SKEWED_FRACTION_VERTICAL_GAP = 42,
    OT_MATH_CONSTANT_OVERBAR_VERTICAL_GAP = 43,
    OT_MATH_CONSTANT_OVERBAR_RULE_THICKNESS = 44,
    OT_MATH_CONSTANT_OVERBAR_EXTRA_ASCENDER = 45,
    OT_MATH_CONSTANT_UNDERBAR_VERTICAL_GAP = 46,
    OT_MATH_CONSTANT_UNDERBAR_RULE_THICKNESS = 47,
    OT_MATH_CONSTANT_UNDERBAR_EXTRA_DESCENDER = 48,
    OT_MATH_CONSTANT_RADICAL_VERTICAL_GAP = 49,
    OT_MATH_CONSTANT_RADICAL_DISPLAY_STYLE_VERTICAL_GAP = 50,
    OT_MATH_CONSTANT_RADICAL_RULE_THICKNESS = 51,
    OT_MATH_CONSTANT_RADICAL_EXTRA_ASCENDER = 52,
    OT_MATH_CONSTANT_RADICAL_KERN_BEFORE_DEGREE = 53,
    OT_MATH_CONSTANT_RADICAL_KERN_AFTER_DEGREE = 54,
    OT_MATH_CONSTANT_RADICAL_DEGREE_BOTTOM_RAISE_PERCENT = 55
};

#define HB_OT_MATH_CONSTANT_ACCENT_BASE_HEIGHT OT_MATH_CONSTANT_ACCENT_BASE_HEIGHT
#define HB_OT_MATH_CONSTANT_FRACTION_NUMERATOR_DISPLAY_STYLE_SHIFT_UP OT_MATH_CONSTANT_FRACTION_NUMERATOR_DISPLAY_STYLE_SHIFT_UP
#define HB_OT_MATH_CONSTANT_FRACTION_NUMERATOR_SHIFT_UP OT_MATH_CONSTANT_FRACTION_NUMERATOR_SHIFT_UP
#define HB_OT_MATH_CONSTANT_STACK_TOP_SHIFT_UP OT_MATH_CONSTANT_STACK_TOP_SHIFT_UP
#define HB_OT_MATH_CONSTANT_FRACTION_DENOMINATOR_DISPLAY_STYLE_SHIFT_DOWN OT_MATH_CONSTANT_FRACTION_DENOMINATOR_DISPLAY_STYLE_SHIFT_DOWN
#define HB_OT_MATH_CONSTANT_FRACTION_DENOMINATOR_SHIFT_DOWN OT_MATH_CONSTANT_FRACTION_DENOMINATOR_SHIFT_DOWN
#define HB_OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP
#define HB_OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP_CRAMPED OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP_CRAMPED
#define HB_OT_MATH_CONSTANT_SUBSCRIPT_SHIFT_DOWN OT_MATH_CONSTANT_SUBSCRIPT_SHIFT_DOWN
#define HB_OT_MATH_CONSTANT_SUPERSCRIPT_BASELINE_DROP_MAX OT_MATH_CONSTANT_SUPERSCRIPT_BASELINE_DROP_MAX
#define HB_OT_MATH_CONSTANT_SUBSCRIPT_BASELINE_DROP_MIN OT_MATH_CONSTANT_SUBSCRIPT_BASELINE_DROP_MIN
#define HB_OT_MATH_CONSTANT_DELIMITED_SUB_FORMULA_MIN_HEIGHT OT_MATH_CONSTANT_DELIMITED_SUB_FORMULA_MIN_HEIGHT
#define HB_OT_MATH_CONSTANT_AXIS_HEIGHT OT_MATH_CONSTANT_AXIS_HEIGHT
#define HB_OT_MATH_CONSTANT_FRACTION_RULE_THICKNESS OT_MATH_CONSTANT_FRACTION_RULE_THICKNESS
#define HB_OT_MATH_CONSTANT_UPPER_LIMIT_GAP_MIN OT_MATH_CONSTANT_UPPER_LIMIT_GAP_MIN
#define HB_OT_MATH_CONSTANT_LOWER_LIMIT_GAP_MIN OT_MATH_CONSTANT_LOWER_LIMIT_GAP_MIN
#define HB_OT_MATH_CONSTANT_UPPER_LIMIT_BASELINE_RISE_MIN OT_MATH_CONSTANT_UPPER_LIMIT_BASELINE_RISE_MIN
#define HB_OT_MATH_CONSTANT_LOWER_LIMIT_BASELINE_DROP_MIN OT_MATH_CONSTANT_LOWER_LIMIT_BASELINE_DROP_MIN
#define HB_OT_MATH_CONSTANT_STACK_GAP_MIN OT_MATH_CONSTANT_STACK_GAP_MIN
#define HB_OT_MATH_CONSTANT_SCRIPT_PERCENT_SCALE_DOWN OT_MATH_CONSTANT_SCRIPT_PERCENT_SCALE_DOWN
#define HB_OT_MATH_CONSTANT_SCRIPT_SCRIPT_PERCENT_SCALE_DOWN OT_MATH_CONSTANT_SCRIPT_SCRIPT_PERCENT_SCALE_DOWN
#define HB_OT_MATH_CONSTANT_RADICAL_DEGREE_BOTTOM_RAISE_PERCENT OT_MATH_CONSTANT_RADICAL_DEGREE_BOTTOM_RAISE_PERCENT

#define unknown (-1)

int
get_ot_math_constant(int f, int n)
{
    int32_t rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = ttxl_get_ot_math_constant(font, (uint32_t) n);

        /* scale according to font size, except the ones that are percentages */
        switch (n) {
            case OT_MATH_CONSTANT_SCRIPT_PERCENT_SCALE_DOWN:
            case OT_MATH_CONSTANT_SCRIPT_SCRIPT_PERCENT_SCALE_DOWN:
            case OT_MATH_CONSTANT_RADICAL_DEGREE_BOTTOM_RAISE_PERCENT:
                break;
            default:
                rval = D2Fix(ttxl_font_units_to_points(font, rval));
                break;
        }
    }

    return rval;
}

/* fontdimen IDs for math symbols font (family 2) */
#define math_x_height   5
#define math_quad       6
#define num1            8   /* numerator shift-up in display styles */
#define num2            9   /* numerator shift-up in non-display, non-\.{\\atop} */
#define num3            10  /* numerator shift-up in non-display \.{\\atop} */
#define denom1          11  /* denominator shift-down in display styles */
#define denom2          12  /* denominator shift-down in non-display styles */
#define sup1            13  /* superscript shift-up in uncramped display style */
#define sup2            14  /* superscript shift-up in uncramped non-display */
#define sup3            15  /* superscript shift-up in cramped styles */
#define sub1            16  /* subscript shift-down if superscript is absent */
#define sub2            17  /* subscript shift-down if superscript is present */
#define sup_drop        18  /* superscript baseline below top of large box */
#define sub_drop        19  /* subscript baseline below bottom of large box */
#define delim1          20  /* size of \.{\\atopwithdelims} delimiters */
#define delim2          21  /* size of \.{\\atopwithdelims} delimiters in non-displays */
#define axis_height     22  /* height of fraction lines above the baseline */

const int32_t TeX_sym_to_OT_map[] = {
    unknown,
    unknown,
    unknown,
    unknown,
    unknown,
    HB_OT_MATH_CONSTANT_ACCENT_BASE_HEIGHT, // x-height
    unknown, // quad
    unknown,
    HB_OT_MATH_CONSTANT_FRACTION_NUMERATOR_DISPLAY_STYLE_SHIFT_UP,
    HB_OT_MATH_CONSTANT_FRACTION_NUMERATOR_SHIFT_UP,
    HB_OT_MATH_CONSTANT_STACK_TOP_SHIFT_UP,
    HB_OT_MATH_CONSTANT_FRACTION_DENOMINATOR_DISPLAY_STYLE_SHIFT_DOWN,
    HB_OT_MATH_CONSTANT_FRACTION_DENOMINATOR_SHIFT_DOWN,
    HB_OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP, // ??
    HB_OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP, // ??
    HB_OT_MATH_CONSTANT_SUPERSCRIPT_SHIFT_UP_CRAMPED,
    HB_OT_MATH_CONSTANT_SUBSCRIPT_SHIFT_DOWN, // ??
    HB_OT_MATH_CONSTANT_SUBSCRIPT_SHIFT_DOWN, // ??
    HB_OT_MATH_CONSTANT_SUPERSCRIPT_BASELINE_DROP_MAX, // ??
    HB_OT_MATH_CONSTANT_SUBSCRIPT_BASELINE_DROP_MIN, // ??
    HB_OT_MATH_CONSTANT_DELIMITED_SUB_FORMULA_MIN_HEIGHT,
    unknown, // using quad instead for now
    HB_OT_MATH_CONSTANT_AXIS_HEIGHT
};

int
get_native_mathsy_param(int f, int n)
{
    int rval = 0;

    if (n == math_quad) {
        rval = font_size[f];
    }
    else if (n == delim2) { // XXX not sure what OT parameter we should use here;
                            // for now we use 1.5em, clamped to delim1 height
        rval = std::min<int>(1.5 * font_size[f], get_native_mathsy_param(f, delim1));
    }
    else {
        if (n < (int) (sizeof(TeX_sym_to_OT_map) / sizeof(int32_t))) {
            int32_t ot_index = TeX_sym_to_OT_map[n];
            if (ot_index != unknown)
                rval = get_ot_math_constant(f, (int)ot_index);
        }
    }
//  fprintf(stderr, " math_sy(%d, %d) returns %.3f\n", f, n, Fix2D(rval));

    return rval;
}

/* fontdimen IDs for math extension font (family 3) */
#define default_rule_thickness  8   /* thickness of \.{\\over} bars */
#define big_op_spacing1         9   /* minimum clearance above a displayed op */
#define big_op_spacing2         10  /* minimum clearance below a displayed op */
#define big_op_spacing3         11  /* minimum baselineskip above displayed op */
#define big_op_spacing4         12  /* minimum baselineskip below displayed op */
#define big_op_spacing5         13  /* padding above and below displayed limits */

const int32_t TeX_ext_to_OT_map[] = {
    unknown,
    unknown,
    unknown,
    unknown,
    unknown,
    HB_OT_MATH_CONSTANT_ACCENT_BASE_HEIGHT, // x-height
    unknown, // quad
    unknown,
    HB_OT_MATH_CONSTANT_FRACTION_RULE_THICKNESS, // default_rule_thickness
    HB_OT_MATH_CONSTANT_UPPER_LIMIT_GAP_MIN, // big_op_spacing1
    HB_OT_MATH_CONSTANT_LOWER_LIMIT_GAP_MIN, // big_op_spacing2
    HB_OT_MATH_CONSTANT_UPPER_LIMIT_BASELINE_RISE_MIN, // big_op_spacing3
    HB_OT_MATH_CONSTANT_LOWER_LIMIT_BASELINE_DROP_MIN, // big_op_spacing4
    HB_OT_MATH_CONSTANT_STACK_GAP_MIN // big_op_spacing5
};

int
get_native_mathex_param(int f, int n)
{
    int rval = 0;

    if (n == math_quad)
        rval = font_size[f];
    else {
        if (n < (int) (sizeof(TeX_ext_to_OT_map) / sizeof(int32_t))) {
            int32_t ot_index = TeX_ext_to_OT_map[n];
            if (ot_index != unknown)
                rval = get_ot_math_constant(f, (int)ot_index);
        }
    }
//  fprintf(stderr, " math_ex(%d, %d) returns %.3f\n", f, n, Fix2D(rval));

    return rval;
}

int
get_ot_math_variant(int f, int g, int v, int32_t* adv, int horiz)
{
    uint32_t rval = g;
    *adv = -1;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        int32_t raw_adv = -1;
        rval = ttxl_get_ot_math_variant(font, g, v, &raw_adv, horiz != 0);

        if (raw_adv != -1) {
            *adv = D2Fix(ttxl_font_units_to_points(font, raw_adv));
        }
    }

    return (int) rval;
}


void *
get_ot_assembly_ptr(int f, int g, int horiz)
{
    void *rval = NULL;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = (void *) ttxl_get_ot_assembly_ptr(font, g, horiz != 0);
    }

    return rval;
}


void
free_ot_assembly(GlyphAssembly* a)
{
    ttxl_free_ot_assembly((XeTeXGlyphAssembly *) a);
}


int
get_ot_math_ital_corr(int f, int g)
{
    int32_t rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = ttxl_get_ot_math_ital_corr(font, g);
        rval = D2Fix(ttxl_font_units_to_points(font, rval));
    }

    return rval;
}

int
get_ot_math_accent_pos(int f, int g)
{
    int32_t rval = 0x7fffffff;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = ttxl_get_ot_math_accent_pos(font, g);
        if (rval != 0x7fffffff) {
            rval = D2Fix(ttxl_font_units_to_points(font, rval));
        }
    }

    return rval;
}

int
ot_min_connector_overlap(int f)
{
    int32_t rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = ttxl_ot_min_connector_overlap(font);
        rval = D2Fix(ttxl_font_units_to_points(font, rval));
    }

    return rval;
}

#define HB_OT_MATH_KERN_TOP_RIGHT 0
#define HB_OT_MATH_KERN_TOP_LEFT 1
#define HB_OT_MATH_KERN_BOTTOM_RIGHT 2
#define HB_OT_MATH_KERN_BOTTOM_LEFT 3

static int
getMathKernAt(int f, int g, int height, int side)
{
    int32_t rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = ttxl_get_ot_math_kern(font, g, side, height);
    }

    return rval;
}

static float
glyph_height(int f, int g)
{
    float rval = 0.0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXLayoutEngine engine = (XeTeXLayoutEngine)font_layout_engine[f];
        getGlyphHeightDepth(engine, g, &rval, NULL);
    }

    return rval;
}

static float
glyph_depth(int f, int g)
{
    float rval = 0.0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXLayoutEngine engine = (XeTeXLayoutEngine)font_layout_engine[f];
        getGlyphHeightDepth(engine, g, NULL, &rval);
    }

    return rval;
}

// keep in sync with xetex.web
#define sup_cmd 0
#define sub_cmd 1

int
get_ot_math_kern(int f,  int g,    // Base font and glyph number
                 int sf, int sg,   // Sub/superscript font and glyph number
                 int cmd,          // sup_cmd or sub_cmd
                 int shift_scaled  // TeX scaled points
                 )
{
    int rval = 0;

    // Cf. https://docs.microsoft.com/en-us/typography/opentype/spec/math#mathkerninfo-table

    if (font_area[f] == OTGR_FONT_FLAG && font_area[sf] == OTGR_FONT_FLAG) {
        XeTeXFont font  = getFont((XeTeXLayoutEngine)font_layout_engine[f]);

        XeTeXFont sfont = getFont((XeTeXLayoutEngine)font_layout_engine[sf]);

        // Do calculations in glyph units.

        // The next four lines could be streamlined by having
        // glyph_height() and glyph_depth() return the metrics in glyph
        // units, but that would require tweaking getGlyphHeightDepth(),
        // which is used elsewhere.

        int g_height = ttxl_font_points_to_units(font, glyph_height(f, g));
        int g_depth  = ttxl_font_points_to_units(font, glyph_depth(f, g));

        int sg_height = ttxl_font_points_to_units(sfont, glyph_height(sf, sg));
        int sg_depth  = ttxl_font_points_to_units(sfont, glyph_depth(sf, sg));

        // Convert the shift amount to base glyph units.
        int shift = ttxl_font_points_to_units(font, Fix2D(shift_scaled));

        // Multiply sub/superscript glyph units by scale_factor to make
        // them commensurate with base glyph units.

        float f_size  = ttxl_font_get_point_size(font);
        float sf_size = ttxl_font_get_point_size(sfont);

        assert(f_size != 0.0);

        float scale_factor = sf_size / f_size;

        if (cmd == sup_cmd) { // superscript
            // (1) Calculate the kern at the bottom of the superscript-glyph
            // bounding box.

            // In base glyph units, the bottom of the superscript is at a
            // distance of (shift - scale_factor * sg_depth) above the
            // baseline.

            int kern = getMathKernAt(f, g, shift - scale_factor * sg_depth,
                                    HB_OT_MATH_KERN_TOP_RIGHT);

            // In superscript glyph units, the bottom of the superscript is
            // a distance of sg_depth below the baseline.

            int skern = getMathKernAt(sf, sg, -sg_depth,
                                        HB_OT_MATH_KERN_BOTTOM_LEFT);

            int top_kern = kern + scale_factor * skern;

            // (2) Calculate the kern at the top of the base-glyph bounding
            // box.

            // In base glyph units, the top of the base glyph is a distance
            // of g_height above the baseline.

            kern = getMathKernAt(f, g, g_height, HB_OT_MATH_KERN_TOP_RIGHT);

            // In superscript glyph units, the top of the base glyph is a
            // distance of (g_height - shift)/scale_factor above the
            // baseline.

            skern = getMathKernAt(sf, sg, (g_height - shift) / scale_factor,
                                    HB_OT_MATH_KERN_BOTTOM_LEFT);

            int bot_kern = kern + scale_factor * skern;

            // (3) The spec says "Take the minimum of these two sums," but
            // surely we want the kern that results in the greater
            // separation between the base and the superscript?  That
            // corresponds to the maximum.  (In the case where both kerns
            // are negative, this is the same as the kern with the minimum
            // *absolute* value, which is presumably what the spec means.)

            rval = (top_kern > bot_kern) ? top_kern : bot_kern;
        } else if (cmd == sub_cmd) { // subscript
            // (1) Calculate the kern at the top of the subscript-glyph
            // bounding box.

            // In base glyph units,, the top of the subscript is at a height
            // of scale_factor * sg_height - shift.

            int kern = getMathKernAt(f, g, scale_factor * sg_height - shift,
                                    HB_OT_MATH_KERN_BOTTOM_RIGHT);

            // In subscript glyph units, the top of the subscript-glyph
            // bounding box is sg_height above the baseline.

            int skern = getMathKernAt(sf, sg, sg_height,
                                        HB_OT_MATH_KERN_TOP_LEFT);

            int top_kern = kern + scale_factor * skern;

            // (2) Calculate the kern at the bottom of the base-glyph
            // bounding box.

            // In base glyph units, the bottom of the base-glyph is at
            // g_depth below the baseline:

            kern = getMathKernAt(f, g, -g_depth,
                                HB_OT_MATH_KERN_BOTTOM_RIGHT);

            // In subscript glyph units, the bottom of the base glyph is at
            // a height of shift - g_depth above the baseline, translated
            // into subscript glyph units.

            skern = getMathKernAt(sf, sg, (shift - g_depth) / scale_factor,
                                    HB_OT_MATH_KERN_TOP_LEFT);

            int bot_kern = kern + scale_factor * skern;

            // (3) See above.

            rval = (top_kern > bot_kern) ? top_kern : bot_kern;
        } else {
            assert(0); // we should not reach here
        }

        rval = D2Fix(ttxl_font_units_to_points(font, rval));
    }

    return rval;
}

int
ot_part_count(const GlyphAssembly* a)
{
    return a->count;
}

int
ot_part_glyph(const GlyphAssembly* a, int i)
{
    return a->parts[i].glyph;
}

bool
ot_part_is_extender(const GlyphAssembly* a, int i)
{
    return (a->parts[i].flags & 0x00000001u) != 0;
}

int
ot_part_start_connector(int f, const GlyphAssembly* a, int i)
{
    int rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = D2Fix(ttxl_font_units_to_points(font, a->parts[i].start_connector_length));
    }

    return rval;
}

int
ot_part_end_connector(int f, const GlyphAssembly* a, int i)
{
    int rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = D2Fix(ttxl_font_units_to_points(font, a->parts[i].end_connector_length));
    }

    return rval;
}

int
ot_part_full_advance(int f, const GlyphAssembly* a, int i)
{
    int rval = 0;

    if (font_area[f] == OTGR_FONT_FLAG) {
        XeTeXFont font = getFont((XeTeXLayoutEngine) font_layout_engine[f]);
        rval = D2Fix(ttxl_font_units_to_points(font, a->parts[i].full_advance));
    }

    return rval;
}
