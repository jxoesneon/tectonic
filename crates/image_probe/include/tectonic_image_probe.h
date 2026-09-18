/* Copyright 2026 the Tectonic Project & FerroTeX Authors
 * Licensed under the MIT License.
 */

#ifndef TECTONIC_IMAGE_PROBE_H
#define TECTONIC_IMAGE_PROBE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Page boundary box types matching XeTeX definitions:
 * 1: CropBox
 * 2: MediaBox
 * 3: BleedBox
 * 4: TrimBox
 * 5: ArtBox
 */
#define TT_PDF_BOX_CROP  1
#define TT_PDF_BOX_MEDIA 2
#define TT_PDF_BOX_BLEED 3
#define TT_PDF_BOX_TRIM  4
#define TT_PDF_BOX_ART   5

/**
 * Probe image dimensions and resolution (PNG, JPEG, BMP, GIF, TIFF).
 *
 * @param path Path to the image file.
 * @param width Output image pixel width.
 * @param height Output image pixel height.
 * @param x_res Output horizontal resolution (DPI).
 * @param y_res Output vertical resolution (DPI).
 * @return 0 on success, non-zero error code on failure.
 */
int32_t tt_probe_image_bbox(
    const char *path,
    double *width,
    double *height,
    double *x_res,
    double *y_res
);

/**
 * Probe total page count of a PDF document.
 *
 * @param path Path to the PDF file.
 * @return Positive page count on success, 0 on failure/unreadable.
 */
int32_t tt_probe_pdf_page_count(const char *path);

/**
 * Probe boundary box coordinates of a PDF page with /Rotate applied.
 * Coordinates are returned in PostScript/PDF points (bp, 1/72 inch).
 *
 * @param path Path to the PDF file.
 * @param page_num 1-based page number.
 * @param box_type Boundary box type (1=Crop, 2=Media, 3=Bleed, 4=Trim, 5=Art).
 * @param x0 Lower-left X coordinate.
 * @param y0 Lower-left Y coordinate.
 * @param x1 Upper-right X coordinate.
 * @param y1 Upper-right Y coordinate.
 * @return 0 on success, non-zero on error.
 */
int32_t tt_probe_pdf_box(
    const char *path,
    int32_t page_num,
    int32_t box_type,
    double *x0,
    double *y0,
    double *x1,
    double *y1
);

/**
 * Probe image dimensions and resolution from an in-memory byte buffer.
 */
int32_t tt_probe_image_bbox_data(
    const uint8_t *data,
    size_t len,
    double *width,
    double *height,
    double *x_res,
    double *y_res
);

/**
 * Probe total page count of a PDF from an in-memory byte buffer.
 */
int32_t tt_probe_pdf_page_count_data(const uint8_t *data, size_t len);

/**
 * Probe boundary box coordinates of a PDF page from an in-memory byte buffer.
 */
int32_t tt_probe_pdf_box_data(
    const uint8_t *data,
    size_t len,
    int32_t page_num,
    int32_t box_type,
    double *x0,
    double *y0,
    double *x1,
    double *y1
);

#ifdef __cplusplus
}
#endif

#endif /* TECTONIC_IMAGE_PROBE_H */
