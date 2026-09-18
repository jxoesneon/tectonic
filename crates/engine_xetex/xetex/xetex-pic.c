/****************************************************************************\
 Part of the XeTeX typesetting system
 Copyright (c) 1994-2008 by SIL International
 Copyright (c) 2009 by Jonathan Kew

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

/*
XeTeX_pic.c
   interface between xetex and graphics files
   only needs to get image dimensions, not actually load/process the file
*/

#include "xetex-core.h"
#include "xetex-xetexd.h"
#include "xetex-ext.h"
#include "tectonic_image_probe.h"


/* load_picture() needs some helper types and functions */

typedef struct {
    float x;
    float y;
} real_point;

typedef struct {
    double a;
    double b;
    double c;
    double d;
    double x;
    double y;
} transform_t;

typedef struct {
    float x;
    float y;
    float wd;
    float ht;
} real_rect;


/* Helper: read an entire open VFS handle into a freshly malloc'd buffer.
 * Returns NULL on failure.  Caller must free(). */
static uint8_t *
slurp_handle (rust_input_handle_t handle, size_t *out_size)
{
    size_t file_size;
    uint8_t *buf;
    ssize_t bytes_read;

    file_size = ttstub_input_get_size(handle);
    if (file_size == 0)
        return NULL;

    buf = (uint8_t *) xmalloc(file_size);
    if (buf == NULL)
        return NULL;

    bytes_read = ttstub_input_read(handle, (char *) buf, file_size);
    if (bytes_read < 0 || (size_t) bytes_read != file_size) {
        free(buf);
        return NULL;
    }

    *out_size = file_size;
    return buf;
}


int
count_pdf_file_pages (void)
{
    rust_input_handle_t handle;
    uint8_t *buf;
    size_t buf_len;
    int32_t pages;

    handle = ttstub_input_open(name_of_file, TTBC_FILE_FORMAT_PICT, 0);
    if (handle == INVALID_HANDLE)
        return 0;

    buf = slurp_handle(handle, &buf_len);
    ttstub_input_close(handle);

    if (buf == NULL)
        return 0;

    pages = tt_probe_pdf_page_count_data(buf, buf_len);
    free(buf);
    return (pages > 0) ? (int) pages : 0;
}


static int
pdf_get_rect (char *filename, rust_input_handle_t handle, int page_num, int pdf_box, real_rect* box)
{
    uint8_t *buf;
    size_t buf_len;
    double x0, y0, x1, y1;
    int32_t total_pages;

    (void) filename; /* VFS paths are not OS paths; we use the open handle */

    buf = slurp_handle(handle, &buf_len);
    if (buf == NULL)
        return -1;

    /* Resolve negative / out-of-range page numbers: clamp to [1, total]. */
    total_pages = tt_probe_pdf_page_count_data(buf, buf_len);
    if (total_pages <= 0) {
        free(buf);
        return -1;
    }

    if (page_num > total_pages)
        page_num = total_pages;
    if (page_num < 0)
        page_num = total_pages + 1 + page_num;
    if (page_num < 1)
        page_num = 1;

    /* TT_PDF_BOX_* constants match xetex-ext.h pdfbox_* 1:1 (1=Crop, 2=Media,
     * 3=Bleed, 4=Trim, 5=Art), so pass pdf_box directly.
     * /Rotate is applied inside tt_probe_pdf_box_data. */
    if (tt_probe_pdf_box_data(buf, buf_len,
                              (int32_t) page_num, (int32_t) pdf_box,
                              &x0, &y0, &x1, &y1) != 0) {
        free(buf);
        return -1;
    }

    free(buf);

    /* Convert from PostScript/PDF points (bp, 1/72 inch) to TeX points
     * (pt, 1/72.27 inch). */
    box->x  = 72.27 / 72.0 * x0;
    box->y  = 72.27 / 72.0 * y0;
    box->wd = 72.27 / 72.0 * (x1 - x0);
    box->ht = 72.27 / 72.0 * (y1 - y0);
    return 0;
}


/* Tectonic customization: use double-precision (see call site for explanation).
 * Reads the already-open VFS handle into a heap buffer and delegates probing
 * to tectonic_image_probe, eliminating the dependency on dpx-*image.h. */
static int
get_image_size_in_inches (rust_input_handle_t handle, double *width, double *height)
{
    size_t file_size;
    uint8_t *buf;
    ssize_t bytes_read;
    double width_px, height_px, x_res, y_res;
    int32_t err;

    *width  = -1;
    *height = -1;

    file_size = ttstub_input_get_size(handle);
    if (file_size == 0)
        return 1;

    buf = (uint8_t *) xmalloc(file_size);
    if (buf == NULL)
        return 1;

    bytes_read = ttstub_input_read(handle, (char *) buf, file_size);
    if (bytes_read < 0 || (size_t) bytes_read != file_size) {
        free(buf);
        return 1;
    }

    err = tt_probe_image_bbox_data(buf, file_size,
                                   &width_px, &height_px, &x_res, &y_res);
    free(buf);

    if (err != 0)
        return 1;

    /* x_res / y_res are in DPI; convert pixel dimensions to inches. */
    *width  = (x_res > 0.0) ? width_px  / x_res : width_px  / 72.0;
    *height = (y_res > 0.0) ? height_px / y_res : height_px / 72.0;
    return 0;
}

/*
  pdfBoxType indicates which pdf bounding box to use (0 for \XeTeXpicfile)
  page indicates which page is wanted (0-based)
  return 0 for success, or non-zero error code for failure
  return full path in *path
  return bounds (tex points) in *bounds
*/
static int
find_pic_file (char **path, real_rect *bounds, int pdfBoxType, int page)
{
    int err = -1;
    rust_input_handle_t handle;

    handle = ttstub_input_open (name_of_file, TTBC_FILE_FORMAT_PICT, 0);
    bounds->x = bounds->y = bounds->wd = bounds->ht = 0.0;

    if (handle == INVALID_HANDLE)
        return 1;

    if (pdfBoxType != 0) {
        /* if cmd was \XeTeXpdffile, use xpdflib to read it */
        err = pdf_get_rect (name_of_file, handle, page, pdfBoxType, bounds);
    } else {
        /* Tectonic customization: if we use single-precision math, we can
         * sometimes get numerical results that vary depending on whether we're
         * compiled in debug or release mode (?!) */
        double wd, ht;
        err = get_image_size_in_inches (handle, &wd, &ht);
        bounds->wd = wd * 72.27;
        bounds->ht = ht * 72.27;
    }

    if (err == 0)
        *path = xstrdup(name_of_file);

    ttstub_input_close (handle);

    return err;
}


static void
transform_point(real_point* p, const transform_t* t)
{
    real_point r;

    r.x = t->a * p->x + t->c * p->y + t->x;
    r.y = t->b * p->x + t->d * p->y + t->y;

    *p = r;
}


static void
make_identity(transform_t* t)
{
    t->a = 1.0;
    t->b = 0.0;
    t->c = 0.0;
    t->d = 1.0;
    t->x = 0.0;
    t->y = 0.0;
}


static void
make_scale(transform_t* t, double xscale, double yscale)
{
    t->a = xscale;
    t->b = 0.0;
    t->c = 0.0;
    t->d = yscale;
    t->x = 0.0;
    t->y = 0.0;
}


static void
make_translation(transform_t* t, double dx, double dy)
{
    t->a = 1.0;
    t->b = 0.0;
    t->c = 0.0;
    t->d = 1.0;
    t->x = dx;
    t->y = dy;
}


static void
make_rotation(transform_t* t, double a)
{
    t->a = cos(a);
    t->b = sin(a);
    t->c = -sin(a);
    t->d = cos(a);
    t->x = 0.0;
    t->y = 0.0;
}


static void
transform_concat(transform_t* t1, const transform_t* t2)
{
    transform_t r;

    r.a = t1->a * t2->a + t1->b * t2->c + 0.0 * t2->x;
    r.b = t1->a * t2->b + t1->b * t2->d + 0.0 * t2->y;
    r.c = t1->c * t2->a + t1->d * t2->c + 0.0 * t2->x;
    r.d = t1->c * t2->b + t1->d * t2->d + 0.0 * t2->y;
    r.x = t1->x * t2->a + t1->y * t2->c + 1.0 * t2->x;
    r.y = t1->x * t2->b + t1->y * t2->d + 1.0 * t2->y;

    *t1 = r;
}


#define SET_POINT(P,X,Y) do { (P).x = (X); (P).y = (Y); } while (0)

void
load_picture(bool is_pdf)
{
    char *pic_path;
    real_rect bounds;
    transform_t t, t2;
    real_point corners[4];
    double x_size_req, y_size_req;
    bool check_keywords;
    double xmin, xmax, ymin, ymax;
    small_number i;
    int32_t page;
    int32_t pdf_box_type;
    int32_t result;
    scan_file_name();
    pack_file_name(cur_name, cur_area, cur_ext);
    pdf_box_type = 0;
    page = 0;
    if (is_pdf) {
        if (scan_keyword("page")) {
            scan_int();
            page = cur_val;
        }
        pdf_box_type = pdfbox_none;
        if (scan_keyword("crop"))
            pdf_box_type = pdfbox_crop;
        else if (scan_keyword("media"))
            pdf_box_type = pdfbox_media;
        else if (scan_keyword("bleed"))
            pdf_box_type = pdfbox_bleed;
        else if (scan_keyword("trim"))
            pdf_box_type = pdfbox_trim;
        else if (scan_keyword("art"))
            pdf_box_type = pdfbox_art;
    }
    if (pdf_box_type == pdfbox_none)
        result = find_pic_file(&pic_path, &bounds, pdfbox_crop, page);
    else
        result = find_pic_file(&pic_path, &bounds, pdf_box_type, page);
    SET_POINT(corners[0], bounds.x, bounds.y);
    SET_POINT(corners[1], corners[0].x, bounds.y + bounds.ht);
    SET_POINT(corners[2], bounds.x + bounds.wd, corners[1].y);
    SET_POINT(corners[3], corners[2].x, corners[0].y);
    x_size_req = 0.0;
    y_size_req = 0.0;
    make_identity(&t);
    check_keywords = true;
    while (check_keywords) {

        if (scan_keyword("scaled")) {
            scan_int();
            if ((x_size_req == 0.0) && (y_size_req == 0.0)) {
                make_scale(&t2, cur_val / ((double)1000.0), cur_val / ((double)1000.0));
                {
                    register int32_t for_end;
                    i = 0;
                    for_end = 3;
                    if (i <= for_end)
                        do
                            transform_point(&corners[i], &t2);
                        while (i++ < for_end);
                }
                transform_concat(&t, &t2);
            }
        } else if (scan_keyword("xscaled")) {
            scan_int();
            if ((x_size_req == 0.0) && (y_size_req == 0.0)) {
                make_scale(&t2, cur_val / ((double)1000.0), 1.0);
                {
                    register int32_t for_end;
                    i = 0;
                    for_end = 3;
                    if (i <= for_end)
                        do
                            transform_point(&corners[i], &t2);
                        while (i++ < for_end);
                }
                transform_concat(&t, &t2);
            }
        } else if (scan_keyword("yscaled")) {
            scan_int();
            if ((x_size_req == 0.0) && (y_size_req == 0.0)) {
                make_scale(&t2, 1.0, cur_val / ((double)1000.0));
                {
                    register int32_t for_end;
                    i = 0;
                    for_end = 3;
                    if (i <= for_end)
                        do
                            transform_point(&corners[i], &t2);
                        while (i++ < for_end);
                }
                transform_concat(&t, &t2);
            }
        } else if (scan_keyword("width")) {
            scan_dimen(false, false, false);
            if (cur_val <= 0) {
                error_here_with_diagnostic("Improper image ");
                print_cstr("size (");
                print_scaled(cur_val);
                print_cstr("pt) will be ignored");
                capture_to_diagnostic(NULL);
                {
                    help_ptr = 2;
                    help_line[1] = "I can't scale images to zero or negative sizes,";
                    help_line[0] = "so I'm ignoring this.";
                }
                error();
            } else
                x_size_req = Fix2D(cur_val);
        } else if (scan_keyword("height")) {
            scan_dimen(false, false, false);
            if (cur_val <= 0) {
                error_here_with_diagnostic("Improper image ");
                print_cstr("size (");
                print_scaled(cur_val);
                print_cstr("pt) will be ignored");
                capture_to_diagnostic(NULL);
                {
                    help_ptr = 2;
                    help_line[1] = "I can't scale images to zero or negative sizes,";
                    help_line[0] = "so I'm ignoring this.";
                }
                error();
            } else
                y_size_req = Fix2D(cur_val);
        } else if (scan_keyword("rotated")) {
            scan_decimal();
            if ((x_size_req != 0.0) || (y_size_req != 0.0)) {
                {
                    xmin = 1000000.0;
                    xmax = -(int32_t) xmin;
                    ymin = xmin;
                    ymax = xmax;
                    {
                        register int32_t for_end;
                        i = 0;
                        for_end = 3;
                        if (i <= for_end)
                            do {
                                if (corners[i].x < xmin)
                                    xmin = corners[i].x;
                                if (corners[i].x > xmax)
                                    xmax = corners[i].x;
                                if (corners[i].y < ymin)
                                    ymin = corners[i].y;
                                if (corners[i].y > ymax)
                                    ymax = corners[i].y;
                            }
                            while (i++ < for_end);
                    }
                }
                if (x_size_req == 0.0) {
                    make_scale(&t2, y_size_req / ((double)(ymax - ymin)),
                               y_size_req / ((double)(ymax - ymin)));
                } else if (y_size_req == 0.0) {
                    make_scale(&t2, x_size_req / ((double)(xmax - xmin)),
                               x_size_req / ((double)(xmax - xmin)));
                } else {

                    make_scale(&t2, x_size_req / ((double)(xmax - xmin)),
                               y_size_req / ((double)(ymax - ymin)));
                }
                {
                    register int32_t for_end;
                    i = 0;
                    for_end = 3;
                    if (i <= for_end)
                        do
                            transform_point(&corners[i], &t2);
                        while (i++ < for_end);
                }
                x_size_req = 0.0;
                y_size_req = 0.0;
                transform_concat(&t, &t2);
            }
            make_rotation(&t2, Fix2D(cur_val) * M_PI / ((double)180.0));
            {
                register int32_t for_end;
                i = 0;
                for_end = 3;
                if (i <= for_end)
                    do
                        transform_point(&corners[i], &t2);
                    while (i++ < for_end);
            }
            {
                xmin = 1000000.0;
                xmax = -(int32_t) xmin;
                ymin = xmin;
                ymax = xmax;
                {
                    register int32_t for_end;
                    i = 0;
                    for_end = 3;
                    if (i <= for_end)
                        do {
                            if (corners[i].x < xmin)
                                xmin = corners[i].x;
                            if (corners[i].x > xmax)
                                xmax = corners[i].x;
                            if (corners[i].y < ymin)
                                ymin = corners[i].y;
                            if (corners[i].y > ymax)
                                ymax = corners[i].y;
                        }
                        while (i++ < for_end);
                }
            }
            SET_POINT(corners[0], xmin, ymin);
            SET_POINT(corners[1], xmin, ymax);
            SET_POINT(corners[2], xmax, ymax);
            SET_POINT(corners[3], xmax, ymin);
            transform_concat(&t, &t2);
        } else
            check_keywords = false;
    }
    if ((x_size_req != 0.0) || (y_size_req != 0.0)) {
        {
            xmin = 1000000.0;
            xmax = -(int32_t) xmin;
            ymin = xmin;
            ymax = xmax;
            {
                register int32_t for_end;
                i = 0;
                for_end = 3;
                if (i <= for_end)
                    do {
                        if (corners[i].x < xmin)
                            xmin = corners[i].x;
                        if (corners[i].x > xmax)
                            xmax = corners[i].x;
                        if (corners[i].y < ymin)
                            ymin = corners[i].y;
                        if (corners[i].y > ymax)
                            ymax = corners[i].y;
                    }
                    while (i++ < for_end);
            }
        }
        if (x_size_req == 0.0) {
            make_scale(&t2, y_size_req / ((double)(ymax - ymin)), y_size_req / ((double)(ymax - ymin)));
        } else if (y_size_req == 0.0) {
            make_scale(&t2, x_size_req / ((double)(xmax - xmin)), x_size_req / ((double)(xmax - xmin)));
        } else {

            make_scale(&t2, x_size_req / ((double)(xmax - xmin)), y_size_req / ((double)(ymax - ymin)));
        }
        {
            register int32_t for_end;
            i = 0;
            for_end = 3;
            if (i <= for_end)
                do
                    transform_point(&corners[i], &t2);
                while (i++ < for_end);
        }
        x_size_req = 0.0;
        y_size_req = 0.0;
        transform_concat(&t, &t2);
    }
    {
        xmin = 1000000.0;
        xmax = -(int32_t) xmin;
        ymin = xmin;
        ymax = xmax;
        {
            register int32_t for_end;
            i = 0;
            for_end = 3;
            if (i <= for_end)
                do {
                    if (corners[i].x < xmin)
                        xmin = corners[i].x;
                    if (corners[i].x > xmax)
                        xmax = corners[i].x;
                    if (corners[i].y < ymin)
                        ymin = corners[i].y;
                    if (corners[i].y > ymax)
                        ymax = corners[i].y;
                }
                while (i++ < for_end);
        }
    }
    make_translation(&t2, -(int32_t) xmin * 72 / ((double)72.27), -(int32_t) ymin * 72 / ((double)72.27));
    transform_concat(&t, &t2);
    if (result == 0) {
        new_whatsit(PIC_NODE,
                    PIC_NODE_SIZE + (strlen(pic_path) + sizeof(memory_word) - 1) / sizeof(memory_word));
        if (is_pdf) {
            mem[cur_list.tail].b16.s0 = PDF_NODE;
        }
        PIC_NODE_path_len(cur_list.tail) = strlen(pic_path);
        mem[cur_list.tail + 4].b16.s0 = page;
        mem[cur_list.tail + 8].b16.s1 = pdf_box_type;
        mem[cur_list.tail + 1].b32.s1 = D2Fix(xmax - xmin);
        mem[cur_list.tail + 3].b32.s1 = D2Fix(ymax - ymin);
        mem[cur_list.tail + 2].b32.s1 = 0;
        mem[cur_list.tail + 5].b32.s0 = D2Fix(t.a);
        mem[cur_list.tail + 5].b32.s1 = D2Fix(t.b);
        mem[cur_list.tail + 6].b32.s0 = D2Fix(t.c);
        mem[cur_list.tail + 6].b32.s1 = D2Fix(t.d);
        mem[cur_list.tail + 7].b32.s0 = D2Fix(t.x);
        mem[cur_list.tail + 7].b32.s1 = D2Fix(t.y);
        memcpy(PIC_NODE_path(cur_list.tail), pic_path, strlen(pic_path));
        free(pic_path);
    } else {

        error_here_with_diagnostic("Unable to load picture or PDF file '");
        print_file_name(cur_name, cur_area, cur_ext);
        print('\'');
        capture_to_diagnostic(NULL);
        if (result == -43) {
            {
                help_ptr = 2;
                help_line[1] = "The requested image couldn't be read because";
                help_line[0] = "the file was not found.";
            }
        } else {

            {
                help_ptr = 2;
                help_line[1] = "The requested image couldn't be read because";
                help_line[0] = "it was not a recognized image format.";
            }
        }
        error();
    }
}
